# 跨设备协作记录

2026-10-19：Web 和 Tauri 的“协作任务”页面增加同一个只读汇总组件。它按来源展示任务、当前回执、问题与回复、结果、设备消息以及协作 Run。各节点保存自己的权威记录，不需要主节点，也不把远端记录写进本机任务库或邮箱。

## 开启与授权

页面默认折叠，不读取远端内容。展开后默认只读本机；用户选择已配置连接的来源，最多同时选择 16 台。正文需要展开单条记录查看，支持搜索和每页 20 条。类型筛选直接展示“全部类型、任务、消息、协作 Run”四个单选项，支持方向键和空格；更改搜索、类型或来源后回到第一页。Web 与 Tauri 共用控件，窄屏自动换行。相同 TaskID 在发送端和接收端是两份来源报告，分别保留来源，不凭时间戳合并成一条全局状态。

远端需单独授予“只读协作记录（包含其他发送方）”。这个权限包含目标节点当前可见的其他发送方消息正文、任务描述、问题、回答与结果，不按项目或发送方隔离。仅有原来的消息权限不能读取完整协作记录；仅有新的只读权限不能投递消息、任务或执行命令。旧授权默认 activity=false，不自动升级。

在目标 NODE_B 上：

~~~sh
cranibox-node peer grant --node NODE_A --output-token ./node-a-activity.token --activity
~~~

已有授权必须先显式撤销后重新授予，使用新的输出文件，并在调用方更新连接凭据；其他仍需要的能力须一起勾选。重新授权不会重新绑定旧的待投递任务或消息。连接配置参见[节点直连](direct-node-peers.md)。

在调用方 NODE_A 上：

~~~sh
cranibox-node peer activity
cranibox-node peer activity --node NODE_B
~~~

省略 node 读取本机，不需要给自己创建远端授权。CLI 可用于 Agent 的明确查询，本轮没有新增 MCP 工具。Web 的同源 POST /api/peers 接受 {"operation":"activity","node":"NODE_B"}，沿用登录、CSRF 和同源检查；Tauri 通过本机 IPC 调用同一个 Rust 实现。

## 协议与数据边界

目标提供 GET /api/v1/mesh/nodes/{NodeID}/activity；旧 /api/v1/nodes 前缀仍兼容。需要目标签发的 Bearer 凭据和匹配的 X-Cranibox-Target，拒绝浏览器跨站请求及重定向。节点探测响应的 activity 表示该凭据是否获准读取；旧节点不支持时显示读取错误。此路径没有写入操作。

响应包含 version=1、owner、observed_at、digest、tasks、messages、runs。每项携带原始 ID 与创建时间；任务只返回当前尝试和当前问答/结果，Run 返回来源拥有的范围、成员、状态与修订号。传输上限 24 MiB，最多 128 个任务、256 条消息和 32 个 Run，沿用来源账本的容量。归档来信和已归档任务不展示；仍属于未归档 Run 的发出任务继续保留，避免隐藏协作报告；已归档 Run 不返回。发件记录仍由邮箱已有生命周期管理。

不返回凭据、路由绑定、投递重试与错误详情、完整尝试历史、执行选项、工作目录或输出日志。任务描述、消息和报告可能自行包含敏感文字；只读授权不是脱敏或保密边界。页面只按纯文本展示，不执行其中 HTML、命令或工具调用。

digest 为 Rust 按确定字段顺序序列化 (version, owner, tasks, messages, runs) 后的 SHA-256；它排除 observed_at，所以重复读取相同内容不会伪造新的记录版本。它用于内容一致性检查，不是来源签名、全局时钟或不可抵赖证据。接收端严格反序列化并检查来源、参与者方向、唯一 ID、任务回执及摘要；连接、路径或凭据改变后的迟到响应拒绝接纳。任务与邮箱分别原子读取，整个响应不是跨账本事务快照。

