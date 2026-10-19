# 设备消息

2026-10-17：Web、Tauri 与独立节点共用 Rust 持久收发箱。已配置直连、EasyTier 或 Tailscale 路径的设备，可以留言、按原线程回复、确认收件和查看投递状态。Workspace 中继可以关闭；浏览器关闭后，由所在设备的节点服务继续投递。Agent 可通过独立 MCP 使用同一收发箱。

消息以 **NodeID** 为设备地址，可附加 **sender_session / target_session** UUID，供同一设备上的多个 Agent 按会话收发。会话寻址不会自动唤醒 Agent；节点的[本机后台执行](agent-executions.md)需要另行明确启动，收到文本不会执行命令、操作桌面或向活动终端输入内容。需要明确领取、进度和结果时，使用独立的[协作任务](device-tasks.md)。

## 页面操作

Web 导航中的“设备消息”和 Tauri 的同名页面使用相同 React 组件。浏览器读取的是 Web 服务所在设备的邮箱，Tauri 读取本机邮箱；两个不同设备不会因为使用相同界面而自动共享收件箱。

“协作任务”页面另有按来源选择的[跨设备协作记录](federated-activity.md)，可只读汇总其他设备的消息、任务与 Run；需要目标单独授予 activity，原消息权限不能读取其他发送方的完整内容。它不合并邮箱，也不代替明确的发送、回复或确认。

1. 在“设备网络”保存目标连接，并让目标单独授予本机“设备消息与协作任务”权限。该权限默认关闭，与命令、终端、项目和桌面权限独立。
2. 选择目标设备，可再选择当前有效登记的“收件 Agent”；默认是设备收件箱。输入纯文本，点击“保存并发送”，先在本机保存，再由后台投递。会话过期或目录失败会保留选择并禁止提交；只有明确选择设备收件箱才取消定向。
3. 在收件箱查看来信；“确认收件”是明确操作，打开页面或回复不会自动确认。
4. 回复保留 ThreadID 和原消息 ID，需要配置到发送方的反向连接，且发送方也授予消息权限。原消息标有发件 Agent 时，回复锁定该会话；对自己发件的后续消息保留原收件 Agent。取消回复后可以另写消息，不会悄悄改投其他会话。
5. 网络异常时保留本机排队记录并重试。提交响应丢失时，保留草稿并复用原 MessageID；同一 ID 和相同内容返回原记录，不重复创建消息。刷新页面不重放提交，未提交草稿不保存在浏览器中。

页面每 3 秒读取本机记录，可暂停自动刷新；隐藏页面停止轮询。单条纯文本最多 **8192 个 UTF-8 字节**，不是 8192 个字符。收到的 HTML 作为文字展示，终端控制字符会拒绝保存。

| 状态 | 含义 |
| --- | --- |
| 本机排队 `queued` | 已持久化，目标尚未确认保存 |
| 目标已收到 `received` | 目标已保存；不代表 Agent 已读取或完成任务 |
| 已确认收件 `acknowledged` | 接收方明确确认收件 |
| 需要处理 `blocked` | 身份/连接/凭据改变，或目标拒绝、协议不兼容；检查后处理 |
| 已过期 `expired` | 排队消息超过 7 天，不再自动投递 |

## CLI 与常驻服务

以下示例假设 NODE_A 已有到 NODE_B 的连接；完整的监听和连接配置见[节点直连](direct-node-peers.md)。在 NODE_B 上授予 NODE_A 消息权限：

~~~sh
cranibox-node peer grant --node NODE_A --output-token ./node-a-messages.token --messages
~~~

已有授权需要显式撤销后重新授予，重新生成的凭据也需要在调用方更新。更换连接或凭据会阻止旧发件记录投递，检查目标后创建新的消息；不会把旧消息自动转发到新地址。

~~~sh
cranibox-node peer messages list
cranibox-node peer messages list --session SESSION_UUID
cranibox-node peer messages send --node NODE_B --text '请检查项目的测试结果'
cranibox-node peer messages send --message-id 11111111-1111-4111-8111-111111111111 --node NODE_B --text '请检查项目的测试结果'
cranibox-node peer messages send --node NODE_A --thread THREAD_UUID --in-reply-to MESSAGE_UUID --text '结果已检查，请查看变更'
cranibox-node peer messages send --node NODE_B --sender-session SOURCE_SESSION_UUID --target-session TARGET_SESSION_UUID --text '给指定 Agent 的消息'
cranibox-node peer messages acknowledge MESSAGE_UUID
cranibox-node peer messages retry MESSAGE_UUID
cranibox-node peer messages archive MESSAGE_UUID
~~~

