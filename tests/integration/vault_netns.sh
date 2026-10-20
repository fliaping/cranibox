#!/usr/bin/env bash
# Three real node processes, three Linux netns, Noise on the bridge, fake keys.
# --local validates the process fixture without requiring network privileges.
set -euo pipefail
cd "$(dirname "$0")/../.."
if [[ $# -gt 0 && "$1" == --local ]]; then
  CRANIBOX_NETNS_RUN=1 cargo test -p cranibox-node --locked peer::server::multi_node::netns::coordinator -- --ignored --exact --nocapture
  exit
fi
for program in ip tcpdump python3 cargo; do
  command -v "$program" >/dev/null || { echo "SKIP: missing $program" >&2; exit 77; }
done
if [[ $(id -u) != 0 ]]; then
  echo 'SKIP: requires root with CAP_NET_ADMIN and CAP_SYS_ADMIN; use --local for the process fixture.' >&2
  exit 77
fi
prefix="cbx-vault-$$"
bridge="cbxv$$"
artifact_dir="$(mktemp -d /tmp/cbx-vault-pcap.XXXXXX)"
capture_pid=''
cleanup() {
  if [[ -n "$capture_pid" ]]; then kill -INT "$capture_pid" 2>/dev/null || true; wait "$capture_pid" 2>/dev/null || true; fi
  for i in 0 1 2; do ip netns del "$prefix-$i" 2>/dev/null || true; done
  ip link del "$bridge" 2>/dev/null || true
}
trap cleanup EXIT
if ! ip netns add "$prefix-0"; then
  echo 'SKIP: this host does not permit network namespaces.' >&2
  exit 77
fi
ip netns add "$prefix-1"
ip netns add "$prefix-2"
ip link add "$bridge" type bridge
ip addr add 10.224.0.1/24 dev "$bridge"
ip link set "$bridge" up
for i in 0 1 2; do
  host="cv$$"h"$i"
  peer="cv$$"p"$i"
  ip link add "$host" type veth peer name "$peer"
  ip link set "$host" master "$bridge"
  ip link set "$host" up
  ip link set "$peer" netns "$prefix-$i"
  ip -n "$prefix-$i" link set lo up
  ip -n "$prefix-$i" addr add "10.224.0.$((i+2))/24" dev "$peer"
  ip -n "$prefix-$i" link set "$peer" up
done
# Capture mesh only. The mock upstream deliberately receives its fake API key.
tcpdump -U -i "$bridge" -w "$artifact_dir/mesh.pcap" 'tcp portrange 49400-49402' > "$artifact_dir/tcpdump.log" 2>&1 &
capture_pid=$!
CRANIBOX_NETNS_RUN=1 CRANIBOX_NETNS_PREFIX="$prefix" cargo test -p cranibox-node --locked peer::server::multi_node::netns::coordinator -- --ignored --exact --nocapture > "$artifact_dir/test.log" 2>&1
kill -INT "$capture_pid"
wait "$capture_pid"
capture_pid=''
python3 - "$artifact_dir/mesh.pcap" <<'PY'
from pathlib import Path
import sys
data = Path(sys.argv[1]).read_bytes()
assert len(data) > 40, 'empty packet capture'
assert data[:4] in [b'\xd4\xc3\xb2\xa1', b'\xa1\xb2\xc3\xd4'], 'unexpected capture format'
for secret in [b'fake-netns-credential-never-on-mesh', b'private-netns-account-label']:
    assert secret not in data, 'plaintext subscription leaked on the mesh'
for i in range(3):
    for j in range(3):
        if i != j:
            assert f'{i:032x}{j:032x}'.encode() not in data, 'route credential leaked on the mesh'
print('PASS: nonempty mesh capture contains no fake credential, account label, or route secret')
PY
cat "$artifact_dir/test.log"
echo "Review artifacts: $artifact_dir"
