# Node identity and end-to-end encryption

> Chinese original: [node-identity-and-e2e.md](node-identity-and-e2e.md).

Cranibox treats a node as the agent's "skull": agents such as Codex or Claude Code are the brain, and the node is responsible for identity, authorization, human approval, and secure transport between nodes. This document covers node identity, credential storage and signed records (phase 1), and the end-to-end encrypted channel, key pinning and pairing-code confirmation between nodes (phase 2), including design, migration and the threat model.

## Node identity

- Every node has a persistent Ed25519 signing key and a separate X25519 Noise static key. Private keys live in `node-identity.json` next to `peers.json` (other files may be named `<name>.identity`), mode `0600` on Unix. Keys are generated once. After a config migration the file's version checkpoint is updated atomically, keeping the original keys and creation time.
- The public identity `PublicIdentity {version, node_id, signing_key, noise_key, signature}` is a signature over the binding of the NodeID and both public keys. Changing any field fails verification.
- A new node's NodeID is derived from the public key: `node-` plus the hex of the first 16 bytes of `SHA-256("cranibox-node-id-v1\0" ‖ public key)`. It has the same shape as the older random `node-<32 hex>`, so existing NodeID checks in the gateway, web UI and tests do not need to change.
- Existing nodes keep their original NodeID. On first run they generate keys and bind them with a self-signature (`key_derived_id: false`). Other nodes pin that public key on first contact.
- When a signed config is read, the public key must match this machine's private key file. A missing private key, an identity mismatch, or a Noise public/private mismatch is an error: the original file is kept, and no replacement key is generated automatically.
- `cranibox peer identity` shows the public keys, a fingerprint such as `abcd-efgh-ijkl-mnop`, and whether the ID is derived. It never prints a private key or a credential.

## Credential digests and migration

- Grant records store only `salt` and `credential = SHA-256(salt ‖ token)`. Verification uses a constant-time comparison. New grants use a 16-byte random salt.
- When a version 1 `peers.json` (plaintext `token`) is read, the token is replaced in memory with a digest immediately. The next write (which happens when the node starts) leaves no plaintext on disk.
- Compatibility: a migrated grant uses an empty salt, so the digest equals the `SHA-256(token)` binding that existing task, execution and project-copy ledgers already store. Unit tests cover digest compatibility, and isolated-node regressions show that an old token can still run commands after migration. Historical ledgers produced by an older binary have not been verified class by class. To get a salted digest, revoke and grant again (this also rotates the token).
- Token files kept by callers are unchanged; outbound routes still read the same `token_file`.

## Signed records

- When the config is written, the target node signs each grant with its Ed25519 private key (covering the caller NodeID, permissions, digest and every other field), writes its own `identity` into the config, and raises `version` to 2.
- In a version 2 config, an unsigned grant, an invalid signature, or a removed identity field makes the config fail to load (existing data is kept, not auto-repaired). That stops a privileges escalation that only edits `peers.json`.
- Verification is anchored to this machine's private key file; swapping in another valid self-signed identity and grants does not bypass it. After a version 2 config is saved successfully, the private key file records `minimum_config_version: 2` and refuses to read a version 1 config used to skip signatures.
- The signature covers grant records, not every other config field (routes, target public-key pins, listeners, and so on). The version checkpoint also does not stop replay of an older, still-valid signed version 2 config.
- Signatures only defend against an attacker who can write `peers.json` but cannot read `node-identity.json`. An attacker who has the private key file can still forge records.

## Compatibility and rollback

- Before upgrading, back up `peers.json` and any existing `node-identity.json`, and keep token files and ledgers safe. The first start after the upgrade writes the config as version 2. An older binary cannot read the new config or a key file with the new checkpoint (`deny_unknown_fields`), so **direct downgrade is not supported**. Restore from matching backups taken at the same time; do not only delete the key file or edit the config version.
- NodeIDs and caller token files are preserved. Config migration, restart, execution with an old token, and restoring the original keys are already covered by isolated-node tests. Full historical ledgers still need the extra checks described above.
- Migration writes the signed config first, then the version checkpoint in the private key file. If it is interrupted after the first key is generated and before the config is saved, the next start can finish the migration with the original keys. A first-edition identity file without a checkpoint is read as version 1 and completed on the next successful config write. The two files are not one atomic transaction. If it is interrupted after the config is saved and before the checkpoint is saved, downgrade protection is not finished yet: start again and confirm both writes succeeded.
- The response shape of `/api/v1/mesh/identity` is unchanged, so older clients are unaffected.

## Threat model (phase 1)

