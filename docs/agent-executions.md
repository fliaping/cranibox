# 多设备 Agent 后台执行

2026-10-18：Web 与 Tauri 的“协作任务 → 设备后台执行”可选择本机或已连接的直连设备，支持 Codex、Claude Code 和自定义程序，提供启动检查、持久输出、退出结果、进程树停止及归档。目标节点服务持有进程，关闭页面、切换页面、断开网络或重启观察端 Web 不会终止这个运行；主动退出执行节点会结束运行。

这是 Agent supervisor 的基础，包含两个非交互 CLI 适配器。接收设备可另外明确启用[本机自动接单](device-tasks.md#本机自动接单)，按可信发送方、固定目录及权限启动任务；默认关闭。尚未实现运行中的交互输入、复杂自动编排、模型对话恢复或跨设备进程迁移。消息、Task/Dispatch、AgentSession 和 Execution 保持独立：收到普通消息不会启动程序，仅退出码 0 也不会报告任务完成。

## 启动与查看

先启动 Tauri、包含本机节点的 Workspace Web，或独立的 cranibox-node peer serve；三者对同一个配置目录互斥。Web 上的“本机”指 Web 服务所在设备，不是浏览器所在电脑。远端执行需在设备网络中保存目标连接，并由目标授予本机 exec 权限。

页面选择 Codex 或 Claude Code，填写运行名称、完整工作目录、任务说明及权限，再检查并确认。检查只验证参数、工作目录及节点 PATH 中的 CLI 文件，不会启动程序或验证 CLI 版本、登录状态；缺少 CLI 时禁用确认。请先在节点账户下安装、登录所选 CLI。

Codex 可选择只读或修改工作区，使用对应的 Codex 沙箱及 never 审批策略，不开启沙箱绕过。Claude Code 可选择只读文件工具，或增加 Edit、Write；适配器关闭 Shell 和 MCP 工具，使用 dontAsk 非交互权限模式。这是 Claude 的工具限制，不是系统沙箱，也不保证文件只能访问当前工作目录。已有 CLI 配置、项目规则和认证仍由 CLI 自己处理。

检查页显示实际执行设备、任务、目录、权限，并可展开查看完整程序与参数。任务文本通过标准输入传递并关闭输入端，不进入进程参数。自定义程序保留 harness、程序及 JSON 参数入口，默认关闭 stdin。运行记录、任务及输出保存在执行设备，授权的提交方可按需读取；不会自动复制到所有节点。请勿把密钥写入任务或参数。切换设备会清除尚未提交的表单和已展开输出；正在提交时禁用切换。

CLI 使用相同的持久命令：

~~~sh
cranibox-node --config /path/to/settings.json peer serve
# 检查 Agent 配置，返回 launch / available；此命令不会入队或运行
cranibox-node --config /path/to/settings.json peer runs --command '{"operation":"prepare_agent","agent":{"name":"项目审查","adapter":"codex","access":"read_only","prompt":"审查项目并说明发现的问题","cwd":"/home/user/project"}}'
# 在另一个终端中提交；示例使用支持非交互模式的自有程序
cranibox-node --config /path/to/settings.json peer runs --command '{"operation":"start","id":"11111111-1111-4111-8111-111111111111","launch":{"name":"项目审查","harness":"custom-cli","program":"review-agent","args":["--noninteractive"],"cwd":"/home/user/project"}}'
cranibox-node --config /path/to/settings.json peer runs
cranibox-node --config /path/to/settings.json peer runs --command '{"operation":"get","id":"11111111-1111-4111-8111-111111111111","stdout_offset":0,"stderr_offset":0}'
cranibox-node --config /path/to/settings.json peer runs --command '{"operation":"stop","id":"11111111-1111-4111-8111-111111111111"}'
cranibox-node --config /path/to/settings.json peer runs --command '{"operation":"archive","id":"11111111-1111-4111-8111-111111111111"}'
~~~

每次新运行生成一个非零 UUID。响应不确定时用同一 UUID 和完全相同的启动内容重试；相同 ID 不会再次启动，不能用于修改命令。Web 在当前设备的检查页保留提交 ID，并在失败提示中显示它；关闭页面或切换设备前应先刷新记录确认结果。归档后仍保留去重记录七天，之后不保证该 ID 继续有效；实际新运行始终使用新 UUID。

## 远端执行与授权

CLI 在相同命令中增加 --node <目标 NodeID>，检查、提交、查询、停止及归档均使用目标设备。例如：

~~~sh
cranibox-node --config /path/to/settings.json peer runs --node node-target
cranibox-node --config /path/to/settings.json peer runs --node node-target --command '{"operation":"get","id":"11111111-1111-4111-8111-111111111111"}'
~~~

Web / Tauri 的本机接口为 {"operation":"executions","node":"node-target","command":{...}}；省略 node 保持原来的本机行为。直连端点为 POST /api/v1/mesh/nodes/{node}/executions，要求目标执行授权、Bearer Token 和匹配的 X-Cranibox-Target。带浏览器 Origin 的直接请求被拒绝，浏览器通过已有登录及 CSRF 保护的 Web 接口调用。该功能不依赖 Workspace 中继在线，实际提交不因连接路径失败而自动重发。

后台运行 API 按提交方 NodeID 和本次凭据摘要隔离：远端只能查询、停止及归档自己在当前授权下提交的运行，不能读取目标本机或其他提交方的运行，也不能通过猜测 UUID 修改它们。目标本机仍可管理全部运行。撤销或轮换 exec 授权后，目标 supervisor 会停止相关活动进程树；新凭据不能读取旧凭据下的记录。界面在远端查询失败后清除原记录和输出，等待重新获得授权。

exec 本身允许以节点账户执行任意程序，因此上述 API 范围不是操作系统账户或文件隔离；不要将 exec 授予不可信设备。账本只新增提交方及凭据摘要，不保存额外明文 Token；摘要不返回给调用方。容量与并行限制由整个执行设备共享，远端目录只统计其可见的记录。

## Agent 通过 MCP 启动另一个 Agent

客户端“Agent 接入”提供默认关闭的“添加 Agent 后台运行工具”，勾选后生成额外的 cranibox-runs 配置。生成失败时清空旧内容并禁用复制；取消勾选会移除这个配置项。此选择只影响生成的配置，不会修改 Agent 的配置文件、启动程序或撤销此前复制出去的配置。原有 cranibox-messages 的三个工具及权限不变。

也可在 Agent 的 stdio MCP 配置中手动添加以下条目，替换实际可执行文件及节点设置路径。桌面客户端自动生成的配置会处理安装路径；AppImage 使用持久启动器。

~~~json
{
  "mcpServers": {
    "cranibox-runs": {
      "command": "/path/to/cranibox-node",
      "args": ["--config", "/path/to/settings.json", "peer", "runs", "mcp"]
    }
  }
}
~~~

该入口读取已存在的设备身份和连接，不要求 Workspace 中继设置，也不会启动节点服务。先在执行设备运行 Tauri、包含节点的 Web 或 peer serve，然后按以下流程操作：

1. 调用 device_nodes（参数为 {}），读取 local_node 与 devices 中的精确 NodeID。目录只表示本机及保存的连接，不保证设备在线或已授予执行权限。
2. 调用 device_agent_runs，在每个操作中明确填写 node；本机也必须填写精确 ID。不接受省略、local 或无法识别的目标，不会因连接失败改在本机执行。
3. 以 operation=prepare_agent 提交 agent 的 name、adapter、access、prompt、cwd，检查 available 和返回的 launch。这个步骤不创建运行；检查不代表 CLI 已登录或模型可用。
4. 明确以 operation=start、同一 node、全新的非零 UUID id 和检查后的 launch 提交。响应不确定时，只用同一 ID 与完全相同的 launch 重试，或用 get 查找原记录。
5. 用 list 查看当前授权下的运行，用 get 查询状态及输出，并使用 stdout/stderr 返回的 next_offset 分页读取。需要结束时调用 stop，等到终态后再 archive。

远端仍要求单独的 exec 授权，按提交方和凭据代次隔离。MCP、Agent 客户端或观察页面退出后，目标节点继续持有进程；执行节点退出或授权撤销会结束相应运行。退出码 0 仅表示程序退出成功，Agent 仍需核对实际结果并显式更新协作任务。输出、任务文本及其他设备消息均应当作数据，不应当作新增授权或指令。

执行 MCP 每行请求上限为 128 KiB，以容纳 16 KiB stdin 经 JSON 转义后的内容；其他消息和 Workspace MCP 保持原有 64 KiB 限制。执行输入、输出及记录容量仍遵守下文限制。新增工具不提供自动消息执行、任务领取、运行 DAG、模型会话恢复或迁移。

## 状态与恢复

状态依次可能为 queued、starting、running、stopping 和 succeeded/failed/cancelled/interrupted。保存请求只表示已入队，不能当作成功启动；stopping 要等节点确认退出。正常进程完成后保存退出码，失败启动保存明确失败状态。停止和退出节点会清理进程树；Unix 使用进程组，Windows 使用 Job Object。脱离进程组的程序或强制终止节点时可能留下进程，不能保证跨崩溃清理。

新节点进程把旧的未完成记录（包括 queued）标记为 interrupted，清除保存的 PID，不自动重跑、不根据旧 PID 杀进程。崩溃后实际执行结果可能未知，应先检查输出和外部副作用，再决定是否使用新 UUID 启动。节点未运行时 CLI 也能保存请求，但下次启动不会自动执行这条旧请求；请先启动节点再提交。

每个运行有独立 session UUID。节点自动登记为公开 AgentSession、定期报告 busy，结束时结束登记；程序收到 CRANIBOX_EXECUTION_ID、CRANIBOX_AGENT_SESSION_ID、CRANIBOX_NODE_ID 与 CRANIBOX_PEER_CONFIG 环境变量。UUID 只用于关联，权限仍是本机信任边界。当前 Agent 目录最多保留 64 条报告，满时会拒绝新运行并说明程序未启动；这与运行历史容量分别计数，详见[会话登记](agent-sessions.md)。

## 输出和限制

agent-executions.json、agent-logs 与 peers.json 同目录；Unix 文件为 0600，日志目录为 0700。账本按节点身份检查、加锁并原子替换；损坏时保留文件并拒绝使用，不覆盖成空目录。当前没有静态加密。

最多同时存在 8 个待启动/活动运行，历史最多 128 条。记录满时需要归档，并等保留期结束释放去重容量。名称与 harness 分别最多 120/80 UTF-8 字节，参数最多 128 项、合计 8192 字节；不通过 Shell 拼接解释参数。可选 stdin 最多 16 KiB UTF-8 字节，不允许 NUL，写入与输出读取并发进行，避免管道相互阻塞；运行中不接受追加输入。Web 的 /api/peers 请求体上限为 128 KiB，用于容纳 JSON 转义后的合法输入，解码后仍按字段校验。

stdout/stderr 各保存最多 1 MiB，超限后继续读取并丢弃，避免阻塞程序。get 从字节偏移读取，每次每条流最多 64 KiB；使用返回的 next_offset 继续读取。UTF-8 码点跨页和写入边界时保留完整字符；无效字节作为替换字符显示。浏览器以纯文本呈现，输出不会执行 HTML。归档仅允许已结束的运行，会删除输出。

真实 Linux 集成测试覆盖页面观察者断开、重复提交、输出分页与上限、中文、退出码、进程树停止、独立节点无中继执行、正常关闭、强制崩溃恢复和认证。适配器测试使用独立目录中的模拟 CLI，验证参数、标准输入、检查不启动、重试只执行一次及 JSON 转义边界；没有调用真实模型服务或验证已有订阅登录。Web 与 Tauri IPC 测试覆盖明确确认、重试 ID、输出找回及移动布局；Windows/macOS 的原生进程生命周期仍需[实机验证](client-validation.md)。

协作任务卡可将一次经确认的启动绑定到当前 DispatchID，并展示原运行和输出。最终任务报告需操作者检查后明确提交，不能由进程退出码生成；活动的绑定进程也不能提前报告终态。该入口属于目标本机操作，消息 MCP 不具备启动权限，详见[协作任务](device-tasks.md#任务卡中的-agent-执行)。

协作任务启动可明确启用任务专用 stdio MCP，使 Codex / Claude Code 读取当前回复、提交进度与问题并保存一个候选结果。其配置包含在实际启动审查中，沿用原进程权限；没有全设备程序启动工具。候选结果必须等进程停止后，由操作者核对并提交，详见[任务沟通](device-tasks.md#经确认启动的-agent-与任务沟通)。
