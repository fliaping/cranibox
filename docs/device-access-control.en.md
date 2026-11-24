# Device access control: per-capability allowlists, timed approval and an audit log

> Chinese original: [device-access-control.md](device-access-control.md).
>
> Status: the design was settled on 2026-10-25 and is implemented in stages. The signed-administrator part of phase 2 follows [`admin-access-policy.md`](admin-access-policy.md). This document defines phase 1, and the implementation order of the two documents together.

## Turn off all approvals on this device (implemented)

The "Automatic review" area of the device access settings has a "Turn off all approvals" switch. It is off by default, so the usual approval behavior stays. It must be turned on on the target device that receives the request, and each device is configured on its own. The switch is not spread by pairing or member sync.

When it is on, capabilities whose mode is "ask" or "allow" run immediately, skipping ordinary approval, dangerous-operation confirmation and Jev / model review, including remote updates and model or account operations. Capabilities whose mode is "off" are still refused. Transport identity and capability grants are still checked, and operations and switch changes are still written to the audit log. Existing permissions, review settings and keys are kept; the justification recorded for an approval-free operation is `approvals_disabled`. The rest of this document's "confirm every time" and model-review rules assume approvals are on.

Local CLI (`off` turns approvals off, `on` turns them back on):

```sh
cranibox access approvals        # show status
cranibox access approvals off    # turn off all approvals
cranibox access approvals on     # restore approvals
```

Local API: `PUT /api/v1/app/access/approvals`, body `{"enabled": false}` to turn approvals off, `{"enabled": true}` to restore them. The response is the access status, including `approvals_disabled`. This route has no remote-node entry. A missing, non-boolean or unknown field is refused. The setting is persisted and survives a restart; an older config without the field keeps normal approvals.

Turning approvals off expires already pending requests and clears the matching challenges and denial cooldowns; the caller can retry under the new policy. Turning approvals back on ends sessions created while approvals were off, and restores the previous permissions and review settings. Reading the result of a task that already started still follows the existing task access rules.

## The problem

After you pair your own devices, each one currently issues **every capability** to the other: `owner_permissions()` writes this in `peer/transport_iroh.rs`, and `trust_members()` fills in anything missing. If one device is compromised, the attacker can open terminals, remote desktops, Computer Use and the browser on every other device, and can update them remotely, with no prompt. [The security review](security-review-2026-10-23.md) already called this out in item 3.

Tailscale has the same starting point, and corresponding controls:

