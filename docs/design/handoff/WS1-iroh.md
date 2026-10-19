# WS1 handoff: iroh default network, embedded relay, link/QR pairing

Branch `ws1/iroh-network` (base `ws0/contracts`, PR #17, **do not merge before #16**). Task brief: `docs/design/tasks/01-iroh-network.md`. User docs: `docs/iroh-network.md`.

## Done
- [x] `IrohNode`: PR #7 ed25519 seed = iroh `SecretKey`; `remote_id` checked on every dial; pooled connections (idle 120 s).
- [x] Relay map: own relays first (QAD off); n0 added after 10 s own-unreachable, removed after own reachable 60 s; hub relay learned at pairing is adopted at runtime (`IrohNode::adopt_relay`).
- [x] ALPN `cranibox/peer-http/1` → `Running::mesh_router()` with `Authenticated{Iroh, e2e: true}` (no Noise in-process); `IrohTransport: PeerTransport` (C2) with streaming bodies.
- [x] ALPN `cranibox/peer-fwd/1`: loopback forwards for CLI/MCP/gateway, spliced to the node's NetworkEndpoint listener (bearer/Noise still required — local users can't borrow the identity).
- [x] Embedded relay in `cranibox serve` (`/relay`, `/ping`, `/generate_204`, same port, before axum); live allowlist from pinned grant/route keys, disconnect on removal; one-time pairing tokens for hub tickets.
- [x] `--relay` / `--no-relay` (default on with https `--public-url`); startup self-check with readable diagnostics (no Upgrade 400, 404, 5xx/530, 200, TLS).
- [x] Pairing: `PairTicket` (postcard+base32) as `cranibox://pair/…`, `https://<hub>/pair#…`, QR SVG; one-time, 15 min, ≤16 open; SAS shown on both sides; mismatch/reject writes nothing; OwnerDevice ↔ grants (all perms incl. llm/vault, inbound Accept), Guest → messages + Hold.
- [x] Back ends `Handle::pair_invite/pair_accept/pair_confirm/pair_flows/status` (C6 types) via `LocalNode::iroh()`; CLI `cranibox-node peer pair invite|accept|confirm|flows|status` over 0600 unix socket `cranibox-net.sock`.
- [x] Route migration to iroh for pinned direct routes — **opt-in** `CRANIBOX_IROH_MIGRATE=1` (changing a route changes its binding and cancels queued messages/tasks).
- [x] Docs `docs/iroh-network.md` (nginx/Caddy/Cloudflare Tunnel, ports, overrides, troubleshooting).
- [x] netns acceptance `tests/integration/iroh_netns.sh` — ALL PASSED locally (see below).

