#!/usr/bin/env bash
# WS1 acceptance: iroh default network over the embedded relay, in network namespaces.
#
#   hub  10.93.0.1  `cranibox serve --relay --public-url https://10.93.0.1` behind an
#                   nginx TLS-terminating proxy on :443 (good) and :8444 (no Upgrade, bad)
#   a    10.93.0.2  `cranibox-node peer serve`   } outbound TCP 443 to the hub only;
#   b    10.93.0.3  `cranibox-node peer serve`   } UDP and everything else dropped
#
# Checks: link pairing hub<->a, hub<->b, a<->b (SAS compared on both sides), exec,
# terminal sessions and messages over the relay-only path, idle survival across the
# nginx read timeout, unpaired keys refused, bad-proxy self-check message, a pcap with
# no plaintext, and PathInfo=Direct once UDP is allowed between a and b.
#
# Needs root (netns, iptables), nginx, openssl, jq, tcpdump. Builds nothing:
#   cargo build -p cranibox -p cranibox-node && sudo -E tests/integration/iroh_netns.sh
# Env: BIN_DIR (default target/debug), IDLE_SECS (default 95; 0 skips), KEEP=1 keeps the work dir.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN_DIR=${BIN_DIR:-$ROOT/target/debug}
CRANIBOX=$BIN_DIR/cranibox
NODE=$BIN_DIR/cranibox-node
IDLE_SECS=${IDLE_SECS:-95}
P=cbxw1
NET=10.93.0
HUB_IP=$NET.1
[ "$(id -u)" = 0 ] || { echo "run as root (sudo -E $0)" >&2; exit 2; }
for tool in nginx openssl jq tcpdump ip iptables; do
  command -v "$tool" >/dev/null || [ -x "/usr/sbin/$tool" ] || { echo "missing $tool" >&2; exit 2; }
done
NGINX=$(command -v nginx || echo /usr/sbin/nginx)
[ -x "$CRANIBOX" ] && [ -x "$NODE" ] || { echo "build first: cargo build -p cranibox -p cranibox-node" >&2; exit 2; }

