# Agent 会话登记、消息与定向任务

2026-10-17：Web 和 Tauri 共用“协作任务”中的会话目录，以及任务与设备消息中的 Agent 选择器。Agent 在所属设备通过本机 CLI 或 MCP 明确登记；拥有该设备消息权限的节点可以读取公开目录，并把任务指定给一个已登记的 session UUID。中继关闭后，已有直连、EasyTier 或 Tailscale 路径仍可使用。

这是主动登记协议。它不会扫描系统进程、启动模型、恢复对话或证明一个 Agent 仍然存活。session UUID 是路由与任务归属标识，不是认证凭据；受信任的本机 CLI/MCP 可以提供这个 UUID。当前未按不同本机 Agent 隔离权限，也未把 UUID 与 harness 进程、OS 用户或公钥绑定。

## 登记自己的会话

沿用[设备消息](device-messages.md)的独立 MCP 入口：

~~~json
{"mcpServers":{"cranibox-messages":{"command":"cranibox-node","args":["peer","messages","mcp"]}}}
~~~

此入口提供 device_messages、device_tasks、device_agent_sessions 三个工具，不需要 Workspace 地址。以下是 device_agent_sessions 的参数：

~~~json
{"operation":"register","id":"33333333-3333-4333-8333-333333333333","name":"构建检查 Agent","harness":"custom-cli"}
{"operation":"list"}
{"operation":"heartbeat","id":"33333333-3333-4333-8333-333333333333","revision":1,"status":"busy"}
{"operation":"end","id":"33333333-3333-4333-8333-333333333333","revision":2}
~~~

- 生成并保留非零 UUID。name 与 harness 是公开文本标签，分别限制为 120、80 UTF-8 字节；不要放路径、凭据、会话正文或个人隐私。
- register 首次创建 ready、revision 1 的记录。同一 ID 和相同标签重复登记返回已有记录，不续期、不恢复已结束的登记；相同 ID 改标签会拒绝。
- 每两分钟读取并使用当前 revision 调用 heartbeat，报告 ready、busy 或 waiting。每次成功更新后 revision 加一，五分钟未续期则过期。状态是执行方自报，不代表系统实际空闲。
- 心跳响应不确定时，复用原 revision 和相同状态重试；此重试不延长原租期。若已经发生其他更新，则重新读取 revision，再决定是否报告。旧的不同状态不能覆盖较新的报告。
- end 需要当前 revision，永久结束保留期内的这个 ID。响应不确定时可复用原 revision 重试。下一次工作使用新 UUID；结束登记不会杀死进程，也不会替任务提交结果。

本机 CLI 使用相同命令：

~~~sh
cranibox-node peer agent-sessions
cranibox-node peer agent-sessions --command '{"operation":"register","id":"33333333-3333-4333-8333-333333333333","name":"构建检查 Agent","harness":"custom-cli"}'
cranibox-node peer agent-sessions --command '{"operation":"heartbeat","id":"33333333-3333-4333-8333-333333333333","revision":1,"status":"busy"}'
cranibox-node peer agent-sessions --node NODE_B
~~~

MCP 远端查询使用 {"operation":"list","node":"NODE_B"}。远端仅支持读目录，不能代替别的设备登记、续期或结束。接收入站查询仍需要目标节点监听；本机登记不需要监听或网关设置。

## 自动发现运行中的会话

