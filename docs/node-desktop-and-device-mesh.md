# 客户端形态与 Agent 设备组网

每台设备运行同一个节点内核，没有中心节点：所有设备对等，经 iroh 配对或 EasyTier/Tailscale 直连互访。内核提供两种形态，功能与界面完全一致，只在系统集成上不同：

| 形态 | 适用 | 界面 |
| --- | --- | --- |
| 图形版 Cranibox（Tauri） | 有桌面的 macOS/Windows/Linux | 应用窗口；托盘、登录自启动、系统通知、原生权限 |
| 命令行版 `cranibox` | 服务器、无桌面、已有进程管理器 | 同一网页界面，经 `127.0.0.1` 的 HTTP 访问 |

图形版启动时在随机回环端口运行内核自带的网页服务，窗口直接打开它；不再有单独的 IPC 界面。Docker 工作区镜像在此基础上增加容器控制台、服务管理和旧网关（`cranibox serve`），用于虚拟空间。同一持久 NodeID 配置目录只允许一个常驻节点进程，同一凭据文件也只允许一个网关连接进程。需要在同一主机模拟独立设备时，为各进程指定不同的 --config 目录。

本文说明现有版本。直连与目标本地授权见[配置与边界](direct-node-peers.md)，[VNC/noVNC 桌面查看器及服务管理](remote-desktops.md)和[虚拟空间创建向导](virtual-spaces.md)。Linux/X11 支持托管 x11vnc，兼容 wlroots 的 Wayland 会话支持托管 wayvnc；Windows、macOS 和其他 Wayland 会话可接入已有本机 VNC 服务。组网见 [iroh 配对](iroh-network.md)与 [EasyTier/Tailscale](device-networks.md)。

“项目与会话”“设备消息”“协作任务”“Agent 会话”和后台执行的说明分别见 [项目与会话](project-session-catalog.md)、[设备消息](device-messages.md)、[协作任务](device-tasks.md)、[Agent 会话](agent-sessions.md)和[后台执行](agent-executions.md)；实机环境见[验证清单](client-validation.md)。

## 一套界面

浏览器、图形版窗口和 Docker 工作区都加载同一份 React 资源，经同一组 HTTP/WebSocket/SSE 接口（`cranibox_node::web`）访问本机内核：App API `/api/v1/app/*`、`/api/peers`、`/api/network`、`/api/desktop/*`、`/api/terminal/*`、`/api/spaces`。页面配置里的 `edition`（`node` 或 `workspace`）决定是否显示容器专用控制台；`shell: true` 表示运行在图形版窗口中，可调用 `POST /api/shell` 的原生操作（自启动、打开外链、隐藏窗口、退出、生成 MCP 配置）。浏览器操作设备时，以内核所在设备为接入端，不会因此获得浏览器所在电脑的本地系统能力。

## 安装和连接

打 `v*` 标签时，安装包和命令行程序会发布到 GitHub Release；平时可在 **Cranibox server and independent node client** 工作流的成功运行中下载 Artifacts：

| Artifact | 内容 |
| --- | --- |
| cranibox-desktop-macos-latest | macOS DMG（Apple Silicon） |
| cranibox-desktop-windows-latest | Windows NSIS 安装程序 |
| cranibox-desktop-ubuntu-latest | Linux AppImage、deb |
| cranibox-cli-{os} | 命令行版 `cranibox`，一个可执行文件 |

Linux 的托盘依赖桌面环境支持 AppIndicator；无桌面的机器使用命令行版。

### 图形版

1. 安装并打开 Cranibox，窗口即本机界面；关闭窗口后仍在托盘运行，托盘可打开窗口、处理审批、暂停屏幕共享或退出。
2. 在“这台设备 → 设置”开启登录自启动。登录自启动属于当前用户，不是登录前的系统守护服务。
3. 用“添加设备”生成或打开配对链接，两台设备核对配对码后互相授权。

安装包目前未配置发布者签名和 macOS 公证，首次打开需在 Finder 中右键选择“打开”。

### 命令行版

~~~bash
cranibox                  # 运行本机节点和网页界面（默认 127.0.0.1:7377）
cranibox ui-url           # 打印登录链接（含访问令牌）
cranibox peer pair invite # 生成一次性配对链接
~~~

- 界面只监听回环地址。首次运行生成访问令牌 `ui.token`（0600，与 `peers.json` 同目录）；打开 `ui-url` 输出的链接后换成 HttpOnly 会话 cookie，令牌不会留在地址栏。远程服务器用 SSH 隧道访问：`ssh -L 7377:127.0.0.1:7377 server`。
- 默认端口被占用且未指定 `--listen` 时自动改用空闲端口，实际地址写入 `ui.address`，`ui-url` 会读取它。经可信反向代理访问时用 `--allow-host` 放行代理的 Host。
- 交互终端里启动时直接打印登录链接；作为服务运行时日志只记录地址，不记录令牌。
- `cranibox peer serve` 和 `cranibox serve`（非容器构建）是同一服务的入口；已保存的旧 Workspace 网关连接会同时保持。
- 远程命令的权限等于运行节点的账户；不要用 root 运行。