- The default policy of a new tailnet allows everything, so [a compromised device can reach all the others](https://tailscale.com/kb/1018/acls). The recommended change is [deny-by-default grants](https://tailscale.com/docs/features/access-control/acls).
- Tailscale SSH [check mode](https://tailscale.com/kb/1193/ssh): before a connection the user must sign in to the identity provider again, then is not checked again for `checkPeriod` (12 hours by default). `"always"` checks every connection ([`0s` used to be ignored because of a parsing bug](https://cve.circl.lu/vuln/ts-2024-003)).
- Tailnet Lock: keys held by the user authorize nodes, so the coordination server is not trusted (see `admin-access-policy.md`).

We follow those three ideas: deny by default, high-risk capabilities approved by a person each time and only for a limited period, and policy that only a person can change. We add one more: the device that was used keeps an audit of **who used what, and when**, and shows a live indicator.

## Principles

1. **The target device decides.** Capability checks already run on the target (`check_permission` in `peer/server.rs`), so the policy lives there too. Nothing the requester says can widen the target's policy.
2. **Deny by default.** A newly paired device of yours gets only "messages and tasks". Other capabilities are turned on one by one on the target device.
3. **High-risk capabilities default to "ask every time".** After you turn one on, the default mode is "ask", not "always allow".
4. **Approval requires the owner to be present.** In phase 1 the approval is a local action on the target device; in phase 2 it can be a signature from an administrator identity (Touch ID / Windows Hello / an administrator password). A compromised requester or a third device cannot forge that.
5. **Policy cannot be widened remotely.** In phase 1 the policy can only be changed on the target device itself; a remote change is always refused. Phase 2 allows remote changes signed by an administrator.
6. **Use leaves a trace.** Session start, end and approval are written to the target's audit log, and a prompt stays visible on the target for the whole session.

## Capabilities and modes

Each pair (requesting device → this device) and each capability has a mode:

| Mode | Meaning |
| --- | --- |
| Off | Refuse, with an actionable hint ("On &lt;this device&gt;, open Settings › Access and allow &lt;requester&gt; to use the terminal") |
| Ask | Each use needs approval before it starts. When approving, choose a lifetime: this session only / 30 minutes / 8 hours |
| Allow | Do not ask again (still audited) |

Capabilities are grouped in the UI (you can still adjust items one by one under "Advanced"):

| Group | Capabilities | Default for a new pair | Default mode when turned on |
| --- | --- | --- | --- |
| Messages and tasks | `messages` | Allow | — |
| Terminal and commands | `terminals`, `exec`, `projects` | Off | Ask |
| Screen | `desktop` (view and control), `computer` (Computer Use) | Off | Ask |
| Browser | `browser` | Off | Ask |
| Remote update | `update` | Off | Ask |
| Virtual spaces | `spaces` | Off | Ask |
| Activity and sync | `activity` | Off | Allow |
| Models and accounts | `llm`, `vault` | Off | Ask |

`update` is not yet a separate capability today: a remote update only requires roster membership. When implementing it, split it out. A remote update runs a new program on the target, so the risk is the same as running a command.

A "session" is one terminal attachment, one remote-desktop connection, one Computer Use control session, one update, one browser control session, or one `exec` call. An approval for "this session only" applies to that session alone; reconnecting after a disconnect needs a new approval. Automatic reconnect of a terminal is the exception: reconnecting the same session within 60 seconds does not ask again.

## Approval flow (phase 1)

1. The requester calls a capability. If the target's mode is "ask" and there is no valid approval, the target creates a **pending request** and returns `403 approval_required` with the request ID. Each requester may have at most one pending request at a time; after a denial there is a 1-minute cooldown, so a flood of pop-ups cannot rush a mis-click. A request expires after 2 minutes.
2. The target device shows the request: the desktop app sends a system notification and opens the approval window; the web UI shows a banner at the top. The text is generated on the target itself: "&lt;requesting device name&gt; wants to use &lt;capability&gt;", plus the requester's device ID and the most recent connection path.
3. The owner chooses "Deny", or "Allow: this session only / 30 minutes / 8 hours". The approval is stored on the target: `{ id, from, capability, scope: session|until, expires_at, approved_by: local, approved_at }`.
4. The requester's UI polls the request. After an approval it continues on its own; after a denial or expiry it shows the result. Meanwhile it keeps showing "Waiting for approval on &lt;target&gt;…".

A device with no local UI (a server without a desktop) can approve "ask" requests with the local CLI. Phase 1 does not accept approval from another device; remote administrator approval is phase 2. When choosing "Allow" on such a device, the UI must warn clearly.

### Local CLI

These commands operate on the local store and do not need a running service. A device argument may be a node ID or an unambiguous device name. Every command supports `--json`, which is useful for scripts. Capability groups are `messages`, `terminal`, `screen`, `browser`, `update`, `spaces`, `activity` and `models`; a single capability from the table also works.

```sh
cranibox access list
cranibox access set "Design Mac" terminal=ask browser=off
cranibox access approve <request-id> --for session
cranibox access approve <request-id> --for 30m
cranibox access approve <request-id> --for 8h
cranibox access deny <request-id>
cranibox access revoke <approval-id>
cranibox access revoke --all
cranibox access log --limit 50 --node "Design Mac" --capability terminals --verify
cranibox access list --json
```

`--verify` exits non-zero when the integrity check fails. Integrity status is always included in both the ordinary output and JSON.

## Audit log and live indicator

- The target appends to `access-audit.jsonl`: session start and end, approvals, denials, policy changes, and refused calls (events of the same kind are merged per minute). Each record has the time, the requester's device ID and name, the capability, the session ID, the justification (`allow` / an approval ID) and the result.
- Each record includes the SHA-256 of the previous one, forming a hash chain. Deleting or editing a record in the middle fails verification. At most the most recent 10,000 records are kept.
- While a session is active, the target keeps showing an indicator: the desktop app uses the tray icon plus a window top bar; the web UI uses a top banner, for example "DESKTOP-HOME is using the terminal · remote desktop", next to a "Disconnect all" button (the existing emergency stop). That button also revokes every timed approval.
- The device page's "Access log" shows the audit entries and can filter by device and capability.
- Phase 2: audit records are synced to the administrator's clients, so a compromised device deleting its own log does not remove the other copies.

## Changing policy (phase 1)

- The policy file `access-policy.json` lives on the target. Only the local UI and CLI can change it. The web UI's modification routes accept only a local source; a remote request forwarded over the mesh is always refused.
- Every change is written to the audit log and notified.
- A grant remains the transport credential. A capability check becomes "the grant includes this capability" **and** "the policy mode allows it, or there is a valid approval". An older all-capabilities grant does not need to be re-signed; the policy tightens on top of it.

## Operations that always need approval

The following are treated as "ask" no matter how the policy is set. In phase 2 they require an administrator signature:

- Changing another device's access policy (refused outright in phase 1);
- Updating another device remotely, and installing a version older than the one already running;
- Adding a new device to your identity;
- Sharing model accounts and the vault (`llm`, `vault`).

A remote update also requires: only a pinned package name, with integrity checked, from npm (already the case); and no downgrade unless the owner approves it on the target.

## Phase 2: administrator identity

Implement steps 1–5 of [`admin-access-policy.md`](admin-access-policy.md), with these adjustments:

- Roster `AccessEdge.permissions` becomes a mode per capability (off / ask / allow). When a device receives the roster it takes the stricter of the roster and its local policy.
- **Remote approval:** an administrator on any client issues an approval token with Touch ID / Windows Hello / an administrator password: `{ from, to, capability, scope, expires_at, request_id }`, signed by the administrator session key. After the target verifies the signature chain, this counts as a local approval. Headless servers can then use "ask".
- An administrator can change every device's policy from any node. Changes are distributed in the signed roster (this replaces phase 1's "local changes only").

## Threat model (after phase 1)

| Scenario | Outcome |
| --- | --- |
| Device B is compromised | It can only use capabilities that other devices set to "allow" for B. "Ask" capabilities need the owner's approval on the target, which pops up a request and writes an audit entry. "Off" capabilities cannot be used |
| B is compromised and floods approval requests | One pending request per requester, and a 1-minute cooldown after a denial. The request text is generated on the target |
| The owner approved B for 30 minutes | B can use that capability for 30 minutes, with a live indicator, and can be disconnected in one click |
| B tries to widen policy remotely | Refused; in phase 1 policy can only be changed locally |
| A headless server is set to "allow" | The same risk as today; the UI warns. Phase 2's remote administrator approval addresses this |
| The target device itself is compromised | Out of scope: the attacker already controls that device |

## Implementation order

Phase 1 (current):

1. Contracts: a `Capability` enum (including `update`), `Mode`, `AccessPolicy { peers: { node → { capability → mode } } }`, `Approval`, `AuditEvent`. A function that builds the default policy.
2. Enforcement: after `check_permission`, every capability entry point (terminal, exec, projects, desktop, Computer Use, browser, spaces, activity, update, llm, vault) calls `access::authorize(caller, capability, session)`. A refusal returns an actionable hint; when approval is needed it returns `approval_required`. A newly paired device gets entries from the default policy.
3. Approval: storage, expiry and cooldown of pending requests. Local approve/deny routes accept only a local source. A route for the requester to poll state. Session-scoped approvals and the 60-second reconnect grace period.
4. Audit: hash-chained log, verification, query routes. "Disconnect all" revokes every approval and ends every session.
5. UI: an Access section on the device page (mode switches by group), approval banners and windows, waiting state on the requester, a live indicator during a session, and the access log. The desktop app sends a system notification.
6. Tests: multi-node cases for default deny, ask then approve then use, expiry, denial cooldown, a refused remote widening, and audit hash-chain tamper detection.

Phase 2: steps 1–5 of `admin-access-policy.md`, plus the adjustments above. Phase 3: Touch ID and Windows Hello (steps 6–8 of that document).

### Phase 1 backend implementation

The target stores `access-policy.json` beside `peers.json`, using the same private atomic-write
and file-lock helpers as other node stores. It contains the `peers` policy map and persisted
requests, approvals, cooldowns, active sessions and job-result authorizations. Transport grants
are unchanged: admission requires the existing grant check **and** target policy authorization.
Update has no legacy grant bit; its existing verified, encrypted owner-roster admission remains
required, followed by the new `update` policy check. Update, LLM and vault still require an
approval when their mode is `allow`, as specified under “Operations that always need approval”.

**One-time migration:** startup creates the policy only when the file does not exist. All current
roster members other than this node receive `allow` for messages/activity and `ask` for the
remaining capabilities, including members whose transport grant has not yet been populated.
Non-member direct grants retain their explicitly granted capabilities as `allow`; ungranted
capabilities and update are `off`. No credentials are rewritten or re-signed. Subsequent peers
without an entry get messages=`allow` and everything else=`off`; loading an existing policy
never repeats migration or silently widens it.

The cookie/CSRF-protected local app adapter and native app dispatcher expose:

| Method and route (under `/api/v1/app`) | Request / response |
| --- | --- |
| `GET /access` | `{peers:[{node,name,capabilities}],pending,approvals,sessions}` |
| `PUT /access/{node}` | `{"terminals":"ask","exec":"off"}` → `{}` |
| `POST /access/requests/{id}` | `{decision:"deny"}` or `{decision:"approve",scope:"session"\|"30m"\|"8h"}` → request record |
| `POST /access/revoke` | `{}` / `null` for all, or `{id:"approval-id"}` → `{}` |
| `GET /access/audit?limit=&before=&node=&capability=` | `{entries,verified,broken_index}`; descending stable indexes, maximum 10,000 |
| `GET /access/requests/{id}?node={target}` | Requester convenience proxy to target request state |

The mesh exposes only `GET /api/v1/mesh/access/requests/{id}`, limited to the authenticated
requester that created it (other requesters get 404). Its state is `pending`, `approved`,
`denied` or `expired`. Mesh calls to the local access routes get 403. The local HTTP adapter
also rejects the transport-injected `Authenticated` extension; no caller-supplied origin header
can confer local authority. Existing `/api/v1/app/events` SSE snapshots now include `access`
events, with the same three-second polling cadence as other app data.

Capability refusals retain HTTP 403 and their JSON across HTTP, encrypted mesh and WebSocket
handshake errors, including the requesting node's HTTP adapter:
Terminal tickets open their target connection during the HTTP ticket request, so browser code
can read approval errors before attempting its WebSocket upgrade. Unused tickets disconnect
after 30 seconds. Both standalone and gateway web adapters preserve remote errors.

```json
{"error":"capability_off","capability":"exec","target_name":"Desktop","hint":"On Desktop, open Devices, select Laptop, and allow exec on its Access tab."}
{"error":"approval_required","request_id":"uuid","capability":"terminals","expires_at":1792886520}
{"error":"approval_cooldown","capability":"terminals","expires_at":1792886460}
```

Requests expire after 120 seconds, with one pending per requester and a 60-second denial
cooldown. An unused session approval expires after 120 seconds. Active session approvals are
bound to a terminal session ID, the originating Computer Use acquisition, or a digest of the
HTTP method/path/query/body and optional `access_session` query or idempotency key. The digest avoids
persisting command bodies or credentials. Retrying an approved operation must preserve that
key and body. A new connection may supply a distinct `access_session` to distinguish identical
operations. Terminal detach/restart preserves approval for 60 seconds; other connection
approvals end with the session. Timed approvals last exactly 1,800 or 28,800 seconds.

Exec and update jobs carry their original authorization into result polling/cancellation.
These persisted records authorize only that requester's existing job and expire no later than
ten minutes after completion/restart (or the original timed approval, if sooner). They never
authorize a second execution. Policy changes and approval revocation invalidate them. Session
guards audit start/end, recheck policy during active use, and close transports/cancel jobs on
revocation. Both existing local Computer Use and Browser emergency-stop paths revoke all
approvals and active sessions, including when no Computer Use provider is configured.

The audit hashes the preceding JSON line's UTF-8 bytes, excluding the newline. Rotation keeps
at most 10,000 records, including a checkpoint committing to the dropped prefix, and re-links
the retained suffix without changing event indexes. Queries verify the entire retained chain
before filtering and report the first broken index. This is local integrity evidence, not a
signature or an external trust anchor: a privileged attacker can rewrite the entire log or
remove a complete suffix; phase 2 replication is still needed for independent evidence.

Shared DTOs, populated fixtures and mock routes live in `cranibox-contracts`. The React UI
provides grouped access settings, queued approval dialogs, active-session controls and a
verified audit view. Both HTTP adapters share approval polling and automatic action retries.
The desktop app uses its existing system notification plugin to open the approval window;
headless devices can review and decide requests with the local CLI above. Administrator
signatures and roster changes remain part of phase 2.