不想手动登记时，`cranibox-node peer discover-sessions` 会根据 Claude Code / Codex 自己写下的会话文件列出本机正在运行的会话（含仓库、分支和活跃度），无需心跳。已放行的定向消息可以通过 hook 送入这些会话，见[设备消息](device-messages.md#投递到运行中的-agent-会话阶段-4)。

## 关闭页面后的同步与离线快照

2026-10-19：Web 和 Tauri 的“协作任务 → 所有设备的 Agent → 后台 Agent 目录同步”共用设置。默认关闭；明确选择 1–32 个已保存连接的设备，按 15、30、60 或 300 秒读取。目标仍须授予消息权限，联网本身不授予读取能力。此设置与项目目录同步独立，两类后台请求共用最多四个并发名额。

同步由 Rust 节点宿主运行，关闭标签页、收起窗口或不打开任务页后继续；节点程序重启后恢复已保存的策略。退出节点程序或系统休眠时不能同步。暂停会阻止后续请求，已在途的只读请求可以完成；已有快照继续保留。CLI 使用相同本机设置：

~~~sh
cranibox-node peer agent-sync
cranibox-node peer agent-sync --command '{"operation":"configure","revision":0,"nodes":["NODE_B"],"interval_seconds":60}'
cranibox-node peer agent-sync --command '{"operation":"pause","revision":1}'
~~~

先读取当前 revision；configure 和 pause 都拒绝过期版本。配置为 peers.json 同目录的 agent-sync.json，运行状态为 agent-sync-state.json；Unix 文件权限为 0600。状态包含各设备的尝试时间、成功时间、登记数量与错误类别，不含 Token 或连接摘要。损坏或属于其他 NodeID 的策略保留原文件并停止该类同步，不影响独立的项目同步。

前台远端目录读取也会保存本机快照，不要求先启用后台同步。快照位于 `<peers.json 文件名>.agent-snapshots/<目标 NodeID 的 SHA-256>.json`，Unix 权限为 0600，上限 128 KiB 加 4 KiB 元数据；不做静态加密。每个来源只保留最近一次经过身份与格式校验的公开登记，不保存对话、密钥或任务正文。

远端读取返回本机添加的 sync：state 为 online 或 offline，synced_at 为 Unix 毫秒；本机目录和入站服务端目录保持原格式。仅在连接超时、断开等可重试的网络故障时回退到与当前连接绑定的快照。快照不更新 observed_at、Agent 报告时间或五分钟租期。页面明确显示“离线快照”、上次同步时间与“状态未确认”；不计为当前登记，不允许据此选择 Agent、发送定向消息或委托。恢复联网并重新读取后才重新判断。CLI/MCP 调用方也必须检查 sync，不能把离线记录当作当前状态。

授权拒绝、身份错误、无效目录均清除该来源快照并报错。移除连接或为已有连接保存新凭据时，本机也会清除快照；主要地址、凭据和本机/目标身份变更后，后台策略要求重新核对所选设备。请求结束后再次校验连接，旧响应不能恢复已经移除的快照。离线设备无法即时获知另一端的撤权，已下载记录也不是可远程擦除的数据；显示的历史标签不赋予任何远端操作权限。

这是按来源保存的只读目录。任务、消息和协作 Run 仍由各自节点持有，没有实现全网复制、离线多方写入合并、进程迁移或尚未接入设备的自动发现。

## 指定执行 Agent

“协作任务”的“所有设备的 Agent”同时汇总本机和已保存连接的设备，不需要逐个切换目标。每个设备独立读取公开登记，最多并行四个前台请求；慢设备不会阻止其他已返回的来源展示或刷新。可按公开名称、harness 或设备 ID 查找，默认显示有效登记以及明确标注的未结束离线记录，也可展开过期和结束的记录。设备身份、目录格式与租期均校验；无可用快照或读取被拒绝时清除该来源的页面记录，保留错误和重试入口，其他来源继续可用。

点击远端卡片的“委托给此 Agent”只填写执行设备和会话、保留标题与说明草稿，并聚焦标题。选择器会重新读取目标目录，确认前禁止提交；重复点击同一会话也会重新检查。页面不自动创建任务或启动进程。本机卡片只显示登记，“登记本机 Agent”复制现有 MCP 说明，不安装或配置外部 Agent。

在“协作任务”选择设备后，再选择一条有效登记。过期、结束或当前未能确认的会话不可新提交；目录读取失败会保留原选择并禁止提交，不悄悄转成任一 Agent 可领取。可以明确改选“设备级委托”，继续沿用旧的任一会话领取模式。切换执行设备会清除上一个设备的会话选择。

CLI/MCP 的 create 可附加 target_session：

~~~json
{"operation":"create","id":"11111111-1111-4111-8111-111111111111","dispatch":"22222222-2222-4222-8222-222222222222","node":"NODE_B","target_session":"33333333-3333-4333-8333-333333333333","title":"检查构建","description":"读取构建日志并报告证据。"}
~~~

调用的是 device_tasks；后续领取、进度、结果见[协作任务](device-tasks.md)。目标收到任务后先持久保存。首次 claim 必须使用 target_session 对应的 UUID，且目标本机登记尚未过期或结束。其他 UUID 会拒绝；并发领取仍由任务锁保证唯一归属。程序调用可为离线目标预先入队，实际领取时再检查登记。

target_session 随任务内容持久保存，不能在相同 TaskID 下替换，也不能在创建新尝试时改变。已经领取的任务不因登记过期或 end 自动失败；原领取者仍可重复读取领取记录、报告进度及最终结果。取消仍是合作请求，必须得到明确结果。

本功能不会自动唤醒、启动或向 Agent 输入指令。请把页面“复制 Agent 领取说明”交给已配置本机 MCP 的 Agent；它在原有权限下读取并决定处理任务。

## 按会话收发消息

设备消息可指定 sender_session 和 target_session。Agent 使用自己的 session UUID 调用 device_messages 的 {"operation":"list","session":"SESSION_UUID"}，筛选明确发给自己的来信及自己发出的消息；完整设备收发箱仍可由本机访问。省略会话的旧消息继续进入设备收件箱。

与任务首次领取不同，消息入队和接收不要求会话处于有效登记中，可提前离线投递；登记结束或过期后地址不变，不自动转给另一个 Agent。收件回执只证明持久保存，不能当作 Agent 已读取。页面只允许向当前已确认的有效登记新提交，回复保留原发件会话。具体 CLI、MCP 与边界见[设备消息](device-messages.md#会话筛选与离线地址)。

## 授权、存储与页面

目标设备的消息权限同时允许查询公开会话目录、投递消息与任务。旧授权默认不增加消息权限，需要显式签发；它不授予命令、终端、项目或桌面访问。撤权与令牌轮换在每次目录请求时重新检查。

GET /api/v1/mesh/nodes/{NodeID}/agent-sessions 返回 owner、服务端 observed_at、lease_secs、capacity，以及公开记录。直连接口校验目标身份和令牌，拒绝浏览器 Origin；没有远端写接口。Web 使用同源登录后的 POST /api/peers，Tauri 使用本机 IPC，调用相同 Rust 命令。

agent-sessions.json 与 peers.json 同目录，Unix 使用 0600、文件锁和原子替换。损坏、超限或属于其他 NodeID 的内容会保留并报错，不覆盖成空目录。每台设备最多 64 条记录、128 KiB；新登记需要容量时清理过期超过七天的记录。数据没有静态加密，不把它作为永久审计日志或不可复用身份凭证。

页面显示公开名称、harness 标签、UUID、最近报告时间与状态。在线目录按服务端观测时间及浏览器单调时钟计算剩余租期，减少浏览器时钟偏差；即使暂停自动刷新，过期提示仍继续计时。来源每五秒前台读取，隐藏页面暂停页面发起的新请求；已明确启用的节点后台同步继续。移除或更换连接后旧响应不会重新加入视图；授权变更事件也会使目录失效重读。浏览器不持久保存目录，离线快照由当前入口的 Rust 节点保存，规则见上节。

Rust 测试覆盖租期、并发旧修订拒绝、幂等、容量及损坏保留；真实双节点 HTTP/CLI/MCP 验证关闭中继、重启、定向领取、无反向路径结果回传及撤权。浏览器验证选择保留、过期阻止提交、纯文本与设备切换，Tauri 模拟 IPC 验证共用功能。节点已支持[明确启动的本机后台执行](agent-executions.md)，自动登记并续期对应会话；原生各平台实机、自动进程发现、更多 harness 适配器、交互输入和运行中接续仍待交付。
