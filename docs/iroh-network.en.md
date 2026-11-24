# iroh default network: built-in relay and link / QR pairing

> Chinese original: [iroh-network.md](iroh-network.md).
>
> WS1. Code: `crates/cranibox-net`, `crates/workspace-node/src/peer/transport_iroh.rs`,
> `crates/workspace-web/src/relay.rs`. Acceptance: `tests/integration/iroh_netns.sh`.

## 1. What it is

- Every device talks to other devices over iroh 1.3 (QUIC): it tries hole punching first, and uses a relay when that fails. The user interface needs no connection settings.
- The device's iroh identity is the ed25519 signing key in `node-identity.json`, so the iroh `EndpointId` is the `signing_key` pinned at pairing. After dialing, the TLS `remote_id` is checked against that public key; a mismatch closes the connection.
- The hub (`cranibox serve`) embeds a relay on the same port as the control center: `/relay` (WebSocket upgrade), `/ping`, and `/generate_204`. Only paired EndpointIds (the public keys pinned in `peers.json` grants/routes) and devices holding an unexpired pairing link may use it.
- Relay order: own relays first (the hub `--public-url`, a hub relay learned at pairing, and relays in `networks.json`). If every own relay has been unreachable for 10 seconds, the n0 public relays are added as a fallback and removed again after an own relay has been reachable for 60 seconds. QAD is turned off for own relays (there is no UDP when the relay sits behind a TLS reverse proxy or Cloudflare Tunnel).

### ALPN

| ALPN | Use | Auth |
|---|---|---|
| `cranibox/peer-http/1` (C2) | In-process caller (`IrohTransport`, for WS6/WS7). The server handles it with `Running::mesh_router()` and injects `Authenticated{transport: Iroh, e2e: true}` | iroh TLS public key equals the grant's pinned `signing_key`, and `grant.usable()` |
| `cranibox/peer-fwd/1` | Loopback forwarding for out-of-process clients (CLI, MCP, gateway): `127.0.0.1:<port>` is connected through to the peer as-is | The route's own bearer / Noise session is still required (another user on the same machine cannot borrow this node's identity) |
| `cranibox/pair/1` | Pairing handshake (only with the one-time secret) | The 16-byte secret in the link |
| `cranibox/peer-hop/1` | Forwards a `peer-fwd` stream through another of your own devices (see §3.1) | Both the requester and the target must be owner devices in the roster; the forwarder sees only end-to-end ciphertext |

`peer-fwd` and `pair` are not yet in the contract; see the CONTRACT-REQUEST in the PR.

## 2. Deploying the hub

```bash
cranibox serve --public-url https://hub.example.com          # relay on by default (https public URL)
cranibox serve --public-url https://hub.example.com --no-relay  # no relay; devices use n0
cranibox serve --relay                                         # force a relay even without an https public URL (debug only)
```

- `cranibox serve` listens on loopback only (default `127.0.0.1:7377`, or `--listen`). TLS is terminated by a reverse proxy or tunnel in front, which forwards plain HTTP to this port.
- **The relay must be at the root of the public URL's origin:** iroh clients always request `<origin>/relay`. Do not mount Cranibox under a subpath.
- About two seconds after start, a self-check visits `<public-url>/relay` from the server itself and expects `101 Switching Protocols`. Logs:
  - `iroh relay self-check: https://hub.example.com/relay OK (101)`
  - `iroh relay self-check FAILED: …` (with the reason and a fix suggestion; see section 5)

### nginx

```nginx
map $http_upgrade $connection_upgrade { default upgrade; '' close; }
server {
  listen 443 ssl;
  server_name hub.example.com;
  ssl_certificate     /etc/ssl/hub.pem;
  ssl_certificate_key /etc/ssl/hub.key;
  location / {
    proxy_pass http://127.0.0.1:7377;
    proxy_http_version 1.1;                       # required
    proxy_set_header Upgrade $http_upgrade;       # required
    proxy_set_header Connection $connection_upgrade;
    proxy_set_header Host $host;
    proxy_buffering off;
    proxy_read_timeout 60s;                       # the relay has heartbeats; 60 s is enough (idle 95 s observed not to drop)
  }
}
```

