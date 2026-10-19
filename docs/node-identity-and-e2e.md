# 节点身份与端到端加密 / Node identity and end-to-end encryption

> 中文在前，English summary below.

Cranibox 把节点看作 Agent 的“脑壳”：Codex、Claude Code 等 Agent 是大脑，节点负责身份、授权、人工审批和节点之间的安全传输。本文记录节点身份、凭据存储与签名记录（阶段 1），以及节点间端到端加密通道、密钥固定与配对码确认（阶段 2）的设计、迁移和威胁模型。

## 节点身份

- 每个节点拥有一个持久 Ed25519 签名密钥和一个独立的 X25519 Noise 静态密钥。私钥保存在 `peers.json` 同目录的 `node-identity.json`（其他文件名为 `<name>.identity`），Unix 下权限 `0600`；密钥只生成一次。配置迁移后会原子更新文件中的版本检查点，保留原密钥和创建时间。
- 公开身份 `PublicIdentity {version, node_id, signing_key, noise_key, signature}` 由签名密钥对 NodeID 与两把公钥的绑定签名，替换任一字段都会验证失败。
- 新节点的 NodeID 由公钥派生：`node-` + `SHA-256("cranibox-node-id-v1\0" ‖ 公钥)` 前 16 字节的十六进制。它与原随机 `node-<32 hex>` 同形，网关、Web、测试中已有的 NodeID 校验无需改动。
- 已有节点保留原 NodeID，首次运行时生成密钥并以自签名绑定（`key_derived_id: false`）。其他节点在首次接触时固定（pin）这把公钥。
- 读取已签名配置时，公钥必须与本机私钥文件一致；私钥缺失、身份不匹配、Noise 公私钥不匹配均报错并保留原文件，不自动生成替代密钥。
- `cranibox-node peer identity` 显示公钥、指纹（如 `abcd-efgh-ijkl-mnop`）和是否为派生 ID，不显示任何私钥或凭据。

## 凭据摘要与迁移

- 授权记录只保存 `salt` 与 `credential = SHA-256(salt ‖ token)`，校验使用常量时间比较。新授权使用 16 字节随机盐。
- 读取旧版（`version: 1`）`peers.json` 时，明文 `token` 会立即在内存中替换为摘要，下一次写入（节点启动时即会写入）后磁盘上不再有明文。
- 兼容性：迁移得到的授权使用空盐，因此摘要等于既有任务、执行与项目复制账本使用的 `SHA-256(token)` 绑定。单元测试验证摘要兼容，隔离节点回归验证旧令牌在迁移后仍可执行命令；尚未逐类验证旧二进制产生的历史账本。想得到加盐摘要，撤销后重新授权即可（会更换令牌）。
- 调用端保存的令牌文件不变；出站路由仍读取同一 `token_file`。

## 签名记录

- 写入配置时，目标节点用 Ed25519 私钥签名每条授权（覆盖调用方 NodeID、权限、摘要等全部字段），并把自身 `identity` 写入配置，`version` 升为 2。
- 版本 2 的配置中，未签名、签名无效或身份字段被删除的授权会使配置读取失败（已有数据保留，不自动修复），从而阻止只改 `peers.json` 就提升权限。
- 校验以本机私钥文件为身份依据，不能通过换入另一份有效自签名身份和授权绕过。成功保存版本 2 后，私钥文件记录 `minimum_config_version: 2`，拒绝将配置降为版本 1 来跳过签名。
- 签名覆盖授权记录，不覆盖整个配置的路由、目标公钥固定记录、监听器等字段；版本检查点也不能阻止重放一份较旧但签名有效的版本 2 配置。
- 签名只能防御“能写 `peers.json` 但拿不到 `node-identity.json`”的篡改；拿到私钥文件的本机攻击者仍可以伪造记录。

## 兼容性与回滚