| Threat | Handling |
| --- | --- |
| `peers.json` leaks from disk | It holds only salted digests, which cannot be used directly. Migrated grants are unsalted SHA-256; the token itself is a 256-bit random value and cannot be brute-forced |
| Tampering with grants in `peers.json`, replacing the self-signed identity, or downgrading the version | The local private-key anchor, grant signatures and the saved version checkpoint refuse the read. This does not cover every config field (including routes) and does not prevent replay of an older valid signed record |
| Lost private key, inconsistent Noise keys, or identity/config mismatch | An error is raised and the original data is kept; a matching backup must be restored. Peers that already pinned the public key will also refuse a replaced identity |
| Local private key leaked | Out of scope; revoke every grant and pair again |
| Network eavesdropping / a relay reading content | See phase 2 below |

## End-to-end encrypted channel (phase 2)

Mesh API calls between modern direct nodes can use a transport-independent encrypted channel. It covers commands, jobs, terminal management, messages, tasks, the project catalog and file copies, HTTP calls for Computer/Browser/Desktop, and the space catalog. A VPN or relay that carries these sealed requests sees only ciphertext and the outer routing path. Legacy target fallback, compatible bearer requests and older gateway streams are not covered by that guarantee; do not read this section as "every entry point is already mandatory encryption".

- **Protocol:** `Noise_IK_25519_ChaChaPoly_BLAKE2s`, implemented by the audited `snow` crate, with no home-grown primitives. The caller already knows the target's Noise public key (from the signed `PublicIdentity`). Handshake message 1 carries the caller identity, the bearer credential issued by the target, and a timestamp (±300 seconds). The credential appears only inside the encrypted handshake, never as a plaintext `Authorization` header.
- **Sessions:** each handshake creates fresh ephemeral keys, giving forward secrecy per session. A session lasts 1 hour, or 10 minutes idle, with at most 512 sessions per node. If the target restarts or the session is no longer valid it returns 410, and the client handshakes again once (the target has not decrypted, and the request was not executed).
- **Framing:** `POST /api/v1/mesh/nodes/{node}/e2e/call`, with an explicit 64-bit nonce and a 4096-message sliding replay window. Large requests are chunked (at most 65,000 bytes of plaintext per chunk, with an end marker) and bound to the request nonce, which prevents truncation, reordering and splicing across requests. An inner request may only target ordinary routes under `/api/v1/mesh/`, never a nested `/e2e/`.
- **Authorization re-check:** every encrypted request re-reads the grant. After a revoke or rotation the old session fails immediately (410, then a new handshake with the current credential).
- **WebSocket:** upgrade requests for terminals, desktops and the browser use `x-cranibox-e2e-auth: <session>.<proof>`. The proof is bound to the request path, is single-use and time-limited, and no bearer is sent. The proof also carries a fresh 32-byte stream secret generated by the caller (sealed by the session, so a relay cannot read it). After the upgrade, every text or binary data frame is encrypted with ChaCha20-Poly1305: each direction uses a key derived from the stream secret and the request path, with an implicit per-direction counter. Tampering, replay, a dropped frame, reordering or reflection fails authentication and closes the connection; a plaintext data frame on the end-to-end stream also closes it. Ping/Pong/Close control frames carry no application data and are not encrypted. A Close on the encrypted stream does not carry a reason string. Older targets and gateway streams that still use a bearer are unchanged.
- **Public endpoints:** `GET /api/v1/mesh/keys` (requires `x-cranibox-target`) returns the signed public identity. The handshake endpoint does not need a bearer.

### Key pinning and pairing codes

- On first connect the caller pins the target's public key on the route (`target_identity`), and the target pins the caller's public key on the grant (`peer`) — trust on first use (TOFU). After that, a changed peer key fails the connection and asks you to pair again. A new key is never accepted silently, and the channel never downgrades to plaintext.
- `cranibox peer identity` on both sides shows the same 12-digit pairing code (for example `1234-5678-9012`, about 40 bits, from the SHA-256 of both identities). After checking it by eye or voice on both devices, each side runs:

```sh
cranibox peer verify --node <peer-NodeID> --code 1234-5678-9012
```

- **New grants are strict by default** (`peer grant` with no extra flags, the default "authorize device" action in the web UI, and `/api/peers` `grant` without `require_verified`): every capability is refused until the target's owner runs `peer verify` (or types the code shown on the other device in the control center's inbound grants), and plaintext bearer requests are always refused. A key pinned on first use (TOFU) still counts as unverified. `--require-verified` is still accepted and means the same as the default.
- Explicit opt-out: `peer grant --allow-unverified`, the web checkbox "Allow use before the pairing code is checked (not recommended)", or `{"operation":"grant",…,"require_verified":false}`. Such a grant can be used before verification and accepts plaintext bearer requests (older nodes, gateways, direct HTTP tools). Once a key is pinned, a changed key is still refused.
- The control center shows pairing state under each inbound grant: waiting for first connect, unverified (shows this device's pairing code; type the code from the other device and choose "Check and trust"), verified, or compatibility mode.
- The caller's "remote node" details also show that connection's pairing code and verification state: tell the target's owner the code shown here so they can enter it on the target's inbound grant. You can also enter the code the target shows, to record the caller's own check (enforcement still comes from the target's strict grant).