读取不改来源账本，也不续期 Agent 登记。损坏账本报错并保留原文，不返回部分成功的空目录。远端错误、撤权或身份不符会清除页面上该来源旧内容；连接设置事件也会清空旧视图并忽略迟到响应。

## 页面刷新与后台同步

页面每 10 秒刷新，最多两个来源并发，同一轮不重叠。隐藏页面、收起面板或离页后停止页面自动读取；“暂停更新”停止页面定时读取，手动刷新仍可用。来源选择、搜索词和记录不写浏览器存储；刷新页面后重新选择来源。

“后台协作记录同步”是另一套默认关闭的节点设置，由 Rust 节点宿主执行，不依赖页面。先在目标取得独立 activity 授权，再在设置中明确选择最多 32 台已连接设备和 15/30/60/300 秒间隔。该选择允许将目标的任务、消息正文与 Run 完整只读快照保存到本机，可能包含目标与其他发送方的协作内容。仅手动读取、选择页面来源或已有 messages 授权都不会自动开启缓存。项目、公开 Agent 登记及协作记录三类后台同步共用最多四个并发请求。

暂停后台同步保留设备选择与快照，不再开始新的后台请求；手动读取仍可更新已选择设备的快照。取消选择某台设备会删除其快照；“清空快照并关闭同步”删除全部已选协作快照并停用本策略。节点重启后恢复策略，Web/gateway 停止但独立节点仍运行时也能继续。

CLI 先查询当前 revision，再提交设置；以下 revision=0 仅适用于从未配置的策略：

~~~sh
cranibox-node peer activity-sync
cranibox-node peer activity-sync --command '{"operation":"configure","revision":0,"nodes":["NODE_B"],"interval_seconds":60}'
~~~

Web 的同源 POST /api/peers 使用 {"operation":"activity_sync","command":{"operation":"status"}}；Tauri 使用相同管理命令。修改策略需携带当前 revision，冲突必须重新读取并核对，不自动覆盖。pause 操作保留选择；configure 配合空 nodes 数组则清空并停用。

快照位于 peers 配置文件旁的 .activity-snapshots 私有目录，文件以 NodeID 的 SHA-256 命名，采用 0600 权限；绑定接收方、来源、主连接地址、安全设置、凭据路径及内容。更换连接、凭据或移除设备后不能重用旧快照。请求返回时再次检查连接与策略 revision，避免清空、暂停或撤销选择后的迟到响应重新保存正文。

只有网络不可达或网关暂时不可用时才允许读取已选择设备的有效快照，响应额外带 sync.state=offline 和毫秒 synced_at。observed_at、正文与 digest 保持原来源观测值，界面明确显示离线及上次同步时间。成功在线读取使用 sync.state=online；本机保存失败仍显示实时内容，但附 warning 且不宣称保存成功。目标原始响应不允许自行注入 sync 字段；接收端也校验缓存大小、schema、来源和内容摘要。

目标拒绝读取、身份或正文无效会清除相应缓存及页面内容。离线期间无法立即知道远端已撤权；恢复联系后的认证拒绝才触发清除。用户可随时在本机清空快照；只读授权和本地文件权限不提供正文加密或远程擦除能力。

## 未实现项

这是统一只读视图与明确选择的离线快照，尚未提供完整的任务、消息、Run 复制协议、远端账本修改、冲突合并、离线写入同步、删除墓碑或跨节点撤权传播。来源已报告的状态不能证明 Agent 进程当前仍在运行；应回到任务/消息入口明确操作。公开 Agent 和项目目录仍使用各自独立的后台同步策略。

验证入口：tests/test_background_activity_sync.py、tests/test_federated_activity.py、三类共享并发回归 tests/test_background_agent_sync.py、Rust peers::activity 单元测试、Web tasks.spec.ts/direct-devices.spec.ts 以及 Tauri client.spec.ts。当前轮次结果和平台限制见[功能与验证交接](implementation-handoff-2026-10-18.md)，失败历史保留在[远端验证记录](remote-workspace-validation-2026-10-18.md)。
