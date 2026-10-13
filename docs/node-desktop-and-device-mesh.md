# 托盘客户端与 Agent 设备组网

cranibox-node-desktop 是独立的 Windows/macOS/Linux 节点应用。Rust 节点引擎负责连接、任务和原生 PTY，Tauri 2 提供系统托盘、窗口和登录自启动，React 提供配置界面。客户端不包含 herdr。服务端保持浏览器访问方式。

无界面的 cranibox-node 仍独立发布，适合服务器或已有进程管理器。两个客户端复用节点引擎；同一个凭据文件只允许一个常驻节点进程，避免互相挤掉连接。

## 安装和连接

在 GitHub Actions 的 **Cranibox server and independent node client** 工作流中，选择成功的运行，从 Artifacts 下载：

| Artifact | 安装内容 |
| --- | --- |
| cranibox-node-desktop-windows-latest | Windows NSIS 安装程序 |
| cranibox-node-desktop-macos-latest | macOS DMG，内含应用 |
| cranibox-node-desktop-ubuntu-latest | Linux AppImage、deb |
| cranibox-node-{os} | 原有无界面客户端，一个可执行文件 |

选择与目标 OS 和 CPU 一致的产物；当前矩阵使用 GitHub 对应平台默认 runner 的 CPU，不是所有架构的通用包。Linux 的托盘依赖桌面环境支持 AppIndicator；无桌面的机器使用 CLI 客户端。

1. 在 Workspace 网页的「远程节点」注册设备，保存本次显示的节点 Token。
2. 安装并打开 Cranibox Node，在「连接设置」填写服务端地址、节点 ID 和 Token。公网填写 wss://workspace.example.com，地址不带路径。
3. 默认勾选「登录系统后自动启动」，保存并连接。
4. 关闭窗口后仍在托盘运行。托盘菜单可打开窗口、重新连接或主动退出。
5. 在 Workspace 将可信设备加入同一设备组，例如 personal；各设备客户端的「设备网络」显示可访问的设备。

登录自启动属于当前用户，会在用户登录后启动，不是登录前的系统守护服务。账户权限决定远程命令的权限。安装包目前未配置发布者签名和 macOS 公证。

## 会话保持

节点引擎在窗口之外运行。关闭窗口或退出浏览器都不会结束 Shell。网络断开后客户端自动重连；「重新连接」只重建网络连接，保留现有 PTY 和 Shell。服务端恢复后可通过浏览器重新附着原会话，断线期间保留最近 256 KiB 输出。

主动退出客户端、重启进程或更改服务端/节点身份会结束本机现有 Shell；界面会先确认。电脑重启后登录自启动恢复的是节点连接，无法恢复此前的 Shell 进程。普通命令任务仍遵循超时、取消及断线清理规则，不自动重放。

## Tailscale 或公网中继

公网模式：所有设备向同一个 Workspace 的 WSS 入口建立出站连接，命令和输出由 Workspace 转发。节点无需开放入站端口；外层 TLS 代理应透传 /ws/nodes/*、/api/v1/mesh/* 的 Authorization 和 WebSocket Upgrade。

Tailscale 模式：使用已有的 Tailscale 客户端，把 Workspace 和设备接入可互通的 tailnet，并按 Tailscale ACL 放行 Workspace 端口。客户端填写 Workspace 的 Tailscale IP 或域名，例如 wss://workspace.example.ts.net。若私网入口没有 TLS，可使用 ws://100.x.x.x:9090 并勾选「允许可信私网的 ws:// 连接」。容器部署需要发布可访问的监听地址，不能把容器回环地址直接用作其他设备的入口。

两种模式使用相同的节点协议和设备组。Tailscale 提供私网连通与它自身的中继；应用层远程命令仍由 Workspace 转发。本功能不是通用 VPN、任意 TCP 转发或文件共享，不会自动安装或登录 Tailscale。

## 每台设备上的 Agent

在托盘界面的「Agent 接入」复制 MCP 配置到本机 Agent。该配置调用本机客户端的 mcp 子命令，复用已保存的节点设置和 Token 文件；不另外启动托盘或节点服务。提供四个工具：

- workspace_nodes：列出本机及同组节点。
- workspace_exec：在允许的设备上提交命令，返回任务 ID。
- workspace_job：查询本机 Agent 提交的任务和输出。
- workspace_cancel：取消本机 Agent 提交的任务。

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

桌面构建要求 Rust 1.90+、Node.js 22+ 和对应平台的 Tauri 系统依赖。三平台 CI 独立构建安装包。浏览器交互测试使用 Tauri IPC 模拟，验证窗口操作、设置、MCP 配置和布局；它不能代替各 OS 的真实托盘、登录自启动与休眠恢复体验测试。网关集成测试使用真实节点验证设备调用、组隔离、任务归属和客户端 MCP。