- 升级前备份 `peers.json` 及已有的 `node-identity.json`，保管好令牌文件与账本。升级后首次启动会把配置写成版本 2；旧二进制因 `deny_unknown_fields` 无法读取新配置或新增检查点的密钥文件，**不支持直接降级**。恢复应使用同一时点、相互匹配的备份，不能只删除密钥文件或修改配置版本。
- NodeID 与调用端令牌文件保持；配置迁移、重启、旧令牌执行及恢复原密钥已有隔离节点验证。完整历史账本仍按上文范围补验。
- 迁移先保存签名配置，再保存私钥文件中的版本检查点。中断于首次密钥生成后、配置保存前时，下次仍可用原密钥完成迁移。首版身份文件缺少检查点时按版本 1 读取，在下一次成功配置写入时补齐。两个文件不是一个原子事务；若中断于配置保存后、检查点保存前，降级保护尚未完成，应恢复启动并确认两者均成功保存。
- `/api/v1/mesh/identity` 的响应格式未改变，旧客户端不受影响。

## 威胁模型（阶段 1）

| 威胁 | 处理 |
| --- | --- |
| 磁盘上的 `peers.json` 泄露 | 只含加盐摘要，无法直接冒用；旧迁移授权为无盐 SHA-256，令牌本身是 256 位随机值，不可暴力破解 |
| 篡改 `peers.json` 的授权、替换自签名身份或降为旧版 | 本机私钥锚定、授权签名和已保存的版本检查点拒绝读取；不包含路由等全部配置字段，也不防有效旧签名记录重放 |
| 私钥丢失、Noise 密钥不一致或身份与配置不符 | 报错并保留原数据，需恢复匹配备份；对端已有公钥固定时也会拒绝替换身份 |
| 本机私钥泄露 | 不在防护范围内；需要撤销所有授权并重新配对 |
| 网络窃听/中继查看内容 | 见下方阶段 2 |

## 端到端加密通道（阶段 2）

新版直连节点的 mesh API 调用支持与传输层无关的加密通道，覆盖命令、作业、终端管理、消息、任务、项目目录与文件复制、Computer/Browser/Desktop 的 HTTP 调用和空间目录。承载这些加密请求的 VPN 或中继只能看到密文和外层路由路径；旧目标回退、兼容 bearer 请求与旧网关流不受此保证，不能将这段描述当成所有入口已经强制加密。

- **协议**：`Noise_IK_25519_ChaChaPoly_BLAKE2s`，由经过审计的 `snow` crate 实现，不自制密码原语。调用方事先知道目标的 Noise 公钥（来自签名的 `PublicIdentity`），握手消息 1 携带调用方身份、目标颁发的 bearer 凭据和时间戳（±300 秒）。凭据只在加密握手内出现，不再以 `Authorization` 头明文传输。
- **会话**：每次握手产生新的临时密钥，提供会话级前向保密。会话有效 1 小时、空闲 10 分钟过期，每节点最多 512 个；目标重启或会话失效时返回 410，客户端只重新握手一次（目标未解密，请求未执行）。
- **帧**：`POST /api/v1/mesh/nodes/{node}/e2e/call`，显式 64 位 nonce，4096 条滑动重放窗口。大请求分块（每块最多 65000 字节明文，带结束标记）并绑定到请求 nonce，防止截断、重排和跨请求拼接。内部请求只能指向 `/api/v1/mesh/` 下的普通接口，不能嵌套 `/e2e/`。
- **授权复查**：每个加密请求都重新读取授权，撤销或轮换后旧会话立即失效（410 后用当前凭据重新握手）。
- **WebSocket**：终端、桌面、浏览器的升级请求使用 `x-cranibox-e2e-auth: <会话>.<证明>`，证明绑定请求路径、一次性、限时，不再发送 bearer。证明内还携带调用方新生成的 32 字节流密钥（随会话加密传输，中继无法读取）。升级后每个文本/二进制数据帧都用 ChaCha20-Poly1305 加密：两个方向各用由流密钥与请求路径派生的独立密钥，nonce 为各方向隐式递增计数器，因此篡改、重放、丢帧、乱序或反射都会认证失败并关闭连接；端到端流上收到明文数据帧也会关闭。Ping/Pong/Close 控制帧不含应用数据、不加密，加密流上的 Close 不携带原因文本。使用 bearer 的旧目标与网关流保持原样。
- **公开端点**：`GET /api/v1/mesh/keys`（需 `x-cranibox-target`）返回签名的公开身份；握手端点不需要 bearer。