Caddy forwards Upgrade by default: `hub.example.com { reverse_proxy 127.0.0.1:7377 }`.

### Cloudflare Tunnel

```yaml
# ~/.cloudflared/config.yml
tunnel: <tunnel-id>
credentials-file: /root/.cloudflared/<tunnel-id>.json
ingress:
  - hostname: hub.example.com
    service: http://127.0.0.1:7377
  - service: http_status:404
```

- Keep **WebSockets enabled** in the zone settings (the default). Do not put Access login in front of `/relay` (devices have no browser session; the relay authenticates by EndpointId itself).
- The `cloudflared` connector needs outbound access to Cloudflare's edge on port 7844 (UDP/QUIC or TCP/HTTP2, `--protocol http2`).

### Ports

| Where | Port |
|---|---|
| Hub, public | TCP 443 (proxy or tunnel); no UDP needs to be open |
| `cranibox serve` | loopback `--listen` (default 7377) |
| Devices | Outbound TCP 443 is enough; when UDP is allowed, hole punching upgrades to a direct path automatically (`bind_port` can pin the UDP port; the default is random) |

A separate relay listener (`--relay-listen`, default `127.0.0.1:7378`) serves only the relay and the read-only identity; the UI stays on loopback.

## 3. Pairing

```bash
cranibox peer pair invite [--guest|--worker] [--qr invite.svg]   # one-time link + QR code (15 minutes, at most 16 unused)
cranibox peer pair accept 'cranibox://pair/…'          # the other device opens the link and shows a pairing code
cranibox peer pair flows                                # the inviter sees the same pairing code
cranibox peer pair confirm <flow_id> [--reject]         # both sides confirm the codes match
cranibox peer pair status                               # EndpointId, relays, and each peer's path (direct/relay, RTT)
```

- The link is `cranibox://pair/<base32>`; the web form `https://<hub>/pair#<base32>` exists on a hub only. The payload is a C2 `PairTicket` (inviter EndpointId, relay URL, 16-byte one-time secret, expiry, and purpose).
- The pairing code (SAS) shown on both sides is derived from the two identities (the same `pairing_code` as in the identity work). Credentials are exchanged and grants saved only after both devices confirm. If only one side has confirmed, it still waits; a reject or timeout writes nothing. The CLI confirm command waits for the other side; run it on each device.
- Purpose decides who issues a grant to whom (A → B means A may use B):
  - `OwnerDevice` (default, “peer device”): each side issues a grant to the other (every capability, including llm/vault), inbound messages are accepted, and the device joins the roster. This device ⇄ the new device.
  - `Worker` (`--worker`, “controlled device”): only the joining device issues a grant to the inviter (messages, exec, terminals, projects). The inviter controls it; it gets none of the inviter's credentials and cannot see the device list. This device → the new device. Use this for a node you only partly trust to run tasks.
  - `Guest` (`--guest`, “guest”): only the inviter issues a grant to the joiner (messages). Inbound messages wait for review; other capabilities are allowed one by one under Access. The new device → this device.
- Each device records the relationship from pairing (peer / controlled / controller / guest / host) in `devices.json`. `peers.json` is unchanged (signed grants and `deny_unknown_fields`, so you can roll back). Owner devices exchange each other's non-owner relationships (`relations.json`, with the same count, size and clock-skew limits as path records). The control center “device network” graph is drawn from that.
- After confirm: the grant pins the peer's identity and marks it checked; the route points at the local loopback forward `http://127.0.0.1:<port>` (with a lease lock), and older paths stay as fallbacks; the hub relay is written to `iroh-peers.json` and used immediately.
- For peers that already have a direct route (pinned public key): with `CRANIBOX_IROH_MIGRATE=1`, the service probes every 10 minutes and, when the peer can use iroh, inserts the iroh loopback forward at the front of the route. This is off by default: changing the route changes the route binding and abandons queued messages and tasks (re-pairing goes to iroh directly).

### 3.1 Devices as routers

When one of your own devices is only reachable over a slow relay (or is briefly unreachable) and another of your own devices can reach it directly, this device's loopback forward to it goes through that device: A → C → B.

