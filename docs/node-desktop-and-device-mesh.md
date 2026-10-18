# 托盘客户端与 Agent 设备组网

cranibox-node-desktop 是独立的 Windows/macOS/Linux 节点应用。Rust 节点引擎负责本机项目、任务和原生 PTY，Tauri 2 提供系统托盘、窗口和登录自启动，React 提供操作界面。应用启动后即可管理本机项目和 Shell，Workspace 网关是可选连接。客户端不包含 herdr。服务端保持浏览器访问方式。

无界面的 cranibox-node 仍独立发布，适合服务器或已有进程管理器。两个客户端复用节点引擎；同一持久 NodeID 配置目录只允许一个常驻节点进程，即使没有启用直连监听也会加锁，避免多个进程分别持有同一设备的 Shell。同一凭据文件也只允许一个网关连接进程。需要在同一主机模拟独立设备时，为各进程指定不同的 --config 目录。

本文说明现有版本。直连与目标本地授权见[配置与边界](direct-node-peers.md)，Web/Tauri 现已共用 [VNC/noVNC 桌面查看器及服务管理](remote-desktops.md)和[虚拟空间创建向导](virtual-spaces.md)。Linux/X11 支持托管 x11vnc，兼容 wlroots 的 Wayland 会话支持托管 wayvnc；两者支持异常重启及节点重启恢复。Windows、macOS 和其他 Wayland 会话可接入已有本机 VNC 服务，这些环境的自动安装与进程托管仍待实现。现已提供 [EasyTier 托管组网、发现、短期邀请及授权连接，以及系统 Tailscale 状态和登录](device-networks.md)，并支持显式多路径的新连接选路；公钥配对、全网目录分发和既有连接无缝迁移仍按[对等节点组网设计](decentralized-node-network-design.md)推进。

Web/Tauri 已共用“项目与会话”页面，支持工作副本目录和持久 Shell 的关联、创建与找回、授权来源的目录增量同步和明确启用的后台同步，见[使用与边界](project-session-catalog.md)。“设备消息”提供持久收发箱、线程回复、明确收件确认，以及不依赖网关的 Agent MCP，见[设备消息](device-messages.md)。直连与网关设备的终端都使用共用 React 浮动窗口，Tauri 通过本机 Rust IPC 附着，无需打开系统浏览器。“协作任务”支持持久委托、明确领取、进度与结果回传，以及默认关闭的接收端自动接单，见[使用与边界](device-tasks.md)。本机 Agent 可主动登记公开状态，按会话 UUID 定向委托，见[Agent 会话](agent-sessions.md)。节点已提供[明确启动的本机后台执行及 Codex/Claude Code 非交互适配](agent-executions.md)，关闭页面后可继续运行和找回输出；实机环境见[验证清单](client-validation.md)。真正运行会话迁移仍见[多设备项目与 Agent 工作空间](distributed-agent-workspace-design.md)。

## 浏览器与 Tauri 共用界面

采用一套 React 业务组件和 Rust 核心，分别构建浏览器资源与 Tauri 应用。浏览器通过 HTTP/WebSocket/SSE 访问运行中的 Rust 服务；Tauri 通过 IPC/事件调用本机 Rust 核心。托盘、自启动和系统权限由桌面接入层处理。Tauri 的 invoke 需要宿主环境，不能作为普通浏览器的后端；浏览器构建需要独立的网络适配器。

目前项目与会话、设备消息、协作任务、直连设备、入站授权、EasyTier/Tailscale 管理、浮动终端、VNC 查看器和虚拟空间已共用组件；其余页面尚未完全统一。已接入授权来源可同步项目目录，但设备、项目、Agent 和任务的全网统一视图与自动分发仍按分布式设计分阶段实现。浏览器操作设备时，以后端所在设备为接入端，不会因此获得浏览器所在电脑的本地系统能力。