## Left (checklist)
- [ ] CI green on PR #17: last pushed commit `4f701b7`; earlier failure (test_background_project_sync) was the migration probe, fixed in `f6c8682` and verified locally. Re-check runs.
- [ ] Local regression not finished for: test_automatic_task_worker, test_browser_use, test_computer_use, test_desktop_relay/service, test_federated_activity, test_node_identity, test_node_peers, test_project_handoffs/streaming, test_remote_agent_executions, test_remote_desktops, test_rust_runtime, test_space_catalog, test_task_*, test_virtual_spaces, test_workspace_bootstrap/services, test_workspacectl_rust (stopped at wrap-up; CI runs them).
- [ ] Cloudflare Tunnel manual acceptance (steps in PR body; box can't reach cloudflared port 7844).
- [ ] Web pairing page `/pair#…` and UI cards (WS7); roster join for OwnerDevice (WS6, currently grants only).
- [ ] CLI/MCP skip Noise over iroh (needs WS0 client.rs change, see contract request 5).
- [ ] Windows: control socket and `peer pair` CLI are unix-only (`run` bails on non-unix); service itself compiles everywhere.
- [ ] `network/invitations.rs` not converted (pairing lives in transport_iroh); decide whether to fold existing invitations into tickets.
- [ ] Re-merge `origin/ws0/contracts` (`git merge`, never rebase) when #16 updates.

## Build / test
```bash
cargo fmt --all --check
cargo clippy --workspace --locked --all-targets -- -D warnings
cargo test -p cranibox-net --locked                       # relay, allowlist, pairing, peer-http, WS via forward
cargo test -p cranibox-net --locked --test iroh_local -- --ignored   # n0 fallback (needs internet 443)
cargo build -p cranibox -p cranibox-node
sudo -E tests/integration/iroh_netns.sh                   # IDLE_SECS=0 to skip 95 s idle; KEEP=1 keeps logs
CRANIBOX_HERDR_BIN=target/herdr/herdr python3 -m unittest discover -s tests -p test_web_direct_peers.py
```
netns script: hub (`cranibox serve --relay` behind nginx TLS :443, bad proxy :8444) + devices a, b allowed only TCP 443 to hub. Checks pairing hub↔a, hub↔b, a↔b with SAS, exec both ways, PathInfo=relay, terminal sessions, messages, stranger refused by relay, 95 s idle, pcap without plaintext, PathInfo=direct after UDP opened.

Local results (2026-10-19): cargo workspace tests 245 passed/0 failed; netns ALL PASSED; control-center Playwright 147 passed (run inside an own netns because another agent's server holds port 19090); Python: web_direct_peers 16, web_peer_paths 4, device_networks 17, device_messages 15, workspace_web_operations 6, cranibox_terminal 8, gateway_terminal 3, workspace_projects 4, project_catalog_sync 7, background_project_sync 9, background_agent_sync 8, background_activity_sync 8, device_tasks 12, agent_executions 10, collaboration_runs 5, web_host_node 3, agent_runs_mcp 6 — all OK.

## Known issues
- CLI/MCP traffic runs Noise inside iroh (double encryption) until client.rs learns `Endpoint.iroh`.
- Relay must be at the origin root of `--public-url` (iroh dials `<origin>/relay`).
- Relay pairing tokens only for hub-issued tickets; device-to-device invites need both devices already paired with the hub (or n0).
- Forward port changes after restart rewrite the route (binding change) — ports are persisted in `iroh-peers.json` to avoid this, but a taken port forces a new one.
- Netns CI not wired (needs root); run manually.

## File map
| Path | Content |
|---|---|
| `crates/cranibox-net/src/node.rs` | IrohNode, relay map + fallback, pool (`Lane::Peer`/`Forward`) |
| `.../mesh.rs` | server side: allowlist, `MeshTarget::Router` (Authenticated) / `Tcp` (splice) |
| `.../transport.rs` | `IrohTransport` (C2 PeerTransport) |
| `.../forward.rs` | loopback forwards |
| `.../relay.rs` | embedded relay, Gate access control, self_check, diagnose |
| `.../allow.rs`, `addr.rs`, `path.rs` | allowlist, id/relay parsing, PathInfo |
| `.../ticket.rs`, `pair.rs` | PairTicket codec/QR, pairing protocol + PairHost trait |
| `crates/cranibox-net/tests/iroh_local.rs` | integration tests |
| `crates/workspace-node/src/peer/transport_iroh.rs` | node service, Host (peers.json writes), Handle (C6), control socket, `peer pair` CLI |
| `crates/workspace-web/src/relay.rs` | `--relay` setup, serve, self-check |
| `docs/iroh-network.md`, `tests/integration/iroh_netns.sh` | docs, acceptance |
| Flagged touches | `workspace-runtime/.../identity.rs` (`signing_seed`), `workspace-node/src/network/config.rs` (`Spec.iroh`), `peer/mod.rs`, `local.rs`, `workspace-web/src/lib.rs`, Cargo.toml deps |

## Proposed contract changes (CONTRACT-REQUEST; contracts untouched)
1. `ALPN_PAIR = b"cranibox/pair/1"` in `cranibox_contracts::net`.
2. `ALPN_PEER_FWD = b"cranibox/peer-fwd/1"`: same routes, receiver does not inject `Authenticated`.
3. `IrohConfig` key accepted by `network/config.rs` `Spec` (deny_unknown_fields) — done locally as `Spec.iroh: Option<IrohConfig>`.
4. `Identity::signing_seed()` accessor (done locally).
5. Client `Api`: skip Noise for endpoints with `Endpoint.iroh` (WS0 client.rs).
6. Document that relay pairing tokens exist only for hub-issued tickets.

## Decisions for the user
1. Keep Noise-inside-iroh for CLI/MCP until (5), or move them in-process?
2. Relay only at origin root — OK?
3. Route migration opt-in (current) vs default.
4. Owner-device pairing grants llm + vault.
