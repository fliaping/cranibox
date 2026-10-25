# 设备访问控制：按能力白名单、限时批准与使用审计 / Device access control

> 状态：方案已确定（2026-10-25），按阶段实现。第二阶段的管理员签名部分沿用
> [`admin-access-policy.md`](admin-access-policy.md)。本文定义第一阶段，以及两份文档合起来的实现顺序。

## 问题

现在自己的设备配对后，每个设备都给对方签发**全部能力**：`owner_permissions()` 在
`peer/transport_iroh.rs` 中写入，`trust_members()` 还会把缺的补齐。任何一台设备失陷，攻击者都能用它
打开其他所有设备的终端、远程桌面、Computer Use 和浏览器，也能远程更新它们，而用户看不到任何提示。
[安全审查](security-review-2026-10-23.md)第 3 条已经指出这个问题。

Tailscale 的情况相同，它也有对应的机制：

- 新建 tailnet 的默认策略是全部放行，[一台失陷的设备能连到所有设备](https://tailscale.com/kb/1018/acls)。
  推荐的做法是改成[默认拒绝的 grants](https://tailscale.com/docs/features/access-control/acls)。
- Tailscale SSH 的 [check 模式](https://tailscale.com/kb/1193/ssh)：连接前用户必须重新登录身份提供方，
  通过后在 `checkPeriod`（默认 12 小时）内免检。`"always"` 表示每次连接都要检查
  （[`0s` 曾因解析问题无效](https://cve.circl.lu/vuln/ts-2024-003)）。
- Tailnet Lock：由用户持有的密钥授权节点，不信任协调服务器（见 `admin-access-policy.md`）。

我们照搬这三点：默认拒绝、高风险能力每次由人批准并限时、策略只能由人改。另外增加一条：被控制的设备上
留下**谁在什么时候用了什么**的审计记录，并实时提示。

## 原则

1. **目标设备说了算。** 能力检查已经在目标端执行（`peer/server.rs` 的 `check_permission`），所以策略也存在
   目标端。请求方说什么都不能放宽目标端的策略。
2. **默认拒绝。** 新配对的自己的设备只有“消息与任务”。其他能力要在目标设备上逐项打开。
3. **高风险能力默认“每次询问”。** 打开后的默认模式是“询问”，不是“始终允许”。
4. **批准要求本人在场。** 批准来自目标设备上的本人操作（第一阶段），或者来自管理员身份的签名
   （第二阶段，Touch ID / Windows Hello / 管理员密码）。失陷的请求方或第三台设备伪造不了。
5. **策略不能被远程放宽。** 第一阶段只能在目标设备本机修改策略，远程修改一律拒绝。第二阶段才允许管理员签名
   的远程修改。
6. **用过就留痕。** 每次会话的开始、结束、批准都写入目标设备的审计日志，会话进行中在目标设备上一直显示提示。

## 能力与模式

每一对 (请求方设备 → 本设备)、每项能力有一个模式：

| 模式 | 含义 |
| --- | --- |
| 关闭 | 拒绝，返回可操作的提示（“在 <本设备> 的 设置 › 访问 中允许 <请求方> 使用终端”） |
| 询问 | 每次开始使用前需要批准。批准时选择有效期：仅本次会话 / 30 分钟 / 8 小时 |
| 允许 | 不再询问（仍然审计） |

能力分组（界面上按组显示，“高级”里可以逐项调整）：

| 组 | 能力 | 新配对的默认值 | 打开时的默认模式 |
| --- | --- | --- | --- |
| 消息与任务 | `messages` | 允许 | — |
| 终端与命令 | `terminals`、`exec`、`projects` | 关闭 | 询问 |
| 屏幕 | `desktop`（看与控制）、`computer`（Computer Use） | 关闭 | 询问 |
| 浏览器 | `browser` | 关闭 | 询问 |
| 远程更新 | `update` | 关闭 | 询问 |
| 虚拟空间 | `spaces` | 关闭 | 询问 |
| 活动与同步 | `activity` | 关闭 | 允许 |
| 模型与账号 | `llm`、`vault` | 关闭 | 询问 |

`update` 现在没有单独的能力，远程更新只要求名册成员资格。实现时把它拆出来：远程更新会在目标设备上运行
新的程序，风险等同于执行命令。

“会话”的定义：一条终端接入、一次远程桌面连接、一次 Computer Use 控制会话、一次更新、一次浏览器控制会话，
或者一次 `exec` 调用。“仅本次会话”的批准只对这一个会话有效，断开后再连要重新批准。终端断线后的自动重连
是例外：60 秒内重连同一会话不重新询问。

## 批准流程（第一阶段）

1. 请求方调用能力。目标端发现模式是“询问”且没有有效的批准，就创建一条**待批准请求**，返回
   `403 approval_required` 和请求 ID。每个请求方同一时间最多 1 条待批准请求；被拒绝后冷却 1 分钟，
   防止用弹窗轰炸诱导误点。请求 2 分钟后过期。
2. 目标设备上显示请求：桌面客户端发系统通知并打开批准窗口，网页界面在顶部显示横幅。内容由目标端自己生成：
   “<请求方设备名> 请求使用 <能力>”，以及请求方的设备 ID 和最近一次连接路径。
3. 本人选择“拒绝”，或者“允许：仅本次 / 30 分钟 / 8 小时”。批准记录保存在目标端：
   `{ id, from, capability, scope: session|until, expires_at, approved_by: local, approved_at }`。
4. 请求方的界面轮询请求状态，看到批准后自动继续，看到拒绝或过期就显示结果。请求方界面上一直显示
   “等待 <目标设备> 上的批准…”。

没有本地界面的设备（无桌面的服务器）可以在本机用 CLI 批准“询问”请求。第一阶段不接受其他设备代为批准，
远程管理员批准留到第二阶段。在这类设备上选择“允许”时，界面要明确警告。

### 本机 CLI

以下命令直接操作本机存储，无需启动服务。设备参数可用节点 ID 或不重名的设备名称；所有命令都支持
`--json` 输出，适合脚本。能力组为 `messages`、`terminal`、`screen`、`browser`、`update`、
`spaces`、`activity`、`models`，也可使用表格中的单项能力。

~~~sh
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
~~~

`--verify` 校验失败时返回非零退出码；记录完整性状态也始终包含在普通输出和 JSON 中。

## 使用审计与提示

- 目标端追加写入 `access-audit.jsonl`：会话开始、结束、批准、拒绝、策略变更、被拒绝的调用（同类事件按分钟合并）。
  每条记录包含时间、请求方设备 ID 和名称、能力、会话 ID、依据（`allow` / 批准 ID）、结果。
- 每条记录带上前一条的 SHA-256，形成哈希链。有人删改中间的记录，校验就会失败。只保留最近 10 000 条。
- 会话进行中，目标设备上一直显示提示：桌面客户端用托盘图标加窗口顶栏，网页界面用顶部横幅，例如“DESKTOP-HOME
  正在使用终端 · 远程桌面”，旁边是“全部断开”按钮（复用现有紧急停止），点击后同时撤销所有限时批准。
- 设备页的“访问记录”显示审计日志，可以按设备和能力筛选。
- 第二阶段：审计记录同步给管理员的客户端，失陷设备删除自己的记录也不影响其他副本。

## 策略修改（第一阶段）

- 策略文件 `access-policy.json` 保存在目标端，只有本机的界面和 CLI 能修改。网页界面的修改接口只接受本机来源，
  经 mesh 转发来的远程请求一律拒绝。
- 每次修改都写入审计日志并通知。
- grant 仍然是传输层凭据。能力检查改成“grant 包含该能力”**且**“策略模式允许，或者有有效批准”。
  旧的全权限 grant 不需要重签，策略在它之上收紧。

## 关键操作必须批准

下列操作无论策略怎样设置都按“询问”处理，第二阶段改为需要管理员签名：

- 修改另一台设备的访问策略（第一阶段直接不允许）；
- 远程更新另一台设备，以及安装比当前版本更旧的版本；
- 把一台新设备加入自己的身份；
- 共享模型账号与密钥库（`llm`、`vault`）。

远程更新另外要求：只安装固定包名、带完整性校验的 npm 包（现状已经如此）；不能降级，除非本人在目标端批准。

## 第二阶段：管理员身份

实现 [`admin-access-policy.md`](admin-access-policy.md) 的第 1–5 步，并做以下调整：

- 名册里的 `AccessEdge.permissions` 改成每项能力一个模式（关闭 / 询问 / 允许）。设备收到名册后取名册与本机
  策略中更严的一项。
- **远程批准**：管理员在任意客户端用 Touch ID / Windows Hello / 管理员密码签发批准令牌
  `{ from, to, capability, scope, expires_at, request_id }`，令牌由管理员会话密钥签名。目标端验证签名链后视同本机批准。
  无桌面的服务器从此可以使用“询问”。
- 管理员可以在任意节点修改所有设备的策略，修改经签名名册分发（取代第一阶段的“只能本机修改”）。

## 威胁模型（第一阶段完成后）

| 场景 | 结果 |
| --- | --- |
| 设备 B 失陷 | 只能使用其他设备对 B 设为“允许”的能力。“询问”能力需要目标设备上的本人批准，目标设备上会弹出请求并写入审计。“关闭”的能力无法使用 |
| B 失陷并轰炸批准请求 | 每个请求方 1 条待批准、拒绝后冷却 1 分钟。请求内容由目标端生成 |
| 本人批准了 B 30 分钟 | 30 分钟内 B 可以使用该能力，全程有提示，可以一键全部断开 |
| B 试图远程放宽策略 | 拒绝，第一阶段策略只能在本机修改 |
| 无桌面服务器设为“允许” | 风险与现在相同，界面警告。第二阶段的管理员远程批准解决此问题 |
| 目标设备本身失陷 | 不在保护范围内：攻击者已控制该设备 |

## 实现顺序

第一阶段（当前）：

1. 契约：`Capability` 枚举（含 `update`）、`Mode`、`AccessPolicy { peers: { node → { capability → mode } } }`、
   `Approval`、`AuditEvent`。默认策略生成函数。
2. 执行：所有能力入口（终端、exec、项目、桌面、Computer Use、浏览器、空间、活动、更新、llm、vault）在
   `check_permission` 之后统一调用 `access::authorize(caller, capability, session)`。拒绝时返回可操作的提示，
   需要批准时返回 `approval_required`。新配对的设备按默认策略生成条目。
3. 批准：待批准请求存储、过期、冷却。本机批准和拒绝接口只接受本机来源。请求方查询状态的接口。会话范围的
   批准与 60 秒重连宽限。
4. 审计：哈希链日志、校验、查询接口。“全部断开”撤销所有批准并结束所有会话。
5. 界面：设备页的“访问”分区（按组切换模式）、批准横幅和窗口、请求方的等待状态、会话进行中的提示、访问记录。
   桌面客户端发系统通知。
6. 测试：多节点用例覆盖默认拒绝、询问到批准到使用、过期、拒绝冷却、远程放宽被拒绝、审计哈希链和篡改检测。

第二阶段：`admin-access-policy.md` 的第 1–5 步，加上上面的调整。第三阶段：Touch ID 与 Windows Hello（同文档第 6–8 步）。

### Phase 1 backend implementation

The target stores `access-policy.json` beside `peers.json`, using the same private atomic-write
and file-lock helpers as other node stores. It contains the `peers` policy map and persisted
requests, approvals, cooldowns, active sessions and job-result authorizations. Transport grants
are unchanged: admission requires the existing grant check **and** target policy authorization.
Update has no legacy grant bit; its existing verified, encrypted owner-roster admission remains
required, followed by the new `update` policy check. Update, LLM and vault still require an
approval when their mode is `allow`, as specified under “关键操作必须批准”.

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
{"error":"capability_off","capability":"exec","target_name":"Desktop","hint":"On Desktop, open Settings > Access and allow Laptop to use exec."}
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

## English summary

Today paired own devices grant each other every capability, so one compromised device controls all of them,
silently. Following Tailscale's deny-by-default grants and SSH check mode: the target device stores a policy
per (requesting device, capability) with three modes, off / ask / allow. Newly paired devices only get
messages. Enabling a high-risk capability (terminal, exec, projects, desktop, Computer Use, browser, remote
update, spaces, llm, vault) defaults to ask: each session needs approval by a person at the target (phase 1),
or an admin-signed approval token (phase 2, Touch ID / Windows Hello / admin password, which also covers
headless servers), scoped to this session, 30 minutes or 8 hours. The target generates the request text
itself and rate-limits requests (one pending per requester, 1-minute cooldown after a denial). In phase 1
policy can only be changed locally on the target; remote widening is refused. Every session, approval,
denial and policy change goes to a hash-chained audit log on the target; active sessions show a persistent
indicator with a one-click disconnect that also revokes approvals. Remote update becomes its own capability;
installing an older version needs approval at the target.