### Linux AppImage 与 FUSE

没有 /dev/fuse 的容器中，AppImage 默认挂载启动会失败。Debian/Ubuntu 环境可使用 .deb；需要 AppImage 时，可使用已在远端 KDE 验证的解包运行模式：

~~~bash
"./Cranibox Node_0.1.0_amd64.AppImage" --appimage-extract-and-run
~~~

本次启动可靠性修复后的构建会在复制的三个 AppImage MCP 入口和 Linux 登录启动项中自动加入 --appimage-extract-and-run，并使用持久 AppImage 路径。该参数由 AppImage 运行时消费，不依赖 Agent 或登录进程继承环境变量；普通安装程序不添加这个参数。默认 MCP 仍仅包含设备与消息入口，后台运行入口需要明确勾选。

Linux 自启动使用绝对 XDG_CONFIG_HOME/autostart；未设置或为相对路径时使用 HOME/.config/autostart。启用时会迁移原插件留在 HOME/.config 的同名条目，关闭时同时清理两处，避免重复启动。启动项正确转义空格和特殊字符。移动 AppImage 后应重新开启自启动、重新复制 MCP 配置。

远端 KDE 已实际验收 6396b93、CI 运行 39110808540 的 .deb/AppImage：原生界面复制配置与 XDG 自启开关通过，三个入口在 GUI 退出、无兼容环境变量时均完成 MCP 初始化和工具枚举；AppImage 生成启动项通过 GIO 在无 FUSE 环境静默进入托盘，单实例唤回及身份保留通过。普通 FUSE 模式、真实注销/登录与休眠仍待验证，详见[交接记录](implementation-handoff-2026-10-18.md#历史验证索引)。旧 afaa137 及更早构建生成的配置仍需自行添加运行时参数或设置 APPIMAGE_EXTRACT_AND_RUN=1。

## 会话保持

节点引擎在窗口之外运行。关闭窗口或退出浏览器都不会结束 Shell。网络断开后客户端自动重连；「重新连接」只重建网络连接，保留现有 PTY 和 Shell。服务端恢复后可通过浏览器或客户端重新附着原会话，断线期间保留最近 256 KiB 输出。

更改网关地址、网关节点别名或 Token 只会替换网关连接，本机持久 NodeID、项目目录和 Shell 保持。旧网关上的附着和命令任务会断开；连接新网关后可重新附着已有 Shell。主动退出客户端需确认，并会结束本机 Shell；重启进程或设备也无法保留原 Shell 进程。登录自启动恢复的是本机服务和已配置的网关连接，不会重建此前的 Shell。普通命令任务仍遵循超时、取消及断线清理规则，不自动重放。

## Tailscale 或公网中继

公网模式：所有设备向同一个 Workspace 的 WSS 入口建立出站连接，命令和输出由 Workspace 转发。节点无需开放入站端口；外层 TLS 代理应透传 /ws/nodes/*、/api/v1/mesh/* 的 Authorization 和 WebSocket Upgrade。

Tailscale 模式：使用已有的 Tailscale 客户端，把 Workspace 和设备接入可互通的 tailnet，并按 Tailscale ACL 放行 Workspace 端口。客户端填写 Workspace 的 Tailscale IP 或域名，例如 wss://workspace.example.ts.net。若私网入口没有 TLS，可使用 ws://100.x.x.x:9090 并勾选「允许可信私网的 ws:// 连接」。容器部署需要发布可访问的监听地址，不能把容器回环地址直接用作其他设备的入口。

这两种网关连接模式使用相同的节点协议和设备组，应用层远程命令仍由 Workspace 转发。独立的直连能力可以通过 EasyTier 或 Tailscale 私网访问，不必经过 Workspace。Tailscale 管理页提供审阅后确认的系统安装/服务启动引导和独立触发的官方登录，不随应用启动自动安装或接管已有系统服务；平台限制与 EasyTier 托管范围见[设备自动组网](device-networks.md)。

## 每台设备上的 Agent

在托盘界面的「Agent 接入」复制 MCP 配置到本机 Agent。配置包含两个独立入口：`cranibox-devices` 调用本机客户端的 `mcp` 子命令，复用已保存的网关节点设置和 Token 文件；`cranibox-messages` 调用 `peer messages mcp`，无需网关设置即可使用 device_messages 读取本机邮箱、发消息和确认收件，以及使用 device_tasks 委托、领取和报告任务，以及 device_agent_sessions 登记、续期和查询公开 Agent 会话。两个入口都不另外打开托盘窗口，配置不含明文 Token。消息 MCP 本身也会保持后台投递；入站连接仍由本机服务监听。已配置的直连目标可另通过 `peer call --node NODE mcp` 访问。网关 MCP 提供五个工具：

- workspace_nodes：列出本机及同组节点。
- workspace_exec：在允许的设备上提交命令，返回任务 ID。
- workspace_job：查询本机 Agent 提交的任务和输出。
- workspace_cancel：取消本机 Agent 提交的任务。
- workspace_projects：读取和管理授权范围内的项目目录及会话关联。

默认不包含后台进程工具。需要由 Agent 明确启动另一个 Agent 时，可在「Agent 接入」勾选“添加 Agent 后台运行工具”，加入独立的 cranibox-runs 入口。它提供 device_nodes 和 device_agent_runs，不依赖 Workspace 中继；每次操作都必须指定精确 NodeID，远端需授予 exec 权限，并先启动目标节点服务。关闭 MCP 不会停止目标节点持有的运行。准备、启动、输出分页、去重及撤权边界见 [Agent 后台执行](agent-executions.md)。

CLI 使用同一设置，例如：

~~~bash
cranibox nodes
cranibox exec --node desktop -- git status --short
cranibox exec --node desktop --detach -- sh -lc 'make test'
cranibox job <job-id>
cranibox cancel <job-id>
cranibox mcp
~~~

也可用托盘应用可执行文件替代 cranibox，传入子命令时只运行 CLI/MCP。需要其他配置时添加 --config /path/to/settings.json。MCP 配置包含可执行文件和配置文件路径，不含明文 Token。默认设置位置：

| 平台 | 文件 |
| --- | --- |
| Windows | %APPDATA%/cranibox/settings.json |
| macOS | ~/Library/Application Support/cranibox/settings.json |
| Linux | $XDG_CONFIG_HOME/cranibox/settings.json，默认 ~/.config/cranibox/settings.json |

Token 保存在设置文件旁的 node.token。Unix 使用私有文件权限；Windows 使用当前用户的私人配置目录。CLI 也支持已有的 --server、--id、--token-file 参数。

## 设备组与权限

设备 Token 只能访问 /api/v1/mesh/*，不具备 Workspace 管理接口权限。默认未分组设备只能看到和控制自身；相同非空设备组的设备可相互执行命令。组名为 1–64 位字母、数字、下划线或短横线。组内设备应被视为相互信任的执行端。

Workspace 管理员可以在 Web 页编辑设备组，也可使用：

~~~bash
workspace-runtime group laptop --group personal
workspace-runtime group desktop --group personal
# 留空撤销跨设备访问权限
workspace-runtime group laptop
~~~

组权限在每次 API 请求时读取，不依赖客户端缓存。节点只能查询或取消自己提交且目前仍有权限访问目标设备的任务。管理员仍可从 Workspace 查看和管理全部节点及任务。撤销节点凭据会同时删除其设备组。

## 构建和验证

常规 workspace 构建不启用桌面特性，服务器镜像无需引入 GTK/WebKit。托盘应用按需构建：

~~~bash
npm ci --prefix apps/node-desktop
npm run build --prefix apps/node-desktop
cargo clippy --locked -p cranibox-node-desktop --features desktop --all-targets -- -D warnings
cd apps/node-desktop
npm run tauri -- build --features desktop -- --locked
~~~

在已生成 `apps/node-desktop/dist` 后，如果直接使用 Cargo 构建供实机运行的二进制，需要同时启用静态资源协议：

~~~bash
cargo build --locked -p cranibox-node-desktop --features desktop,custom-protocol
~~~

仅启用 `desktop` 的直接 Cargo 构建会使用 `tauri.conf.json` 中的 Vite 开发地址；开发服务器关闭后窗口无法加载。Tauri CLI 的正式构建会自动配置 `custom-protocol`。实机验证应在开发服务器关闭时确认窗口内容及本机 IPC 均可用。

桌面构建要求 Rust 1.90+、Node.js 22+ 和对应平台的 Tauri 系统依赖。三平台 CI 独立构建安装包。浏览器交互测试使用 Tauri IPC 模拟，验证窗口操作、设置、MCP 配置和布局；它不能代替各 OS 的真实托盘、登录自启动与休眠恢复体验测试。网关集成测试使用真实节点验证设备调用、组隔离、任务归属和客户端 MCP。

2026-10-19 对产品提交 2c95dd4 的 CI Linux .deb/AppImage 另做真实 KDE 窗口验收：两包均通过直接连接 Selkies、中文/emoji/多行双向剪贴板、等待后新文字和断开重连清空。.deb 实际安装后卸载；AppImage 使用 --appimage-extract-and-run。为保留日常客户端，两包使用独立身份与私有 D-Bus，不能据此宣布正常登录、托盘、自启、休眠或其他物理设备已验收。完整需求、测试版本和剩余工作见[交接入口](implementation-handoff-2026-10-18.md)。