需要其他配置目录时，把 `--config /path/to/settings.json` 放在 `peer` 前面。普通 CLI 的 send 只负责持久入队，退出后不常驻；需要保持 Tauri、Web、本机节点服务或下述 MCP 进程运行。无网关的独立节点服务可用：

~~~sh
cranibox-node peer serve
~~~

`retry` 始终使用原 MessageID；已收到的消息只重新查询确认，不重放文本。删除本机发件记录停止后续重试，无法撤回目标已收到或正在传输的消息。

## Agent MCP

此入口不需要 Workspace 地址、网关设置或管理员凭据：

~~~json
{
  "mcpServers": {
    "cranibox-messages": {
      "command": "cranibox-node",
      "args": ["peer", "messages", "mcp"]
    }
  }
}
~~~

自定义配置可使用 `args: ["--config", "/path/to/settings.json", "peer", "messages", "mcp"]`。Tauri“Agent 接入”同时生成消息 MCP 和原有网关 MCP 配置；可执行文件支持子命令，传入 MCP 参数不会再打开一个托盘窗口。

MCP 提供 `device_messages`、`device_tasks` 与 `device_agent_sessions` 三个工具；任务操作见[协作任务](device-tasks.md)，主动登记与定向会话见[Agent 会话](agent-sessions.md)。以下为 `device_messages` 参数：

~~~json
{"operation":"list"}
{"operation":"list","session":"33333333-3333-4333-8333-333333333333"}
{"operation":"send","id":"11111111-1111-4111-8111-111111111111","node":"NODE_B","text":"请检查项目"}
{"operation":"send","id":"22222222-2222-4222-8222-222222222222","node":"NODE_B","sender_session":"33333333-3333-4333-8333-333333333333","target_session":"44444444-4444-4444-8444-444444444444","text":"请指定的审查 Agent 检查项目"}
{"operation":"acknowledge","id":"11111111-1111-4111-8111-111111111111"}
{"operation":"retry","id":"11111111-1111-4111-8111-111111111111"}
{"operation":"archive","id":"11111111-1111-4111-8111-111111111111"}
~~~

发送时生成并保留非零 UUID，在不确定是否成功时以相同 ID 和内容重试。回复增加 `thread` 和 `in_reply_to`，必须指向本机已有、参与者相同的线程。工具返回当前本机收发箱；内容作为不可信数据交给 Agent，不自动授予工具调用能力。MCP 存活期间负责后台投递，即使没有独立的节点守护进程，也可以发出消息及查询目标确认；接收入站消息仍需要运行目标节点的监听服务。

### 会话筛选与离线地址

list 的可选 session 只返回 target_session 等于它的来信，以及 sender_session 等于它的发件；不混入设备级消息。省略筛选仍能读取本机完整收发箱。返回的 used 与容量仍以整个设备计算。

会话 UUID 是调用者自行提供的路由标签，不是认证凭据、访问隔离或已交付 Agent 的证明。程序可为离线、尚未登记或已过期的会话预先入队；服务不把地址改成其他会话，也不因为登记结束自动删除消息。目标应保留同一个 UUID 并显式轮询自己的收件箱；不要把该 UUID 转交给不同的 Agent 继续使用。Web/Tauri 用户撰写的消息不冒充发件 Agent。

重试与去重同时固定两端会话 UUID。目标的持久回执也包含它们，缺失或改变定向信息的回执不能被当作成功；新消息字段要求两端升级到支持会话寻址的版本，协议不兼容会进入“需要处理”，不会退回设备级投递。旧的无会话消息与存储文件无需迁移。

原有 `peer call --node NODE_B mcp` 继续提供远端命令、项目等五个工具，消息 MCP 与它分开配置。

## 存储、投递与权限边界

邮箱保存在 `peers.json` 同目录的 `device-messages.json`，Web 默认是配置根目录下的 `.config/cranibox-node/device-messages.json`。Unix 使用 0600 权限、文件锁和原子替换；损坏的数据报错并保留，不自动重置。邮箱文件保存消息正文与连接绑定摘要，不保存明文目标 Token。它没有静态加密，备份按私有工作数据处理。