Tauri 的前端配置要求提供静态 Web 资源，React/Vite SPA 符合这一方式，参见 [Tauri 官方前端配置](https://v2.tauri.app/start/frontend/)。

## 安装和连接

在 GitHub Actions 的 **Cranibox server and independent node client** 工作流中，选择成功的运行，从 Artifacts 下载：

| Artifact | 安装内容 |
| --- | --- |
| cranibox-node-desktop-windows-latest | Windows NSIS 安装程序 |
| cranibox-node-desktop-macos-latest | macOS DMG，内含应用 |
| cranibox-node-desktop-ubuntu-latest | Linux AppImage、deb |
| cranibox-node-{os} | 原有无界面客户端，一个可执行文件 |

选择与目标 OS 和 CPU 一致的产物；当前矩阵使用 GitHub 对应平台默认 runner 的 CPU，不是所有架构的通用包。Linux 的托盘依赖桌面环境支持 AppIndicator；无桌面的机器使用 CLI 客户端。

1. 安装并打开 Cranibox Node。「设备网络」显示“这台设备”的持久 NodeID，可直接打开本机终端；「项目与会话」可注册本机目录和关联 Shell。
2. 需要随登录启动时，在「连接设置」勾选「登录系统后自动启动」，该选项独立保存，无需配置中继。
3. 需要连接 Workspace 网关时，在 Workspace 网页的「远程节点」注册设备并保存节点 Token，再在客户端填写服务端地址、网关节点 ID 和 Token。公网填写 wss://workspace.example.com，地址不带路径，点击「保存并连接」。
4. 关闭窗口后仍在托盘运行。托盘菜单可打开窗口、重新连接网关或主动退出。
5. 在 Workspace 将可信设备加入同一设备组，例如 personal；各设备客户端的「设备网络」显示可访问的网关设备。直连连接和入站授权可独立配置，不依赖网关。

登录自启动属于当前用户，会在用户登录后启动，不是登录前的系统守护服务。账户权限决定远程命令的权限。安装包目前未配置发布者签名和 macOS 公证。

### Linux AppImage 与 FUSE

没有 /dev/fuse 的容器中，AppImage 默认挂载启动会失败。Debian/Ubuntu 环境可使用 .deb；需要 AppImage 时，可使用已在远端 KDE 验证的解包运行模式：

~~~bash
APPIMAGE_EXTRACT_AND_RUN=1 "./Cranibox Node_0.1.0_amd64.AppImage"
~~~

当前「Agent 接入」复制的 MCP 配置会保留 AppImage 的持久路径，但不会自动添加这个环境变量。在没有 FUSE 的环境中，需给每个使用该 AppImage 的 MCP 条目补充或合并以下 env 字段，使 Agent 启动时也使用兼容模式：

~~~json
"env": { "APPIMAGE_EXTRACT_AND_RUN": "1" }
~~~

兼容模式已验证真实窗口、托盘和消息 MCP 初始化；普通 FUSE 模式及 AppImage 登录自启动尚未实测，不能假设登录进程会继承手动启动时的变量。具体包版本与证据见[交接记录](implementation-handoff-2026-10-18.md#最新-linux-ci-安装包与-shell-重新附着)。

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
cranibox-node nodes
cranibox-node exec --node desktop -- git status --short
cranibox-node exec --node desktop --detach -- sh -lc 'make test'
cranibox-node job <job-id>
cranibox-node cancel <job-id>
cranibox-node mcp
~~~

也可用托盘应用可执行文件替代 cranibox-node，传入子命令时只运行 CLI/MCP。需要其他配置时添加 --config /path/to/settings.json。MCP 配置包含可执行文件和配置文件路径，不含明文 Token。默认设置位置：

| 平台 | 文件 |
| --- | --- |
| Windows | %APPDATA%/cranibox-node/settings.json |
| macOS | ~/Library/Application Support/cranibox-node/settings.json |
| Linux | $XDG_CONFIG_HOME/cranibox-node/settings.json，默认 ~/.config/cranibox-node/settings.json |

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