- Forwarding is only between owner devices in the roster; guests, controlled devices and devices of another identity neither forward nor are forwarded. Forwarder C only splices two QUIC streams; the inner session is still A's and B's end-to-end Noise session, so C cannot see the content.
- Only one hop: C reaches B directly, never through a further device. Each requester may have at most 64 concurrent streams, and the device as a whole at most 256.
- Path selection (`peer/routing.rs`): a direct path always wins when it exists. Otherwise the estimate is “direct RTT from here to C + C's measured direct RTT to B (within 2 minutes) + 2 ms”. A hop is used only when it is at least 10 ms faster than the relay and no more than 75% of the relay's RTT. If nothing is reachable, it waits 20 seconds for a direct attempt, then uses any available hop. An in-use hop is replaced only when another is clearly better.
- While a hop is in use, the direct path is retried every 60 seconds and wins as soon as hole punching succeeds. If a single streamed hop fails, that stream falls back to direct.
- Settings live in `routing.json` (`use_devices`, `forward_for_devices`, both on by default). Control center “Network settings → Forward through devices” can turn each off independently. The App API is `GET/PUT /api/v1/app/network/routing` and takes effect immediately.
- The path is shown as `relay`, with `via` of `device:<node>`, written in the UI as “via Mac mini”.

## 4. Advanced overrides (not in the UI)

`networks.json` (the same file as Tailscale/EasyTier):

```json
{ "iroh": { "enabled": true, "relays": ["https://relay.example.com"], "n0_fallback": true, "bind_port": 0 } }
```

- `enabled: false` turns iroh off (other transports are unaffected). A non-empty `relays` list stops automatically using a relay learned at pairing. `n0_fallback: false` uses only own relays.

Environment variables (tests, private CAs):

| Variable | Effect |
|---|---|
| `CRANIBOX_IROH_RELAYS=https://a,https://b` | Override the relay list |
| `CRANIBOX_IROH_EXTRA_ROOT=/path/ca.der` | Trust an extra CA (DER) for a self-signed relay; the self-check uses it too |
| `CRANIBOX_IROH_RELAY_ONLY=1` | Do not open UDP; relay only |
| `CRANIBOX_IROH_NO_LOOKUP=1` | Disable DNS/pkarr address discovery (offline environments) |
| `CRANIBOX_IROH_MIGRATE=1` | Prefer iroh on direct routes that already have a pinned public key |

## 5. Troubleshooting

| Symptom | Cause and fix |
|---|---|
| Self-check `expected HTTP 101, got 400` / the relay keeps reconnecting in the log | The proxy is not forwarding `Upgrade`. For nginx, add `proxy_http_version 1.1` and the `Upgrade`/`Connection` headers; enable WebSockets on Cloudflare |
| Self-check `HTTP 404` | The public URL does not point at the root of `cranibox serve`, or serve was started with `--no-relay` |
| Self-check `HTTP 502/503/504/530` | The proxy or tunnel cannot reach serve (`cloudflared` not running, wrong port, outbound 7844 blocked) |
| Self-check `HTTP 200` | The proxy is buffering or rewriting the WebSocket request |
| Self-check unreachable / certificate error | DNS or the certificate chain; for a self-signed certificate use `CRANIBOX_IROH_EXTRA_ROOT` |
| `peer pair accept`: The other device did not answer | The inviter is offline, or neither side can reach the relay in the link; check `relays` in `peer pair status` |
| `pair status` shows relay as `false` | This device is not paired to that hub (the relay rejects unpaired EndpointIds), or the network cannot reach it |
| The path stays `relay` | Normal when only 443 is open; allowing UDP upgrades it to `direct` automatically |

## Configure this device as a relay in the UI

In the control center, open the device's network settings, turn on “Use this device as a relay”, set the public HTTPS origin and the separate listen address (default `127.0.0.1:7378`), then save and restart Cranibox. Expose that entry through an HTTPS reverse proxy or tunnel that forwards WebSockets first; the UI spells out that requirement and that a restart is pending. The separate relay port does not serve the management UI. For configuration and start-up priority, see [device networks](device-networks.md).