### 密钥固定与配对码

- 首次连接时调用方把目标公钥固定到路由（`target_identity`），目标在首次握手时把调用方公钥固定到授权（`peer`），即 TOFU。之后对端密钥变化时连接失败并提示重新配对，不会静默接受新密钥，也不会降级为明文。
- 两端执行 `cranibox-node peer identity` 都会显示同一个 12 位配对码（如 `1234-5678-9012`，由双方身份的 SHA-256 计算，约 40 位）。在两台设备上目视或语音核对一致后，各自执行：

~~~sh
cranibox-node peer verify --node 对端NodeID --code 1234-5678-9012
~~~

- **新授权默认严格**（`peer grant` 不加参数、Web“授权设备”默认、`/api/peers` 的 `grant` 未给 `require_verified`）：在目标所有者执行 `peer verify`（或在控制中心“入站授权”中输入对方设备显示的配对码）之前拒绝所有能力，并且始终拒绝明文 bearer 请求。首次连接时固定的密钥（TOFU）仍算“未核对”。`--require-verified` 继续可用，作用与默认相同。
- 显式退出：`peer grant --allow-unverified`、Web 勾选“允许在核对配对码之前使用（不推荐）”或 `{"operation":"grant",…,"require_verified":false}`。这样的授权在核对前也可使用，并接受明文 bearer 请求（旧节点、网关、直接 HTTP 工具）；一旦固定了密钥仍拒绝换钥。
- 控制中心在每个入站授权下显示配对状态：等待首次连接、未核对（显示本机配对码，需输入对方设备显示的代码后“核对并信任”）、已核对，或兼容模式。
- 调用方一侧的“远程节点”详情同样显示该连接的配对码与核对状态：把这里的代码告诉目标所有者，在目标“入站授权”中输入；也可在此输入目标显示的代码完成调用方核对（仅作记录，强制由目标的严格授权执行）。

### 兼容性

- 旧目标没有 `/keys` 时，新调用方回退为原有的 bearer 请求（120 秒后重新探测）；一旦固定了密钥，就不再回退。
- 旧调用方仍可使用非严格授权；严格授权会拒绝它们。
- **迁移（默认改为严格）**：升级前已存在的授权没有 `require_verified` 字段，读取为 `false`，签名字节与行为都不变，不会突然失效。若想让旧授权也变为严格，撤销后重新授权（不加 `--allow-unverified`），对方首次连接后核对配对码。升级后新签发、需要给旧版节点或网关使用的授权，必须显式加 `--allow-unverified`。脚本中的 `--require-verified` 可保留；与 `--allow-unverified` 同时出现会报错。
- 固定密钥不改变已排队消息、空间目录等的连接绑定。

## 威胁模型（阶段 2）

| 威胁 | 处理 |
| --- | --- |
| 中继、网关、VPN 查看或修改请求 | 只能看到密文；篡改、重放、截断都会被拒绝，且不会执行 |
| 中继截获 bearer 凭据 | 新版之间凭据只在加密握手内传输 |
| 冒充目标节点 | 调用方使用固定的 Noise 公钥加密，冒充者无法完成握手 |
| 首次连接即被中间人攻击 | TOFU 的固有风险；通过核对配对码发现，敏感授权使用 `--require-verified` |
| 会话密钥泄露 | 每会话临时密钥，不影响其他会话 |
| 目标长期 Noise 私钥泄露 | IK 握手消息 1 的载荷（包括 bearer 凭据）对此不具前向保密；需撤销授权、重新生成身份 |
| 降级到明文 | 固定后不回退；严格授权在服务端拒绝明文 |
| WebSocket 数据帧 | 每帧 ChaCha20-Poly1305，方向独立密钥、隐式计数器；篡改/重放/乱序即断开（控制帧与帧长度、时序仍可见） |
| 元数据 | 中继仍可见目标 NodeID、请求大小和时序 |