WORK=$(mktemp -d /tmp/cbx-iroh-netns.XXXXXX)
chmod 755 "$WORK"
PIDS=()
cleanup() {
  set +e
  for pid in "${PIDS[@]}"; do kill "$pid" 2>/dev/null; done
  sleep 1
  for pid in "${PIDS[@]}"; do kill -9 "$pid" 2>/dev/null; done
  [ -f "$WORK/nginx.pid" ] && kill "$(cat "$WORK/nginx.pid")" 2>/dev/null
  # `ip netns exec` children can outlive their wrapper: kill whatever is still inside.
  for ns in hub a b lan; do
    pids=$(ip netns pids "$P-$ns" 2>/dev/null)
    [ -n "$pids" ] && kill -9 $pids 2>/dev/null
    ip netns del "$P-$ns" 2>/dev/null
  done
  if [ "${KEEP:-0}" = 1 ] || [ "${FAILED:-0}" = 1 ]; then echo "logs kept in $WORK"; else rm -rf "$WORK"; fi
}
trap cleanup EXIT
FAILED=1
step() { printf '\n== %s\n' "$*"; }
ok() { printf '   ok: %s\n' "$*"; }
die() { echo "FAIL: $*" >&2; for f in "$WORK"/*.log; do echo "--- $f"; tail -n 30 "$f"; done >&2; exit 1; }
in_ns() { local ns=$1; shift; ip netns exec "$P-$ns" "$@"; }

step "namespaces"
for ns in hub a b lan; do ip netns del "$P-$ns" 2>/dev/null || true; ip netns add "$P-$ns"; done
in_ns lan ip link add br0 type bridge
in_ns lan ip link set br0 up
i=1
for ns in hub a b; do
  ip link add "$P-$ns" type veth peer name eth0 netns "$P-$ns"
  ip link set "$P-$ns" netns "$P-lan"
  in_ns lan ip link set "$P-$ns" master br0 up
  in_ns "$ns" ip addr add "$NET.$i/24" dev eth0
  in_ns "$ns" ip link set eth0 up
  in_ns "$ns" ip link set lo up
  i=$((i + 1))
done
# Devices: only TCP 443 to the hub leaves; UDP (hole punching, QAD) and the rest are dropped.
for ns in a b; do
  in_ns "$ns" iptables -A OUTPUT -o lo -j ACCEPT
  in_ns "$ns" iptables -A OUTPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
  in_ns "$ns" iptables -A OUTPUT -p tcp -d "$HUB_IP" --dport 443 -j ACCEPT
  in_ns "$ns" iptables -A OUTPUT -j DROP
  in_ns "$ns" iptables -A INPUT -i lo -j ACCEPT
  in_ns "$ns" iptables -A INPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
  in_ns "$ns" iptables -A INPUT -j DROP
done
in_ns hub iptables -A INPUT -i lo -j ACCEPT
in_ns hub iptables -A INPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
in_ns hub iptables -A INPUT -p tcp --dport 443 -j ACCEPT
in_ns hub iptables -A INPUT -p tcp --dport 8444 -j ACCEPT
in_ns hub iptables -A INPUT -j DROP
ok "hub $HUB_IP, a $NET.2, b $NET.3 (443-only)"

step "TLS proxy"
# A throwaway test CA and a leaf for the hub IP (webpki refuses a CA as end entity).
openssl req -x509 -newkey ed25519 -nodes -days 1 -subj "/CN=cranibox-netns-ca" \
  -keyout "$WORK/ca.key" -out "$WORK/ca.pem" 2>/dev/null
openssl req -newkey ed25519 -nodes -subj "/CN=$HUB_IP" -keyout "$WORK/key.pem" -out "$WORK/leaf.csr" 2>/dev/null
printf 'subjectAltName=IP:%s\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n' "$HUB_IP" >"$WORK/leaf.ext"
openssl x509 -req -in "$WORK/leaf.csr" -CA "$WORK/ca.pem" -CAkey "$WORK/ca.key" -CAcreateserial \
  -days 1 -extfile "$WORK/leaf.ext" -out "$WORK/cert.pem" 2>/dev/null
openssl x509 -in "$WORK/ca.pem" -outform der -out "$WORK/cert.der"
SERVE_PORT=3391
cat >"$WORK/nginx.conf" <<NGINX
worker_processes 1;
pid $WORK/nginx.pid;
error_log $WORK/nginx-error.log info;
events { worker_connections 256; }
http {
  access_log $WORK/nginx-access.log;
  map \$http_upgrade \$connection_upgrade { default upgrade; '' close; }
  server {
    listen $HUB_IP:443 ssl;
    ssl_certificate $WORK/cert.pem;
    ssl_certificate_key $WORK/key.pem;
    location / {
      proxy_pass http://127.0.0.1:$SERVE_PORT;
      proxy_http_version 1.1;
      proxy_set_header Upgrade \$http_upgrade;
      proxy_set_header Connection \$connection_upgrade;
      proxy_set_header Host \$host;
      proxy_buffering off;
      proxy_read_timeout 60s;
    }
  }
  # Typical misconfiguration: no Upgrade forwarding.
  server {
    listen $HUB_IP:8444 ssl;
    ssl_certificate $WORK/cert.pem;
    ssl_certificate_key $WORK/key.pem;
    location / { proxy_pass http://127.0.0.1:$SERVE_PORT; }
  }
}
NGINX
in_ns hub "$NGINX" -c "$WORK/nginx.conf" -e "$WORK/nginx-error.log"
ok "nginx on $HUB_IP:443 (Upgrade) and :8444 (no Upgrade)"

export CRANIBOX_IROH_EXTRA_ROOT=$WORK/cert.der
# No DNS/pkarr in the namespaces: tickets carry the relay URL, lookups only add noise.
export CRANIBOX_IROH_NO_LOOKUP=1
export RUST_LOG=${RUST_LOG:-warn}

step "pcap"
in_ns lan tcpdump -i br0 -U -s 0 -w "$WORK/lan.pcap" >"$WORK/tcpdump.log" 2>&1 &
PIDS+=($!)
sleep 1

step "hub: cranibox serve --relay"
HUB=$WORK/hub
mkdir -p "$HUB/home"
in_ns hub env HOME="$HUB/home" "$CRANIBOX" serve --config-root "$HUB" --port $SERVE_PORT --gateway-port 0 \
  --public-url "https://$HUB_IP" --relay >"$WORK/hub.log" 2>&1 &
PIDS+=($!)
HUB_SETTINGS=$HUB/.config/cranibox-node/settings.json
for _ in $(seq 60); do [ -S "$HUB/.config/cranibox-node/cranibox-net.sock" ] && break; sleep 0.5; done
[ -S "$HUB/.config/cranibox-node/cranibox-net.sock" ] || die "hub iroh service did not start"
for _ in $(seq 30); do grep -q "self-check" "$WORK/hub.log" && break; sleep 0.5; done
grep -q "iroh relay self-check: https://$HUB_IP/relay OK" "$WORK/hub.log" || die "hub self-check did not pass"
ok "relay self-check passes through nginx"

step "self-check through the proxy without Upgrade"
BAD=$WORK/bad
mkdir -p "$BAD/home"
in_ns hub env HOME="$BAD/home" "$CRANIBOX" serve --config-root "$BAD" --port 3392 --gateway-port 0 \
  --public-url "https://$HUB_IP:8444" --relay >"$WORK/bad.log" 2>&1 &
BAD_PID=$!
PIDS+=($BAD_PID)
for _ in $(seq 60); do grep -q "self-check" "$WORK/bad.log" && break; sleep 0.5; done
grep -q "self-check FAILED: The proxy in front of cranibox serve does not forward the WebSocket Upgrade header" "$WORK/bad.log" \
  || die "bad proxy was not diagnosed"
kill "$BAD_PID"
ok "$(grep -o 'self-check FAILED: [^(]*' "$WORK/bad.log" | head -1)"

start_device() {
  local name=$1 dir=$WORK/$1
  mkdir -p "$dir/home"
  in_ns "$name" env HOME="$dir/home" "$NODE" --config "$dir/settings.json" peer serve >"$WORK/$name.log" 2>&1 &
  PIDS+=($!)
  for _ in $(seq 60); do [ -S "$dir/cranibox-net.sock" ] && return 0; sleep 0.5; done
  die "$name iroh service did not start"
}
cli() { # cli <ns> <settings> args...
  local ns=$1 settings=$2; shift 2
  in_ns "$ns" env HOME="$(dirname "$settings")/home" "$NODE" --config "$settings" peer "$@"
}
node_id() { jq -r .node_id "$(dirname "$1")/peers.json"; }

step "devices"
start_device a
start_device b
A_SETTINGS=$WORK/a/settings.json
B_SETTINGS=$WORK/b/settings.json
ok "a and b running"

# pair <inviter-ns> <inviter-settings> <joiner-ns> <joiner-settings>
pair() {
  local ins=$1 iset=$2 jns=$3 jset=$4 invite accepted flow sas flows iflow isas confirm_pid
  invite=$(cli "$ins" "$iset" pair invite) || die "invite failed"
  link=$(jq -r .link <<<"$invite")
  [[ $link == cranibox://pair/* ]] || die "bad link $link"
  accepted=$(cli "$jns" "$jset" pair accept "$link") || die "accept failed"
  flow=$(jq -r .flow_id <<<"$accepted")
  sas=$(jq -r .sas <<<"$accepted")
  flows=$(cli "$ins" "$iset" pair flows)
  iflow=$(jq -r '[.[] | select(.state == "comparing")][0].flow_id' <<<"$flows")
  isas=$(jq -r '[.[] | select(.state == "comparing")][0].sas' <<<"$flows")
  [ "$sas" = "$isas" ] || die "pairing codes differ: $sas vs $isas"
  cli "$ins" "$iset" pair confirm "$iflow" >"$WORK/confirm-$iflow.json" &
  confirm_pid=$!
  cli "$jns" "$jset" pair confirm "$flow" >"$WORK/confirm-$flow.json"
  wait "$confirm_pid" || die "inviter confirmation failed"
  jq -e '.state == "paired"' "$WORK/confirm-$iflow.json" >/dev/null || die "inviter did not pair"
  jq -e '.state == "paired"' "$WORK/confirm-$flow.json" >/dev/null || die "joiner did not pair"
  for _ in $(seq 40); do
    if jq -e --arg n "$(node_id "$iset")" '.routes[$n]' "$(dirname "$jset")/peers.json" >/dev/null 2>&1 \
      && jq -e --arg n "$(node_id "$jset")" '.grants[$n]' "$(dirname "$iset")/peers.json" >/dev/null 2>&1; then
      ok "$ins <-> $jns paired (code $sas, link reused: $(cli "$jns" "$jset" pair accept "$link" >/dev/null 2>&1 && echo yes || echo no))"
      return 0
    fi
    sleep 0.5
  done
  die "pairing $ins -> $jns did not complete"
}

step "pairing by link (SAS compared on both sides)"
pair hub "$HUB_SETTINGS" a "$A_SETTINGS"
pair hub "$HUB_SETTINGS" b "$B_SETTINGS"
pair a "$A_SETTINGS" b "$B_SETTINGS"
HUB_NODE=$(node_id "$HUB_SETTINGS")
A_NODE=$(node_id "$A_SETTINGS")
B_NODE=$(node_id "$B_SETTINGS")

MARK=cbx-plaintext-marker-$RANDOM$RANDOM
retry() { local n=$1; shift; for _ in $(seq "$n"); do "$@" && return 0; sleep 1; done; return 1; }
exec_ok() { # exec_ok <ns> <settings> <node> <expected>
  local out
  out=$(cli "$1" "$2" call --node "$3" exec -- sh -c "echo $4" 2>&1) || return 1
  grep -q "$4" <<<"$out"
}

step "exec over the relay-only path"
retry 20 exec_ok a "$A_SETTINGS" "$HUB_NODE" "$MARK-a-hub" || die "a -> hub exec"
ok "a -> hub"
retry 20 exec_ok b "$B_SETTINGS" "$A_NODE" "$MARK-b-a" || die "b -> a exec"
ok "b -> a (via the hub relay)"
retry 20 exec_ok a "$A_SETTINGS" "$B_NODE" "$MARK-a-b" || die "a -> b exec"
ok "a -> b"
path=$(cli b "$B_SETTINGS" pair status | jq -r --arg n "$A_NODE" '.peers[] | select(.node == $n) | .path.kind')
[ "$path" = relay ] || die "b -> a path is '$path', expected relay"
ok "PathInfo b -> a: relay"

step "terminal sessions"
cli a "$A_SETTINGS" call --node "$B_NODE" sessions create --name "w1-$MARK" >"$WORK/session.json" 2>&1 \
  || die "terminal session create: $(cat "$WORK/session.json")"
cli a "$A_SETTINGS" call --node "$B_NODE" sessions list | grep -q "w1-$MARK" || die "terminal session list"
ok "a created and listed a terminal session on b"

step "messages"
cli a "$A_SETTINGS" messages send --node "$B_NODE" --text "$MARK-message" >/dev/null || die "message send"
retry 20 sh -c "ip netns exec $P-b env HOME=$WORK/b/home $NODE --config $B_SETTINGS peer messages list 2>/dev/null | grep -q $MARK-message" \
  || die "message did not arrive on b"
ok "a -> b message delivered"

step "unpaired keys are refused"
STRANGER=$WORK/stranger
mkdir -p "$STRANGER/home"
in_ns a env HOME="$STRANGER/home" CRANIBOX_IROH_RELAYS="https://$HUB_IP" "$NODE" --config "$STRANGER/settings.json" peer serve >"$WORK/stranger.log" 2>&1 &
STRANGER_PID=$!
PIDS+=($STRANGER_PID)
for _ in $(seq 60); do [ -S "$STRANGER/cranibox-net.sock" ] && break; sleep 0.5; done
sleep 3
status=$(cli a "$STRANGER/settings.json" pair status)
[ "$(jq -r '[.relays[] | select(.[1])] | length' <<<"$status")" = 0 ] || die "unpaired key got a relay session: $status"
kill "$STRANGER_PID"
ok "relay refused the stranger's EndpointId"

if [ "$IDLE_SECS" -gt 0 ]; then
  step "idle ${IDLE_SECS}s (nginx proxy_read_timeout 60s)"
  sleep "$IDLE_SECS"
  retry 10 exec_ok b "$B_SETTINGS" "$A_NODE" "$MARK-idle" || die "exec after idle"
  ok "exec still works after ${IDLE_SECS}s idle"
fi

step "pcap has no plaintext"
sleep 1
kill "${PIDS[0]}" 2>/dev/null || true
sleep 1
for needle in "$MARK" "$(cat "$WORK"/a/iroh-*.token 2>/dev/null | head -c 40)"; do
  [ -n "$needle" ] || continue
  if grep -aq -- "$needle" "$WORK/lan.pcap"; then die "plaintext '$needle' on the wire"; fi
done
[ "$(stat -c %s "$WORK/lan.pcap")" -gt 10000 ] || die "pcap is suspiciously small"
ok "no exec marker, message text or route token in $(stat -c %s "$WORK/lan.pcap") bytes of capture"

step "same LAN: direct path once UDP is allowed"
for ns in a b; do in_ns "$ns" iptables -F; done
direct=""
for _ in $(seq 30); do
  exec_ok b "$B_SETTINGS" "$A_NODE" "$MARK-direct" >/dev/null 2>&1 || true
  direct=$(cli b "$B_SETTINGS" pair status | jq -r --arg n "$A_NODE" '.peers[] | select(.node == $n) | .path.kind')
  [ "$direct" = direct ] && break
  sleep 2
done
[ "$direct" = direct ] || die "path stayed '$direct' with UDP open"
ok "PathInfo b -> a: direct"

FAILED=0
printf '\nALL PASSED\n'