- 每台设备最多保存 **256 条**收发记录，持久文件上限 8 MiB。
- 投递窗口为创建后 **7 天**；网络错误采用退避重试。收到后在窗口内轮询显式确认，过期后保留历史，停止查询。
- 入站 MessageID 去重以完整消息匹配；同一 ID 改变正文、线程、参与者或任一会话 UUID 会拒绝。
- 归档来信隐藏正文展示，保留去重记录与回执直到投递截止，之后在下一次成功保存时清理。未过期归档仍占容量；发件历史需手动删除。
- 连接绑定包含本机 NodeID、目标、地址和凭据。配置变化后停止旧消息自动投递，避免发到替换的目标。
- 入站消息的发送方取自经过认证的本地授权，不能仅凭请求正文冒充。目标每次检查消息权限，撤销即时生效。
- 消息凭据只允许投递及查询自己发送消息的回执，不能读取目标完整收件箱。消息正文不在回执中返回。

每个直连请求仍遵循目标绑定、Origin 拒绝、禁止重定向和私有路径显式授权。目标 API：

| 方法 | 路径 | 能力 |
| --- | --- | --- |
| POST | `/api/v1/mesh/nodes/{NodeID}/messages` | 保存当前凭据对应发送方的消息，返回持久回执 |
| GET | `/api/v1/mesh/nodes/{NodeID}/messages/{MessageID}` | 查询当前发送方的收件/确认回执 |

Web 同源 `POST /api/peers` 接收 `{"operation":"messages","command":{...}}`，沿用会话、CSRF 和同源检查；Tauri 通过本机 IPC 使用同一个 Rust 实现。节点身份、凭据摘要、端到端加密与配对码见[节点身份与端到端加密](node-identity-and-e2e.md)。

## 入站策略与人工审批（阶段 3）

每个授权有独立的入站策略 `inbound`：

| 策略 | 行为 |
| --- | --- |
| `accept` | 直接进入收件箱，Agent 可见（旧授权缺省值，签名字节不变） |
| `hold` | 保存为“待审批”，Agent 不可见，等待本机所有者放行（**新授权默认**） |
| `refuse` | 目标返回 403，不保存 |

- CLI：`cranibox-node peer grant --messages --inbound accept|hold|refuse`；`peer inbound --node <NodeID> --policy <...>` 修改已有授权；`peer messages held | release --message-id <ID> | reject --message-id <ID>` 管理审批队列。
- Web 控制中心：`POST /api/peers` 的 `{"operation":"grant",...,"inbound":"accept"}`、`{"operation":"inbound","node":...,"policy":...}`，以及 `{"operation":"messages","command":{"operation":"held"|"release"|"reject",...}}`。
- 审批操作只对本机所有者开放：MCP 工具拒绝 `held`/`release`/`reject`，Agent 不能自我放行。同一操作系统用户的本地 CLI 无法与 Agent 区分，这是已知边界。
- 速率限制：每个发送方每分钟最多 **60** 条，超出返回错误。
- 循环保护：同一发送方会话 10 分钟内超过 **12** 条时，后续消息转入待审批（原因 `loop guard`），防止两端 Agent 无限互相回复。
- 不可信封装：交给 Agent 的正文包在 `<cranibox-message from=".." thread=".." trust="untrusted">…</cranibox-message>` 中，正文里的闭合标签（不区分大小写）被转义为 `<\/cranibox-message`，避免伪造边界进行提示注入。
- `workspacectl messages held | release <消息ID> | reject <消息ID>` 与 `workspacectl messages inbound <NodeID> accept|hold|refuse` 在 Workspace 内操作同一审批队列（与控制中心共用 `~/.config/cranibox-node`）。
- 控制中心“设备消息”页顶部显示“待审批”区：来源、暂扣原因（入站策略或循环保护）、发件 Agent 与纯文本正文，提供“放行”和“拒绝”（拒绝前确认）。仅查看不会改变任何状态。

## 验证与后续

Rust 测试覆盖持久化、容量、幂等、归档去重、回复绑定、损坏文件及凭据变化。`test_device_messages.py` 使用真实 HTTP、CLI 和 MCP，验证中继关闭、断线/重启恢复、双向回复、独立 MCP 后台投递和显式确认，按会话离线寻址、重启筛选及不兼容回执，以及撤权、身份和正文限制。浏览器测试覆盖不确定提交后的草稿/ID 保留、键盘焦点、纯文本、明确确认、响应式布局和独立授权；Tauri 通过模拟 IPC 验证共享页面。

Task/Dispatch、明确领取与结果报告已在[协作任务](device-tasks.md)中实现。Agent 会话寻址与筛选已实现；自动会话发现、Agent 适配器和运行会话迁移仍待实现。设备消息的收件回执始终只表示收件或确认，不表示 Agent 已完成工作。