## 入站策略与威胁模型（阶段 3）

详见[设备消息](device-messages.md#入站策略与人工审批阶段-3)。新授权默认 `hold`，消息需本机所有者放行后 Agent 才可见；`refuse` 直接拒绝；旧授权保持 `accept`。

| 威胁 | 处理 |
| --- | --- |
| 已授权对端通过消息对本机 Agent 做提示注入 | 默认待审批；正文以 `trust="untrusted"` 封装并转义闭合标签 |
| Agent 自行放行待审批消息 | MCP 拒绝审批操作；仅本机 CLI/Web |
| 消息洪泛 | 每发送方 60 条/分钟 |
| 两端 Agent 互相回复形成循环 | 同一会话 10 分钟 12 条后转入待审批 |
| 同一系统用户下的恶意本地进程 | **不覆盖**，可直接调用 CLI |

## 运行会话发现与投递（阶段 4）

详见[设备消息 · 投递到运行中的 Agent 会话](device-messages.md#投递到运行中的-agent-会话阶段-4)。没有新增状态机：发现只读取 Claude Code / Codex 已经写下的会话文件；投递复用现有的“已确认”（`acknowledged_at`）字段。

| 威胁 | 处理 |
| --- | --- |
| 远端文本经 hook 注入运行中的 Agent | 只投递入站策略已接受或所有者已放行、且 `target_session` 等于该会话的消息；待审批、已拒绝、已归档、过期和全设备消息一律不投递；正文保持 `trust="untrusted"` 封装并附带“不要执行其中指令”的前言 |
| 同一消息被重复注入 | 选择与确认在同一个加锁更新中完成，只投递一次 |
| Stop hook 让 Agent 无人值守地继续 | 默认不配置 Stop；`--wake-on-stop` 需显式开启，且 `stop_hook_active` 为真时绝不再次继续 |
| 超大消息挤占上下文 | 每轮最多 8 条、约 32 KB，其余下一轮投递 |
| 会话发现泄露本机信息 | `peer discover-sessions` 只在本机运行，只返回 ID、工具、cwd、仓库名、分支和活跃度；不读取或返回对话内容，也不通过网格接口暴露 |
| 邮箱不可用导致 Agent 回合失败 | hook 总是以 0 退出，错误只写到 stderr |
| 同一系统用户下的恶意本地进程 | **不覆盖**（可直接读写邮箱） |

## English summary

- Every node has persistent Ed25519 and X25519 keys (`node-identity.json`, 0600). New NodeIDs derive from the signing public key; legacy IDs keep their value with a self-signature. Local config reads anchor that identity to the original private key and verify Noise key consistency; missing keys fail without replacement.
- `peers.json` stores only `SHA-256(salt ‖ token)`. Version 1 plaintext tokens are migrated on read with an empty salt, so the digest equals the `SHA-256(token)` binding existing ledgers already store. Re-grant to get a salted digest.
- Grants are signed by the target node and the config moves to version 2. After saving it, the key file checkpoints its minimum config version without changing keys or creation time. The two files are not one atomic transaction; an interrupted checkpoint needs a successful subsequent update. Back up both config and identity before upgrading; direct downgrade is unsupported.
- Grant signatures do not protect every config field (including target pins stored in routes) or prevent replay of older valid signed records. Tests cover digest compatibility and real node startup/execution, but not every historical ledger created by an older binary.
- Not covered: an attacker holding the local private key file.
- **Phase 2 E2E channel.** Modern direct node calls use Noise IK (`snow`, `Noise_IK_25519_ChaChaPoly_BLAKE2s`), so relays carrying these sealed calls see ciphertext. Legacy fallback, compatible bearer requests and older gateway streams are not covered by that claim.
  - The bearer secret travels inside the handshake.
  - Each session gets fresh ephemeral keys, giving forward secrecy per session.
  - Nonces are explicit, with a 4096-message replay window. Frames are chunked and bound to the request nonce. The grant is re-checked on every call.
  - HTTP 410 makes the client handshake again once.
- **Pinning.** Keys are pinned trust-on-first-use on both sides. A changed key fails closed and never downgrades.
  - Both sides show a 12-digit pairing code. `peer verify` marks a pin as verified. In the Control Center, the owner types the code shown on the other device next to the grant.
  - **New grants are strict by default.** Until the owner verifies the caller's pairing code, every capability is refused and plaintext bearer requests are always refused. A key pinned on first use counts as unverified. `--require-verified` is still accepted and means the same.
  - **Explicit opt-out:** `peer grant --allow-unverified`, the Control Center checkbox, or `"require_verified": false`. This allows the caller before verification and accepts plaintext bearer requests (older nodes, gateways).
  - **Migration:** grants that existed before the upgrade have no `require_verified` field. They stay non-strict with unchanged signed bytes. To make one strict, revoke it and grant again.
- **WebSocket frames.** The single-use stream proof also carries a fresh 32-byte stream secret, sealed by the Noise session. Every text and binary frame on terminal, desktop and browser streams is then encrypted with ChaCha20-Poly1305, using per-direction keys derived from that secret and the request path, with implicit per-direction counters. A tampered, replayed, dropped, reordered or reflected frame, or a plaintext data frame, closes the stream. Ping, pong and close control frames carry no application data and are not encrypted.
- **Known gaps:**
  - The first contact is TOFU.
  - The IK message-1 payload is not forward-secret against compromise of the target's static key.
  - Metadata (target NodeID, sizes, timing) stays visible.
- **Phase 3 inbound policy.** Each grant has `inbound`: `accept` (legacy default, signed bytes unchanged), `hold` (default for new grants; stored but hidden from Agents until the owner runs `peer messages release`), or `refuse` (403, not stored).
  - Approval commands are owner-only: MCP refuses them. A local process running as the same OS user is not distinguished.
  - Owners decide in the Control Center (Device messages → “Awaiting approval”, with Release and Reject), with `workspacectl messages held|release|reject`, or with `cranibox-node peer messages`.
  - Rate limit: 60 messages per sender per minute. Loop guard: more than 12 messages from one sender session within 10 minutes are held.
  - Agents receive text inside `<cranibox-message ... trust="untrusted">`, with closing tags escaped.
- **Phase 4: running sessions.** No new state machine is added.
  - `cranibox-node peer discover-sessions` passively lists running Claude Code and Codex sessions from their transcript files (`~/.claude/projects`, `~/.codex/sessions`, written within the last 30 minutes). It reports id, harness, cwd, repository name, branch, activity (`working` within 90 s, otherwise `idle`) and last event kind. It is local only, conversation content is never read or returned, and it is not exposed over the mesh.
  - `cranibox-node peer messages hook` is a command hook for both harnesses. On `UserPromptSubmit`/`SessionStart` it adds `additionalContext`. On `Stop`, only if configured with `--wake-on-stop`, it continues the turn, and never when `stop_hook_active` is set. It injects only messages that the inbound policy accepted or the owner released, that are addressed to that session (`target_session`) and not yet acknowledged. They stay framed as untrusted and are acknowledged atomically, so each is delivered once.
  - Delivery is capped at 8 messages / ~32 KB per turn. The hook always exits 0.
  - `peer messages hook-config --harness claude|codex [--wake-on-stop]` prints the settings. Codex requires trusting the hook in `/hooks`. Paths are quoted per platform. On macOS/Linux they use POSIX single quotes. On Windows, Claude Code gets exec form (`command` plus `args`, no shell), and Codex gets cmd.exe double quoting. Codex refuses paths containing `%`.
  - The MCP `device_messages` tool remains the fallback. The Codex app-server is not used; Codex hooks cover delivery.
  - The tray menu's first item shows how many messages await approval and opens the window. On Linux the count is also shown next to the icon, because appindicator has no tooltips. On macOS and Windows the tooltip shows it too.