### Compatibility

- If an older target has no `/keys`, a modern caller falls back to the original bearer request (and probes again after 120 seconds). Once a key is pinned, it does not fall back.
- Older callers can still use a non-strict grant; a strict grant refuses them.
- **Migration (default became strict):** grants that existed before the upgrade have no `require_verified` field. They are read as `false`, and both the signed bytes and the behavior stay the same, so they do not suddenly fail. To make an old grant strict, revoke it and grant again (without `--allow-unverified`), then check the pairing code after the peer's first connect. A grant issued after the upgrade that must still work with an older node or a gateway needs an explicit `--allow-unverified`. Scripts may keep `--require-verified`; using it together with `--allow-unverified` is an error.
- Pinning a key does not change the connection binding of already queued messages, the space catalog, and similar state.

## Threat model (phase 2)

| Threat | Handling |
| --- | --- |
| A relay, gateway or VPN reading or changing a request | It sees only ciphertext; tampering, replay or truncation is refused and the request is not executed |
| A relay capturing a bearer credential | Between modern nodes the credential travels only inside the encrypted handshake |
| Impersonating the target node | The caller encrypts to the pinned Noise public key; an impersonator cannot finish the handshake |
| A man-in-the-middle on first connect | Inherent TOFU risk; the pairing code detects it. Sensitive grants use `--require-verified` (now the default) |
| A session key leaks | Ephemeral keys are per session and do not affect other sessions |
| The target's long-term Noise private key leaks | The payload of IK handshake message 1 (including the bearer) is not forward-secret against this; revoke grants and regenerate the identity |
| Downgrade to plaintext | No fallback after pinning; a strict grant refuses plaintext on the server |
| WebSocket data frames | Per-frame ChaCha20-Poly1305, per-direction keys and implicit counters; tampering, replay or reordering closes the stream (control frames and frame size/timing remain visible) |
| Metadata | A relay can still see the target NodeID, request size and timing |

## Inbound policy and threat model (phase 3)

See [device messages](device-messages.md) (inbound policy and human approval, phase 3). New grants default to `hold`: the local owner must release a message before an agent can see it. `refuse` rejects it outright. Older grants stay `accept`.

| Threat | Handling |
| --- | --- |
| An authorized peer uses messages for prompt injection against this device's agent | Held by default; the body is wrapped in `<cranibox-message ... trust="untrusted">` and closing tags are escaped |
| An agent tries to release held messages itself | MCP refuses approval operations; only the local CLI / web UI can decide |
| Message floods | 60 messages per sender per minute |
| Two agents replying to each other in a loop | After 12 messages in the same session within 10 minutes, further messages are held |
| A malicious local process running as the same OS user | **Not covered**; it can call the CLI directly |

## Discovering and delivering to running sessions (phase 4)

See [device messages](device-messages.md) (delivery to a running agent session, phase 4). No new state machine is added: discovery only reads session files that Claude Code / Codex have already written; delivery reuses the existing "acknowledged" (`acknowledged_at`) field.

- `cranibox peer discover-sessions` passively lists running Claude Code and Codex sessions from their transcript files (`~/.claude/projects`, `~/.codex/sessions`, written within the last 30 minutes). It reports id, harness, cwd, repository name, branch, activity (`working` within 90 s, otherwise `idle`) and last event kind. It is local only, conversation content is never read or returned, and it is not exposed over the mesh.
- `cranibox peer messages hook` is a command hook for both harnesses. On `UserPromptSubmit`/`SessionStart` it adds `additionalContext`. On `Stop`, only if configured with `--wake-on-stop`, it continues the turn, and never when `stop_hook_active` is set. It injects only messages that the inbound policy accepted or the owner released, that are addressed to that session (`target_session`) and not yet acknowledged. They stay framed as untrusted and are acknowledged atomically, so each is delivered once.
- Delivery is capped at 8 messages / ~32 KB per turn. The hook always exits 0.
- `peer messages hook-config --harness claude|codex [--wake-on-stop]` prints the settings. Codex requires trusting the hook in `/hooks`. Paths are quoted per platform. On macOS/Linux they use POSIX single quotes. On Windows, Claude Code gets exec form (`command` plus `args`, no shell), and Codex gets cmd.exe double quoting. Codex refuses paths containing `%`.
- The MCP `device_messages` tool remains the fallback. The Codex app-server is not used; Codex hooks cover delivery.
- The tray menu's first item shows how many messages await approval and opens the window. On Linux the count is also shown next to the icon, because appindicator has no tooltips. On macOS and Windows the tooltip shows it too.
