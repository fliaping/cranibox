# Linux 远端 Workspace 验证环境

本次按用户要求使用 `admin@192.168.217.6:22022`，在远端 Docker 中启动独立验证容器，不使用本机图形桌面验证客户端。

最新补验见[IPC 测量与撤回方案](#2026-10-19-补验ipc-测量及撤回的传输方案)，此前有[原生连接积压](#2026-10-19-补验原生连接积压)；功能与完整交接入口为[需求与测试状态](implementation-handoff-2026-10-18.md)。

## 部署

| 项目 | 配置 |
| --- | --- |
| 容器名 | `cranibox-validation` |
| 基础镜像 | 与当前开发容器相同的 `registry.cn-hangzhou.aliyuncs.com/fliaping/agent-workspace:ubuntu-kde` |
| 镜像 ID | `sha256:a2077e4276f3e6a51e1fa5b3a92b6ef72a3336e303e955fa8034b0ae1724a81b` |
| Web | `https://192.168.217.6:3080` |
| KDE / Selkies | `https://192.168.217.6:3031` |
| 登录用户名 | `cranibox` |
| 密码位置 | 远端 `/vol3/1000/cranibox-validation/container.env`，权限 `0600` |
| 持久数据 | 远端 `/vol3/1000/cranibox-validation/data` 挂载到 `/config` |
| 资源 | 4 CPU、4 GiB 内存、2 GiB 共享内存 |
| 重启策略 | `unless-stopped` |
| 桌面 | KDE Plasma / Wayland |

两个 HTTPS 入口使用自签名证书，仅绑定远端局域网地址。未登录返回 401，认证后返回 200。容器没有挂载宿主 Docker socket，未启用特权模式。

采用相同基础镜像和当前工作区的源码快照，并部署当前 Rust Web / 节点程序；没有复制开发容器中的个人凭据及完整用户配置。源码位于容器内 `/config/agent-workspace-manager/source`。快照清单见远端 `build-manifest.json`，部署配置见 `deployment.json`。

旧镜像的服务管理脚本与当前 Rust 单元格式不兼容，因此将项目的 `scripts/user-systemctl.sh` 绑定到 `/usr/local/bin/systemctl`。Web 由 Rust 服务管理器随容器启动。健康检查检查 Rust Web HTTP、桌面 HTTP 入口和 Selkies TCP 监听。

## 已验证

- 容器启动及显式 `docker restart cranibox-validation` 后均为 `running / healthy`，Web 与 KDE 自动恢复。重启后的认证检查见远端 `post-restart-checks.json`。
- 最后复核容器仍为 `running / healthy`，两个浏览器入口均在未认证时返回 401、认证后返回 200；凭据文件权限已确认为 `0600`。结果保存在远端 `final-entrypoint-checks.json`。
- 浏览器真实打开 Workspace 概览，创建并连接持久终端 `remote-validation`，执行测试命令成功，`hostname` 返回 `cranibox-validation`。
- Selkies 的桌面视频和鼠标操作可用，能够看到远端 KDE 中运行的 Tauri 客户端。
- 在远端构建客户端前端及原生程序成功。原生构建使用 `cargo build --locked -p cranibox-node-desktop --features desktop,custom-protocol`，目标目录 `/config/validation-build/target`。
- 修复 `desktop_close` / `terminal_close` 异步 Tauri 命令的返回类型，使带 `State` 参数的命令返回 `Result<(), String>`。
- 关闭原生窗口后窗口隐藏，进程继续运行。重启容器后重新启动客户端，KDE StatusNotifierWatcher 已注册客户端托盘项；托盘菜单交互尚未验收。

原生客户端使用独立的 `/config/validation-client/{config,data,state}`，与 Workspace 内置节点保持不同 NodeID。当前保留一个可见的原生客户端进程，便于通过远端桌面继续操作；尚未配置该测试客户端自动启动。

证据保存在容器 `/config/validation-artifacts`，包括 `remote-web-terminal.png`、`remote-desktop-native.png`、`native-hidden.png`、`native-tray-check.json`、`client-native.log`。任务专用自动化浏览器已经关闭。

## 未覆盖

KDE Wayland 下尚未验收节点自带的 VNC 桌面共享；此次使用镜像提供的 Selkies 访问桌面，不能据此宣称 VNC 节点互访已经通过。Windows、macOS、跨设备实机授权和网络中断恢复仍需分别实测。虚拟空间的浏览器创建、终端重连与生命周期已在同一宿主的临时管理容器中验证，见下文。此主验证容器不提供 Docker 引擎。

运行库与测试依赖安装在该容器的可写镜像层，重启会保留；将来删除并重建容器时，需要恢复这些依赖及上述绑定配置。完整跨平台清单见 [客户端实机验证环境](client-validation.md)。

## Agent 启动适配器更新

同日在此远端环境更新了 Web 与 Tauri 共用的“本机后台执行”表单，支持 Codex、Claude Code 和自定义程序。检查页显示执行设备、工作目录、权限及任务；任务通过 stdin 传递。检查不启动进程，缺少 CLI 时禁止确认。具体权限边界及恢复规则见 [Agent 后台执行](agent-executions.md)。

远端通过 6 条运行账本单元测试、10 条 Linux 进程集成测试、5 条 Web 表单用例和 2 条客户端模拟 IPC 用例，Web / 客户端前端与 Rust 程序均构建成功。进程测试覆盖模拟 CLI 参数与 stdin、16 KiB UTF-8 与 JSON 转义边界、大输出和输入并发、去重、退出及恢复；未调用真实模型或验证订阅登录。

实际浏览器检查了已部署 Web 的启动检查、缺少 Codex 的提示和禁用确认，375px 宽度下深浅主题均无横向溢出。通过 Selkies 操作真实 KDE 中的 Tauri 客户端，读取独立节点的后台执行目录并打开共用表单；修复了 Linux WebKit 深色页面中下拉框文字不可见的问题。客户端仍使用独立配置目录，尚未配置登录自动启动。

Web 程序通过 `workspacectl service` 更新并检查恢复，上一版本保存在 `/config/validation-artifacts/cranibox.before-agent-presets-20261018`。客户端构建补充安装了 GTK、WebKit、AppIndicator、SVG 和 D-Bus 开发包。证据文件位于 `/config/validation-artifacts/agent-presets-validation.json`，包含部署程序及源码摘要、认证检查、测试范围和限制；新截图使用 `agent-launch-` 前缀。

## KDE Wayland VNC 实测

在同一远端容器编译 TigerVNC `v1.16.2` 的 `w0vncserver`，并安装 PipeWire 和 KDE desktop portal 进行独立评估，没有将它加入项目的托管服务。临时服务仅监听 `127.0.0.1:15900`，要求 VNC 密码；错误密码被拒绝，正确密码通过，实际 KDE 确认窗口能够请求并批准屏幕及输入权限。

批准后没有收到桌面画面。KDE portal 日志报告 `failed to start streaming` / `不支持的合成类型`，VNC 在返回 ServerInit 前关闭连接。另行启动的隔离 KWin virtual / X11 windowed 测试会话也回退为 QPainter，未获得可用于验证的 OpenGL 会话。因此本环境的 KDE VNC 画面及输入验收仍未通过；不能以密码校验和授权窗口成功代替完整桌面测试，也不能将这一容器结果推断为所有 KDE 设备都不支持 VNC。

评估记录为远端 `/config/validation-artifacts/desktop-vnc-validation.json`，日志和截图使用 `wayland-vnc-`、`validation-w0vncserver`、`validation-portal-kde` 前缀。临时 VNC、手动启动的 portal 后端、PipeWire、WirePlumber 和隔离 KWin 进程均已停止，临时 VNC 密码已删除，端口 15900 已关闭，任务浏览器已关闭。原有 Workspace Web、KDE / Selkies 和 Tauri 客户端继续保留。

## 虚拟空间真实 Docker 生命周期

同日在该 Linux 宿主上使用独立网络 `cranibox-spaces-validation-20261018` 和临时管理容器 `cranibox-spaces-manager-20261018` 验证虚拟空间。只有该临时管理容器挂载宿主 Docker socket；主验证容器和创建的空间均没有该挂载。测试没有使用本机桌面，也没有修改宿主已有的业务容器。

验证镜像 `cranibox-space-validation:20261018` 使用上述相同 KDE 基础镜像，叠加当前项目启动脚本、服务、Rust 程序和 Herdr。它是本地验证叠加镜像，不是完整 Dockerfile 的干净构建，也未验证仓库拉取和系统依赖自动安装。最终叠加镜像 ID 为 `sha256:b104dc58570459e964b5ccabd3daaff34c74f50c07568a85cc98d3fa276cb891`。

已通过真实节点接入和授权命令，确认空间不启用 privileged、端口仅发布在宿主回环地址。随后验证停止、再次启动、归档移除容器、保留命名数据卷，以及归档重建后 NodeID 与 `/config/Workspace/validation/persistent.txt` 文件内容保持不变。创建和重建使用 `never` 镜像策略，不调用镜像拉取。空间 ID 为 `dbee0a9a-f335-45ff-a298-b9458157096d`。

首次启动发现私有工作区的脚本权限会被 COPY 保留，单独增加执行位仍不能让 abc 用户读取；Dockerfile 现显式设置运行文件权限。CLI 的全局节点 ID / Token 参数也与子命令参数冲突，已使用独立 Clap 参数键修复，并补充 3 条解析回归用例。空间完整生命周期同时保留 Web API 和 CLI 集成覆盖。

本轮远端结果为节点单元测试 32 条通过、2 条 EasyTier 实网用例因独立环境要求而跳过，空间集成测试 7 条通过、Web 空间浏览器测试 4 条通过。Web 与 Tauri 前端、Rust 程序均构建成功，375–1440px 的空间页面深浅主题布局无横向溢出。主验证 Web 与原生客户端已更新，Web 上一版本保存在 `/config/validation-artifacts/cranibox.before-space-policy-20261018`。

节点全部 target 的 Clippy 在 `-D warnings` 下通过。生命周期结果保存在远端 `/config/validation-artifacts/spaces-lifecycle-validation.json`，最终部署、源码摘要和认证复核见 `space-policy-validation.json`。临时空间及管理容器均已停止；空间数据卷、身份与管理目录保留，便于复核。主验证容器继续运行。Windows/macOS 依赖安装、真实 Tauri 向导创建及空间图形桌面尚未验收。

## 虚拟空间真实浏览器验收

继续使用同一远端 Docker 宿主和临时管理容器，以当前 Web 程序、独立管理目录及本地验证镜像进行验收。临时 Web 入口有 HTTP 登录保护，只在 Docker 网络内监听，没有向宿主发布端口。浏览器实际运行在远端主验证容器中，操作真实后端和 Docker；此处不使用前端 API 模拟。

通过创建向导新建 `Browser workflow validation`（空间 ID `683e1ac0-1d80-4201-aae6-f69643561508`），选择本地镜像及 `never` 策略。创建后关闭页面并重新进入，空间正常就绪。由卡片打开浮动终端，经 WebSocket 连接该空间，写入文件和 Shell 环境变量；关闭整个浏览器上下文后重新登录，从会话列表附着原会话，环境变量及文件均仍可读取。最小化与恢复浮动终端也保留连接。

随后全部通过界面执行停止、再次启动、归档与恢复。归档重建后 NodeID `node-d84636c804f14e35bc55807b9217af17` 保持不变，新终端可以读取先前写入的 `/config/Workspace/validation/browser.txt`。停止及归档会结束原进程，验证不宣称这些操作能够保留原 Shell。375px 宽度下检查深浅主题，布局无横向溢出；截图等待主题过渡结束后采集。

浏览器终端、生命周期与最终复核记录分别为远端 `/config/validation-artifacts/spaces-browser-validation.json`、`spaces-browser-lifecycle-validation.json`、`spaces-browser-final-checks.json`；截图使用 `spaces-browser-` 前缀。验证叠加镜像仍继承旧基础镜像要求匿名返回成功的健康检查，启用登录保护后该检查因 401 报 unhealthy；这不替代完整 Dockerfile 的健康检查验收。

结束后两个测试空间和临时管理容器均已停止，数据卷、身份和管理目录保留；临时管理容器已断开额外接入的默认 bridge 网络，临时登录校验文件已删除，任务浏览器已关闭。主验证容器仍为 running / healthy，未挂载宿主 Docker socket；Web 与 KDE 入口均在匿名时返回 401、登录后返回 200。空间图形桌面和编辑器仍仅发布在 Docker 宿主回环地址，尚未提供远程浏览器的跨节点入口。

## 多设备空间目录展示优化

Web 与 Tauri 共用的目录界面现在直接展示可读目录，将未共享和不可用设备收进默认折叠的状态列表。展开可查看设备、检查时间、连接错误及授权说明；检查中的设备显示汇总进度。刷新保留展开选择，撤权和连接移除仍清除原目录，空间节点仍能展示它管理的下层空间。

所有构建和浏览器测试均在上述远端主验证容器执行。Web 空间测试 5 条通过，其中新增 27 个模拟设备的混合状态、键盘展开、恢复、375px 与 1440px 深浅主题检查；这些模拟设备不代表 27 台实机接入。既有撤权、迟到响应、创建向导和双语真实 Rust 目录用例继续通过。客户端目录模拟 IPC 用例 1 条通过，Web / Tauri 前端及两个 Rust 程序构建成功。

已更新主验证 Web 和独立 Tauri 客户端，上一版本备份路径见 `/config/validation-artifacts/directory-ui-deployment.json`。部署后的真实 Web 使用现有登录读取空目录，检查新静态资源、375px / 1440px 深浅主题和浏览器错误；通过结果为 `directory-ui-browser-validation.json`。最终复核 `directory-ui-final-checks.json` 确认主容器 running / healthy、未挂载宿主 Docker socket、原生客户端仍运行，Web 与 KDE 入口匿名 401、登录后 200。截图使用 `directory-ui-` 前缀，任务浏览器已关闭。此轮没有新增 VNC、跨平台安装或空间图形桌面的验收结论。

## Selkies 会话识别与原生窗口恢复

远端 Web 后台进程只继承 `DISPLAY=:1` 和 `PIXELFLUX_WAYLAND=true`，没有 Wayland 登录变量；旧检测将 XWayland 误判成 X11。节点现识别镜像中的 Selkies Wayland 环境，缺少所选会话时拒绝无效的 VNC 安装与启动，并在 Web / Tauri 提示使用已有 Selkies 入口或接入兼容 VNC 服务。显式 X11 会话仍可使用原有提供者。Web 的“打开桌面”也不再要求安装编辑器扩展。

回归检查发现 wayvnc 控制 socket 的临时目录权限依赖启动环境。现于创建时显式要求 `0700`，并用 `umask=000` 启动 Web 的隔离用例验证目录、配置及恢复流程。远端通过提供者单元测试 3 条、桌面服务集成测试 8 条和桌面浏览器测试 10 条；后两类使用隔离 VNC 替身和最小 RFB 服务，不表示当前 KDE 桌面的 VNC 已通过。Web / Tauri 前端和 Rust 构建通过，节点库 Clippy 检查通过。

真实截图还发现上一轮直接 Cargo 构建的 Tauri 窗口仍连接 Vite，关闭开发服务器后显示连接失败；先前仅检查进程运行不足以确认页面可用。本轮已以 `desktop,custom-protocol` 重新构建并启动，确认 `5173/5174` 均关闭时原生窗口正常显示。本机节点状态、设备目录和桌面服务页面通过真实 IPC 读取，并可看到新的 Selkies 提示；该客户端继续使用独立配置目录。此修复针对实机验证的构建命令，Tauri CLI 的正式构建仍按原有流程配置静态资源协议。

部署后真实浏览器确认后台检测、禁用不适用操作、已有桌面入口、手动 VNC 表单、375px / 1440px 深浅主题及无浏览器错误。最终记录为 `/config/validation-artifacts/selkies-detection-final-checks.json`，部署与备份记录为 `selkies-detection-deployment.json`，截图使用 `selkies-detection-` 前缀。主容器仍 running / healthy，未挂载宿主 Docker socket；Web 与 KDE 入口匿名返回 401、认证后返回 200。任务浏览器已关闭。

本轮没有替换 KDE 会话或部署 Selkies → VNC 适配器。已安装的 wayvnc 不能用于本容器的 KDE 合成器，当前仍通过已有 Selkies 浏览器入口访问桌面；跨节点 VNC 实机画面及 Windows/macOS 验收继续保留为未完成。

## 同一设备的多条连接路径

已更新远端 Web 和独立 Tauri 客户端。同一 NodeID 可显式保存最多四条路径，并在 Web、Tauri 和 CLI 添加、设为首选或移除。新增地址必须使用已保存的目标授权，并先完成只读身份及授权检查；局域网、EasyTier 和 Tailscale 可以同时为同一设备提供地址。配置兼容原有单路径设备，新增路径不会产生第二个设备身份。

新连接按顺序检查路径，仅连接失败、超时、502–504 或已停止的 EasyTier 端口租约允许尝试下一条。授权、身份及协议错误立即返回。实际命令和 WebSocket 升级不跨路径重发，现有连接也不被排序或移除操作主动关闭。断线后的终端重新附着会重新选路；这是重新连接原 Shell，并非已建立 TCP 连接的无缝迁移。首选路径恢复后，后续连接重新优先使用它。

远端通过运行库 peers 测试 45 条、节点单元测试 33 条、相关 HTTP / WebSocket 集成测试 20 条、Web 设备浏览器用例 7 条，以及客户端路径面板模拟 IPC 用例 1 条。节点另有 2 条 EasyTier 实网用例因独立环境要求而跳过。Rust 格式检查及运行库、节点所有 target 的 Clippy（`-D warnings`）通过，Web / Tauri 前端和 Rust 程序构建成功；原生客户端继续使用 `desktop,custom-protocol` 静态资源构建。

集成测试使用实际节点服务和可停止的 TCP 转发路径，验证首选路径中断后备用路径重新附着保留相同 Shell PID 及环境变量，首选路径恢复、新 Web 进程读取原配置、错误授权与错误身份不回退，以及命令响应丢失后只执行一次。另验证过期的托管端口租约不会向仍占用该端口的进程发送凭据。上述转发路径是隔离测试设施，不表示本轮完成了真实 Tailscale 或 EasyTier 网络切换验收。

部署后的真实登录页面接入临时节点，通过界面检查新增、排序、移除、刷新保留、主路径关闭及恢复，检查 375px / 1440px 深浅主题无横向溢出和浏览器错误，且凭据未写入浏览器存储。临时节点、路径、授权文件及浏览器均已清理。KDE 截图确认更新后的 Tauri 窗口正常显示设备网络页；新增路径表单的客户端自动化使用模拟 IPC，尚未代替全部原生实机操作验收。

部署及前一版本备份记录为 `/config/validation-artifacts/peer-paths-deployment.json`，真实页面结果为 `peer-paths-browser-validation.json`，最终容器及认证复核为 `peer-paths-final-checks.json`。截图使用 `peer-paths-` 前缀。主容器继续 running / healthy，未使用 privileged 或宿主 Docker socket，Web 与 KDE 入口匿名返回 401、认证后返回 200。

## 多设备 Agent 后台执行

已在远端更新 Web 和独立 Tauri 客户端的共用后台执行界面。可从设备列表选择本机或已保存连接的节点，检查并启动 Codex、Claude Code 或自定义非交互程序，查看持久输出、停止与归档。目标必须授予执行权限；远端运行 API 按提交方 NodeID 和当前凭据代次限制记录范围，撤权或轮换凭据后停止相关活动进程树。权限及恢复边界见 [Agent 后台执行](agent-executions.md)。

远端通过运行库 peers 测试 47 条、Linux 进程集成测试 15 条（本机 10 条、远端 5 条）、Web 后台执行用例 7 条和客户端模拟 IPC 用例 3 条。覆盖远端 CLI、16 KiB 输入及 JSON 转义、同一请求 ID 去重、观察端 Web 重启、其他调用方和本机记录隔离、凭据轮换、撤权终止进程树、设备切换后的迟到响应和启动响应不确定时保留请求 ID。Rust 格式检查、运行库和节点所有 target 的 Clippy（`-D warnings`）通过；Web / Tauri React 前端及 Rust 程序构建成功，原生客户端仍使用 `desktop,custom-protocol`。

部署后的真实浏览器通过登录后的 Web 接入一个隔离节点服务，在界面选择目标、检查并启动 Python 程序。程序输出节点身份后等待；关闭整个浏览器及上下文，确认进程仍运行，随后解除等待并重新登录，页面可找回原运行和后续输出，启动计数保持一次。再启动另一程序，通过目标 CLI 撤权，界面刷新后清除记录及输出，目标确认进程已停止。375px / 1440px 深浅主题无横向溢出，浏览器没有脚本错误，授权凭据未写入浏览器存储。

远端 KDE 中也实际打开了更新后 Tauri 客户端的协作任务页，展开后台执行面板，确认独立节点身份、设备选择框与深色文字正常显示；该操作不启动程序。截图为 remote-runs-native-selector.png。

上述执行节点是同一远端容器中使用独立身份与配置的真实进程，不是第二台物理设备；没有调用真实模型或验证订阅登录。客户端远端设备选择与执行流程有模拟 IPC 覆盖，不代替跨平台原生实机验收。自动 worker 交付、交互式 Agent 输入、运行 DAG、模型会话恢复及跨设备进程迁移仍未实现。

临时节点、连接、授权文件及任务浏览器已清理。部署与前一版本备份见远端 `/config/validation-artifacts/remote-runs-deployment.json`，真实页面结果为 `remote-runs-browser-validation.json`，最终认证及容器检查为 `remote-runs-final-checks.json`，源码摘要为 `remote-runs-source-manifest.json`；截图使用 `remote-runs-` 前缀。主容器继续 running / healthy，Web 与 KDE 入口匿名返回 401、登录后返回 200，独立 Tauri 客户端仍运行。

## 虚拟空间桌面与编辑器入口

Web 与 Tauri 的共享空间卡片新增“桌面”和“编辑器”，通过 Rust 管理端转发空间的 HTTP / WebSocket。入口只接受本管理端创建且容器标签、身份和状态匹配的空间，不接受任意目标 URL；上游密码和应用 Cookie 留在后端。一次性链接兑换为限定路径的 HttpOnly Cookie；Web 入口使用独立来源并绑定原控制台会话，停止空间、授权失效或管理端退出会结束转发。配置与部署方式见 [虚拟空间](virtual-spaces.md)。

所有构建、测试与浏览器仍在 `192.168.217.6:22022` 的远端环境运行。Rust 节点和 Web 单元测试合计 50 条通过、3 条因独立环境要求跳过；空间集成测试 8 条、Web 空间浏览器测试 6 条、客户端空间模拟 IPC 测试 3 条通过。HTTP 和 WebSocket 转发用例覆盖文本、二进制、停止取消、凭据隔离及一次性兑换。回归同时补齐旧客户端测试的连接地址和镜像拉取策略数据。Rust 格式检查及 node、web、ctl、desktop 全部 target 的 Clippy（`-D warnings`）通过，Web / Tauri React 和 Rust 构建成功，原生程序使用 `desktop,custom-protocol`。

真实浏览器使用同一独立管理容器及空间 `683e1ac0-1d80-4201-aae6-f69643561508` 验证。临时控制台和应用入口在私有 Docker 网络内使用不同 HTTPS 端口；浏览器实际打开 Selkies 桌面和 code-server 工作台，均收到 WebSocket 数据，编辑器无需再次输入密码。桌面验收等待初始提示消失并检查实际图像像素，截图确认显示 KDE 桌面，避免将初始黑屏计为成功。一次性参数从地址中移除，新标签页没有 opener，应用 Cookie 带 Secure、HttpOnly、SameSite=Strict 且没有暴露上游登录 Cookie；375px 深浅主题无横向溢出。旧叠加镜像的 code-server 有两个可选 vsda 资源返回 404，工作台及 WebSocket 正常加载；此结果不代表完成编辑器所有插件的验收。

真实应用结果记录为 `/config/validation-artifacts/space-applications-real-validation.json`，截图为 `space-application-desktop.png`、`space-application-editor.png` 及 `space-applications-375-*.png`。验收后两个空间和临时管理容器均已停止，数据与身份保留，临时登录文件已删除，额外 bridge 连接已移除，任务浏览器已关闭。此次仍使用本地叠加验证镜像，不是完整 Dockerfile 的干净构建。

已更新主验证 Web 和独立 Tauri 客户端，部署与前一版本备份见 `/config/validation-artifacts/space-applications-deployment.json`。新镜像及 Compose 提供 HTTPS 3002 应用入口；本轮没有重建主验证容器或给它增加宿主 Docker socket，主容器也没有发布该应用端口。直接升级旧镜像的部署仍需补充独立应用入口和 `applicationsUrl`。原生入口有模拟 IPC 与 Rust 回环网关覆盖，尚未在真实 Tauri 窗口中通过系统浏览器打开真实空间；Windows/macOS 的该流程同样未验收。其他设备的空间目录仍只读，不会自动获得图形访问授权。

部署后真实登录的 Web 空间页通过 375px / 1440px 深浅主题检查，KDE 截图确认更新后的 Tauri 窗口正常显示本机身份及设备网络页。最终容器、程序摘要、认证与清理检查记录为 `/config/validation-artifacts/space-applications-final-checks.json`，对应源码摘要为 `space-applications-source-manifest.json`。主容器保持 running / healthy，镜像 ID 与当前原始工作区相同，Web 与 KDE 入口匿名 401、认证后 200；运行程序与构建产物摘要一致。

## 可选的 Agent 后台执行 MCP

客户端“Agent 接入”新增默认关闭的后台运行选项，生成独立的 cranibox-runs 配置。执行 MCP 提供设备 ID 目录和检查、启动、查看输出、停止、归档操作，每次必须指定精确 NodeID；远端沿用已有 exec 授权。原有消息、协作任务及会话 MCP 的工具和权限保持不变。目标节点服务持有进程，MCP 退出不会结束运行。配置生成不会启动程序或修改外部 Agent 设置，详见 [Agent 后台执行](agent-executions.md)。

构建、测试和浏览器操作继续全部在 192.168.217.6:22022 运行。真实双节点执行集成测试 21 条通过，其中新增 MCP 6 条；原有 Workspace、消息、协作任务和会话 MCP 另有 5 条针对性回归通过。覆盖明确选择本机或远端、未知目标不回退、重复 ID 去重、关闭观察端后继续运行、输出分页、JSON 转义后的 16 KiB 输入、撤权停止与轮换授权后的记录隔离。Codex 适配器使用隔离的本地替身验证参数和标准输入，不调用真实模型或验证登录。

客户端浏览器模拟 IPC 测试 2 条通过，验证默认配置、可选入口、异步生成、错误清空和 375px / 1440px 布局。Tauri React 与 Rust 服务端、独立节点和原生客户端构建成功；Rust 格式检查以及 runtime、node、web、desktop 全部 target 的 Clippy（-D warnings）通过。CI 已加入执行 MCP 及远端执行回归，此处不代表 GitHub Actions 已执行。

更新主验证程序与独立 Tauri 客户端后，在真实 KDE 中点击“Agent 接入”，分别复制默认和勾选后的配置，通过 KDE 剪贴板验证实际 Rust IPC 返回的两个／三个入口、执行文件和节点设置路径，且不包含明文 Token。更新后的原生程序也直接启动 stdio MCP，返回正确的本机 NodeID 和两个新工具。原生截图为 agent-runs-mcp-native-default.png 与 agent-runs-mcp-native-enabled.png；结果见远端 /config/validation-artifacts/agent-runs-mcp-native-ui.json、agent-runs-mcp-native-cli.json。

本轮仍只使用远端 Linux 和同一容器内独立身份的节点，不代表跨物理设备、Windows/macOS 或模型执行验收。该入口没有自动领取 Task、启动消息 worker、DAG 调度或 Agent 会话迁移。部署及旧版本备份记录为 agent-runs-mcp-deployment.json，最终容器及认证复核为 agent-runs-mcp-final-checks.json，源码摘要为 agent-runs-mcp-source-manifest.json。文档沿用用户环境日期 2026-10-18，远端原始记录保留该机器当时的 UTC 时间戳。

## 项目交接说明和目标 Agent 接续

Web 与 Tauri 共用的项目复制面板现在可以携带四项交接说明：目标与约束、当前进展、下一步、验证结果。Rust 将说明写成快照中的普通 Markdown 文件，参与原有清单、大小和 SHA-256 校验；来源项目不被修改，普通文件复制保持兼容。CLI 支持 `peer copy prepare --handoff-file` 及 `peer copy agent --command`。字段、权限和恢复规则见 [项目与会话目录](project-session-catalog.md)。

复制完成后可以检查目标 Codex / Claude Code、完整接续指令、目录和权限，再明确确认启动。默认只读；检查与复制均不启动 Agent。每个复制 UUID 对应一个接续运行，重开页面可查询原任务，启动响应未知时用相同内容重试不会创建第二个进程。输出按 UTF-8 字节偏移读取，停止只作用于该接续任务。原节点 Agent、模型记忆和协作派发状态不随快照迁移。

构建和测试继续全部位于 `192.168.217.6:22022` 的验证容器。后端真实 HTTP / CLI 集成测试 25 条通过，覆盖交接、原有复制和远端后台执行；Web 项目复制浏览器测试 6 条、客户端项目复制模拟 IPC 测试 3 条通过。客户端回归同时补全旧模拟连接中的地址字段，另有离线项目、协作任务、设备消息和任务问答 4 条相关回归通过。Rust 格式检查及 runtime、node、web、desktop 全部 target 的 Clippy（`-D warnings`）通过，Web / Tauri React 和 Rust 程序构建成功，原生构建使用 `desktop,custom-protocol`。CI 已加入交接测试，此处不代表 GitHub Actions 已执行。

部署后的真实登录 Web 接入两个具有独立配置和身份的节点进程，通过页面填写交接、预览清单、确认复制、检查 Agent 并启动。核实未提交文件与 Markdown 均到达接收目录，`.env` 被排除，复制及检查期间没有启动进程。关闭整个浏览器后目标进程仍运行；重新登录查询同一运行能读取后续中文输出，启动计数保持一次。375px / 1440px 深浅主题无横向溢出，浏览器无脚本错误，凭据和交接说明没有写入浏览器存储。

此次目标 Codex 是隔离模拟 CLI，用于验证参数、标准输入、进程和页面恢复，没有调用模型或验证真实 CLI 登录。两个节点位于同一远端容器，不是两台物理设备；原生交接流程有模拟 IPC 覆盖，真实 KDE 截图确认新 Tauri 窗口正常显示设备网络页，尚未代替跨平台完整原生流程验收。Windows/macOS 和 KDE VNC 的既有验证限制保持如前。

主 Web 和独立 Tauri 客户端均已更新。临时节点、连接、复制记录、授权文件和任务浏览器已清理；主容器仍为 running / healthy，两个入口匿名 401、认证后 200，运行程序与构建摘要一致。部署与备份记录为远端 `/config/validation-artifacts/project-handoff-deployment.json`，真实流程结果为 `project-handoff-deployed-browser.json`，最终复核为 `project-handoff-final-checks.json`，源码摘要为 `project-handoff-source-manifest.json`。截图使用 `project-handoff-` 前缀，KDE 流检查见 `project-handoff-kde-deployed-browser.json`。

## 协作任务绑定 Agent 执行与明确结果报告

Web 与 Tauri 的共享任务卡支持在接收设备上选择 Codex / Claude Code、工作目录及权限，检查完整指令和实际命令，再确认领取并启动。收到任务和检查配置均不会启动进程。每个派发尝试固定绑定一个运行及会话；启动响应未知或排队失败时，可以查询原记录或明确重试相同配置，避免重复执行。关闭页面后节点继续持有进程和输出。

任务卡显示 stdout / stderr、运行状态，并可确认停止对应进程树。退出码为 0 不会完成协作任务；用户须核对实际结果，手动选择完成、失败或取消，填写证据后提交。进程仍活动时，任务卡和普通任务 Finish 入口均不允许提前报告最终结果。消息 MCP 没有获得这一本机启动权限。详细边界见 [设备协作任务](device-tasks.md)。

构建和测试仍全部在 `192.168.217.6:22022` 的验证容器完成。新增任务运行集成测试 6 条通过，覆盖真实双节点、检查不执行、标准输入、唯一运行、UTF-8 输出分页、观察端重启、队列失败后的明确恢复、停止与新尝试、撤权及 MCP 边界；相关任务、后台执行、执行 MCP 和项目交接回归 37 条通过。Web 任务与后台执行浏览器测试 18 条通过，客户端任务及问答模拟 IPC 测试 3 条通过。Rust 格式检查和 runtime、node、web、desktop 所有 target 的 Clippy（`-D warnings`）通过；Web / Tauri React、服务端、独立节点及原生客户端构建成功，原生构建使用 `desktop,custom-protocol`。CI 已加入新测试，此处不代表 GitHub Actions 已执行。

更新后的实际 Web 已验证收到独立发送节点的任务，页面检查完整指令且不启动进程，任务文本中的 HTML 保持纯文本。为避免调用真实模型或替换账户 CLI，该实页验收通过明确的本机 CLI 操作、仅对该调用设置的私有 PATH，启动隔离替身并绑定已部署节点的同一任务；页面随后读取真实绑定和输出。关闭整个浏览器后进程仍运行，退出后任务仍为 running；重新登录恢复相同运行，启动计数保持一次。页面必须选择结果，提交的失败报告已由发送节点同步确认。此实页流程不等于在主节点使用真实 Codex / Claude 登录启动模型；页面确认启动及原生 IPC 路径另有上述自动化覆盖。

375px / 1440px 深浅主题无横向溢出，浏览器无脚本错误，凭据和任务正文没有写入浏览器存储。KDE / Selkies 视频流正常，新原生程序保持运行且摘要与构建一致；本轮没有将客户端模拟 IPC 用例视为完整原生任务界面实机验收。测试记录已归档，临时授权、监听、发送节点、文件和任务浏览器均已清理，归档回执继续按账本保留规则去重。两个验证空间及其管理容器保持停止，主容器继续 running / healthy，两个入口匿名 401、认证后 200。

部署及上一版本备份见远端 `/config/validation-artifacts/task-runs-deployment.json`；实际页面结果为 `task-runs-deployed-browser.json`，最终容器与认证复核为 `task-runs-final-checks.json`，KDE 流检查为 `task-runs-kde-deployed-browser.json`，源码摘要为 `task-runs-source-manifest.json`，截图使用 `task-runs-` 前缀。记录保留远端机器的原始 UTC 时间戳。本轮未验收真实模型执行、Windows/macOS、跨物理设备组网、KDE VNC、自动 worker、DAG 或运行会话迁移。

## 授权项目目录的增量拉取与断线补齐

Web 与 Tauri 的共享 Rust 读取适配器支持 project-index-v1：首次读取完整授权目录，后续发送 ProjectID 与内容摘要索引，只取得变化项目、删除 ID、当前顺序和目录校验值。所有者先按持久 NodeID 过滤目录，每次请求重新检查当前授权；读取方核对基准、身份、权限、唯一 ID、全部副本的归属及重建摘要，再替换私有快照。工作副本删除可跨断线补齐，所有者恢复较旧备份也能按当前内容重新收敛，不依赖持续增长的修订号或无限保留的删除历史。旧节点继续完整读取；只有新端点明确返回 404 才允许兼容回退。

同时修复了离线分类：主 EasyTier 转发停止时，备用路径返回的明确撤权不能被当成网络中断，不能继续展示旧缓存。已停止的转发使用类型化错误，仅实际连接不可用才进入只读快照；认证和无效协议响应仍清除缓存。

所有构建和运行验证继续在 192.168.217.6:22022 的 cranibox-validation 容器执行。新增真实 HTTP 集成测试 7 条通过，覆盖空增量、修改、部分副本及完整项目删除、离线快照、Web 和节点重启、备份回退、撤权、停止转发与备用路径组合、错误响应、旧节点兼容、索引边界和目标/Origin 检查；相关目录、直连主机和多路径回归 27 条通过。运行库单元测试 77 条通过，其中本轮增加目录合并和私有写入检查；整组测试将临时目录放到远端 NAS 持久化文件系统执行。Rust 格式和 runtime、node、web、desktop 所有 target 的严格 Clippy 检查通过，服务端、节点、运行库、Web 和原生客户端构建成功；原生构建使用 desktop,custom-protocol。已将增量集成测试加入 CI，此处不表示 GitHub Actions 已运行。

实际部署验收最初发现 NAS 的 trimacl 挂载继承 ACL：即使创建参数为 0600，文件也显示为 0700；显式设置权限后才是 0600。共享私有写入助手现于写入任何内容前，对已打开的临时 inode 显式收紧到 0600，再按原有原子替换/不覆盖规则提交。补测验证新建与替换的文件权限、不覆盖已有内容及临时文件清理；重新部署后的真实快照权限也已通过检查。本轮没有批量调整其他既有文件权限。

已部署的实际浏览器页面使用独立真实节点及透明 HTTP 代理验证完整读取后转为更小的空增量、外部新增与删除、目标停止、整个浏览器关闭和重新登录后恢复只读快照、目标重启后的遗漏变更补齐，以及撤权后移除页面记录与持久缓存。目录与授权响应来自真实 Rust 节点；代理只在目标停止时将连接错误转换为 503。两个节点身份仍位于同一远端容器，不代表两台物理设备。375px / 1440px 深浅主题无横向溢出，没有页面脚本错误、Token 或项目元数据写入浏览器存储。测试节点、连接、私有授权文件及浏览器已清理。

主 Web 和独立原生客户端均已更新，运行摘要与构建一致；主容器继续 running / healthy，Web 与 KDE 入口匿名 401、认证后 200，两个验证空间及管理容器保持停止。部署及备份记录位于远端 /config/validation-artifacts/catalog-sync-deployment.json，实页结果为 catalog-sync-deployed-browser.json，最终复核为 catalog-sync-final-checks.json，源码摘要为 catalog-sync-source-manifest.json，截图使用 catalog-sync- 前缀。保留远端机器原始 UTC 时间戳。

此轮实现的是已配置授权直连来源的目录元数据增量拉取，不复制项目文件，不提供全网自动分发、离线编辑、信任传播、通用事件复制或运行会话迁移。网关旧目录入口与直接 CLI/MCP 读取仍走完整目录；跨平台和 KDE VNC 实机验收限制保持如前。原生客户端本轮完成构建、更新和运行核对，共享协议通过真实 Web 适配器验证，没有将这些结果当作完整原生交互验收。

## 任务 Agent 的沟通工具与待核对候选结果

Web 与 Tauri 的共享任务卡新增默认关闭的“启用任务沟通工具”。本机操作者检查并确认后，Codex / Claude Code 的本次启动配置加入专用 stdio MCP，只提供绑定当前任务、派发和运行的 cranibox_task 工具。Agent 可读取上下文、报告进度、提问、等待回复或取消，以及保存候选结果；不能通过该工具访问其他任务、启动程序或完成任务。每次调用复核原授权及运行归属，撤权、轮换、旧派发、归档和进程停止会阻止继续使用。详细参数和信任边界见 [设备协作任务](device-tasks.md)。

候选结果在接收节点持久保存，不增加任务回执线协议。进程退出不会完成任务，页面刷新也不会自动采用候选。操作者需点击“填入报告供我核对”；已有草稿先确认替换，再允许修改结果和证据，明确提交后才同步给发送节点。原有消息 MCP 没有增加执行权限，未启用沟通的旧绑定保持原有启动行为。

所有构建、运行及浏览器验证继续位于 192.168.217.6:22022 的 cranibox-validation 容器。任务 MCP 集成测试 6 条与相关任务、执行回归 24 条通过；Rust 运行库单元测试 77 条、Web 任务及后台执行浏览器测试 19 条、客户端任务与问答模拟 IPC 测试 3 条通过。新增测试覆盖真实 stdio / HTTP、等待回复与超时、取消、撤权与凭据轮换、旧派发、UTF-8 限额、单工具范围、候选持久化与不可改写、观察端退出和各独立 Rust 入口。两种隔离模型 CLI 直接解析实际注入的 TOML / JSON 配置，再启动真实 MCP 子进程，未调用模型。

Rust 格式检查及 runtime、node、web、desktop 全部 target 的 Clippy（-D warnings）通过；Web / Tauri React 与服务端、节点、运行库、独立 Web、原生客户端构建成功，原生使用 desktop,custom-protocol。构建时确认静态前端先生成，再编译内嵌资源的服务端；首次浏览器测试遇到旧资源，按该顺序重建后 19 条全部通过。CI 已加入任务 MCP 集成测试，此处不表示 GitHub Actions 已执行。

已部署的真实 Web 页面收到独立发送节点任务后，验证默认关闭、明确勾选和检查期间不启动程序。通过仅本次调用使用的私有 PATH 启动隔离 Codex 替身，读取真实注入配置，调用真实 Rust MCP 提问。关闭整个浏览器后发送节点通过真实任务协议回复，执行方等待返回并保存候选；重开页面时进程仍活动，候选可见但不能填入报告。再次关闭浏览器并让进程退出后，重新登录恢复相同运行和候选，启动计数仍为一次。页面拒绝无确认覆盖草稿，明确采用后可以修改为失败，最终人工报告已由发送节点同步确认，候选中的 HTML 始终为纯文本。375px / 768px / 1440px 深浅主题无横向溢出，浏览器无脚本错误，凭据和任务正文没有写入浏览器存储。

更新后的原生客户端二进制也直接通过内部入口读取同一活动任务，确认只公开一个作用域工具；实际 KDE 客户端保持运行，摘要与构建一致。这不代替完整原生任务交互验收。两个节点仍是同一远端容器内的独立身份进程，不代表跨物理设备、Windows/macOS、真实模型或安装版本的兼容性已验收。自动 worker、DAG、交互式终端输入和运行会话迁移仍待实现。

主 Web 和独立 Tauri 客户端均已更新并保留前一版本备份。临时任务及运行归档，测试发送节点、私有授权文件、临时监听和浏览器均已清理；两个验证空间及管理容器保持停止。主容器继续 running / healthy，镜像 ID 与本工作区原容器一致，Web / KDE 入口匿名 401、认证后 200。部署记录为远端 /config/validation-artifacts/task-worker-deployment.json，实页结果为 task-worker-deployed-browser.json，最终复核为 task-worker-final-checks.json，源码摘要为 task-worker-source-manifest.json；截图使用 task-worker- 前缀。文档日期沿用用户环境的 2026-10-18，部署记录保留远端机器原始 UTC 时间戳。

## 所有设备的 Agent 汇总与定向委托

Web 与 Tauri 的共享任务页面新增“所有设备的 Agent”，同时展示本机与已保存连接设备的公开登记，支持按名称、运行工具或设备 ID 查找。默认仅展示有效登记，可以显式查看已过期和已结束的登记。每个来源单独读取，正常情况下最多四个并行请求；一个设备读取失败时清除该来源旧登记，其余来源继续刷新。暂停自动刷新不会暂停五分钟登记租期的计算。连接变更会丢弃旧一轮的迟到响应，页面隐藏时暂停发起目录请求；目录不写入浏览器存储。

“委托给此 Agent”保留用户已写的任务草稿，选择精确设备与会话并聚焦任务标题，不提交任务或启动程序。提交前重新读取该设备登记，读取失败或会话过期时阻止新委托；不会自动改投其他 Agent。共享解析器核对目录所有者、租期、容量、唯一会话 ID 及公开字段格式。登记仍是 Agent 自报状态，不代表进程发现、自动 worker 或自动取得执行权限。

构建、测试及真实浏览器操作继续全部位于 192.168.217.6:22022 的 cranibox-validation 容器。新增汇总目录浏览器用例 4 条通过，覆盖混合状态、草稿保留、重复选择重新核对、独立失败与恢复、错误身份、连接移除与迟到响应、暂停后的租期流逝及慢来源不阻塞其他来源。相关任务和后台执行用例 19 条、消息用例 7 条通过，合计 30 条不同 Web 用例；消息回归将旧的过期登记测试数据修正为固定 300 秒租期后重新通过。客户端任务、消息与问答模拟 IPC 用例 4 条通过。Web 和 Tauri 的 TypeScript 及生产构建、Rust 服务端、独立 Web 和原生客户端构建、独立 Web HTTP 与操作检查通过；原生构建使用 desktop,custom-protocol。本轮没有 Rust 源码改动。

已部署的真实登录页面同时读取两个独立真实 Rust 节点的登记，验证含 HTML 的名称按纯文本显示，以及 375px / 768px / 1440px 深浅主题无横向溢出。通过页面明确提交给一个会话后，目标节点使用同一 UUID 领取并报告，发送页面同步显示实际结果。随后撤销授权，受影响来源的卡片清空、已选目标禁止提交，另一来源仍可读取；新文件导出的轮换令牌经显式重新连接后恢复，移除连接后来源消失。没有调用模型；这两个节点仍位于同一远端容器，不代表两台物理设备或 Windows/macOS 验收。客户端模拟 IPC 及构建运行核对也不代替完整原生界面操作验收。

主 Web 与独立 Tauri 客户端已更新，运行程序摘要与构建一致，旧程序备份保留。临时任务归档，测试连接、节点进程、授权文件及浏览器已清理，原有连接数量保持不变。主容器继续 running / healthy，镜像与本工作区原容器一致，Web 与 KDE 入口匿名 401、认证后 200；两个验证空间和管理容器保持停止。部署及备份见远端 /config/validation-artifacts/federated-agents-deployment.json，实页结果为 federated-agents-deployed-browser.json，最终复核为 federated-agents-final-checks.json，源码摘要为 federated-agents-source-manifest.json，截图使用 federated-agents- 前缀。远端原始 UTC 时间戳仍按机器实际时间保留。

## 多设备协作 Run、统一停止与核对归档

共享任务页面新增持久的多设备协作：固定名称、共同目标、参与设备及可选 ProjectID；各委托保留 NodeID + RunID 上下文，接收方 MCP 和经确认的 Agent 启动检查均能读取。进度、成功/失败/取消及待回复任务按协作汇总，接收的同名 RunID 按协调节点区分。表单保留失败草稿，创建响应未知时沿用相同 ID；加入协作时限制目标为原成员，不丢失已有标题和说明。

统一停止一次持久保存全部未完成任务的取消请求，停止后拒绝新委托与新尝试。离线和超时不代表工作已停止，只有全部当前尝试收到最终回执后才允许填写核对总结并结束；归档整次协作与其委托，去重记录保留到保存期限。状态更新使用修订号，连接/凭据变化不能把原协作改投新绑定。详细接口与升级边界见 [协作任务](device-tasks.md#将多项委托放入同一次协作)。

所有构建和运行验证继续在 192.168.217.6:22022 的 cranibox-validation 容器完成。Rust 任务单元测试 17 条通过，其中新增协作测试 4 条；新增真实 HTTP/CLI/MCP 集成测试 5 条，以及任务、任务执行与任务 MCP 回归 24 条通过。覆盖独立接收节点、固定上下文、结果与问题汇总、离线统一取消、观察端重启、成员与凭据绑定、原 ID 恢复、归档去重，以及 MCP 不新增进程启动权限。Web 浏览器测试 20 条通过（协作 4、任务 12、Agent 汇总 4），客户端相关模拟 IPC 测试 2 条通过。Rust 格式检查及 runtime、node、web、desktop 全 target 严格 Clippy 通过，Web/Tauri TypeScript 与生产构建、Rust 服务端、运行库、独立 Web、独立节点及原生客户端构建成功，原生使用 desktop,custom-protocol。CI 加入新集成测试，此处不表示 GitHub Actions 已执行。

部署后的真实登录 Web 接入两个独立身份的 Rust 节点，页面创建协作并分别委托，接收方核对完全一致的上下文、明确领取、提问并报告。整个浏览器关闭后，节点仍能传递答案与最终回执；重新登录恢复同一协作，页面准确汇总一项成功与一项失败，经核对总结后整次归档。另一次协作在执行节点停止监听后请求统一取消，重新登录仍为等待确认且不允许结束；恢复节点后取得明确取消回执，才可核对并归档。375px / 768px / 1440px 深浅主题无横向溢出，HTML 作为纯文本显示，浏览器无脚本错误，凭据与共同目标未写入浏览器存储。

主 Web 和独立 Tauri 客户端均已更新；程序摘要与构建一致，旧程序和升级前主任务账本有私有备份。测试协作归档，原有连接数量恢复，临时节点、私有文件与任务浏览器已清理。主容器继续 running / healthy，镜像与原容器相同；Web/KDE 入口匿名 401、认证后 200，两个验证空间及管理容器保持停止。部署及备份记录为远端 /config/validation-artifacts/collaboration-runs-deployment.json，真实流程为 collaboration-runs-deployed-browser.json，最终复核为 collaboration-runs-final-checks.json，源码摘要为 collaboration-runs-source-manifest.json；截图使用 collaboration-runs- 前缀。记录保留远端原始 UTC 时间。

两个接收节点仍为同一远端容器内的独立进程，不等于跨物理设备或 Windows/macOS 验收；本轮不调用模型，原生客户端模拟 IPC、构建和运行核对也不替代完整原生交互。当前 Run 提供持久协作上下文与汇总，没有 DAG、自动 worker、任务库全网复制、协调者选主或运行会话迁移。

## 协作任务的前置依赖与派发

在上述协作 Run 基础上补齐基础依赖：Web 和 Tauri 共用前置任务选择器，最多选择同一协作中 8 项已保存任务。发送方只有在全部当前回执明确成功后，才持久记录放行并投递后续任务；失败、取消、失联及手动同步均不能绕过条件。页面保留创建失败的选择与请求 ID，显示等待原因、前置任务报告和等待数量。尚未放行的委托可在本机明确取消；已经放行的委托仍等待原目标确认，避免把网络不确定性当成未发送。

本轮构建、运行及浏览器验证全部在 admin@192.168.217.6:22022 的 cranibox-validation 容器完成：

- 23 项 Rust 任务单元测试通过，包含非法依赖、持久放行、重启、凭据变化、统一停止、并发边界与损坏账本保护。
- 34 项真实服务集成测试通过，其中新增 5 项依赖测试，并回归协作 Run、设备任务、任务执行与专用 MCP。
- 23 项 Web 浏览器测试、2 项 Tauri 模拟 IPC 测试通过；Web 和 Tauri 的 TypeScript、生产资源及 Rust 构建通过，相关 Rust 包和桌面特性的严格 Clippy 通过。独立 Web HTTP、页面资源及操作检查通过。
- 部署后的页面创建跨两个独立身份节点的协作，验证双前置等待、失败后等待、明确重试后放行；浏览器关闭期间后台继续同步，目标收到任务时尚未被领取。重新打开页面可核对前置报告并结束、归档协作。
- 两个目标离线时统一停止，未派发任务立即保留本机取消记录；已派发任务仍等待目标恢复并返回明确取消回执，之后才能结束协作。375、768、1440 像素的明暗主题均无横向溢出，页面无 JavaScript 错误，报告保持纯文本。

Web 与 KDE 中运行的 Tauri 客户端均已更新，部署程序摘要与构建一致，旧程序及升级前任务账本有私有备份。测试协作已归档，原连接数量恢复，临时进程、配置与任务浏览器已清理。容器继续 running / healthy，镜像与原容器一致；Web 3080 和 KDE 3031 入口匿名返回 401、认证后返回 200。验证空间及其管理容器保持停止。

远端 /config/validation-artifacts 下的 task-dependencies-deployment.json 记录部署及备份，task-dependencies-deployed-browser.json 记录真实流程，task-dependencies-final-checks.json 记录容器、入口和程序复核，task-dependencies-source-manifest.json 记录源码摘要；截图使用 task-dependencies- 前缀。记录保留远端机器的原始 UTC 时间。依赖状态只保存在发送方账本，不扩展接收方任务协议；使用新字段后回退发送方程序需要匹配的账本备份。

此次验收仍使用同一远端容器内的独立节点进程，未验证跨物理设备、Windows/macOS 或真实模型。依赖只约束派发顺序，不复制项目产物、不自动领取或启动 worker；复杂自动编排、协调者状态复制与运行会话迁移仍待实现。

## Tailscale 多端口持续发现与连接地址选择

Web 和 Tauri 共用的网络页支持填写 1–8 个节点监听端口，以逗号或空格分隔，按填写顺序选择主要地址。后台对每轮最多 128 台在线设备使用共享的 8 路并发检查；同一设备的多个监听返回相同 NodeID 时合并为一个节点，并允许在接入前选择地址。不同设备返回相同 NodeID 时隐藏该冲突身份。发现不授予终端、桌面或其他能力，接入仍需目标单独授权。旧的单端口配置和 API 保持可读可用，新数组配置在节点重启后恢复；修改端口或网络成员会取消旧扫描并清除旧结果。

所有构建和运行验证继续位于 admin@192.168.217.6:22022 的 cranibox-validation 容器：

- 20 项 Rust 网络单元测试通过，另有 2 项真实 EasyTier 测试本轮未执行。隔离 TCP 监听验证多个端口、同机多节点、相同身份的多地址合并、跨设备身份冲突和主地址顺序不受响应速度影响。
- 14 项真实 Rust 服务集成测试通过，包括多端口配置保存与重启、旧字段兼容、重复/越界/过多端口拒绝且不改原文件，以及原有网络授权、邀请和转发回归。
- 8 项 Web 浏览器测试与 2 项 Tauri 模拟 IPC 测试通过。界面覆盖错误输入聚焦、首次点击不因错误提示移动而失效、保存失败保留草稿、刷新恢复、选择第二个地址后准确提交该端口、停止发现和不自动接入。
- Web 与 Tauri 的 TypeScript、生产资源和 Rust 构建通过，原生使用 desktop,custom-protocol；相关 Rust 包所有 target 的严格 Clippy 及全工作区格式检查通过。

实际部署的 Web 返回新的 ports 状态字段，未连接 Tailscale 时不展示发现设置；375、768、1440 像素及横屏的深浅主题均无横向溢出，放大根字号后仍能访问内容，浏览器没有脚本错误。主网络配置未被实页检查改动，测试浏览器已关闭。容器当前尚未连接真实 Tailnet，因此部署页面只验收未连接状态；发现结果和多地址选择的界面测试使用模拟 API，扫描使用隔离监听，配置接口使用真实 Rust 服务。上述结果不代表真实 Tailnet、跨物理设备、Windows/macOS 或完整原生交互已验收。

主 Web 与独立 Tauri 客户端均已更新，运行程序摘要与构建一致，旧程序保留备份；已有网络配置也按存在情况备份。容器继续 running / healthy，镜像与本工作区原容器一致；Web 3080 和 KDE 3031 入口匿名 401、认证后 200。两个验证空间及管理容器保持停止。

远端 /config/validation-artifacts 中，multiport-discovery-deployment.json 记录部署及备份，multiport-discovery-deployed-browser.json 记录实页检查，multiport-discovery-final-checks.json 记录入口、容器和程序复核，multiport-discovery-source-manifest.json 记录源码摘要；截图使用 multiport-discovery- 前缀，其中 mock 明确表示模拟网络响应。记录保留远端原始 UTC 时间。新数组配置使用后回退旧程序需要匹配的网络配置备份。此功能仍是显式配置端口的有界探测，不提供能力广播或自动路径切换。

## 设备邀请的中断恢复

修复首次领取已在服务端保存、响应却丢失后不能重试的问题。客户端在请求前保存私有恢复凭据，服务端只保留它的哈希；原设备可以在邀请原有的 15 分钟有效期内恢复，其他领取请求被拒绝，重试不延长有效期。双方服务重启以及本地配置写入失败都保留恢复机会。网络配置成功保存后清理客户端恢复文件。Web 显示已领取状态、隐藏原来展示的邀请 Token，并保留撤销入口；Web/Tauri 加入表单提示使用相同地址和邀请重试，请求期间禁用输入。接口和升级边界见 [设备网络](device-networks.md#使用-workspace-地址与短期邀请)。

本轮继续只在 admin@192.168.217.6:22022 的 cranibox-validation 容器构建及验证：

- 23 项 Rust 网络单元测试通过，另 2 项真实 EasyTier 网络测试本轮未执行。新增覆盖私有恢复凭据、并发领取只允许一个胜出、重启恢复、撤销、到期不延长以及损坏文件保护。
- 16 项真实 Rust 服务集成测试通过。代理先成功领取再丢弃响应，随后重启双方服务，并在下一次领取响应到达前制造本地配置写入失败；最终仍用同一恢复凭据成功保存，撤销后再次请求被拒绝。另覆盖恢复请求边界、旧服务端明确拒绝时提示升级且不降级重试。
- 8 项 Web 浏览器测试与 2 项 Tauri 模拟 IPC 测试通过。检查加入失败保留输入、请求期间禁用输入、领取后隐藏 Token、页面重载后的已领取状态与撤销，以及客户端通过 IPC 加入。共计 49 项相关测试通过。
- Web/Tauri TypeScript 与生产构建、Rust 服务端、独立节点、运行库、Web 及原生客户端构建通过，原生使用 desktop,custom-protocol。工作区格式检查及 runtime、node、web、desktop 所有 target 的严格 Clippy 通过。

另用两个独立 NodeID 的实际 Rust Web 页面创建并领取邀请，模拟服务端已领取但响应丢失，确认错误提示和输入保留；关闭整个浏览器后重新打开，原设备使用同一邀请成功保存配置。邀请方页面显示已领取、隐藏原 Token，并能在重新打开页面后撤销。375、768、1440 像素的深浅主题均无横向溢出，浏览器无脚本错误，邀请及恢复凭据没有写入浏览器存储。此流程使用隔离的 EasyTier 进程替身，只验证引导配置，不代表真实网络、跨物理设备或 Windows/macOS 已验收；Tauri 模拟 IPC 也不替代完整原生交互。

主 Web 和 KDE 中的独立 Tauri 客户端已更新，运行程序摘要与构建一致，旧程序保留备份。主网络配置没有被实页验证改动，隔离节点、临时凭据、监听和任务浏览器已清理。主容器仍为 running / healthy，镜像与当前开发容器一致；Web 3080、KDE 3031 入口匿名 401、认证后 200。两个验证空间及其管理容器保持停止。

远端 /config/validation-artifacts 下，invitation-recovery-deployment.json 记录部署与备份，invitation-recovery-real-ui.json 记录实际邀请页面流程，invitation-recovery-deployed-browser.json 记录主 Web 实页检查，invitation-recovery-final-checks.json 记录入口和程序复核，invitation-recovery-source-manifest.json 记录源码摘要；截图使用 invitation-recovery- 前缀，claimed 表示隔离 Rust 服务的已领取页面。已领取记录包含新字段，回退旧服务端时需要匹配的邀请账本备份。

## 项目文件分块传输、准备恢复与进度

新版节点声明 project_copy_protocol: 2，项目复制改为私有磁盘暂存及最多 1 MiB 的分块，限制为总计 2 GiB、单文件 1 GiB、32768 条目录及文件、8 MiB 清单。每块和整文件均校验 SHA-256；两端有旧版节点时仍使用原有 16 MiB 协议。快照准备暂停后可以沿用原 ID 从来源固定快照继续；发送阶段先查询目标持久回执，再按目标字节位置续传。关闭页面不会取消入口已接管的工作，进程重启后需要明确继续。发布到新目录并登记完成之后的重复确认保留目标后续修改，不覆盖用户目录。

Web 与 Tauri 共享复制进度、暂停恢复和确认界面。页面轮询精简进度，不重复发送文件清单或自动重试写请求；超过 200 条的清单支持路径筛选及分批展开。准备恢复完成后仍须检查清单并勾选确认。入口、来源和目标分别限制记录数量；目标达到 256 份新版回执后仍可恢复已有记录，新增请求在创建目录前拒绝。详细接口、限制和升级行为见 [项目与会话目录](project-session-catalog.md)。

本轮构建、运行与浏览器操作均在 admin@192.168.217.6:22022 的 cranibox-validation 容器完成，本地只编辑和同步文件：

- 9 项 Rust 项目复制单元测试通过，覆盖分块幂等、未确认尾部修复、发布边界恢复、撤权、路径及链接保护、容量边界与满额续传；runtime 和 node 所有 target 的严格 Clippy、工作区格式检查通过。
- 13 项真实 Rust HTTP 服务集成测试通过，其中 4 项新测试覆盖约 20 MiB 快照准备中断、来源及入口重启、来源文件随后修改而固定快照不变、写入成功但回包丢失、接收端重启、关闭调用连接继续执行及旧版降级。最终构建另重跑写入丢回包与双端重启用例通过，不重复计数。
- 8 项 Web 浏览器测试和 4 项 Tauri 模拟 IPC 测试通过，覆盖进度、关闭再打开面板、暂停恢复、清单筛选、显式确认和迟到响应不能回退完成状态；合计 34 项不同测试通过。
- Web/Tauri 的 TypeScript 与生产构建，以及 Rust 服务端、独立节点、运行库、独立 Web 和原生客户端构建通过。原生客户端启用 desktop,custom-protocol。CI 已纳入新集成测试，此处不表示 GitHub Actions 已执行。

2 GiB/1 GiB 是代码限制，并用合成清单验证边界；真实传输验证规模约 20 MiB，不代表已完成 2 GiB 压力测试。原生 IPC 测试和 Linux 构建也不替代 Windows/macOS 原生交互验收。当前仍是明确确认的项目快照复制，尚不提供工作文件持续增量同步或运行中 Agent 的检查点迁移；项目目录记录的增量读取与文件同步是不同能力。

主 Web 和 KDE 中的独立 Tauri 客户端已更新，运行程序摘要与构建一致，旧程序保留备份。实际登录页面能读取新版精简进度接口，375、768、1440 像素与横屏的深浅主题、200% 根字体均无横向溢出，浏览器无脚本错误，主网络配置未改变。容器继续 running / healthy，镜像与当前开发容器相同；Web 3080 和 KDE 3031 入口均为匿名 401、认证 200。两个验证空间和管理容器保持停止。

远端 /config/validation-artifacts 下，project-streaming-deployment.json 记录部署与备份，project-streaming-deployed-browser.json 记录实页检查，project-streaming-final-checks.json 记录入口和程序复核，project-streaming-source-manifest.json 记录源码摘要。实页截图使用 project-streaming- 前缀，progress-375 为隔离浏览器用例的传输进度截图。

## 接收设备的可选后台自动接单

Web 与 Tauri 的共享任务页新增“本机自动接单”。入口默认折叠、关闭；接收设备明确选择当前消息授权的发送方、固定目录、Agent、文件权限、1–4 个并发进程及回报方式，检查后勾选确认保存。配置绑定当前授权摘要、目录、实际程序和修订号。保存响应未知时可幂等重试；其他窗口修改配置后保留表单，允许按新版本重新检查并再次确认，不自动覆盖。发送方及消息 MCP 不能配置接收端策略。

节点后台每两秒处理队列，关闭页面不影响运行。唯一的 TaskID / DispatchID、执行和会话绑定先持久保存再入队；执行历史满额时可恢复同一绑定，已有进程记录和节点重启中断均不自动重跑。暂停只阻止新任务，撤销或轮换消息授权会停止关联自动进程，重新授权仍须重新确认策略。默认人工报告；可明确选择在进程正常停止、没有未答问题且 Agent 已提交提案后自动回报，成功提案要求退出码 0。退出本身不会完成任务。详细接口与边界见[设备协作任务](device-tasks.md#本机自动接单)。

本轮仍仅在 admin@192.168.217.6:22022 的 cranibox-validation 容器构建和运行，本地只编辑、读取与同步文件：

- 新增 14 项真实 Rust HTTP / 进程管理集成测试通过，另有任务运行 6 项、任务 MCP 6 项、依赖派发 5 项和协作 Run 5 项通过，合计 36 项不同后端测试。覆盖并发和暂停、关闭观察端、取消、授权轮换、正常与失败提案、未答问题阻止回报、重启中断不重跑、历史满额时取消尚未入队的绑定，以及原绑定恢复后只启动一次。
- Web 浏览器相关测试 16 项通过，含新增自动接单 4 项；Tauri 模拟 IPC 相关测试 6 项通过，含新增自动接单 1 项。本轮共 58 项不同测试通过，重跑不重复计数。界面覆盖默认关闭、检查不执行、明确确认、保存响应丢失后原请求重试、配置冲突保留草稿并重新检查、错误身份拒绝、移动布局及外部文本按纯文本显示。
- Web / Tauri TypeScript 与生产构建、Rust 服务端、独立节点、运行库、独立 Web 和原生客户端构建通过，原生启用 desktop,custom-protocol。runtime 和 node 所有 target 的严格 Clippy（-D warnings）、全工作区格式检查通过。最终 Web 用例使用重新编译后的内嵌资源验证，避免旧二进制内嵌前一版页面。CI 已加入新集成用例，此处不表示 GitHub Actions 已执行。

部署后实际登录主 Web，验证新的自动接单状态接口返回正确 NodeID、空策略与修订号 0；配置表单默认只读、人工报告，缺少发送设备和目录时不能确认。375、768、1440 像素及横屏的深浅主题、200% 根字号均无横向溢出，浏览器无脚本错误。截图复核后修正继承样式导致的复选框纵向排列；实页再验确认复选框与说明同行，整行点击区域不少于 44px。实页操作仅查询和打开草稿，未保存策略；网络配置及 task-worker.json 均保持不存在，未授予新权限，浏览器已关闭。

主 Web 与 KDE 中独立 Tauri 客户端已更新，运行程序摘要与构建一致，旧版本保留备份。主容器仍为 running / healthy，使用与当前工作区相同的镜像；Web 3080 和 KDE 3031 入口匿名 401、认证后 200。主容器未增加宿主 Docker socket，两个验证空间及管理容器保持停止。

测试使用同一远端容器内的独立 NodeID 和隔离 Codex / Claude Code 替身，没有调用真实模型或验证账户登录。模拟 IPC 及原生程序运行核对不替代完整 Tauri 自动接单交互、Windows/macOS、跨物理设备组网或 KDE VNC 验收。复杂自动编排、交互输入和运行中会话迁移仍未实现。

远端 /config/validation-artifacts 下，automatic-worker-deployment.json 记录部署与备份，automatic-worker-deployed-browser.json 记录实际页面检查，automatic-worker-final-checks.json 记录容器、入口和程序复核，automatic-worker-source-manifest.json 记录源码摘要。截图使用 automatic-worker- 前缀，记录保留远端机器的原始 UTC 时间戳。

## 节点后台项目目录同步

Web 与 Tauri 共用项目页新增默认折叠、默认关闭的后台目录同步设置。操作者明确选择 1–32 个已有连接的设备及 15/30/60/300 秒间隔；Rust 节点在页面关闭后继续拉取项目目录和会话关联，最多四路并发。配置和进度跨节点宿主重启保留，不同步工作文件，也不会自动把所有设备加入策略。凭据或连接改变后需重新选择；暂停阻止后续读取，已开始的读取可以完成。并发交互读取持有凭据锁时，后台记录忙碌并按间隔再试，不阻塞交互读取。接口与恢复规则见[项目与会话目录](project-session-catalog.md)。

本轮所有应用、构建、套接字、浏览器和原生验证仍仅在 admin@192.168.217.6:22022 的 cranibox-validation 容器执行，本机只编辑、核对 Git、计算摘要和同步文件：

- 9 项新增后台同步真实服务集成和 7 项既有项目目录增量集成通过，共 16 项。覆盖默认关闭、无页面请求的定时更新、Web 和独立 headless 节点重启、暂停恢复、交互读取锁、离线保留、拒绝后清理、连接更换需重新确认、配置冲突/损坏保护与最多四路并发。实际节点均为同一远端容器内隔离进程与独立 NodeID。
- Web 浏览器测试 9 项通过，含新同步设置 4 项、联合项目目录 3 项、项目 Shell 2 项；Tauri 前端模拟 IPC 相关测试 9 项通过，含新增同步设置、快照恢复与项目复制/交接。本轮共 34 项不同后端与界面测试通过，重跑不重复计数。保存、暂停、配置冲突保留草稿、错误身份拒绝和外部文本显示使用隔离服务或模拟响应验证。
- Web/Tauri TypeScript 与生产构建通过；Rust 合并服务端、独立节点、运行库、独立 Web、原生客户端构建通过，原生启用 desktop,custom-protocol。全工作区格式检查、runtime/node 所有 target 的严格 Clippy（-D warnings）通过。新增后端测试已纳入 CI；GitHub Actions 状态需按提交查看，不把本轮结果等同于完整三平台矩阵。

主 Web 与 KDE 中独立 Tauri 客户端已更新并保留旧二进制备份，运行程序摘要与构建产物一致。实际登录主页面，确认默认关闭、折叠时不轮询设置、空选择不能启用、默认一分钟间隔；375/768/1440px 及横屏的明暗布局和 200% 根字号无横向溢出，浏览器无脚本错误。实页没有启用同步或添加设备，配置及网络文件保持原状；保存和多节点后台行为由上述隔离测试覆盖，不能写成主实例已启用真实跨设备同步。浏览器已关闭。

主容器保持 running / healthy，镜像与当前开发容器相同，没有 privileged 或宿主 Docker socket；Web 3080 和 KDE/Selkies 3031 均为匿名 401、认证 200。两个验证空间和管理容器保持停止。KDE Wayland 的 VNC 捕获此前未通过，当前 Selkies 可用；Windows/macOS、真实 Tailnet、多台物理设备、全新 Docker 镜像和真实模型调用的未验收状态不变。

远端 /config/validation-artifacts 下，project-sync-deployment.json 记录部署与备份，project-sync-deployed-browser.json 记录实际页面，project-sync-final-checks.json 记录入口和程序复核，project-sync-source-manifest.json 记录本地与远端源码摘要核对。实页截图使用 project-sync- 前缀。完整需求、功能状态、失败边界与下一阶段顺序见[交接记录](implementation-handoff-2026-10-18.md)。

## 提交后的 CI 复核

首次功能提交 3882dc2 对应 GitHub Actions 矩阵运行 39094741447，最终失败。真实 Wayland 作业通过，使用隔离 Sway 和 wayvnc 验证认证、画面、输入及独立节点恢复；这不改变 KDE Wayland VNC 捕获尚未通过的结论。Linux 的 Rust 检查、单元测试、Web 资源复现、浏览器与此前的后端步骤通过，但最终在真实 EasyTier 两节点测试失败，详见下文。

其余平台暴露三个问题，修复提交 6aa57d4 已推送 main：

- 三个平台的 Tauri 前端生产构建均无法解析共享 VNC 组件导入的 @novnc/novnc。客户端自身已有该依赖，问题在 Vite 从共享源码位置解析；客户端 dedupe 列表补入 noVNC，不要求安装整个 Web UI 的依赖。
- Windows 的 workspace 严格 Clippy 在两处 DirBuilder 报 unused_mut，因其可变方法只存在于 Unix 条件分支。改用可变引用绑定，保留 Unix 0700 和 Windows 原有语义。
- macOS 的两项项目复制测试报 copy root changed。临时路径存在 /var 与 /private/var 别名，测试现使用 root() 解析后的根路径；生产代码的规范路径与根目录变化保护保持有效。

修复验证仍只在 admin@192.168.217.6:22022 的 cranibox-validation 容器执行：

- 干净客户端目录为 /config/validation-build/clean-node-ui-20261018，仅复制客户端构建所需源码、共享源码和许可证，没有 Web UI node_modules。先用原 Vite 配置复现同一错误，再用修复后的配置完成 npm ci 和 npm run build --prefix apps/node-desktop；TypeScript 与生产资源生成成功。
- cargo fmt --all --check、cargo clippy --workspace --locked --all-targets -- -D warnings 通过。
- cargo test --locked -p agent-workspace-runtime projects::copy 的 9 项单元测试通过，覆盖复制回执、撤权、路径及链接保护、分块恢复等；修复前后重复运行只计同一组测试。

远端 /config/validation-artifacts/clean-node-ui-before.log、clean-node-ui-after.log 和 clean-node-ui-build.json 保留干净构建复现及修复证据，JSON 确认无 Web 依赖目录且生成生产 index。ci-fixes-source-manifest.json 记录三个修复源码的本地与远端摘要核对。

修复后的三平台矩阵 **39095261447 已结束**：Windows/macOS 的 native 作业、三个系统的 node desktop 安装包作业、真实 Sway/wayvnc 作业均通过；Linux native 作业 117580266069 在同一个旧 EasyTier 两节点测试失败，错误仍为 additional path must use the same target access token，其此前步骤均通过。该提交未包含下面的测试修复，整条运行的结论仍是失败。Sync to GitCode、Release 两条工作流已通过；后者没有完成完整 Dockerfile 镜像构建，不能将其绿色状态写作新镜像验收。此前部署验证和未验收项继续有效。

### Release 检查与构建上下文

旧 Release 工作流用 || echo 吞掉 Dockerfile 和 Compose 检查错误。本次显式准备 Buildx，并保留两项检查的真实退出码与日志；ShellCheck 的现有告警策略未改。新增根目录 .dockerignore，排除 Git 元数据、非示例环境文件、Rust/前端编译产物、安装依赖、测试报告和默认 webtop-data 持久数据，保留已提交的 Web 资源、noVNC 许可证及环境示例。

所有 Docker 验证在同一远端 Linux 主机完成，六项均通过：

- 实际 BuildKit scratch 构建将有效上下文导出到隔离目录；11 个必需源码/静态资源均保留，11 个合成敏感或生成文件均被排除，无缺失或泄漏。合成文件不使用真实凭据。
- 当前 Compose 配置检查返回 0；故意损坏的 Compose 返回 1。
- 当前 Dockerfile 的 Buildx --check 返回 0；故意损坏的 Dockerfile 返回 1。

证据报告位于远端容器 /config/validation-artifacts/ci-gates-validation.json；工作目录与私有日志位于 /config/validation-artifacts/ci-gates-__f09ral。此专项仅进行了构建检查及 scratch 上下文验证，没有完整构建产品镜像、创建新产品实例或改变部署。

### 真实 EasyTier 测试的授权恢复

首次矩阵的 Linux 作业 117578716627 在真实 EasyTier 测试处出现 1 项通过、1 项失败。两节点测试先保存错误令牌，验证探测被拒绝，随后错误地通过 easy_tier_connect 直接换成有效令牌。多路径功能已要求新增路径使用同一个目标签发令牌，不能隐式替换现有授权，因而被正常拒绝。三节点入口故障切换测试已通过。

本次仅更新两节点测试的恢复步骤：断言替换令牌被拒绝、原错误连接仍不能探测，然后通过公开 Forget 接口删除错误连接，再使用有效令牌重新连接。没有放宽生产授权检查，也没有删去错误凭据的反向测试。

远端容器下载官方 EasyTier v2.6.4 Linux x86_64 归档，验证 SHA-256 为 61b659eaedba658fa66fe47d17e1426cdd77e5d02fa15fed447bb4357c09dfd6，仅解出两个二进制至 /config/validation-tools/easytier-2.6.4。隔离测试使用独立临时配置与 NodeID，未变更主节点网络：

- 全工作区 cargo fmt --all --check 通过。
- cranibox-node 的所有 target 严格 Clippy（-D warnings）通过。
- 两项真实 EasyTier 测试均通过，0 失败；覆盖错误授权、显式恢复、远程命令、持久 PTY、节点重启、入口故障及备用入口接续。测试执行约 17 秒，不含编译。

命令与 CI 相同：CRANIBOX_TEST_EASYTIER_DIR 指向上述目录，执行 cargo test --locked -p cranibox-node network::tests::real_ -- --ignored --nocapture --test-threads=1。证据在 /config/validation-artifacts/ci-easytier-d_4dykd5/result.json、clippy.log 与 real-easytier.log。它们是同一远端容器内的真实独立网络进程，不替代跨物理设备 NAT、真实 Tailnet 或休眠恢复验收。

## 完整镜像构建与首次启动

本节更新前文“全新镜像未验收”和“CI 仍待复核”的当时状态，保留前面的历史结果。本轮功能代码为 71f239755f9a95f528151ddbebf8ec758a866087；之后的交接提交仅更新文档。所有构建、服务和运行验证继续在 admin@192.168.217.6:22022 的远端 Linux 主机完成，本机只读取、编辑、归档与同步源码和操作 Git。

### 最终 CI

上述功能代码的矩阵运行 39096427553 已完成，结论 success，七个作业全部成功：

| 作业 | Job ID | 结果 |
| --- | --- | --- |
| native (macOS) | 117583743413 | success |
| native (Windows) | 117583743469 | success |
| native (Linux) | 117583743505 | success |
| node desktop (Windows) | 117583743464 | success |
| node desktop (macOS) | 117583743492 | success |
| node desktop (Linux) | 117583743496 | success |
| Real Wayland desktop (Sway and wayvnc) | 117583743535 | success |

Release 运行 39096427579 和 Sync to GitCode 运行 39096427547 也均 completed / success，三条运行的 headSha 均核对为 71f2397。Release 的 test-scripts 与 update-readme-links 成功，已采用严格 Dockerfile/Compose 检查；此次 main 分支运行没有构建和发布完整产品镜像。最终运行与步骤元数据保存为下述证据目录中的 ci-final.json。旧运行 39094741447、39095261447 仍是失败，不因新运行通过而更改。CI 的原生检查与安装包生成不替代目标 OS 的安装、GUI、权限、自启和休眠恢复实测；Sway 的真实 Wayland 结果也不等于 KDE 捕获已修复。

### 完整 Dockerfile 构建

源码来自功能提交的 git archive，大小 9,369,600 字节，SHA-256 为 d846d7d1870340f54752380850997e6a9854cb6b8061d5ee6ebe9df84a348814。没有复制现有 /config、环境文件或工作区未提交文件。使用独立的新 BuildKit 构建器 cranibox-validation-full-build-20261018，版本 v0.25.2、并发上限 2，内存 6 GiB、无额外 swap、CPU 配额 2 核并限定 CPU 10–11；未切换默认构建器。

在归档解出的源码目录实际执行：

~~~sh
sudo -n docker buildx build \
  --builder cranibox-validation-full-build-20261018 \
  --pull --load --progress=plain \
  --build-arg DESKTOP=kde \
  --build-arg USE_CHINA_MIRROR=false \
  --metadata-file ../build-metadata.json \
  -t cranibox-full-validation:71f2397 .
~~~

完整构建于 2026-10-18 04:17:27–04:35:17 UTC 完成，退出码 0，约 1,069.72 秒（17 分 50 秒，包含首次 BuildKit 镜像下载）。Rust release 构建、固定 Herdr 下载、KDE 基础镜像、系统工具、安装脚本、服务与静态资源、最终镜像导出和载入均通过，没有用旧产品镜像叠加替代。基础镜像为 linuxserver/webtop:ubuntu-kde@sha256:ae4419f39bc0b84da2129100df2ee75cb8e2974e0372cfb6b122fdb8d22a0602。

生成镜像 cranibox-full-validation:71f2397，镜像 ID sha256:571d20538e6bae77fbc70cc0209246b4f4c8ca770a6f6f5f96772b8b566a3a04，大小 6,656,198,462 字节，约 6.2 GiB。该镜像包含 KDE 与完整开发工具，不能将 Rust 改写解释成产品镜像很小；本项也未测空闲内存、CPU 或大规模节点资源。镜像仅载入远端主机，没有推送产品镜像仓库。

### 空配置首次启动与真实重启

使用新镜像创建专用容器 cranibox-full-firstboot-71f2397，唯一挂载为独立空目录 firstboot-config 到 /config；未挂旧配置、源码或宿主 Docker socket，未设 privileged。内存上限 4 GiB、CPU 2 核、共享内存 512 MiB、pids 上限 1024；3001/3002 只绑定宿主 127.0.0.1 的随机端口。测试用随机凭据仅保存在远端 0600 环境文件，不提交或展示。

首次启动以及真实 docker restart 后均验证通过：

- 容器健康检查通过，Rust workspace-services 查询 agent-workspace-web.service 为 enabled / active，不需要 workspacectl runtime install。
- 内置 cranibox、workspacectl、cranibox-node 均为 0.1.0，Herdr 为 0.9.3，版本命令退出码 0；兼容控制入口指向合并 Rust 程序。
- 容器内 9080 与 9090 健康接口均返回 HTTP 200、ok: true、protocol_version: 1；Web 根页面和 web.js / web.css 返回 200。
- 真实 Cookie/CSRF 认证后的 POST /api/peers 列表请求成功，API 返回 NodeID 与持久身份一致；运行凭据文件权限为 0600。
- 桌面 HTTPS 3001 的匿名请求返回 401，认证后返回 200。3002 根路径匿名请求为 404，仅记录该路径结果，不将其写作通用认证验收。
- 写入项目目录的临时标记文件，在容器重启后内容保持不变；NodeID 和运行凭据摘要也保持不变。新实例 NodeID 为 node-d5a28862de8e44f08bfbaf620c60ba63，与主验证实例独立。

初版临时辅助脚本错误地断言健康接口包含独立 Web 的 implementation: rust 字段，而合并程序实际返回网关协议版本。两端接口当时均已返回 200；核对生产路由后，只修正辅助脚本对实际协议的预期，未改生产代码。初次检查中止的报告、探测和原脚本分别保留为 firstboot-initial-helper-error.json、first-boot-probe-initial.json、firstboot-run-initial.py；继续验证前核对了专用容器的标签、镜像与挂载归属。最终 firstboot.json 为 passed: true，首次启动和重启探测均保留。该脚本修正不是产品缺陷修复，不把重复探测累计成额外测试数量。

本项覆盖 HTTP/API、默认服务恢复、身份和文件持久化；没有验证新实例的实际桌面画面、输入或 VNC。首次启动通过不能代替公开产品镜像发布/拉取、其他桌面/架构/软件源构建、Windows/macOS 原生安装及其依赖安装向导。

### 清理状态与证据

检查结束后停止专用首次启动容器和专用 BuildKit 构建器，保留镜像、构建缓存、容器及 /config 数据以便复核。停止后的测试容器 State.Health 可能显示 unhealthy，这是停机后的状态；启动和重启期间的健康成功见 firstboot.json 与两个 probe。主容器 cranibox-validation 继续 running / healthy，镜像、NodeID、端口与权限边界保持不变；此前两个虚拟空间及管理容器继续停止。

证据目录：宿主 /vol3/1000/cranibox-validation/data/validation-artifacts/full-image-71f2397，主验证容器内为 /config/validation-artifacts/full-image-71f2397。保留文件包括：

| 文件 | 用途 |
| --- | --- |
| source.json、source/ | 归档提交与摘要、实际构建源码 |
| build.json、build-metadata.json、full-build.log | 完整构建退出码、耗时、镜像元数据与私有日志 |
| buildkitd.toml、create-builder.log | 专用构建器配置与创建记录 |
| firstboot.json、first-boot-probe.json、after-restart-probe.json | 最终通过记录与两次服务探测 |
| firstboot-run.py、firstboot-initial-helper-error.json、first-boot-probe-initial.json、firstboot-run-initial.py | 修正后的辅助脚本及最初检查预期错误的原始记录 |
| firstboot-create.log、firstboot-container.log | 新实例创建及私有服务日志 |
| main-before.json、main-after.json | 主实例前后身份与运行状态核对 |
| ci-final.json | 功能提交三条 Actions 的最终元数据 |

firstboot.env 与 firstboot-config 含私有凭据和运行状态，日志也可能包含敏感字段；保留在远端，不输出或纳入 Git。后续接手优先级与未验收需求以[交接记录](implementation-handoff-2026-10-18.md)为准。

## Linux CI 安装包与原生交互验收

本节补充实际安装包验证，功能源码仍为 71f239755f9a95f528151ddbebf8ec758a866087，没有修改生产代码。全部安装、原生窗口、托盘、套接字和浏览器运行均在 admin@192.168.217.6:22022 的 cranibox-validation 容器完成；本机只下载产物、校验摘要、同步文件及查看截图。环境为 Ubuntu 26.04 LTS、amd64、KDE Wayland，使用现有图形会话的 abc 用户。此项不是新的完整产品镜像首次启动验证。

### 产物来源与实际安装

从已通过的矩阵运行 39096427553 下载 cranibox-node-desktop-ubuntu-latest，Artifact ID 为 12125116753。ZIP 大小 95,334,337 字节，SHA-256 为 0401d3af4ebb9b2192c82def68defe8874317cfc28766ef5d4f5d7f7da7ca929，本机和远端摘要均与 GitHub 元数据的 digest 相同。只解出指定的 .deb 与 AppImage；本节实际运行的是 .deb，AppImage 仅保留待测。

安装前用 dpkg-query 确认 cranibox-node 尚未安装，然后在验证容器内执行：

~~~sh
apt-get install -y --no-install-recommends \
  '/config/validation-artifacts/linux-installer-71f2397/deb/Cranibox Node_0.1.0_amd64.deb'
~~~

安装退出码 0；dpkg-query 确认 install ok installed、版本 0.1.0、架构 amd64。包名是 cranibox-node，安装的 GUI 程序是 /usr/bin/cranibox-node-desktop，--version 返回 cranibox-node 0.1.0、退出码 0。包声明依赖 libayatana-appindicator3-1、libwebkit2gtk-4.1-0、libgtk-3-0；同时安装应用入口和图标。已安装二进制 SHA-256 为 14d38b70b3a84681bebb14b268dec3227dc0b526e59045643140b226bd7a1e10。

临时停止原验证客户端前核对其 /proc 可执行文件与子进程，确认没有活动 Shell；仅停止已确认归属的进程。随后以 abc、真实图形会话的 Wayland/D-Bus 环境和独立 XDG 配置/数据目录启动已安装程序。检查 /proc/PID/exe 与安装文件一致，避免误把此前 debug 客户端的界面算作 release 验收。测试身份为 node-56633093fc67426db15d118245e5850c，与主服务和原客户端分离。

### 真实原生交互结果

通过已认证的 Selkies 3031 桌面画面操作实际 Tauri 窗口；鼠标操作由浏览器发送至桌面，使用真实 IPC，没有模拟 Tauri 响应。以下为人工集成验收，不累计到前文 34 项自动化测试，也不意味着所有客户端业务均已重测。

| 检查 | 结果与边界 |
| --- | --- |
| 已安装 release 程序 | 原生界面实际可见，本机节点状态正常；进程可执行文件、文件摘要和安装包对应 |
| Agent 接入与真实 IPC | 点击“复制配置”，解析剪贴板 JSON；cranibox-devices 与 cranibox-messages 均使用 /usr/bin/cranibox-node-desktop，并指向独立测试 settings.json，配置不含节点 Token |
| 托盘归属与菜单 | 将注册的 D-Bus 托盘服务关联到 release PID；读取到打开、重新连接和明确退出菜单，实际触发打开与退出动作。重连菜单未测试 |
| 关闭窗口与重新打开 | 点击原生窗口关闭按钮后窗口消失，原进程仍运行、托盘 Active；对该托盘的真实 D-Bus 菜单发送 Event，窗口重新出现 |
| 自启动配置开关 | 通过真实设置界面启用，再禁用；检查实际 desktop 文件创建、Exec 参数及删除，并保留开/关截图。没有实际注销/登录系统 |
| 退出确认 | 托盘退出动作打开确认对话框；“继续运行”保留原进程，之后再次打开并选择“退出并结束会话”，进程退出，无需发送终止信号 |
| 后台启动与身份保留 | 手动用 --background 和相同隔离配置启动，进程及托盘存在、窗口隐藏，NodeID 与前次相同 |
| 单实例唤回 | 再次启动已安装程序，第二个进程退出码 0，原后台进程仍运行，原窗口被唤回；最终再次通过确认界面退出 |

自启动插件在本环境实际使用 HOME/.config/autostart，而不是独立 XDG_CONFIG_HOME。测试前该项禁用、文件不存在；启用后文件为 /config/.config/autostart/Cranibox Node.desktop，Exec 为 /usr/bin/cranibox-node-desktop --background。禁用后文件移除，最终再次确认不存在。后续隔离测试不能只设置 XDG_CONFIG_HOME 就假定自启项也被隔离。

最初只给 agent-browser 配置 HTTP 认证时，页面 HTML 可加载，但 Selkies WebSocket 未连接，得到黑色预览。改用独立 Playwright Chromium、带 HTTP 认证的浏览器上下文和仅监听 127.0.0.1:9227 的 CDP 后，确认接收到 162 帧 WebSocket 数据并实际看见桌面，再由命名 agent-browser 会话接入。另修正临时 D-Bus 检查脚本，改用 org.freedesktop.DBus.GetConnectionUnixProcessID 关联进程。这两处都是验证辅助工具修正，没有改动产品；初始黑色截图 release-first-window.png 保留，不计作通过证据。

### 环境恢复与证据

测试后先通过真实退出确认结束临时 release 进程；确认包版本及二进制摘要属于本次安装后执行 dpkg --remove cranibox-node，退出码 0，安装包与 /usr/bin/cranibox-node-desktop 已移除，没有自动清除其他依赖，也没有在远端宿主系统安装该包。原 debug 客户端已恢复，PID 596906、可执行文件 /config/validation-build/target/debug/cranibox-node-desktop，NodeID 仍为 node-82bf96c6f15d4f00923656f7877b8609；托盘归属和原生界面已复核。

测试前的剪贴板已恢复，临时备份文件已删除；测试自启项不存在，专用 agent-browser 会话、预览浏览器与辅助进程已关闭。主容器仍 running / healthy，镜像和 NodeID 与测试前一致；3080 Web 与 3031 桌面入口均为匿名 401、认证 200。产物、隔离数据和证据留在私有目录，未影响此前已停止的空间及镜像测试容器。

最终 acceptance.json 为 passed: true。证据目录为宿主 /vol3/1000/cranibox-validation/data/validation-artifacts/linux-installer-71f2397，容器内 /config/validation-artifacts/linux-installer-71f2397。目录权限 0700，最终报告权限 0600；对文本证据的凭据检查未发现本次使用的环境密码。以下文件保留在远端，不将运行凭据、剪贴板或二进制产物提交 Git：

| 文件 | 用途 |
| --- | --- |
| acceptance.json | 源码与产物绑定、最终通过项、限制、环境恢复、主服务状态及截图摘要 |
| package-before.json、install.log、uninstall.log | 安装前状态、实际安装及卸载记录 |
| release-launch.json、release-tray.json、release-menu.json、release-mcp.json | 已安装进程、托盘菜单归属与真实 IPC 配置检查 |
| release-background-restart.json、release-single-instance.json | 后台启动、身份保留和第二次启动唤回检查 |
| quit-restart-actions.json、final-exit-actions.json | 退出取消/确认、重启及最终退出记录 |
| release-preview.png、release-tray-reopened.png、release-autostart-on.png、release-autostart-off.png | 实际可见窗口、托盘打开和自启动开关截图 |
| release-quit-cancelled.png、release-background-hidden.png、release-single-instance-reopened.png、original-client-restored.png | 退出取消、后台隐藏、单实例窗口与原客户端恢复截图 |
| preview-ready.json、preview.log、check.py、browser.py、preview.mjs | 真实桌面传输与临时验证工具，供复核而非生产实现 |

范围仍限定为该远端 KDE 容器上的 Linux .deb。未执行 AppImage、实际 OS 注销/登录或休眠/恢复、Windows/macOS 原生安装、托盘重连及真实网络恢复；未在 release 包中重测 PTY/Shell 保留和每个业务流程。Selkies 画面不能证明 KDE VNC 捕获成功，前文 KDE Wayland VNC 失败结论不变。下一阶段仍按[交接优先级](implementation-handoff-2026-10-18.md#下一阶段优先级与验收条件)推进。

## 交接前 KDE 捕获条件复核

收尾时对同一远端 cranibox-validation 做只读检查，没有安装依赖、启动新的 VNC、修改合成器设置或重启原桌面。通过现有 Plasma 会话的 D-Bus 环境调用 qdbus6 org.kde.KWin /KWin org.kde.KWin.supportInformation，返回 Compositing Type: QPainter；容器内 /dev/dri 不存在。

分别连接内层 KDE wayland-0 和 Selkies 外层 wayland-1 运行 wayland-info，两次退出码均为 0。两层都未公布 wlr screencopy 或 ext image-copy-capture 协议；内层没有虚拟键盘/指针，外层只有 zwp_virtual_keyboard_manager_v1、没有虚拟指针。因此不能通过把 wayvnc 的 WAYLAND_DISPLAY 改成外层 socket 来完成当前桌面共享。该结论仅针对本环境，不能推断所有 KDE 主机均不可用，也没有将协议探测算作新的 VNC 画面/输入验收。

证据保存在 /config/validation-artifacts/kde-capture-followup-20261018，目录权限 0700：inspection.json、kwin-support-information.txt、wayland-0-protocols.txt、wayland-1-protocols.txt 均为 0600。同时重新读取功能提交 71f2397 的 ci-final.json，确认矩阵七个作业及 Release、GitCode 同步的保存记录均为 success；Linux 安装包 acceptance.json 仍为 passed: true。上述是既有证据复核，不是重跑 CI 或安装包验收。最后重新请求 3080 Web 和 3031 桌面入口，均为匿名 401、认证 200，结果为同目录 0600 的 entrypoint-checks.json。主验证容器此时 running / healthy，仍使用原镜像，没有 privileged 或宿主 Docker socket。

## Tailscale 安装引导收尾

按最终交接要求，收尾既有的系统 Tailscale 安装引导，更新[功能需求总表、验收边界与后续优先级](implementation-handoff-2026-10-18.md)。以下测试均在 admin@192.168.217.6:22022 的 cranibox-validation 中以 abc 用户执行；本机只编辑、同步文件、查看截图和操作 Git。没有在本机创建产品服务或测试套接字。

### 实现与限制

共享 Web/Tauri 页面先请求 tailscale_setup_plan，展示平台、实际命令、内置脚本摘要和管理员权限；勾选确认后才发送 tailscale_setup。后端仅接受十分钟内、单次、当前系统条件仍一致的方案 ID，不接受任意程序、参数或下载地址。开始前保存私有状态，关闭网页不会终止节点任务；重启后将进行中状态标记 interrupted，不自动重放；系统包事务不会因监控三十分钟超时而被主动终止。

Debian/Ubuntu 图形主机使用 pkexec、内置 APT 脚本与官方稳定源，启用 tailscaled；macOS 复用已有 Homebrew 下载 tailscale-app、核对包签名，再由系统管理员弹窗授权 installer 并打开应用；Windows 复用 winget，已有服务可通过 UAC 启动。容器拒绝主机安装；缺少前置条件时提供官方手动入口。已有可用系统服务不修改；安装、登录、目标能力授权是分开的操作。平台矩阵与状态文件详见[设备网络](device-networks.md#安装与系统服务引导)。

### 命令与结果

运行目录 /config/agent-workspace-manager/source，环境 CARGO_HOME=/config/.cargo、CARGO_TARGET_DIR=/config/validation-build/target、CARGO_BUILD_JOBS=2。集成和 Web 预览将 WORKSPACE_WEB_BIN、WORKSPACE_RUNTIME_BIN、CRANIBOX_NODE_BIN、CRANIBOX_BIN 指向该 target/debug 下对应程序。

| 命令 | 最终结果 | 说明 |
| --- | --- | --- |
| cargo test --locked -p cranibox-node --lib | 47 通过，2 忽略 | 新增 4 项安装方案/状态单元测试，使用无害临时 Shell 夹具；真实 EasyTier 两项本轮未跑 |
| python3 -m unittest discover -s tests -p test_device_networks.py -v | 17 通过 | 真实 Rust Web API，网络进程使用隔离替身；包含容器安装限制和已有服务保护 |
| npm test --prefix addons/control-center/ui -- tests/networks.spec.ts | 11 通过 | 新增 3 项；确认、错误恢复、过期、后台状态与小屏可访问性 |
| npm test --prefix apps/node-desktop -- --workers=2 | 43 通过 | 全套原生前端测试，模拟 IPC，包含安装与原生官方外链 |
| cargo fmt --all --check | 通过 | 最终格式复核 |
| cargo clippy --locked --workspace --all-targets -- -D warnings | 通过 | 全工作区严格检查 |
| cargo clippy --locked -p cranibox-node-desktop --features desktop --all-targets -- -D warnings | 通过 | 最终原生特性检查 |
| node scripts/build-control-center-ui.cjs | 通过 | 包含 TypeScript、Vite 和实际发布资源复制；重复生成四份资源摘要一致 |
| npm run build --prefix apps/node-desktop | 通过 | TypeScript 与客户端生产资源 |
| cargo build --locked --workspace | 通过 | Web 发布资源生成后重新构建嵌入资源的程序 |
| cargo build --locked -p cranibox-node-desktop --features desktop,custom-protocol | 通过 | 最终代码及原生静态资源构建 |
| /bin/sh -n 两份 install-tailscale 脚本 | 通过 | 仅语法检查，不代表在 macOS 或真实 Linux 主机执行安装 |

合计 118 项不同自动化测试通过，不将同一用例的重跑或此前验证相加。Web 第一次运行 8 通过、3 失败：辅助构建只执行 npm build，遗漏从 ui/dist 复制到 Rust include_str 使用的发布资源。按仓库 scripts/build-control-center-ui.cjs 生成、重新构建 Rust 后，11 项全部通过；原失败日志单独保留。实际 web.js、web.css 已与源码一同更新。

页面覆盖 375/768/1440px、明暗、667×375 横屏、200% 根字号及减弱动画设置，无横向溢出；已查看手机宽度下包含真实 macOS 内置脚本的两张主题截图。未执行人工读屏或各系统原生安装授权。

### 证据与交接状态

证据为 /config/validation-artifacts/tailscale-setup-20261018，目录 0700，报告和日志 0600：rust-final.json、desktop-final.json、web-final.json 记录命令与退出码；rust-tests.log、network-api.log、web-networks.log、native-ui.log 记录测试；web-networks-initial.log 保留初始失败；asset-digests.json 记录重复生成一致的资源摘要。手机明暗截图保留在该目录的 mobile-light.png、mobile-dark.png。

功能提交为 b6ed81ef3c0b7551e14ed43094f156a222e97442。source-digests.json 核对了本机提交源码与远端实际测试的 13 份代码、发布资源和测试文件，摘要全部一致；文档未计入该代码摘要清单。

### 当前提交的 CI 最终结果

以下三条运行均已结束，headSha 均核对为上述完整功能提交；最终工作流与步骤元数据保存在同目录的 ci-final.json。

| 工作流 | 运行编号 | 结果 |
| --- | --- | --- |
| Cranibox server and independent node client | 39101805562 | completed / success，7/7 作业成功 |
| Release | 39101805607 | completed / success |
| Sync to GitCode | 39101805611 | completed / success |

七项作业包括三平台原生检查、测试与发布构建，三平台 Tauri 安装包，以及真实 Sway/wayvnc 作业。Linux 原生作业还通过完整 Web、HTTP/CLI/MCP 回归、真实 X11 VNC 认证/画面/输入/重启、真实用户态 EasyTier 网络/重启/入口故障切换及资源基线检查。前述远端专项验证中忽略的两项 EasyTier 测试由 CI 的独立真实网络步骤执行，不将不同范围的结果合并计数。安装包生成不代表实际安装验收，Release 的 main 分支运行没有构建和发布产品镜像。后续交接提交仅更新文档，不再触发有路径过滤的三平台矩阵；该绿色结果绑定 b6ed81e 的功能代码。

本轮未运行真实系统安装器，没有提升容器权限、安装 Tailscale、登录账号、修改 tailnet 或宿主路由，也未替换日常主服务和原生客户端；新代码只在隔离测试进程运行。Windows/macOS 实机、系统管理员授权、macOS 网络扩展、真实 Tailscale ACL/互访仍未验收。此前 KDE VNC 捕获失败结论不变，继续使用 Selkies。此前 71f2397 的 CI、完整镜像与 .deb 验收绑定旧提交，不应自动外推到此次安装引导。

## Selkies 节点桌面收尾

2026-10-18 在相同远端 cranibox-validation 容器收尾 KDE 桌面互访。新增 Rust Selkies WebSocket 适配器与 Web/Tauri 共用 JPEG 查看器，不复制上游 Web UI，不安装新的桌面服务，不重启合成器；VNC 捕获的历史失败保持原结论。最新功能、未完成项及后续优先级以[交接入口](implementation-handoff-2026-10-18.md)为准。

### 实现与真实验收

目标配置可显式选择 vnc 或 selkies，旧配置默认 VNC 且序列化保持兼容。Selkies 适配器只连接已配置的回环 `/websockets`，检查时只等待 MODE，不发送显示设置。授权、配置代次、250ms 撤权检查和单控制者名额复用现有节点实现；慢消费者不能阻止撤权或占住已撤销的名额。输出记录最多 16 MiB，分为不超过 64 KiB 的传输块；输入仅允许受限桌面命令，队列保持有界。

Web/Tauri 从首包识别 RFB 或 Selkies 标记，共享原有工具栏。JPEG 查看器请求 primary/30FPS/质量65，不改显示分辨率，支持键鼠、只读、移动输入、重连及关闭释放按键；无音频、剪贴板或文件传输。接入必须是支持 JPEG、无 token 握手且位于可信容器/回环网络中的现有服务；连接会接管上游主显示器，原 Selkies 查看器会断开，外部查看器也可反向接管。没有实现 Selkies 自动安装或托管。

真实验收使用隔离 Harness 启动统一 Web/网关和出站 test-node，在目标单独授予调用方桌面权限，再用系统 Chromium 打开真实 React 页面，经 Workspace 中继连接现有 KDE/Selkies。采样画面为 1024×768、113 种颜色；打开仅属于测试的 kdialog，使用页面键盘输入框发送 cranibox-selkies-input-20261018，断言原生对话框 stdout 完整返回这 31 个字符；随后断开重连、撤销目标授权并断言查看器断开，页面异常数组为空。对话框与隔离进程均清理，主服务、身份、用户文件和合成器设置不变。

第一次误用了 8081，实际镜像监听 8082，查明后修正默认值。画面接通后的第一次连续输入失败：每个按下/释放独立发送，短时间挤满节点 32 项有界队列。修复为同一任务中的输入按不超过 64 KiB 合并传输，不放宽队列；真实 kdialog 与新增 Tauri 模拟 IPC 的完整文字测试均通过。初始失败日志保留。

### 命令与结果

目录与 Cargo 环境同上。所有运行均在用户指定的远端；本机仅编辑、Git、同步和查看合成截图。

| 命令/检查 | 结果 | 可信范围 |
| --- | --- | --- |
| cargo test --locked -p agent-workspace-runtime peers:: | 61 通过 | 含协议兼容、分片/合并/限额、受限命令、无接管探测、慢消费者撤权与名额释放 |
| cargo test --locked -p cranibox-node --lib | 47 通过，2 忽略 | 节点回归；真实 EasyTier 两项本轮未跑 |
| python3 -m unittest discover -s tests -p test_remote_desktops.py -v | 7 通过 | 真实 Rust 服务与隔离 TCP/WebSocket，包括 Selkies 128 KiB 输出分块、键盘命令及撤权 |
| 同上，test_desktop_relay.py | 7 通过 | 中继授权、身份/组、ticket 与取消、跨路径单控制者 |
| 同上，test_desktop_service.py | 8 通过 | 进程替身的托管、配置、恢复与权限回归 |
| npm test --prefix addons/control-center/ui -- desktops.spec.ts | 11 通过 | 系统 Chromium、真实 Rust 通道、合成 RFB/JPEG；连续文字、鼠标、只读、重连、取消、认证及服务引导 |
| npm test --prefix apps/node-desktop -- --workers 2 | 44 通过 | 模拟 IPC 全套；新增首包和 JPEG 分片、帧确认、连续输入合并及关闭清理 |
| CRANIBOX_TEST_REAL_SELKIES=127.0.0.1:8082 python3 -m unittest discover -s tests -p test_desktop_real_selkies.py -v | 1 通过 | 上述当前 KDE 原生画面/文字、中继、重连、撤权，无页面异常 |
| cargo fmt --all --check | 通过 | 全工作区 |
| cargo clippy --locked --workspace --all-targets -- -D warnings | 通过 | 全工作区严格检查 |
| node scripts/build-control-center-ui.cjs | 通过 | TypeScript/Vite 及 Rust 实际嵌入的 web.js/web.css 发布资源 |
| npm run build --prefix apps/node-desktop | 通过 | TypeScript 与原生生产页面 |
| cargo build --workspace | 通过 | 生成 Web 发布资源之后重新嵌入 |
| cargo build --locked -p cranibox-node-desktop --features desktop,custom-protocol | 通过 | 原生静态资源及桌面特性编译，不等于安装包实际安装 |

共 186 项不同测试通过，2 忽略，不与历史测试数量累加。Web 初始运行存在英/中文预览夹具端口冲突（改用各自 webPort+38）、旧提示文案断言及取消请求偶发 ERR_ABORTED；取消用例单独及最终全套均通过，没有修改生产取消逻辑。另一次最终启动遗漏二进制环境变量，在测试开始前失败，补齐四个 BIN 变量后 11/11 通过。全部日志保留，不将这些失败算作成功。Web 375px/1440px、明暗均无横向溢出；人工查看了合成画面的手机明色与桌面暗色截图。

### 证据与后续

证据目录 /config/validation-artifacts/selkies-desktop-20261018，目录 0700、文件 0600；包含 acceptance.json、rust-final.json、backend-results.json、peers-tests-final.log、node-tests-final.log、三份桌面后端日志、web-desktops-final.log、tauri-ui-final.log、actual-kde-final.log，以及构建/静态检查日志。真实结果日志同时记录采样分辨率/颜色数、keyboard/reconnect/revocation 为 true、pageErrors 为空。source-digests.json 核对本机待提交与远端实际测试的全部 24 份变更源码、测试和发布资源，摘要一致；文档未计入该源码清单。初始失败与中间复测日志分开保留；Playwright 输出和四份 fixture-宽度-主题 合成截图同样保留在远端，不把真实桌面截图提交 Git。

通过复现该真实测试须明确设置环境变量，并提供现有图形会话的 DISPLAY、XDG_RUNTIME_DIR 与 D-Bus 环境；Node、Playwright、Chromium 与 kdialog 必须已存在。测试会接管当前主显示器，普通 CI 不自动执行。当前内部端口 8082 来自实际监听，不能假设所有 Selkies 部署都相同。

本轮没有替换日常 71f2397 Web/客户端，没有重新安装原生包、构建完整镜像、运行 Windows/macOS 实机、连接真实 Tailnet 或执行真实模型任务。KDE/Selkies 节点通道的成功不改变原 VNC 捕获失败，也不能外推为三台物理设备、所有功能或所有平台已完成。

### 本次提交 CI 收尾

功能提交 afaa137ce78c1847cb893e97d1527e3b06b23d6f 已推送 origin/main。逐一核对完整 headSha 后，结果如下：

| 运行 | ID | 结果 | 范围 |
| --- | --- | --- | --- |
| Cranibox server and independent node client | 39106262547 | completed / success，7/7 | 三平台 Rust 检查、测试和发布构建；三平台 Tauri 安装包；独立 Sway/wayvnc；Linux 完整 Web/后端、真实 X11 VNC、EasyTier、资源报告 |
| Release | 39106262625 | completed / success | main 工作流成功，本次没有发布产品镜像 |
| Sync to GitCode | 39106262578 | completed / success | 本次提交同步 |

元数据与十项 Artifacts 的清单存入私有证据目录 ci-final.json（0600）。安装包为 cranibox-node-desktop-{ubuntu,windows,macos}-latest；其余为三平台 cranibox/cranibox-node 及 resource-baseline-linux。这里的产物生成与自动化测试成功，不替代新包实际安装、Windows/macOS 图形权限、系统登录/休眠、Tailscale 实际登录或多台物理设备验收。收尾只读检查远端 cranibox-validation 仍为 running / healthy，privileged=false、未挂 Docker socket。

## 最新 Linux CI 安装包与 Shell 重新附着验收

本节是上述 CI 结束后补做的真实安装包验收，绑定功能提交 afaa137ce78c1847cb893e97d1527e3b06b23d6f、矩阵运行 39106262547。所有产品程序、套接字、浏览器及原生交互仍只在 admin@192.168.217.6:22022 的 cranibox-validation 容器执行，本机仅下载和校验产物、同步辅助脚本、查看截图及编辑/Git。

### 产物校验与执行方式

| Artifact | ID | 归档字节数 | SHA-256 | 使用情况 |
| --- | --- | --- | --- | --- |
| cranibox-node-desktop-ubuntu-latest | 12128354180 | 95,673,837 | 2999969618d43206a225553e6bc4d218828780018181c0843d3dd4fa6df6ad2a | 解出 .deb 与 AppImage，均实际运行 |
| cranibox-ubuntu-latest | 12128832666 | 19,569,678 | f3ef7d6a49002c05a8a34ef3e9c355203d9fa59aa9aef9d5fb20096a3f97a0ec | 仅下载、校验和解包，未运行或部署 release 服务端 |

两份归档摘要均与 GitHub 元数据及远端接收结果一致。环境为 Ubuntu 26.04 amd64 / KDE Wayland；测试 profile 独立，NodeID 为 node-708d42e4fa964fc49fd3c058d9a72e44。先备份原客户端可执行文件和启动环境、记录自启与剪贴板，再停止原客户端，避免同一身份多进程。通过既有认证 Selkies 页面观察并操作真正的原生窗口，托盘菜单通过 PID 对应的 D-Bus 服务触发；此处没有使用模拟 Tauri IPC。

### .deb 与原生 PTY

使用 dpkg -i 实际安装 Cranibox Node_0.1.0_amd64.deb，核对 cranibox-node 0.1.0 amd64；/usr/bin/cranibox-node-desktop 的 SHA-256 为 2e9850fead1de8a23f7c93e3ef93ddb6fe92be7181fa0ccdadf9b76a0649ce44，与运行进程对应。

| 检查 | 结果 | 验证内容 |
| --- | --- | --- |
| 原生界面、IPC 与托盘 | 通过 | release 窗口可见，托盘服务属于实际 GUI PID；复制的 MCP 指向安装程序和隔离配置 |
| 自启动开关 | 通过 | 实际创建/删除 HOME/.config/autostart/Cranibox Node.desktop，Exec 为安装程序和 --background；没有执行注销/登录 |
| 窗口与退出 | 通过 | 关闭原生窗口后进程保留，托盘打开窗口；退出确认可取消，确认后进程退出 |
| 后台与单实例 | 通过 | --background 重启后窗口隐藏，第二次启动退出码 0 并唤回原进程，NodeID 不变 |
| 本机 Shell 重新附着 | 通过 | 在真实浮动终端创建 Shell、导出变量并记录 PID；关闭浮动终端和原生窗口，操作托盘重新连接及打开，重新附着后变量和 PID 均相同 |

PTY 两次记录均为 afaa137-pty-persist 713970，证据为 pty-first.txt、pty-reconnected.txt 与 pty-acceptance.json。结束时在终端显式执行 exit，未保留测试 Shell。本次没有配置 Workspace 网关，也没有制造网络中断；托盘「重新连接（保留 Shell）」操作仅证明没有销毁该本机 Shell，不能据此宣称真实网关重连验收通过。

### AppImage 的默认失败与兼容模式

实际执行 Cranibox Node_0.1.0_amd64.AppImage --version，默认模式退出 127，日志报告 fuse: device not found / Cannot mount AppImage；容器没有 /dev/fuse。这是本环境的默认运行失败，保留原日志，不标为通过。

设置 APPIMAGE_EXTRACT_AND_RUN=1 后，版本命令返回 cranibox-node 0.1.0、退出 0；实际 GUI、PID 对应托盘、复制 MCP、退出确认均通过，并复用了 .deb 的 NodeID。AppImage 解包后的内部程序 SHA-256 为 642eb7d6395b9f8e13196372d0cef23c92ee2aa1c8a8d220b2465de2ef9ed0a4，与 .deb 分别记录，不假定两个打包产物应当字节一致。

从原生界面实际复制的 MCP command 指向持久 AppImage 路径，没有指向临时解包目录；GUI 退出后，以该命令、补充兼容环境变量，实际执行消息 MCP 的 initialize、notifications/initialized 与 tools/list，退出 0，返回三个工具。复制配置自身没有 APPIMAGE_EXTRACT_AND_RUN 环境字段，无 FUSE 的 Agent 环境仍需手动添加，已写入[客户端说明](node-desktop-and-device-mesh.md#linux-appimage-与-fuse)。没有测试普通 FUSE 桌面上的默认 GUI、AppImage 登录自启动或全部 MCP 入口，不能由本次兼容模式结果外推。

### 清理恢复与证据

测试 .deb 已用 dpkg -r 卸载，两个安装包进程、临时浏览器与测试 Shell 均已退出；自启恢复禁用，剪贴板恢复，私有剪贴板备份删除。原客户端先前运行的 debug inode 已因构建被替换，恢复时使用测试前备份的原字节，保存于 /config/validation-client/bin/cranibox-node-desktop，避免覆盖最新构建产物。恢复后 PID 为 719446，SHA-256 为 e4199d9316bb4cd257ba7505865cdf35f78363a6ecb78e750a4eae949eb31510，原 NodeID node-82bf96c6f15d4f00923656f7877b8609 和 /config/validation-client/{config,data,state} 不变；已查看恢复后的真实窗口并核对托盘 PID。

最终记录主 Web PID 538744 未变，容器 running / healthy，镜像仍为 sha256:a2077e4276f3e6a51e1fa5b3a92b6ef72a3336e303e955fa8034b0ae1724a81b，privileged=false、无 Docker socket；3080 和 3031 均为未认证 401、认证 200。本次没有替换主服务、修改宿主网络、安装 Tailscale 或重启合成器。

远端证据在 /config/validation-artifacts/linux-installer-afaa137，目录 0700，日志/记录/截图 0600，需执行的产物 0700。acceptance.json 状态为 completed_with_documented_limits；同时保留 install.log、uninstall.log、release-tray.json、release-mcp.json、自启记录、PTY 三份记录、后台/单实例记录、AppImage 默认失败日志、兼容模式启动/MCP 日志、restored.json，以及 22 份带 SHA-256 的私有截图。真实桌面截图和含私有环境的备份不提交 Git。

这是独立人工集成验收，不增加上文 186 项不同自动化测试的计数；未重跑安装包内的真实桌面查看器，不代替源码专项的 KDE/Selkies 测试，也没有构建新完整镜像或发布/拉取产品镜像。Windows/macOS 原生交互仍未验收：macOS fliaping@192.168.217.85 拒绝当前 SSH 公钥，Windows 原生登录入口尚未提供；已请求可用访问方式，不能用 WSL2 代替原生系统。

## Linux AppImage MCP 与登录启动修复

本节继续处理上一节已确认的启动缺陷。代码新增 apps/node-desktop/src-tauri/src/launch.rs 与纯 Rust 库入口，将持久启动路径和 MCP 参数统一；Linux 自启改用遵循 XDG_CONFIG_HOME 的原子私有文件写入，开启时迁移旧 HOME/.config 条目，关闭时清理两处。Windows/macOS 自启插件保留。AppImage 的 MCP 和自启自动添加 --appimage-extract-and-run，普通安装程序不添加；后台运行工具仍默认关闭。

只在同一指定远端容器运行，证据目录 /config/validation-artifacts/linux-launcher-20261018（0700，记录/截图 0600）。复现命令在远端源码目录执行，Cargo 环境沿用前文：

~~~sh
cargo fmt --all --check
cargo test --locked -p cranibox-node-desktop --lib
cargo test --locked -p cranibox-node-desktop --lib gio_launches -- --ignored --nocapture
cargo clippy --locked -p cranibox-node-desktop --features desktop --all-targets -- -D warnings
cargo build --locked -p cranibox-node-desktop --features desktop,custom-protocol
~~~

五项常规单元测试通过；需要 GIO 的一项默认忽略，另显式执行通过，合计六项不同测试，不与历史轮次累加。格式、严格 Clippy、原生构建通过。真实 GIO 用例实际执行含空格、引号、反斜杠、美元符号、反引号和 %f 的脚本，核对最终参数。第一次用例失败，发现 GIO 在展开 %% 之前检查可执行文件存在；仅百分号路径改经 /usr/bin/env -- 传入后通过，gio-test.log 保留失败，gio-test-final.log 为最终结果。Linux 桌面 CI 已增加该显式真实启动用例及 GIO 依赖。

新构建原生程序使用独立 profile 和 NodeID node-49d60097514d4257a28173d306c759ec，指定已校验 afaa137 AppImage 的持久路径作为 APPIMAGE 启动器。通过真实 KDE 窗口操作，开启/关闭/再开启登录自启：启动项实际出现在独立 config/autostart/Cranibox Node.desktop，权限 0600，HOME/.config 未新增条目；Exec 正确引用含空格路径并包含兼容参数和 --background。再实际复制默认两个 MCP 入口、勾选后复制三个入口，均指向持久 AppImage 和独立设置文件。

退出测试 GUI 后，未设置 APPIMAGE_EXTRACT_AND_RUN，按三个所复制命令完成 initialize、notifications/initialized 和 tools/list，退出码均 0：cranibox-devices 返回五个工具、cranibox-messages 三个、cranibox-runs 两个。设备入口使用隔离测试设置与凭据，仅做协议初始化和工具枚举，没有连接真实网关或执行远程命令；测试设置随后删除。

再由 GIO 实际执行上述新启动项，当前容器无 /dev/fuse，启动环境未设置 APPIMAGE 或 APPIMAGE_EXTRACT_AND_RUN；AppImage runtime 成功解包并启动 GUI PID 736005，业务程序只收到 --background，未收到运行时参数。窗口保持隐藏、同 PID 托盘存在；通过其 D-Bus 托盘打开实际窗口，NodeID 与新源码测试保持相同。此处运行的 AppImage 仍是 afaa137 包，因此组合验证证明新配置逻辑与已有运行时兼容，不能称为新安装包实测。没有真实系统注销/登录、普通 FUSE 模式或休眠验证。

完成后清理测试启动项、退出测试 GUI 与预览浏览器、恢复剪贴板，日常客户端从原持久路径以原环境恢复，PID 737018、二进制 SHA-256 e4199d9316bb4cd257ba7505865cdf35f78363a6ecb78e750a4eae949eb31510、原 NodeID 不变。主 Web PID 538744 未替换；容器 running / healthy，privileged=false。证据包括 acceptance.json、mcp-acceptance.json、三份 MCP 原始输出、native-autostart.json、gio-appimage-acceptance.json、restored.json、source-digests.json 和私有截图；没有将私有环境、剪贴板或截图提交 Git。

### 启动修复提交的最终 CI

功能提交 6396b93ed53a2f6a820a67fb3af0f0f0d8188859 已推送 origin/main，逐一核对完整 headSha，最终结果如下：

| 运行 | ID | 结果 | 范围 |
| --- | --- | --- | --- |
| Cranibox server and independent node client | 39110808540 | completed / success，7/7 | 三平台 Rust 检查、测试与发布构建；三平台 Tauri 安装包；真实 Sway/wayvnc；Linux 完整 Web/后端、真实 GIO/X11 VNC、EasyTier 重启/入口切换及资源报告 |
| Release | 39110808411 | completed / success | main 工作流成功，本次没有发布产品镜像 |
| Sync to GitCode | 39110808393 | completed / success | 本次功能提交同步 |

本节证据目录 ci-final.json 保存三个运行的最终元数据、七项作业及步骤、十项 Artifacts 的 ID/大小/SHA-256。客户端产物为 cranibox-node-desktop-{ubuntu,windows,macos}-latest，分别包含 .deb/AppImage、NSIS 与 .dmg；另有三平台 cranibox、cranibox-node 及 resource-baseline-linux。当前版本号仍是 0.1.0，后续验收须按运行号与摘要选择新包。

本节当时没有安装或运行 6396b93 的 CI 包；上述新源码 IPC 与旧 AppImage 的组合验收不因此变成新包实测。同提交 Linux 新包随后补验，见下一节。Windows/macOS 原生图形、真实登录/休眠、Tailnet、跨物理设备与产品镜像发布/拉取等边界维持前述结论。只更新交接文档不重复触发路径过滤的完整矩阵；再次改动代码时需按新提交重新核对。

## 6396b93 Linux CI 安装包补验

本节补齐新源码与旧 AppImage 组合验收之后的新包验证，绑定功能提交 6396b93ed53a2f6a820a67fb3af0f0f0d8188859、矩阵运行 39110808540。仍只在指定的 admin@192.168.217.6:22022 / cranibox-validation 运行产品、浏览器和网络测试，本机仅下载校验、编辑与 Git。没有改产品代码，也未重跑全部自动化矩阵。

### 产物与隔离

Linux 桌面 Artifact ID 为 12130851033，归档 95,609,544 字节，SHA-256 为 163aad45bf8c26a37c262e4066d1a09cd81ca8f88ff78a8376c099a66992a63b，与 GitHub 元数据及远端接收结果一致。

| 文件 | 字节数 | SHA-256 | 执行 |
| --- | --- | --- | --- |
| Cranibox Node_0.1.0_amd64.deb | 9,019,106 | 005fb28348388024fc0de04b190cccbd9741f1457bbf8329ea67c3b25e275a8e | dpkg 实际安装，版本命令与 GUI 均运行 |
| Cranibox Node_0.1.0_amd64.AppImage | 87,230,968 | 106d8d78822fd8d67a8de8fb6d3930037b1fc0ca8616e1958da32ef6993fb788 | 使用运行时兼容参数实际启动，未设置兼容环境变量 |

先确认没有已安装的同名系统包、日常客户端没有业务 Shell，保存原启动环境、原二进制摘要和剪贴板，再暂停日常客户端。测试 XDG config/data/state/cache 均独立，两个包共用 NodeID node-744e8b5c298a4fa29c5c375b3efabe11，不与日常身份共享配置。实际安装后的 .deb 程序 SHA-256 为 033cb487ff6c1d0ae714d194cd75a521cdc60554bced9d5ce2185c82d828c9ff；AppImage 内部程序为 1aafc8e363346bbccf597fc83bf166b0e11efbb83a2f1b16173a05549198751b。

通过已认证的 Selkies 页面观察和操作真实 KDE/Tauri 窗口；托盘菜单对应实际 GUI PID 的 D-Bus 服务。没有模拟 IPC，也不将测试辅助脚本 PID 当作 GUI。首次 AppImage 辅助检查误以为运行时父进程就是 GUI，缺少 APPIMAGE 时断言失败；随后按隔离 XDG、APPIMAGE 和实际内部可执行文件唯一定位子进程，未重复启动第二个节点宿主。此为检查脚本修正，非产品修复。

### 结果

| 检查 | 结果 | 证据与边界 |
| --- | --- | --- |
| 两包原生界面与托盘 | 通过 | 窗口真实可见，GUI PID 与托盘一致；退出确认后相应进程结束，两包 NodeID 相同 |
| 两包 MCP 配置复制 | 通过 | 默认两个入口，明确勾选后三个；command 指向持久安装路径，不指向临时解包目录；仅 AppImage 自动带兼容参数 |
| 两包 XDG 自启开关 | 通过 | 实际生成/删除独立 config/autostart/Cranibox Node.desktop，权限 0600；HOME 旧目录无新增条目；AppImage Exec 带兼容参数和 --background |
| 六个实际 MCP 命令 | 通过 | GUI 退出后执行所复制的 devices/messages/runs；两包均 initialize 与 tools/list 成功、退出 0，工具分别 5/3/2 个，无手动兼容环境变量 |
| .deb 单实例 | 通过 | 第二次启动退出 0，原 GUI PID 和 NodeID 保持 |
| .deb Shell 保留 | 通过，范围受限 | 关闭浮动终端、收起原生窗口、托盘重连和打开、重新附着后，Shell PID 761420 及 /proc 启动时间 10776469 不变；取消退出保持 Shell，确认退出后进程消失 |
| AppImage 生成启动项 | 通过 | 无 /dev/fuse、无 APPIMAGE/兼容变量的初始环境，GIO 实际启动生成条目；运行时参数被消费，业务程序仅收到 --background |
| AppImage 后台、唤回、单实例 | 通过 | GIO 启动后窗口隐藏、GUI PID 766603 的托盘存在；第二次启动退出 0，托盘打开原窗口，身份保持 |

设备 MCP 仅用隔离的 ws://127.0.0.1:9 与测试凭据完成协议枚举，没有连接真实网关、执行远程命令或实际后台 Agent 工作；测试设置与凭据随后删除。本轮 .deb 终端的命令标记输入受到远程 Selkies 键盘/剪贴板转发干扰，未形成有效变量记录，保留对应截图；因此改以同一 Shell PID 与进程启动时间验证上述操作没有重建 Shell，不宣称本轮导出变量保留或真实网络故障恢复。此前 afaa137 的变量专项仍独立有效。

GIO 启动的 AppImage 日志包含 EGL/Mesa 与缺少 GStreamer appsink 的环境警告，实际原生页面与托盘仍可操作；本轮没有复测该包的媒体或远程桌面查看器，不将这些功能记为通过。普通 FUSE 挂载、实际系统注销/登录、休眠、网关故障、Windows/macOS、真实 Tailnet、多台物理设备和公开产品镜像发布/拉取仍待验收。

### 恢复与证据

两个包的 GUI、AppImage 运行时、测试 Shell 和任务专属浏览器已退出；.deb 已卸载，9228 调试浏览器进程不再存在，自启恢复禁用，剪贴板恢复且备份删除。原客户端从 /config/validation-client/bin/cranibox-node-desktop 恢复，PID 767446，SHA-256 e4199d9316bb4cd257ba7505865cdf35f78363a6ecb78e750a4eae949eb31510、原 NodeID node-82bf96c6f15d4f00923656f7877b8609 与配置保持，已实际查看恢复窗口并核对托盘。

主 Web PID 538744 未变；容器 running / healthy，原镜像 sha256:a2077e4276f3e6a51e1fa5b3a92b6ef72a3336e303e955fa8034b0ae1724a81b 未替换，privileged=false、无 Docker socket。远端实际绑定地址 192.168.217.6 的 3080/3031 均为未认证 401、认证 200；首次健康检查误用宿主回环地址导致连接拒绝，改用已有绑定地址后通过，未修改端口或服务配置。

私有证据目录 /config/validation-artifacts/linux-installer-6396b93（0700），包含 acceptance.json（completed_with_documented_limits）、artifact.json、install.log/uninstall.log、两包复制配置/自启/托盘/单实例记录、mcp-acceptance.json 与六组原始输出、deb-pty-persist.json、gio-appimage-acceptance.json、restored.json、final-health.json，以及 16 份带摘要的截图。记录/截图 0600，AppImage 0700；私有桌面、环境和剪贴板不提交 Git。此项独立人工验收不增加历史自动化测试数量，最新代码的完整 CI 仍绑定 6396b93。

## 3c1a38d Linux 包及真实 Computer Use 补验

本节绑定 3c1a38d19c66d3b9050f3003705f47a5c7187ace、矩阵 39117202769，功能提交为 973f1c8。该矩阵最终 7/7 成功，首次 Wayland 失败与单项重跑的边界见[交接记录](implementation-handoff-2026-10-18.md#历史验证索引)，没有因此宣称认证超时根因已修复。本节另补真实包和桌面验收，没有改产品代码、部署日常服务或重跑全部自动化矩阵；411 项不同自动化测试及四项忽略的数量维持原口径。

### 包与运行隔离

Artifact 12133315868（cranibox-node-desktop-ubuntu-latest）归档 95,907,899 字节，SHA-256 为 18753c2ce1ebb94c1ed9824966e2593291e5f99fddc0633f0a11701fbb293c98，与 GitHub 元数据和远端接收文件一致。

| 文件 | 字节数 | SHA-256 | 实际执行 |
| --- | --- | --- | --- |
| Cranibox Node_0.1.0_amd64.deb | 9,180,816 | 650d987b8d601728782ae52b607505631860a7994cbb606491a34c24c7065aba | dpkg 安装，release CLI、节点、MCP 与真实 GUI |
| Cranibox Node_0.1.0_amd64.AppImage | 87,366,136 | a1286262ffb37b1865af846eef3f76b66eea6bc941c49d4d6169e301234d9646 | --appimage-extract-and-run，无手动兼容环境变量，实际 GUI 与 MCP |

.deb 的包管理名称是 cranibox-node，二进制为 /usr/bin/cranibox-node-desktop，安装后 SHA-256 为 38ed6b42d4b6fa8e5e3ad6c61a4e7c8e5adebbada64e380fb5cd870e3c0b9e63；AppImage 实际内部程序为 ab5c62ea6c0a144ce660f3d707eecd48da3675fdf94726a6db1078ff8b21259c。包版本仍为 0.1.0，应以运行号和摘要辨别代码。

全部产品执行在 admin@192.168.217.6:22022 的 cranibox-validation，真实 KDE Wayland 图形会话及 Selkies PixelFlux 原生后端 8764。独立 Python bridge 使用私有状态和随机回环端口，不替换日常 bridge；两个节点身份和 XDG 目录独立于日常配置，目标 NodeID 为 node-244f1995638040ebad68513c24708915。没有重启合成器、Selkies 或日常客户端。

Tauri 单实例插件按桌面 D-Bus 会话识别应用，即使 XDG 目录不同，普通第二次 GUI 启动也会唤回日常旧包。因此本轮使用 dbus-run-session 启动测试 GUI，并按隔离 XDG、实际 exe 和 PID 识别新包；保留原 DISPLAY、WAYLAND_DISPLAY 和 XDG_RUNTIME_DIR。私有会话没有正常 KDE 托盘集成，这一方法只证明可见窗口和真实 IPC，不是托盘、portal、登录自启或系统集成验收。

### 真实桌面与 MCP

使用实际安装的 .deb 启动调用方与目标身份，经独立 computer 授权、真实 Python bridge 访问 PixelFlux，不使用原生动作夹具：

| 检查 | 结果 | 证据与边界 |
| --- | --- | --- |
| 真实截图 | 通过 | 1440×1050 PNG，实际保存并查看，含输入前后与 MCP 图片 |
| 查找与聚焦窗口 | 通过 | 找到自建 kdialog 并聚焦；对话框使用 xcb，不能据此声称任意原生 Wayland 窗口管理可用 |
| 控制权与真实输入 | 通过 | 无控制权输入拒绝；取得短期控制权后输入并发送 Return，kdialog 退出 0，实际 stdout 完整等于 cranibox-real-computer-use-3c1a38d-20261018 |
| 目标绑定 MCP | 通过 | tools/list 仅暴露 device_computer，tools/call 返回真实 image/png 内容块 |
| 紧急停止与重启 | 通过 | 旧控制权输入、新 acquire 均拒绝；目标节点重启仍暂停，本机 owner 明确恢复后可取得新控制权 |
| 撤权 | 通过 | 接口拒绝请求，MCP tools/list 返回空列表 |
| AppImage GUI 宿主与 MCP | 通过 | GUI 实际持有同一隔离节点；AppImage 自身以兼容参数启动 computer-mcp，从该节点取得真实 PNG；测试授权随后撤销 |

这证明真实桌面后端和实际包能通过节点授权接口工作；两个身份在同一容器，未调用真实模型，不能视为多物理设备、真实 Tailnet 或模型 Agent 完成工作流。首轮真实输入已通过，随后验收辅助程序将 MCP 逐行 JSON 误当成单个 JSON，发生 JSONDecodeError；只修正辅助程序解析，完整重测通过。real-computer-attempt-1.json 保留原始失败，real-computer-acceptance.json 为最终结果。

### 两包原生界面

.deb 与 AppImage 均实际显示新的“Agent 桌面操作”区域，点击“检查桥接状态”得到“桌面桥接就绪”。分别点击紧急停止后，界面显示暂停及仅限本机恢复，peers.json 的 computer_paused 实际为 true；点恢复出现原生确认框，取消保持暂停，重新确认后字段变为 false。没有模拟 Tauri IPC；输入这些本机所有者操作使用原生 PixelFlux，独立于被暂停的 Agent bridge。

两包重用相同隔离 NodeID，分别核对真实业务 PID 与可执行文件，而非仅检查 AppImage 运行时父进程。这轮不重新覆盖 6396b93 的托盘、自动启动项、单实例、完整终端和原有全部业务。普通 FUSE、系统登录/休眠、真实网关网络故障、Windows/macOS、跨设备模型工作流及公开产品镜像发布/拉取仍待验收。

### 清理与最终状态

测试 GUI、AppImage 运行时、bridge 与节点全部退出，测试端口 32991/57303 关闭。私有 D-Bus 所生成的四个 portal/keyring 辅助进程退出后仍存活，按隔离 XDG 和进程名核对并逐一终止；未操作日常会话的同名进程。第一次卸载误用了二进制名称 cranibox-node-desktop，dpkg 返回未安装警告；按包归属改用 cranibox-node 后实际卸载，原警告保存在 uninstall-attempt-1.log，最终确认 /usr/bin/cranibox-node-desktop 不存在。

测试授权撤销、失效 Token 文件删除、无测试自启项，真实输入检查后剪贴板恢复、备份删除。此轮没有创建浏览器会话。日常主 Web PID 538744、客户端 PID 767446 始终保留，客户端 SHA-256 仍为 e4199d9316bb4cd257ba7505865cdf35f78363a6ecb78e750a4eae949eb31510，NodeID 仍为 node-82bf96c6f15d4f00923656f7877b8609；没有部署新包到日常服务。

最终容器 running / healthy、privileged=false、无 Docker socket；LAN 3080/3031 均匿名 401、认证 200，没有本轮配置的残留进程。私有证据目录 /config/validation-artifacts/linux-installer-3c1a38d（0700）包含 acceptance.json、artifact.json、packages.json、real-computer-acceptance.json、两包 *-native-start.json、*-computer-ui.json、appimage-real-mcp.json、auxiliary-cleanup.json、final-health.json、安装/卸载日志和截图。文件保持 0600，AppImage 0700；桌面截图、凭据、剪贴板和环境不提交 Git。

## a8a97f5 Linux 包、原生桌面与剪贴板补验

绑定功能提交 a8a97f5f5af91619c379e64fd2f2f9aeda45bc34、最终矩阵 39128492641（7/7 成功）。此次按用户要求收尾验证并整理模型交接，没有修改产品代码或更新日常部署。所有包执行、套接字、原生 GUI 和浏览器仍只在 admin@192.168.217.6:22022 的 cranibox-validation 进行。自动化 392 项及四项忽略的原口径不变；下述 Browser Use 是同一现有用例重跑，人工包验收单列，不累计成新的全量测试数。

### 包摘要与隔离

Artifact 12136694653（cranibox-node-desktop-ubuntu-latest），归档 96,118,077 字节，SHA-256 为 2d732eeec383ccc8438a225ddd9317c768688ba3d1ad9dbe421fb474fa9d1c1f。下载与远端接收后均核对摘要，实际执行的是该运行的产物。

| 文件 | 字节数 | SHA-256 | 执行 |
| --- | --- | --- | --- |
| Cranibox Node_0.1.0_amd64.deb | 9,293,998 | 1ac53f7569540ce31bf3ee7e985b5e32281e19fa9ed988116bffac4cccac4992 | dpkg 安装 cranibox-node，release CLI/节点、MCP、真实 Tauri GUI |
| Cranibox Node_0.1.0_amd64.AppImage | 87,464,440 | f6c77e7e07f6ab03edf4e8ffa8fa11e9cfd18a54045f1466fe0c2c3a316a0395 | --appimage-extract-and-run，真实 GUI 节点与 MCP |

安装后二进制 /usr/bin/cranibox-node-desktop 的 SHA-256 为 c43b114dbe60a59a73ae3438663b2c10a446040612d1e5c3cc273ba226018a03；AppImage 实际内部程序为 903f669e29d4d1ddff08d41693d57d377a91b3bec8faf1422f2b272f87563db3。版本号仍为 0.1.0，不能只按文件名识别源码。

使用独立 XDG config/data/state/cache，两包共用测试 NodeID node-58c96310e3a84859b90b18253d1ec42b，调用身份另行生成。真实桌面是现有 KDE Wayland，复用 PixelFlux 8764，项目 Python bridge 使用私有状态与随机回环端口 55935；节点监听随机回环端口 59201。私有 dbus-run-session 避免单实例唤回日常旧包，保留真实图形会话环境。日常进程始终运行，未与其共用身份；这种私有 D-Bus 测法不证明正常托盘、portal、登录或自启集成。

### 实际通过的检查

| 检查 | 结果与范围 |
| --- | --- |
| .deb Computer Use | 真实 1024×768 PNG、查找并聚焦自建 xcb kdialog；无控制权输入拒绝；取得控制权后实际输入并 Return，stdout 完整符合标记；目标绑定 MCP 返回 image/png |
| 停止、重启与撤权 | 紧急停止拒绝旧输入及新 acquire；目标重启仍暂停；本机 owner 明确恢复后可取得新控制权；撤权后 status 拒绝、MCP 工具为空 |
| .deb Browser Use | 使用 /usr/bin/cranibox-node-desktop 运行 test_browser_real_chromium，1/1、12.733 秒；独立 Chromium profile 与随机 CDP、官方 MCP，打开页面、快照、填表提交、PNG、停止后无重放及正常退出；不访问日常 9222 |
| .deb 原生 Selkies | 真实管理界面配置本机 Selkies 并连接，看到实际 KDE 画面；断开重连重新出现画面，同时清空草稿和收到的文字 |
| .deb 原生剪贴板发送 | 通过实际 GTK 粘贴填充中文/emoji/多行草稿，再将目标剪贴板另置标记，点击发送后 wl-paste 取得 55 字节，与预期逐字节一致；发送不会自动粘贴到应用 |
| AppImage GUI 与 MCP | 真正启动 AppImage 内部 GUI，保持相同隔离 NodeID；Selkies 查看器真实 IPC 与画面；AppImage 自身启动 Computer Use MCP，连接该 GUI 节点宿主并返回真实 1024×768 PNG；测试授权随后撤销 |

首次用于填草稿的 PixelFlux type 操作没有完整输入 Unicode，改用真实 GTK 上下文菜单粘贴后通过字节检查。这是本轮操作驱动限制，没有修改产品或据此声称 Unicode 自动键盘输入已修复。多个辅助脚本的命令转义问题亦只修正验证脚本，不算产品修复。Computer Use 的原生键盘验收使用 ASCII 标记，中文输入的通过结论仅限上述文字剪贴板流程。

### 未通过：原生 Selkies 反向剪贴板

在 .deb 原生界面发送完成后，经 wl-copy/Klipper 将目标剪贴板改为另一段 Unicode 多行文字，并用 wl-paste 确认实际字节已更新。点击“获取远端文字”后，接收栏仍停留在先前文字；断开、重连、重新展开剪贴板后，两栏已清空，再次获取仍为空，未显示错误。查看器仍保持连接；没有证据证明画面被冻结。

此项明确记为未通过，根因未定位。环境为私有 D-Bus、同一 KDE 主机上的原生查看器访问本机 Selkies，尚未排除上游剪贴板通知、原生传输及会话环境因素。此前 Web 的真实双向文字往返和模拟 Tauri IPC 测试通过，均不能替代此实际原生结果；AppImage 本轮只补验画面与 MCP，没有重新测双向剪贴板。后续先复现，再对比正常桌面会话及另一个物理设备，保留失败记录后修复。

### 清理、证据与后续边界

两包 GUI、AppImage 运行时、独立 bridge、节点和测试 Chromium/MCP 均已退出；确认隔离 XDG 后终止残留 portal/keyring/剪贴板辅助进程。已卸载 cranibox-node，确认 /usr/bin/cranibox-node-desktop 不存在；临时授权撤销、Token 文件删除、无测试自启项。原剪贴板通过 Klipper 恢复，wl-paste 逐字节核对后删除备份，未输出原内容。

最终健康检查确认容器 running / healthy、非 privileged、无 Docker socket；LAN 3080/3031 都为匿名 401、认证 200。日常 Web PID 538744，二进制 SHA-256 59075b010d1cd8b745e0f39645e5f7a8d8ab7c40600e7970dbc4fd18a90e295c；日常客户端 PID 767446，SHA-256 e4199d9316bb4cd257ba7505865cdf35f78363a6ecb78e750a4eae949eb31510、NodeID node-82bf96c6f15d4f00923656f7877b8609 保持。未升级主服务或替换日常程序。

私有证据目录 /config/validation-artifacts/linux-installer-a8a97f5（0700）。关键文件为 artifact.json、install.log/uninstall.log、real-computer-acceptance.json、deb-browser-real.log、两包 *-native-start.json、deb-native-clipboard-send.json、**deb-native-clipboard-receive.json（passed=false）**、appimage-real-mcp.json、auxiliary-cleanup.json、restored.json、acceptance.json、final-health.json，以及实际桌面截图。acceptance 状态为 completed_with_documented_failure_and_limits。文件保持 0600、AppImage 0700，截图、环境、凭据和剪贴板不提交 Git。

正常托盘/自启/完整终端仍参照 6396b93 的独立验收；本轮不覆盖普通 FUSE、实际系统登录/休眠、网关真实断网、Windows/macOS 原生安装与权限、真实 Tailnet、多物理设备、模型 Agent 工作流及公开产品镜像发布/拉取。接手入口与按优先级排列的验收条件见[交接文档](implementation-handoff-2026-10-18.md#下一阶段优先级与验收条件)。

## 原生断线原因保留与反向剪贴板排查

2026-10-18 在同一指定远端容器继续诊断，使用隔离 XDG、私有 D-Bus 与测试身份，未更新日常服务。保留上节所有失败证据，不把后续成功片段改写成完整验收通过。

1. 直接连接真实 Selkies WebSocket，cr 在发送前后分别取得新的 60/59 字节 Unicode 标记；cw 写入的 35 字节文字由 wl-paste 逐字节核对。原剪贴板恢复并删除备份。该诊断没有经过 Tauri IPC。
2. a8a97f5 AppImage 经仅记录消息类型、方向、大小与自建标记匹配结果的透明转发器访问本机 Selkies。原生接收栏实际显示一次 54 字节 Unicode 多行标记；后续重连后接收栏为空，转发器观测到 cr 及匹配的 64 字节上游文字，随后原生查看器断线。没有证明每条上游消息均进入查看器，也未定位断线原因。
3. 发现独立错误反馈缺陷：NativeDesktopChannel 在通知 onerror 前未保存原因，且 finish() 会清除解码器已有错误。现改为先保存、关闭时保留；新增两个模拟 IPC 用例验证节点/解码错误显示原原因，并且附件只释放一次。
4. 指定远端运行错误提示专项 2/2、客户端完整界面 48/48（约 1.1 分钟），TypeScript/Vite 与 desktop,custom-protocol 原生 debug 构建通过。专项包含在 48 项中，不累计为 50，也未重跑此前完整 Rust/Web 测试。
5. 实际运行修复后的 debug GUI：可连接且看到 KDE 画面，稍后仍断开并显示通用原因；没有取得稳定的原生反向剪贴板往返。该程序不是新的 CI 安装包，不覆盖 a8a97f5 .deb 的原失败。

反向获取仍未解决。后续先给原生 ACK 等待、输入/输出结束及上游关闭增加有界、不含文字内容的诊断，核对两端时序；再对照正常图形登录 D-Bus 与另一台物理设备。未证明 WebKit、IPC、上游通知或测试会话是哪一项根因，不应据猜测调整超时或放宽背压。

证据目录 /config/validation-artifacts/native-clipboard-debug-20261018：native-errors-focused.log、native-ui-full.log、native-build.log、upstream.json、protocol.jsonl、native-cleanup.json，以及 patched 下的独立 GUI 元数据、截图、清理记录。诊断转发器与 GUI、私有会话辅助进程均停止，两次原剪贴板逐字节恢复且备份删除。截图、真实文字与配置不提交 Git；源代码和文档独立提交。日常服务、身份和认证入口由 final-health.json 复核，不以测试进程存在代替健康检查。

## Rust 桌面传输收尾与最终交接

2026-10-18 在指定远端容器继续完成同一故障的有限排查和交接，未扩大功能范围。修改 Rust 原生转发循环：ACK 10 秒超时、无效 ACK、输入失败、提供者退出现在返回可见错误；显式关闭保持正常。运行时输出队列仍有界为 32 个位置，预留一个位置保存最终错误，另保存失败原因供后续输入返回，避免满队列吞掉错误且不等待消费者释放控制权；Selkies 输出阻塞 5 秒增加具体提示。没有放宽时限、取消背压或改写上游。

新增四项原生 desktop Rust 测试：实际回环 TCP、Tauri Channel 回调、未 ACK 超时、正确 ACK 后继续、无效 ACK、显式退出与提供者退出。另增运行时满队列错误保留/控制名额释放测试。三平台 Tauri CI 显式运行原生四项测试；常规工作区测试不会隐式运行带 desktop 功能的二进制测试。

真实 debug GUI 检查保留成功与失败，不作为安装包验收：

| 对照 | 观察 | 结论边界 |
| --- | --- | --- |
| transport、transport2、transport3 | 真实 KDE 画面；部分轮次收到新的 Unicode 多行文字，后续获取未更新，约 60–61 秒断线 | 所有轮次经过 Python WebSocket 诊断代理；代理自身可能影响时序，不能直接归因于产品 |
| direct | 原生直连本机 Selkies 8082，未经过代理；61 字节标记真实显示，另置的 68 字节新标记经 wl-paste 确认但未更新到界面；约三分钟后仍显示连接，面板收起可响应 | 原生反向剪贴板仍未通过；此轮无上游消息追踪，没有证明断在 Selkies、IPC 或查看器；未重现代理的固定断线 |

四轮采用实际新 debug 程序、独立 XDG 和私有 D-Bus，不模拟 IPC，也不证明正常图形登录、托盘或其他设备。每轮 GUI、诊断代理、私有会话辅助进程均停止，原剪贴板逐字节恢复且备份删除。direct 中的代理进程未被使用，已一同停止。既有 a8a97f5 .deb 失败及此前 Web 双向通过记录不改写。

84fdf88 的原生/安装包矩阵 39138324015 最终 completed / success、7/7 成功；Release 39138324009 和 GitCode 39138324019 亦成功，完整 headSha 已核对。这些结果只属于上一版，不覆盖本节 Rust 修改。当前修改的远端自动化结果、提交及 CI 状态见[交接入口](implementation-handoff-2026-10-18.md#历史验证索引)。

最终回归在指定远端执行：全工作区 Rust 178 通过/4 按条件忽略，原生 desktop 二进制四项全部通过，桌面后端 remote_desktops 7 + desktop_relay 7 + desktop_service 8 共 22 项全部通过（52.674 秒）。合计 204 项不同自动化测试，不重复累计早期专项与历史测试；原生四项不在工作区 178 项内。fmt、工作区与原生 desktop 所有 target 严格 Clippy、工作区构建、实际运行的 desktop,custom-protocol debug 构建通过。本轮没有改前端或重跑完整 Web/Tauri UI，旧界面结果保持原提交绑定。

收尾代码 fa1d4019aaf9f79e690c937fcf75b34b5c63b6c2 已推送 origin/main。矩阵 39141291171 第二次尝试最终 completed / success、7/7 作业成功，GitHub 更新时间为 2026-10-18 16:25:03 UTC，完整 headSha 已核对。三平台原生构建、三平台 Tauri 打包和真实 Sway/wayvnc 全部成功；三个 Tauri 作业均实际通过新增的原生传输与 ACK 测试步骤。Wayland 重跑作业 117726271297 执行真实桌面测试 1 项通过、4.411 秒。

第一轮 Wayland 作业 117716093179 无法解析 index.crates.io，依赖下载失败，真实测试未执行；首次请求重跑时整轮尚未结束，返回 job cannot be rerun。整轮结束后重跑成功，不将依赖网络故障误记为产品断言失败，也不删除原日志。Release 39141291175、GitCode 39141291229 已成功，均核对完整 headSha。原结果和失败日志为 final-ci.json、ci-wayland-first-failure.log；最终重跑元数据和真实测试日志为 ci-final-retry.json、ci-wayland-retry.log。CI 全绿不覆盖原生剪贴板的人工失败记录。

final-health.json 确认容器 running/healthy、非 privileged、无 Docker socket；3080/3031 均匿名 401、认证 200。日常 Web PID 538744、客户端 PID 767446、原二进制摘要和 NodeID 不变；测试安装包仍已卸载。transport-cleanup-final.json 复核所有本轮隔离 GUI 目录无存活进程及剪贴板备份。未升级日常服务、未安装系统 Tailscale，也未重启合成器。

私有证据根目录仍为 /config/validation-artifacts/native-clipboard-debug-20261018；新增 final-rust-regression.log、final-desktop-backend.log、source-snapshot-final.json、transport* 与 direct 的截图、GUI 元数据、协议生命周期、清理和 direct/result.json。原始截图、剪贴板、配置、凭据及环境不进入 Git。接手优先修复原生反向文字更新，并在实际安装包、正常图形会话及另一物理设备验证；不要用新增自动化测试替代真实验收。

## 2026-10-19 补记：原生查看器有限诊断

使用 fa1d401 的同一 debug GUI、隔离 XDG 与私有 D-Bus，原生直接连接真实 Selkies 8082。通过实际 WebKit Inspector 添加仅在本次运行中生效的计数，没有修改产品源码、二进制或上游。最终计数为 32,191 次 createImageBitmap 开始、32,191 次完成，并观测到至少一条剪贴板文字消息；此轮没有观察到永久挂起的 JPEG 解码。

没有完成连续更换目标文字后的有效 UI 对照，不能将这些计数写为剪贴板稳定、排除间歇解码问题或定位根因。尝试统计 ACK/请求的临时包装未截获真实调用链，零计数不作结论；部分较长诊断命令被 GUI 输入工具改变，生成的语法错误仅属于操作失败，不是产品异常。这轮不增加自动化通过数，也不覆盖既有 .deb 和 direct 的反向更新失败。

证据保存在 inspector/result.json、first-stats.png、diag-final.png、native-cleanup.json 与 cleanup-final.json。GUI、未使用的转发器及私有会话辅助进程均已停止；再次扫描隔离 XDG 无存活进程，原剪贴板逐字节恢复并删除备份。final-health.json 重新核对日常进程、二进制、NodeID 和 LAN 认证入口保持。所有截图和诊断文件仍仅存私有远端，不提交 Git。

## 2026-10-19 补记：真实原生剪贴板的显示环境对照

继续在指定远端容器运行同一 fa1d401 debug 程序，通过真实 GTK AT-SPI 按按钮名称操作、读取原生接收栏；发送通过聚焦实际文本框、粘贴和点击按钮完成。使用实际 Tauri IPC、独立 XDG 和私有 D-Bus，直接连接真实 Selkies 8082；没有模拟 IPC、协议代理或产品/上游修改。

| 对照 | 实际观察 | 边界 |
| --- | --- | --- |
| 原 KDE 桌面中显示查看器 | 48/50 字节新标记已由 wl-paste 确认，接收栏仍为空；上游 TCP 可连接，但 HTTP 读取及新的 WebSocket 握手超时，结束测试 GUI 后连接恢复 | 原失败仍存在；没有定位上游停滞与查看器的因果关系 |
| 独立 Xvfb/X11 显示原生 GUI，目标仍为 KDE/Wayland Selkies | 六次新标记逐字节匹配：48、50、52、63、62、66 字节；包括超过一分钟后的更新、发送后更新和断开重连 | 真实 debug GUI 的有限集成通过；不是 CI 安装包、正常登录或多物理设备验收 |
| 发送与显式复制 | 原生发送的 51 字节中文/emoji/多行文字由目标 wl-paste 核对；点击复制后，本机独立显示的剪贴板与 62 字节远端文字一致 | 未自动覆盖日常剪贴板；测试过程中明确接管并在结束时恢复 |
| 断开与重连 | 断开后剪贴板面板移除；重连展开时旧文字为空，再获取 66 字节新标记成功 | 不覆盖休眠、长期断网和实际安装包重启 |

独立显示同时改变了本机显示后端和共享画面的持续变化程度，不能仅据此归因为递归画面、Wayland、WebKit、IPC 或 Selkies。原 .deb 与 KDE 同屏失败不改写，下一步应在正常桌面和另一物理设备对照画面负载、上游响应及 IPC/ACK 时序。辅助工具不支持的 AT-SPI EditableText 调用、接口签名错误和首次误切“仅查看”是测试操作失败，不作为产品断言失败；后续通过实际聚焦和粘贴完成发送。

新私有证据根目录 /config/validation-artifacts/native-clipboard-debug-20261019：result.json 汇总两种显示、逐字节核对及结论边界；accessibility 与 offscreen 保存各轮元数据、截图、接收结果、发送/复制和清理；final-health.json 保存日常健康。两轮原剪贴板均逐字节恢复且备份删除，原生 GUI、Xvfb 和私有会话辅助进程均停止，隔离 XDG 无存活进程。结束后重新连接上游取得 MODE websockets；容器 running/healthy、3080/3031 匿名 401/认证 200、日常 Web/客户端 PID、摘要和 NodeID 保持。该轮没有增加或重跑自动化测试，收尾总数仍为 204 通过、4 忽略；截图、文字、配置和环境不提交 Git。

## 2026-10-19 收尾：剪贴板等待反馈与交接

仍只在指定远端 cranibox-validation 构建、运行测试及原生 GUI；本机用于静态编辑和 Git。本次改动 Web/Tauri 共用剪贴板面板：请求时显示忙碌并禁止重复点击，10 秒未响应时显示重试/重连提示，保留旧文字和连接；使用事件序号处理相同文字，收到空文字或无效内容会结束等待，迟到响应清除超时，关闭面板清理计时器。原生模拟 IPC 的现有 Selkies 用例新增上述边界及旧错误不遮住新超时的验证，没有新增依赖或改动 Rust 传输。

最终源码基于 ad9db15637e19ddc964943a34d15dd2d61eaaf88，关键文件摘要如下；最终提交与 CI 另见[交接入口](implementation-handoff-2026-10-18.md)。

| 文件 | SHA-256 |
| --- | --- |
| DesktopClipboard.tsx | ba8ffdf5efa1deeb23e194b8e9d6f335f5013ee3ed38891a2cd07fd1beff382a |
| VncViewer.tsx | 4b4f2f2fe18d3cbaa7d69027424c1ad7bd856dec81f51e10018c1ccc8df8b638 |
| apps/node-desktop/tests/client.spec.ts | 0808af12c2bf3c2af40dda3a252fedf8c052feab3612e30128fb6a2561e46a18 |
| addons/control-center/web.js | 3cc56ca5175c30c4b742384297441ed3fb2537ec69e5c4bdcb720810a50246c9 |

最终远端客户端全部界面 48/48 通过（91.146 秒），Web 桌面 12/12 通过（99.289 秒），合计 60 项不同测试。客户端使用模拟 IPC；Web 使用重新生成并嵌入 cranibox 的静态资源、真实 Rust 服务与受控 RFB/JPEG 提供者。两端 TypeScript/Vite、Web 静态生成、cranibox 与 desktop,custom-protocol debug 构建均通过。首次 Web 配置未找到构建产物，以及旧嵌入资源的通过结果均不作最终验证；重跑、专项和历史测试不重复累计。该远端专项未重跑 fa1d401 的 204 项后端测试或完整 Web 126 项；随后的 CI 另执行完整矩阵，结果见下段。

实际新 debug GUI 使用原 KDE 桌面、隔离 XDG、私有 D-Bus、AT-SPI 操作和真实 Tauri IPC，直接连接 Selkies 8082。第一次响应使等待结束；另置 47 字节新标记后，接收栏仍为旧 48 字节，10 秒超时提示可见，获取按钮恢复，桌面仍显示连接。**等待失败反馈实际通过，原生反向文字更新仍未通过**。首次读取 AT-SPI 失效节点以及仅检查 accessible name 漏掉段落文字属于验证工具问题，后续按实际 Text 接口核对；未据其修改产品或宣称根因。

另一轮为上游诊断复制 Selkies 到独立目录，只在该副本增加栈采样并绑定随机回环端口。剪贴板可更新，JPEG 也到达，但实际查看器画面为黑屏，无法确认捕获的是原 KDE 桌面，故对照无结论。副本及测试 GUI 已停止，原上游未改写；不要将此记录写成原故障已排除。证据在 native-clipboard-debug-20261019 的 isolated-server 与 selfview-isolated。

本次证据根目录 /config/validation-artifacts/clipboard-feedback-20261019：native-ui、web-desktops、native-build、web-assets、web-build 的 json/log 记录最终结果；first-pass、pre-asset-refresh 是先前代码或旧资源基线；native-kde 的 gui.json、feedback-last.json、received-last.json、cleanup.json 记录实际等待反馈、未更新及逐字节恢复原剪贴板。截图、环境、真实文字与凭据不提交 Git。需求表、未完成项、优先级、复现入口及跨平台验收条件集中在交接文档，下一位模型先处理 P0。

全部测试及 GUI 停止后重新检查日常健康：容器 running/healthy、privileged=false、无 Docker socket；LAN 3080/3031 均匿名 401/认证 200；Web PID 538744、客户端 PID 767446、原二进制摘要和 NodeID 保持，测试包仍未安装，未发现预览或 MCP 测试进程。final-health.json 保存该结果，acceptance.json 的总体状态为 completed_with_documented_failure_and_limits。此前在测试仍运行时执行的残留扫描被预期的预览进程阻断，不是日常服务故障；最终健康检查在清理完成后通过。

代码提交 2312b640d5a754d29e2d0269b1d3c2a6a0f74753 的矩阵 39151770009 最终 completed / success、7/7 成功，GitHub 更新时间 2026-10-18 18:18:49 UTC。三平台服务端/独立节点构建、三平台 Tauri 打包和原生 ACK 回归、Linux 完整 Web 126/126 与客户端界面 48/48 均通过；真实 Sway/wayvnc 为 1 项、4.291 秒，独立真实 Chromium/MCP 为 1 项、4.181 秒，真实 X11 VNC 为 1 项、3.558 秒，真实 EasyTier 步骤亦成功。不同环境的重跑不与前述 60 项累计。Release 39151770010、GitCode 39151769875 同为 completed / success，三份记录逐一核对完整 headSha；10 份未过期 Artifacts 已核对名称，包括三平台安装包。CI 成功不覆盖原生 KDE 剪贴板失败、Windows/macOS 实机、Tailnet 或产品镜像的验收缺口。

最终 CI 元数据、作业步骤、产物清单及日志摘要在本节证据根目录的 ci-final.json；ci-native-ubuntu.log、ci-desktop-ubuntu.log、ci-wayland.log 保留对应完整日志。交接入口补充了 Web 静态资源生成与 Rust 嵌入构建命令，避免后续只运行 npm build 而验证到旧资源。此次只补充交接和 CI 证据，没有再次部署或启动产品进程。

## 2026-10-19 补验：原生连接积压

使用相同 2312b64 debug 客户端、原 KDE 桌面与日常 Selkies 8082，另建隔离 XDG 和私有 D-Bus；不经过诊断转发器，不改上游代码。真实 GUI 经 AT-SPI 连接桌面、展开剪贴板并请求文字。代码文件摘要与此前等待反馈构建一致，实际程序摘要保存在本节 acceptance.json。

| 本次观察 | 结果与边界 |
| --- | --- |
| 原生反向获取 | 首段为 48 字节；目标改为 52 字节且 wl-paste 逐字节核对后，接收栏仍为 48 字节；超时提示、重试按钮和连接状态均可见 |
| 独立上游读取 | 同次运行、未发送 SETTINGS 的另一个连接能够读到新 52 字节文字；可见窗口时两次成功记录在命令输出，upstream-read.json 仅保留收起后最后一次成功结果 |
| 可见窗口时 TCP 队列 | Selkies 到原生连接的发送队列两次分别为 4,164,782 与 4,162,469 字节；对应客户端接收队列也有积压，样本见 stats-before-hide.json 和 stats-visible-new.json |
| 上游进程采样 | 两次均未发现 wl-paste 子进程；分别处于运行/epoll 等待状态。不是完整线程追踪，不能据此排除其他上游阻塞 |
| 收起对照 | 收起后后续 TCP 队列排空，但 AT-SPI 无法再读剪贴板字段；通过同一私有会话唤回时查看器已断开。因此不能称为恢复、有效性能对照或修复 |
| 清理与健康 | 原剪贴板逐字节恢复并删除备份；测试 GUI/辅助进程退出，日常服务原 PID、摘要、NodeID 保持，容器健康、两入口匿名 401/认证 200 |

这些观察支持优先检查画面吞吐、IPC ACK 和有序流中的控制消息排队；尚无每条消息的追踪，不能确定延迟在哪一层，也不能覆盖此前上游握手超时的记录。本次没有改产品、没有重跑自动化矩阵、不增加通过数量，原生反向剪贴板继续为 P0 未通过。不要直接以延长超时、降低固定帧率或另开一个控制连接代替定位和实机复验。

私有证据为 /config/validation-artifacts/native-clipboard-layer-20261019：check.py、finalize.py、native-kde 下各次采样与 GUI/反馈/接收/清理记录，以及根目录 acceptance.json、final-health.json。健康检查在全部测试进程退出后执行；未替换日常程序、未重启 Selkies 或 KDE。Git 提交仅包含本节和交接文档，不包含剪贴板、截图或配置。

## 2026-10-19 补验：IPC 测量及撤回的传输方案

继续只在指定远端容器使用真实 KDE/Tauri GUI、隔离 XDG、私有 D-Bus，直接连接日常 Selkies 8082。以 2312b64 产品源码为基线，先加入不含正文的约五秒聚合计数，再分别试验合并已排队的小块、二进制输出及组合。所有轮次保留字节顺序、64 KiB 载荷上限、单块待 ACK 和原超时；没有等待填满块或放宽队列限制。

| 私有原始证据目录（均在 /config/validation-artifacts 下） | 采样区间数 | IPC MiB/s 范围 | 区间平均 ACK 等待 | 真实 GUI 结果 |
| --- | --- | --- | --- | --- |
| native-desktop-throughput-20261019 | 22 | 0.355–0.728 | 11.9–24.6 ms | 初始 48 字节可读；目标换为 55 字节经 wl-paste 核对后仍显示旧文字 |
| native-desktop-batching-20261019 | 29 | 0.408–0.838 | 28.0–81.2 ms | 目标 56 字节已核对，仍显示旧 48 字节，超时后保持连接 |
| native-desktop-binary-20261019 | 11 | 0.419–1.471 | 7.4–25.9 ms | 目标 54 字节已核对，仍显示旧 48 字节 |
| native-desktop-coalesced-20261019 | 1 | 1.016 | 40.4 ms | 实际界面出现 Selkies decoder cannot keep up. Reconnect using a lower server frame rate.，随后断开；未完成持续剪贴板验收 |

画面内容、负载和启动阶段不同，这些顺序运行的试验不是受控性能排名。聚合 ACK 平均值不能代表单条剪贴板消息延迟，组合触发现有解码队列保护也不能证明原故障的根因。三个方案均未通过实际 KDE 验收；后续需同时核对上游生产、IPC 消费、JPEG 解码和控制消息到达，并在正常窗口和另一物理设备复验，不能仅提高传输速度、扩大队列或延长超时。

所有产品源码与未验收的测试改动已逐字节恢复到仓库版本，未计入任何新增自动化通过项。远端重新完成客户端 TypeScript/Vite 及未插桩 desktop,custom-protocol debug 构建；恢复后的 desktops.rs 摘要为 f87322338256064ea187e1559ae1d7f34c182bb8e5bad82ce1e73ed6a0b88848，程序摘要为 8a9682473193a9c638a0db30b6ca560bb192557f7c1a44a769c9d77b1c52faf8。产品 CI 仍绑定 2312b64，不能将诊断构建写成通过的正式安装包。

汇总证据根目录 /config/validation-artifacts/native-desktop-transport-trials-20261019：acceptance.json 状态 diagnostic_trials_reverted_no_product_fix；expected-source.json 逐一核对六项关键源码，build-final.json/log 记录恢复构建，final-health.json 记录日常服务。四轮测试 GUI/辅助进程均已停止，隔离 XDG 无存活进程；每轮原剪贴板逐字节恢复并删除备份。容器健康、原日常 PID/程序摘要/NodeID 与两入口匿名 401、认证 200 保持，未替换或升级日常程序。本次仅提交文档；私有辅助工具、日志、截图、文字和配置均不提交 Git。

## 2026-10-19 补验：控制消息分段追踪

以 2312b64 为基线，在指定远端临时插桩 desktops.rs 与 peers/desktop/selkies.rs，只输出阶段名称和毫秒时间戳，不改变字节、队列、协议或时限，不记录画面和剪贴板正文。客户端 TypeScript/Vite 与 desktop,custom-protocol 构建通过后，运行实际 KDE/Tauri GUI，使用隔离 XDG、私有 D-Bus、AT-SPI 操作，直接连接日常 Selkies 8082。没有协议代理、模拟 IPC 或上游服务重启。

| 阶段标记 | 位置和含义 |
| --- | --- |
| native_cr_ipc_received | desktop_input 解码输入块后发现完整的 cr 帧；不包括此前浏览器排队和 Tauri 调用耗时 |
| selkies_cr_before_send / selkies_cr_sent | 运行时向上游发送 cr 的前后 |
| selkies_clipboard_received / selkies_clipboard_forwarded | 运行时读到 clipboard 文字消息、将其全部分块放入有界输出队列之后 |
| native_clipboard_ipc_output | 原生转发器将含 clipboard 前缀的块交给 Tauri Channel 之前；不是 JS 接收或解码完成 |

三次操作的原始事件共 18 条，使用同一进程的系统时间。没有消息编号或正文摘要；以下只按事件顺序报告区间，**不能认定下一条 clipboard 就是该行请求对应的新文字**，因为 Selkies 同时主动广播、独立读取也可能引发广播。

| 操作 | native_cr_ipc_received | cr 发出至下一条 clipboard 被 Rust 读到 | Rust 读到至 native_clipboard_ipc_output | IPC 收到 cr 至发送完成 |
| --- | --- | --- | --- | --- |
| 初始获取，目标 48 字节 | 1792350768167 ms | 30.134 s | 226 ms | 8 ms |
| 更换为 54 字节新标记后获取 | 1792350873784 ms | 20.656 s | 408 ms | 1 ms |
| 更换为 52 字节新标记后获取 | 1792350984533 ms | 24.404 s | 1,377 ms | 20 ms |

初始接收栏后来逐字节匹配 48 字节标记。独立上游连接没有发送 SETTINGS，分别成功读取当时的 48/54 字节内容。第二次新标记没有稳定显示；第三次由脚本连续操作，先确认实际忙碌、按钮不可重复点击，随后确认 10 秒超时、重试恢复、连接仍在，目标为 52 字节时接收栏仍是旧 48 字节。末次检查距第三次 Rust 接收请求约 89 秒，栏内为旧 54 字节，仍不匹配当前目标。等待反馈通过，新文字持续更新未通过；不能把迟到的旧内容当作成功。

三次进程/TCP 采样中，匹配该连接的 Selkies 发送队列约为 2,627,141、1,981,932、1,836,278 字节；客户端接收端亦有积压。这支持继续检查该连接的视频生产与消费，不能独自证明所有延迟由网络、Tauri 或 JPEG 解码中的一层造成。本轮没有记录 JavaScript 到达或 createImageBitmap 时间，也没有把消息内容逐条对应到标记；下一位维护者应补齐这些关联，不能把表中区间换算为每个请求的精确响应延迟。

另外只读核对容器实际安装的 Selkies：包元数据版本为 0.0.0，故以源码摘要标识具体副本。selkies.py 的 primary 分支通过 websockets.broadcast 向所有 primary 查看者发送视频，虽然更新背压统计，但此路径没有依据 backpressure_enabled 跳过发送；辅助显示分支存在该判断。input_handler.py 的 cr 则调用 read_clipboard，再广播结果。这是该副本的源代码事实，未通过修改上游证明因果，不应泛化为所有 Selkies 版本。

| 上游文件（/lsiopy/lib/python3.14/site-packages/selkies 下） | SHA-256 |
| --- | --- |
| selkies.py | a03a812ca68ebc6f200c32b79e8e5f238c5796629173ff776a6971ce1174ecb4 |
| input_handler.py | 00fb38a48c0249d54d1bae0e27d093b5aeab62a49ea8542e21edfc4e86a76779 |

本轮插桩只在远端构建源码中临时使用，两个 Rust 文件已逐字节恢复为原始源码；恢复后客户端 TypeScript/Vite 及未插桩 desktop,custom-protocol 构建通过。没有修改产品实现、增加测试或重跑自动化矩阵，产品 CI 仍对应 2312b64。实际 GUI 与私有会话辅助进程已停止，原剪贴板逐字节恢复并删除备份；日常 Web/客户端原 PID、程序摘要、NodeID 保持，容器 healthy、非 privileged、无 Docker socket，3080/3031 均匿名 401、认证 200。

私有证据根目录 /config/validation-artifacts/native-desktop-stages-20261019：stage-final.json 为原始时间戳；third-observation.json 为第三次真实操作与反馈；native-kde 包含 GUI 元数据、各次采样、独立读取及 cleanup.json；build-instrumented.json/log 与 build-final.json/log 分别记录诊断和恢复构建，gui.json 指向前者；source-restored.json、acceptance.json 和 final-health.json 记录逐字节恢复与最终健康。原始日志、标记、配置、凭据和辅助脚本不随 Git 提交。交接入口需求表及 P0 已同步此结论，下一轮应从这些证据继续，不重做已撤回的三个传输试验。

## 2026-10-19 Selkies 慢查看器与原生剪贴板修复

本次产品变更以 407b540 为基线，所有运行验证仍在 admin@192.168.217.6:22022 的 cranibox-validation。先在真实 KDE/Tauri 窗口做关联追踪，随后撤回全部插桩，构建并验收最终产品代码；本机只编辑、静态核对和 Git 操作。日常 Web、原生客户端、Selkies 和 KDE 均未升级或重启。

### 关联追踪与首次失败

临时 Rust/JavaScript 追踪只记录阶段、时间戳、长度和队列数量，不记录画面或剪贴板正文。一次 cr 上游发送完成于 1792351869618 ms，下一条剪贴板消息被 Rust 读到于 1792351914052 ms，相隔 44.434 秒；JavaScript 完成该消息拼接到事件派发约 1 ms。当时接收栏后续仍是旧文字。协议没有请求编号，且存在主动广播，这不是逐请求精确响应延迟；它将该轮主要等待定位在 Rust 读到消息之前，不能将所有停滞都归为 JPEG 解码。前端另用受控挂起的 JPEG 解码验证独立问题：旧实现的文字更新断言失败，修复后通过。

第一版上游有界缓冲和 JPEG 合并在实际窗口触发 Selkies output queue exceeded its bounded capacity。只读协议元数据显示，每组 JPEG 区域之间重复出现查看器原本不处理的 cursor 消息，它们成为合并屏障，导致队列继续增长。保留该失败，最终版本在缓冲前忽略未实现的光标位图与音频输出，再做区域合并。上游实际 JPEG 的完整宽度为 1024、区域高 192，y 为 0/192/384/576；只检查元数据，不把像素写入 Git。

### 最终实现

- Selkies 输出不再等待所有块交付后才读下一条上游消息；持续读取与有界缓冲并行。待发送记录最多 256 条、剩余数据最多 32 MiB、单条最多 16 MiB，每传输块最多 64 KiB。Selkies 输出通道仅两个数据位置和一个保留错误位置；其他桌面通道仍用原 32 个位置。
- 只删除尚未开始发送、同宽且被更新 JPEG 区域完全覆盖的旧区域；不跨分辨率/控制消息屏障，部分重叠仍保留，已部分发送的记录必须完整发完。剪贴板在完整记录边界优先，multipart/secret 等消息保持自身顺序。
- 前端完整收到剪贴板文字后立即处理，不等待异步 JPEG 解码；其他控制与画面仍按原顺序。单块 ACK、10 秒原生 ACK 超时、5 秒阻塞保护、250 毫秒授权检查和所有容量上限保留。没有增加依赖、另建控制连接或放宽阈值。

### 当前源码的运行验证

| 验证 | 结果 | 说明 |
| --- | --- | --- |
| Rust 工作区测试 | 183 通过，4 忽略；67.333 秒 | 含新增 5 项回归；桌面专项 10 项已包括在内，不重复累计 |
| 原生传输专项 | 4/4；0.10 秒测试时间 | desktop,custom-protocol；ACK、超时、提供者断开、主动关闭 |
| 构建/静态检查 | 全部通过 | 工作区构建、fmt、工作区与原生 all-targets 严格 Clippy；TypeScript/Vite、Web 资源生成与原生 debug 构建 |
| 客户端界面 | 48/48；81.006 秒 | 模拟 IPC；慢 JPEG 的断言扩展于既有用例，不新增计数 |
| Web 桌面界面 | 12/12；37.706 秒 | 重新嵌入最新资源的 Rust 服务与受控 RFB/JPEG 提供者 |
| 桌面后端集成 | 22/22；53.281 秒 | 直连/中继/权限/单控制者/守护与生命周期 |
| 真实 KDE Web 中继 | 1/1；18.726 秒 | 真实 Selkies 画面、键盘、双向文字、重连、撤权；pageErrors 为空 |
| 真实 KDE 原生窗口 | 五轮通过 | 实际 Tauri IPC，无插桩、无代理、隔离 XDG/私有 D-Bus；见下一表 |

工作区构建 73.483 秒，严格 Clippy 工作区 19.623 秒、原生 17.246 秒。已有 Vite 大包及依赖 use client 提示保留，不影响构建。实际原生 GUI 程序 SHA-256 为 372bb5eea65f5314a50d512d757d07de3c8a861e95e0e41702ded3757b7f4162。源码快照保留实际构建的摘要；后续工作区构建和测试不能代替该 GUI 运行版本。

| 原生真实操作 | 结果 |
| --- | --- |
| 初始获取 | 48 字节新标记逐字节匹配 |
| 第二段不同文字 | 66 字节逐字节匹配，连接保持，无超时 |
| 连接超过一分钟后再换文字 | 63 字节逐字节匹配 |
| 主动断开后重连 | 接收栏先清空，再获取新的 50 字节并逐字节匹配 |
| 重连超过两分钟后再换文字 | 62 字节逐字节匹配，连接保持，无超时 |

各段标记包含中文、emoji、多行。实际画面可见且非黑屏；两次 TCP 样本中上游发送队列为 0，但样本不是吞吐/延迟基准。每轮管理脚本约数秒包含 SSH 和辅助进程启动，不能当作协议响应延迟。原生额外发送自动化在准备文本框输入时失败，未执行发送操作，不计为产品通过或产品失败；原生发送的真实证据仍绑定旧 a8a97f5。重连脚本首次误用按钮名称后按真实界面修正并通过，属于测试辅助脚本修正。

最终全部测试停止后核对：原剪贴板逐字节恢复并删除备份，测试 GUI 与私有 D-Bus 等辅助进程退出，无预览/MCP 测试残留；容器 running/healthy、privileged=false、无 Docker socket；LAN 3080/3031 均匿名 401、认证 200；日常 Web PID 538744、客户端 PID 767446、原程序摘要与 NodeID 保持，测试安装包不在系统中。本轮没有替换日常服务或改变自启配置。

私有证据位于 /config/validation-artifacts/native-desktop-correlated-20261019（临时追踪、旧实现失败与新实现通过）、native-desktop-coalescing-20261019（首轮真实失败与协议元数据）、native-desktop-coalescing-v2-20261019（最终源码摘要、构建、各测试 json/log、五轮操作、截图、清理和健康）。辅助脚本、剪贴板、截图、配置与凭据不随 Git 提交。

**该轮限制：新修复的真实原生验收是 debug 程序及隔离桌面会话，不是新版正式安装包或正常登录桌面，也未覆盖另一物理设备、Windows/macOS、系统权限和实际登录/休眠。**后续安装包补验见下节“同提交 CI Linux 安装包实测”，完整需求和当前优先级见[交接入口](implementation-handoff-2026-10-18.md)。

### 提交与最终 CI 核对

产品提交 2c95dd49d7a11a6a6c6aedf87cf67dc769e9bd6a 已推送 origin/main；对应矩阵运行 39162097500 的七项作业全部成功，Release 39162097404 与 Sync GitCode 39162097385 也成功。Linux 日志确认 Web Playwright 126/126、原生模拟 IPC 48/48，真实 Chromium/官方 MCP、X11 VNC、独立 Sway/wayvnc、用户态 EasyTier 两项及 GIO 启动器均通过。三平台 Rust 与 Tauri 打包成功，共上传 10 项产物（三平台各一份服务端、节点与桌面客户端，以及 Linux 资源报告）；下载和后续验收须绑定该运行与提交。

最终证据目录 native-desktop-coalescing-v2-20261019 增加 ci-final.json 与三份 Linux/Wayland 原始作业日志，保留提交、状态和产物元数据；acceptance.json 追加 Git 发布信息。该次补记提交只改文档，不重计测试。CI 结果与上述远端专项有重叠，不能累加；截至此次 CI 核对，新版安装包、另一物理设备及 Windows/macOS 实机仍待验收，随后 Linux 包的实测见下节。

## 2026-10-19 同提交 CI Linux 安装包实测

在上述 CI 之后下载运行 39162097500 的 cranibox-node-desktop-ubuntu-latest（Artifact 12149318027），核对完整产品提交 2c95dd49d7a11a6a6c6aedf87cf67dc769e9bd6a；传到指定远端 admin@192.168.217.6:22022 的 cranibox-validation 后再次逐字节核对包摘要。本轮只增加验收与文档，没有改变产品源码，也不重跑或重计已通过的自动化矩阵。

| 产物 | 大小 | SHA-256 |
| --- | --- | --- |
| Cranibox Node_0.1.0_amd64.deb | 9,304,766 字节 | 897ba2ce2b266f12993766a6ef89587d50930e9127c09a7ec176230ae4c3dec3 |
| Cranibox Node_0.1.0_amd64.AppImage | 87,476,728 字节 | 46e21f5b2e809f1b8e96cf0e1cc24e1e71951aa91417b949f506a029cbc5677a |

.deb 通过 dpkg 实际安装，包名为 cranibox-node，程序位于 /usr/bin/cranibox-node-desktop；启动程序 SHA-256 为 b72d4c277e6f9579fb9e34c1a77470933923a2771ce17957656cf9ddbabb9b4e。AppImage 通过 --appimage-extract-and-run 启动，实际内置程序摘要为 67a31350877bec72e5b52f8cbeb12081fabe2f42183b03fa798a16280bc3ad35。各包使用独立 XDG、全新 NodeID、私有 D-Bus，在现有 KDE 中运行真实 Tauri 窗口与 IPC，直接连接日常 Selkies 8082，无诊断代理、插桩或模拟 IPC。

| 真实操作 | .deb | AppImage |
| --- | --- | --- |
| 初次获取新文字 | 48 字节逐字节匹配 | 48 字节逐字节匹配 |
| 更换文字并再次获取 | 连接数分钟后 62 字节匹配 | 第二段 56 字节、连接超过一分钟后 57 字节匹配 |
| 原生界面主动发送 | 41 字节匹配 | 46 字节匹配 |
| 主动断开并重连 | 接收栏移除，重连后先清空，新 55 字节匹配 | 接收栏移除，重连后先清空，新 55 字节匹配 |
| 重连后等待再更新 | 超过一分钟，新 63 字节匹配 | 超过两分钟，新 63 字节匹配 |

各段文字包含中文、emoji、多行，实际画面可见且非黑屏。管理脚本时间包含 SSH 与辅助进程启动，不当作协议响应延迟。发送测试先逐字节核对界面草稿，再将目标系统剪贴板改为不同的哨兵文字，确认目标与草稿不同且草稿未变后，点击真实“发送文字”并用 wl-paste 检查目标。准备输入时使用 AT-SPI 聚焦和 Selkies 键盘 Ctrl+A/V，未通过产品发送接口提前写入目标；因此发送结果不等同于准备阶段写剪贴板。

最初输入准备尝试中，PixelFlux type 只输入了 ASCII 部分，几种快捷键字符串也未完成粘贴；未执行产品发送，不计为产品失败。更正辅助脚本后中文完整准备及真实发送通过；保留 draft-ui-type.json、draft-ui.json、key-probe.json 与最终 send-preparation.json、send-result.json。最初 dpkg 查询误用了可执行文件名作为包名，实际安装成功，随后按包元数据核对 cranibox-node；同样属于辅助脚本修正。

最终恢复各轮原剪贴板并逐字节检查，停止测试窗口与私有会话辅助进程，卸载临时 .deb。日常 Web PID 538744、客户端 PID 767446、程序摘要与 NodeID 保持；容器 running/healthy、非 privileged、无宿主 Docker socket，3080/3031 入口均匿名 401/认证 200。未升级日常程序、重启 KDE/Selkies 或改动日常自启配置。

私有证据根目录 /config/validation-artifacts/linux-installer-2c95dd4 包含包元数据、安装/卸载日志、baseline/final-health.json、各包 GUI 元数据、操作/发送/截图/清理记录，以及最终 acceptance.json、final-health.json。正文、截图、凭据、配置与辅助工具不随 Git 提交。

本轮补齐同提交 Linux 正式产物的隔离 KDE 双向剪贴板与重连；**仍未覆盖正常登录 D-Bus/托盘/自启、真实登录和休眠、普通 FUSE、另一物理设备、Windows/macOS，以及新包的超时/断网/撤权流程**。P0 已调整为这些未覆盖的日用流程，详见[交接入口](implementation-handoff-2026-10-18.md)。

## 2026-10-19 已有桌面服务检测与配置核对

本轮以 799eae3499a5933aa7423cc94f9e900667331682 为基线，补齐已有 VNC/Selkies 的接入引导。所有运行验证仍在 admin@192.168.217.6:22022 的 cranibox-validation，普通用户 abc；本机只编辑、静态核对、管理私有证据和 Git 操作。未增加依赖。

### 实现与需求边界

Rust 桌面服务新增显式 discover：检查当前共享和已保存的托管地址，以及 IPv4/IPv6 回环的 VNC 5900、Selkies 8082，去重后最多六项并发、每项最多三秒。只读取 RFB 握手或 MODE websockets；不扫描局域网、不认证、不发送 SETTINGS、不启动或保存服务，不接管桌面。接口不接受调用方指定的扫描范围。

Web/Tauri 共用“检测已有服务”、结果核对与可编辑表单；保存前重新检查所选协议，通过后才开启共享。取消恢复配置按钮焦点；开启或停止同步更新当前共享标记。补齐空结果、重试、加载、忙碌保护、离页迟到响应、键盘操作、明暗主题与窄屏；核对表单紧接结果，平台说明引用按钮名称。这是已有服务引导，Windows/macOS 的提供者安装、系统权限和进程托管仍未完成。

### 构建与自动化

| 验证 | 结果 | 范围 |
| --- | --- | --- |
| Rust 工作区测试 | 183 通过、4 忽略，20 个测试目标 | 忽略项为离线测量、两项真实 EasyTier 和 GIO；本轮没有重跑这些忽略项 |
| 原生传输专项 | 4/4 通过 | desktop,custom-protocol；不是 GUI 验收 |
| 桌面服务集成 | 11/11 通过，无跳过 | 真实 Rust、隔离提供者；新增 IPv4/IPv6、无效/无响应及重试、已保存的自定义 Selkies、配置/凭据不变和不认证/不接管 |
| 桌面传输集成 | 7/7 通过 | 真实 Rust 与隔离 VNC/Selkies 提供者 |
| Web 全量 | 128/128 通过；命令耗时 215.26 秒 | 桌面 14 项已包括其中；检测响应可控，保存检查、配置及连接仍使用实际 Rust |
| 客户端界面全量 | 49/49 通过；命令耗时 103.99 秒 | 浏览器模拟 Tauri IPC，不能当作原生包测试 |
| Web API 检查 | 通过 | 访问保护、资源、缓存、表单、后台操作、刷新恢复和紧急停止 |
| 静态检查及构建 | 通过 | fmt、工作区及原生 all-targets Clippy -D warnings、工作区构建、两端 TypeScript/Vite，以及嵌入资源的实际 Web/原生 debug 构建 |

全量后仅调整平台说明文案及表单与说明的 20 像素间距，重新构建两端资源与实际 Web/原生 debug；Web 桌面 14/14、客户端相关 3/3 及下述真实服务流程再次通过。最后两处调整后没有重复全量 128/49；专项不与全量累计。

### 首次失败与修正

- 客户端首轮 48 通过、1 失败：已有 MCP 加载用例使用 250 毫秒延迟，断言前响应已完成；测试改为显式释放响应，最终 49/49。产品 MCP 行为没有为此修改。
- 构建发现新增组件未接受可省略的 protocol 类型；改用共享目录类型，标记计算沿用缺省 VNC 规则，之后 TypeScript 与实际程序构建通过。
- Web 全量首轮 119 通过、9 个终端用例失败：辅助脚本漏配 CRANIBOX_HERDR_BIN，界面显示需要安装 Herdr。补齐容器已有 /config/bin/herdr 后全量 128/128；没有改终端产品代码。九项失败上下文与 trace 均保留。
- 辅助脚本曾误用 workspace-web 包名，正确 Cargo 包名为 agent-workspace-web；API 检查曾在容器根目录解析不到程序，随后使用源码工作目录和明确 WORKSPACE_WEB_BIN 通过。它们属于执行脚本修正，不计为产品测试通过。

### 真实服务与清理

实际 Rust Web 浏览器流程检测到日常 Selkies 127.0.0.1:8082，选择后协议与地址正确、焦点在地址输入；取消后回到配置按钮。记录的服务操作只有 status 与 discover，没有开启共享或打开查看器。测试前后共享配置逐字节不变，未生成托管配置或密码。最终重新检查 375/1440 像素明暗主题截图；自动化另覆盖 768/1024、812 横屏和减少动画。无页面异常。

最终 source-binaries.json 核对 12 份产品源码与资源和本地待提交文件一致，并记录五个 Rust 程序摘要。全部测试清理后，容器 running/healthy、privileged=false、无宿主 Docker socket；3080/3031 均匿名 401、认证 200。日常 Web PID 538744、原生客户端 PID 767446、程序摘要与 NodeID 保持，无测试预览/MCP 进程残留，临时安装包不存在。未升级日常程序、重启 KDE/Selkies，未操作桌面输入或剪贴板。

私有证据根目录 /config/validation-artifacts/desktop-discovery-20261019：checks.json 与首轮日志；final 的 Rust 测试和构建；ui-final 的全量失败日志及 failure-contexts；verified 的全量成功日志、API 检查和初次真实服务检查；post-review 的最终构建、14/3 专项、real 的截图/操作/配置不变记录、source-binaries.json 和 final-health.json。凭据、截图、配置、原始正文与辅助脚本均不提交。

本轮新检测功能没有做正式安装包的真实 GUI 验收；之前 Linux .deb/AppImage 的通过仍仅绑定 2c95dd4。正常登录/托盘/自启、跨物理设备、Windows/macOS、系统 Tailscale 安装、公开镜像拉取、完整同步及运行中 Agent 迁移仍有未完成项，已按 P0/P1/P2 列在[交接入口](implementation-handoff-2026-10-18.md)。本轮提交与 CI 状态在后续补记绑定，不能挪用旧矩阵的成功。

### 提交与 CI 交接补记

产品提交 84454e695d915157d245630c4ad3b8b77091c0b6 已推送 origin/main，功能、生成资源、回归与需求/验证文档均在此提交。GitHub 矩阵 39169225796、Release 39169225825、Sync to GitCode 39169225719 均核对完整 headSha，交接时为 in_progress，**尚未记为通过**。后续文档补记没有改变产品源码。下一位维护者继续核对运行结果，再下载该矩阵的新版安装包验收新检测流程；不要以 39162097500 的旧绿色矩阵替代。

## 2026-10-19：默认公开空间镜像兼容性核对

在指定远端容器中通过匿名 Docker Registry API 读取默认镜像 xuping/agent-workspace:ubuntu-xfce 的索引、Linux amd64 manifest 和配置；没有拉取镜像层、启动容器或使用宿主 Docker socket。公开索引摘要为 sha256:f2e58c4fffd5652e6b3736f6af432a81ededd3349f04c53195d87169f6eb06fa，amd64 manifest 为 sha256:4d60b036e39d8194457f8d2c12b340101e00163606b20aa3fe7ac4b304b4ad97。配置 blob 摘要 sha256:9340d7269f2133220b3a712d69ccd6b82d483b68789e6e8e8115eca584965f58 与实际字节匹配。

来源标签仍为 c56508ce5008180e067a9c489f4527aa30b80b31，镜像配置创建于 2026-10-13T10:40:40.352550645Z，缺少 io.cranibox.space-protocol。对照当前 spaces/provision.rs 的保护条件，该公开版本会在数据卷创建前被拒绝，属于已确认的兼容性阻塞，不是运行成功。没有覆盖 arm64、阿里云镜像站、真实拉取或首次启动；没有修改、发布公开标签。

私有证据 /config/validation-artifacts/public-image-review-20261019/metadata.json。交接 P0 改为“发布兼容镜像并完成真实拉取/首次启动/重启验收”；保留此前 71f2397 本地 KDE 镜像成功的版本边界，不将其冒充公开镜像结果，也不放宽协议检查。

## 2026-10-19：84454e6 的 CI Linux 安装包发现流程实测

下载矩阵 39169225796 的 cranibox-node-desktop-ubuntu-latest（artifact 12151304011），核对完整 headSha 84454e695d915157d245630c4ad3b8b77091c0b6；Linux 客户端作业已成功。两份文件在本机下载后与远端传输后核对大小及 SHA-256：

| 包 | 字节 | SHA-256 |
| --- | --- | --- |
| Cranibox Node_0.1.0_amd64.deb | 9,316,726 | b4cdd7157696f74fed8af6add38ccc7c5f9ec181cfe6877c58604718bf9147eb |
| Cranibox Node_0.1.0_amd64.AppImage | 87,484,920 | 26869e475df7538bc7211c8e598637089993a3982d63f758228adffe534a364a |

.deb 通过实际 dpkg 安装，AppImage 使用 --appimage-extract-and-run。两包分别在指定远端 KDE 中以新 NodeID、隔离 XDG 与私有 D-Bus 启动真实 Tauri 窗口，未模拟 IPC。使用实际无共享初态，点击远程桌面、检测已有服务，找到真实 Selkies 127.0.0.1:8082；核对后协议为 Selkies、地址正确且输入聚焦，取消后 peers.json 摘要逐字节不变，托管配置和密码均不存在。

再次核对并点击保存后，实际 peers.json 保存 Selkies 协议和地址，列表出现“当前共享配置”、停止共享按钮可用；点击停止后实际共享为空、标记及停止按钮消失，身份保持，未生成托管配置或密码。两包操作/状态断言与截图均完成，acceptance.json 汇总通过。没有打开查看器、发送桌面输入或改动剪贴板。

两份初始脚本失败分别为 AT-SPI button 角色名称和列表内联标记文本匹配错误，保存在 deb-native/exercise-initial-role-failure.json 与 exercise-initial-badge-failure.json。根据真实控件树修正管理脚本，并先通过界面停止试验共享，再重跑完整流程；两包最终通过，未修改产品代码。失败不计作产品通过。

退出两包 GUI 及属于对应 XDG 的辅助进程，卸载测试 .deb、清除本次 AppImage 解压目录；审计无试验进程残留。容器 running/healthy、非 privileged、无宿主 Docker socket，3080/3031 匿名 401、认证 200；日常 Web PID 538744、客户端 PID 767446、程序摘要与 NodeID 均保持。源码及日常程序未升级。

范围仅为已有服务发现与配置，不重验正常登录/托盘/自启、普通 FUSE、休眠、网关断线、另一物理设备、桌面查看器、双向剪贴板、撤权或 Windows/macOS 实机。旧版 2c95dd4 的剪贴板结果保留独立版本边界。私有证据目录 /config/validation-artifacts/linux-installer-84454e6 包含包元数据/摘要、安装/卸载记录、每包 GUI/操作/状态/截图、失败/修正记录、cleanup-audit.json、acceptance.json、before-health.json 和 final-health.json；不提交截图、配置、凭据或辅助脚本。

### 本轮 GitHub CI 最终结果

产品提交 84454e695d915157d245630c4ad3b8b77091c0b6 的矩阵 39169225796 已完成，7/7 作业全部 success：Linux/Windows/macOS 原生 Rust、三平台 Tauri 客户端，以及独立真实 Sway/wayvnc。Release 39169225825 与 Sync to GitCode 39169225719 也 success，均核对完整 headSha。此前补记的 in_progress 为当时状态，本节记录最终结果。

原始日志确认 Linux CI Web 全量 128/128（3.1 分钟）、客户端界面全量 49/49（22.9 秒）、真实 Chromium/官方 MCP、X11 VNC、Sway/wayvnc、真实 EasyTier 2/2、GIO 启动器均通过。完整 CI 使用最终文案/间距后的产品提交，不再仅有此前远端专项的覆盖；与远端或历史结果重叠部分不累计。客户端 Playwright 仍模拟 IPC，不能替代上节真实 Linux 包，也不代表 Windows/macOS 实机验收。

10 项未过期产物为 cranibox、cranibox-node、cranibox-node-desktop 各三平台，加 resource-baseline-linux。Windows 客户端为 NSIS .exe，macOS 为 .dmg；本轮只下载并在指定远端实际运行 Linux .deb/AppImage。Release 工作流只做脚本/Dockerfile/Compose 检查和脚本地址维护，ShellCheck 允许失败；它不发布镜像，默认公开镜像不兼容的问题仍未解决。

远端 linux-installer-84454e6/ci-final.json 保存七项作业及步骤、两个其他工作流、完整来源与产物元数据；ci-native-linux.log、ci-desktop-linux.log、ci-wayland.log 保存原始日志。此后仅整理交接与验证文档，没有修改已经验收的产品代码。

## 2026-10-19：镜像发布前门槛与完整 XFCE 镜像

本轮只增加镜像发布流程、独立验收脚本、其回归和文档，Rust/React 产品代码仍绑定 84454e6。Build Multi-Arch Image 默认手动 publish=false，可选择桌面；正式发布先推送唯一候选，按摘要完成 amd64 启动验收后才提升公开标签，阿里云候选也独立验收。没有触发新 GitHub 镜像工作流或发布公开镜像；默认标签的已确认阻塞仍存在。

检查器回归最终 14/14，通过缺少协议/来源错误的提前拒绝、实际就绪与容器健康的区分、授权/执行失败、私有报告、不泄漏子进程正文，以及资源归属/清理失败门槛。首轮 11 项中四项发现报告写入误用 Path.open(opener=...)；改用内置 open 并先 fchmod 后通过，之后补到 14 项。两个工作流的 actionlint v1.7.7 语法/表达式检查通过（关闭 ShellCheck），另有 14 段 run 的 bash -n 检查通过。上述均在指定远端容器 abc 下执行，不是产品全量重跑。

完整 Git 源码快照为 18c321654dcc21cdb8c9eb1c5686f7ed73d4d6c4，归档 SHA-256 为 957510fd71eb72aa86b44657906f9ff6688e49afb37d9353efd43c7e94f530e0。指定远端宿主使用既有独立 buildx 构建器，限制 2 CPU、6 GiB、并发 2，未选为默认构建器；从拉取基础镜像开始完整构建默认 XFCE linux/amd64 镜像，没有运行本机 Docker。初始辅助脚本因宿主 Python 不支持 tarfile 的 filter 参数在 Docker 调用前中止，修正可信 Git 归档的解包方式后完成构建，不算产品失败或启动通过。

构建耗时 666.17 秒，镜像 cranibox-image-release:18c3216-xfce，ID sha256:fa81e38cb9c42d7cfc3ea6b156b0ab2002b412dfffd7ae54830801e0ce4f08d0，大小 5,261,024,756 字节。协议标签为 1、来源标签匹配完整提交；构建器已停止，默认选择不变。该源码快照不含本轮新检查器/CI 文件，启动运行产品代码与当前产品相同，不能冒称新 CI 提交镜像已经发布。

待提交检查器实际内容 SHA-256 d18ea9d50f0ed89d8b36248dd1f242833aafa912cc709d4e5b059b47ca9f3b82，与远端执行文件一致。对空普通卷与预置独立身份/授权的空间卷，各执行首次启动和重启，43/43 通过，耗时 86.73 秒：两端 Rust 健康、Web 静态资源与节点 API、内置程序及链接、默认服务、0600 凭据、桌面匿名 401/正确认证 200、应用根 404；空间身份保持、有效令牌列节点/执行真实命令、无效令牌 401；两类卷的身份、凭据摘要和测试文件重启保持。

测试容器不发布端口、非 privileged、无宿主 Docker socket，使用独立命名卷并限制资源；初始化配置的辅助容器另设无网络及资源上限。两个主容器、两个卷与初始化容器均已按本轮标签清理，无资源被留作日常服务。新镜像保留在远端缓存中。旧日常镜像上的只读探针预检因缺少内置 cranibox-node 中止，记录 probe-read-only-failure.log，不计入本镜像通过，也未修改日常镜像。

清理后日常容器 running/healthy、非 privileged、无宿主 Docker socket；3080/3031 匿名 401、认证 200。日常 Web PID 538744、客户端 PID 767446、程序摘要和 NodeID 保持，无测试预览/MCP 进程或安装包残留。没有接管 KDE/Selkies 输入或剪贴板。

私有证据 /config/validation-artifacts/image-release-20261019 保存构建与启动 JSON/日志、脚本/工作流快照、单元日志、actionlint.log、workflow-bash-syntax.json、final-health.json；不提交配置、密码、令牌、原始日志或辅助脚本。本轮不覆盖公开镜像拉取/发布、arm64 运行、Aliyun、新 GitHub 镜像工作流、浏览器桌面输入或完整空间生命周期。公开发布仍按[镜像发布说明](workspace-image-release.md)继续，不放宽协议保护。

### 收尾提交与 CI

代码提交 dff9ed957eb756fb82b32a5b9fe70f367f9b8cd7 已推送 origin/main。Release 39173470993 与 Sync to GitCode 39173470952 均 completed/success，核对完整 headSha。Release 中新增 Test image release gate 及安装脚本、Dockerfile、Compose 步骤均成功，原生七项矩阵未触发，产品矩阵仍绑定 84454e6。ci-dff9ed9.json 和 ci-dff9ed9-test-scripts.log 保存本次原始结果，cleanup-audit.json 另确认检查器容器/卷为空、独立构建器停止且默认构建器保持 default。此后仅补记文档。

## 2026-10-19：GitHub 镜像工作流与正常 KDE 会话补验

本轮没有修改 Rust/React、镜像或检查器源码。对最新 main 的 b76aaba2563af1869999228a664cf20cc7f93d4f 手动触发 Build Multi-Arch Image，desktop=xfce、publish=false；运行 **39173924368** 最终 completed/success，完整 headSha 已核对。GitHub 记录时间为 2026-10-18T22:49:57Z 至 23:06:00Z。

完整候选构建从 22:51:00Z 到 23:04:29Z，约 13 分 29 秒；首次启动、空间接入与重启步骤成功，原始报告 **43/43**、68.44 秒，四项测试容器/卷清理均 removed=true。镜像为 linux/amd64，ID sha256:48212c54012c771d86e82a1933dc31fe8adb1e2a2255cdd0bddb870893e14fe9，来源标签为完整 b76aaba，空间协议为 1。覆盖范围与前节远端 43 项一致，不将两次运行累计为新的不同用例。

工作流上传 artifact **12153698875**（image-startup-xfce），实际下载并逐项核对通过字段及清理结果。Docker Hub 登录、公开标签提升和 Aliyun 作业按只验证参数跳过，没有发布候选或公开镜像；此模式只构建 amd64。默认公开镜像不兼容的既有阻塞保持，不能将工作流成功写成公开发布或 arm64 验收。私有 image-release-20261019/ci-b76aaba 包含 run.json、artifacts.json、job.log 和原始启动报告。文档提交 b76aaba 的普通 Release 39173870697 与 GitCode 39173870757 也已核对 success；产品三平台矩阵仍是 84454e6 的 39169225796。

### 同产品提交 Linux 包的正常 KDE 总线

复用上节已下载并核对的 84454e695d915157d245630c4ad3b8b77091c0b6 CI 包，.deb SHA-256 b4cdd7157696f74fed8af6add38ccc7c5f9ec181cfe6877c58604718bf9147eb，AppImage SHA-256 26869e475df7538bc7211c8e598637089993a3982d63f758228adffe534a364a。仍在指定远端 cranibox-validation、abc 下操作；.deb 实际安装，AppImage 使用持久包路径及 --appimage-extract-and-run。

先备份原客户端程序、启动环境和身份摘要，核对原 PID 767446 没有活动 Shell，仅有 WebKit 子进程后暂停。测试包分别使用隔离 XDG 和新身份，直接连接既有 plasmashell 的正常 D-Bus，不创建私有会话总线，未模拟 Tauri IPC。KDE、Selkies、日常 Web 持续运行。

| 实际操作与断言 | .deb | AppImage |
| --- | --- | --- |
| 正常 KDE StatusNotifierWatcher 注册、实际托盘菜单 | 通过 | 通过 |
| Wayland 窗口经 KWin 关闭，窗口隐藏而进程存活 | 通过 | 通过 |
| 再次启动退出码为 0、原 PID 保持、窗口唤回 | 通过 | 通过 |
| 托盘退出显示确认框，确认后 GUI/启动器退出 | 通过 | 通过 |
| 实际界面开关登录自启动、启动项路径/后台参数/0600 权限 | 通过 | 通过 |
| GIO 执行生成的启动项，初始窗口隐藏、托盘唤回 | 通过 | 通过 |
| 前台启动与后台重启的 NodeID、二进制摘要保持 | 通过 | 通过 |
| 界面关闭自启动后条目消失、清理无残留 | 通过 | 通过 |

.deb 的“收起到系统托盘”按钮另通过真实界面与隐藏/唤回断言；两包的原生窗口关闭都通过实际合成器操作和 AT-SPI SHOWING 状态检查。GIO 使用生成的真实 Exec 行，AppImage 启动项包含兼容参数，不依赖临时解压路径或手动 APPIMAGE_EXTRACT_AND_RUN 环境变量。没有实际退出/重新登录系统，因此启动项验收不能记作系统登录验收，也不代表普通 FUSE。

保留三类测试环境/辅助脚本失败：正常 org.a11y.Bus 进程存在但广告的 socket 已丢失，首轮 AT-SPI 中止；核对进程归属后只恢复辅助功能服务，socket 重建后实际窗口树可读，KDE/Selkies 未重启。其次 xdotool 无法按 PID 找到真实 Wayland 窗口，改用目标 PID 限定的临时 KWin 脚本；初次调用重载 loadScript 的推断签名不符，明确 ss 签名后通过窗口关闭断言。临时 KWin 脚本完成后卸载并删除。失败不计入产品通过，没有为此修改产品代码。

测试后两包 GUI/启动器和所属 XDG 辅助进程均退出，测试 .deb 已卸载，本次 AppImage 解压目录已移除，两份测试自启项均关闭，无测试进程残留。恢复原客户端，当前 PID **1403228**，程序 SHA-256 e4199d9316bb4cd257ba7505865cdf35f78363a6ecb78e750a4eae949eb31510、NodeID node-82bf96c6f15d4f00923656f7877b8609 及身份文件原始字节保持。日常 Web PID 538744 与原程序保持。最终容器 running/healthy、非 privileged、无宿主 Docker socket；3080/3031 匿名 401、认证 200。

私有 normal-desktop-84454e6-20261019 保存 acceptance.json、各包启动/后台/托盘/关闭/单实例/退出/自启状态、events.jsonl、accessibility-recovery.json、automation-failures.json、安装日志、原程序备份与私有环境、恢复/清理记录、final-health.json。原环境与配置只留在 0700/0600 证据中，不提交。未操作剪贴板、重验桌面查看器/撤权、制造网络故障或休眠；未覆盖实际系统登录、第二物理设备和 Windows/macOS 原生 GUI。原客户端本次仅恢复，没有升级，不应将旧日常程序作为 84454e6 包使用。

## 2026-10-19：公开 Agent 目录后台同步与离线快照

本轮在指定远端容器 cranibox-validation、abc 下完成构建和运行测试；本地仅编辑、核对与 Git 操作。Rust 节点增加独立 agent-sync 策略与凭据绑定的公开目录快照，Web/Tauri 共用配置。默认不启用，1–32 个显式目标、15/30/60/300 秒间隔，与项目同步共用四路后台请求。关闭页面后继续，节点重启恢复；暂停不重放操作。沿用目标消息授权，未开放全设备任务/消息账本。

在线读取严格检查目标身份和目录结构；远端不能注入本机 sync 标记。离线回退只适用于可重试网络错误，保留原 observed_at 和租期；页面标记历史记录并禁止以其选择当前 Agent。授权、身份和内容错误清除快照；连接变更需重新核对，请求完成后重读绑定，移除后的迟到响应不能恢复缓存。CLI/MCP 也返回 online/offline 标记。操作说明见[会话文档](agent-sessions.md#关闭页面后的同步与离线快照)。

| 验证 | 最终结果 | 范围 |
| --- | --- | --- |
| Rust 工作区 | 183 通过、4 忽略 | 20 个目标；忽略项不计通过 |
| 原生传输专项 | 4/4 通过 | desktop,custom-protocol，不是实际 GUI |
| 后端集成 | 42/42 通过，无跳过；312.406 秒 | Agent 后台同步 8、项目后台 9、项目目录 7、任务 12、worker MCP 6；独立 Rust 身份与真实 HTTP/CLI，部分受控服务器 |
| Web 全量 | 130/130 通过；约 3.7 分钟 | 最终生成资源和实际 Rust Web；新增目录界面使用可控响应 |
| 客户端全量 | 50/50 通过；约 4 分钟 | Playwright 模拟 Tauri IPC；无 relay 的共享设置、导航保留、暂停和离线禁用 |
| 构建与静态检查 | 通过 | 两端 TypeScript/Vite、工作区与原生 debug 构建、fmt、两类 all-targets Clippy -D warnings；最终再次生成资源并重建嵌入 Web |
| CI 文件 | 通过 | actionlint v1.7.7，关闭 ShellCheck；Linux CI 新增 Agent 后台集成 |

新增八项后端用例覆盖无页面、独立节点无 Web/网关、重启、0600/敏感字段排除、离线租期不续期、恢复/撤权、暂停/重选、伪造 owner/sync 拒绝、移除后迟到响应、两类目录四路总并发与暂停队列、缺少授权、旧版本及损坏策略隔离。目录只保留按来源的公开登记；测试不等于多物理设备、完整数据复制、模型调用或运行会话迁移。

### 失败与修正

- 新增集成首轮 2 通过、6 失败：测试复用已被夹具创建的令牌输出文件名。改为本测试独立文件后通过，保留产品拒绝覆盖规则。
- 首次 Web 专项在用例前因预览脚本缺少 Rust 程序路径中止。补齐后 17 通过、1 失败：option 本来已有 disabled 属性，Playwright 的 toBeDisabled 不适用；改为检查 DOM disabled 属性，仍单独检查提交按钮不可用、没有提交请求。
- 首次 Web 全量 129 通过、1 失败：项目同步旧用例仍匹配 Unexpected project sync settings；共享组件现使用 Unexpected catalog sync settings。更新该断言后，重新运行最终全量 130/130。不是放宽身份校验或隐藏错误。
- 首轮 TypeScript 构建发现共享命令类型缺少 agent_sync，已补齐并通过两端编译。Vite 的已有大包和依赖 use client 提示仍在，没有放宽 Rust 警告检查。

失败日志与 trace 保留，不计作通过；不同轮次与专项重叠不累计。最终全量使用重新生成并嵌入的 Web 程序。客户端资源、原生 debug 与严格检查也已完成，但没有对本轮功能重新做真实安装包窗口或系统登录验收。

### 源码与环境清理

19 份本地待提交产品源码、测试、工作流与生成资源和远端实际快照逐项 SHA-256 一致，另保存五个 Rust 程序摘要。后台设置用例检查 375/768/1440 像素无横向溢出、设备选择行至少 44 像素，审阅 375/1440 截图；项目共用控件已有明暗主题回归。没有将浏览器模拟 IPC 当作原生实机。

最终容器 running/healthy、privileged=false、无宿主 Docker socket，3080/3031 均匿名 401、认证 200。日常 Web PID 538744、原生客户端 PID 1403228、程序摘要及 node-82bf96c6f15d4f00923656f7877b8609 身份保持；KDE/Selkies 未重启，未操作桌面输入或剪贴板，无预览/MCP 进程或测试安装包残留。日常程序没有升级。

私有证据 /config/validation-artifacts/agent-catalog-sync-20261019：integration-second.log、build-checks.log、final-assets-checks.{json,log}、client-full.log、web-final.log、首次失败日志及 web-second-failure/web-full-failure trace、source-binaries.json、final-health.json 与 375/1440 截图。配置、令牌、截图、原始正文与辅助脚本不提交。Windows/macOS 实机、三物理设备同步、系统登录、完整账本复制、公开兼容镜像发布及实际 Agent 接续仍按交接入口的 P0/P1/P2 继续。

### 本轮提交与 CI 交接

产品提交 87f20ee3ceea0e0de65161f62e19d4cb28427c81 已推送 origin/main。完整 headSha 核对的 Rust/Tauri/Wayland 矩阵 39178636288 交接时为 in_progress，**尚未记作通过**；Release 39178636180 与 Sync to GitCode 39178636169 均 completed/success。GitHub 创建时间为 2026-10-18T23:57:30Z、23:57:29Z、23:57:29Z。ci-handoff.json 保存当时作业和步骤；后续只补文档，不改变测试过的产品代码。下一位维护者核对矩阵终态，再使用该提交的安装包做原生验收；默认公开空间镜像仍未发布。

## 2026-10-19：公开 Agent 同步的 CI 终态与原生包验收

本节补齐上一节交接后仍在进行的 CI 与 Linux 包检查，没有修改产品源码。三项工作流的完整来源均为 **87f20ee3ceea0e0de65161f62e19d4cb28427c81**：矩阵 **39178636288 completed/success、7/7**（三个平台 Rust、三个平台 Tauri、独立 Sway/wayvnc），最终更新时间 2026-10-19T00:23:12Z；Release **39178636180** 与 GitCode **39178636169** 也均 completed/success。保留先前 ci-handoff.json，不把进行中记录覆盖成最终状态。

下载 Linux Rust 与客户端作业原始日志，确认 Web 全量 **130/130，3.7 分钟**，客户端界面 **50/50，34.4 秒**，后台 Agent 新增集成 **8/8，80.564 秒**，真实 Chromium/官方 MCP、X11 VNC、EasyTier **2/2**、GIO 启动器通过。界面全量仍模拟 Tauri IPC，Windows/macOS 的作业成功不代表原生桌面或权限实测；与远端已有回归重叠的结果不累加。Release 未发布公开空间镜像。

### 包来源与真实窗口

实际下载 artifact **12155552479**（cranibox-node-desktop-ubuntu-latest），归档元数据摘要为 sha256:3310f7458465ca914678e54b29cf3e1db358f84c60427adbaf3b23f227096f8a；其 workflow_run.head_sha 与成功的 Linux 客户端作业均匹配上述产品提交。传到指定远端后再次核对大小与包摘要。

| 包 | 字节 | SHA-256 |
| --- | --- | --- |
| Cranibox Node_0.1.0_amd64.deb | 9,363,574 | f300792cc4ca6ece469e3d554c60d53faa70e6a746a7147ea94982cfa7adbb81 |
| Cranibox Node_0.1.0_amd64.AppImage | 87,517,688 | 27ba2283c4720440485ef65ea50f541b6a1c9915dbebaba1b10c8a11e81c9d02 |

仍在 admin@192.168.217.6:22022 的 cranibox-validation、abc 下验收，使用现有 KDE/Wayland。两包分别使用全新身份、隔离 XDG 与私有 D-Bus，实际 Tauri 窗口/IPC，目标为同容器独立身份的真实 Rust 节点，经回环 HTTP 直连，不用 Web/网关或 IPC 模拟。.deb 实际 dpkg 安装，系统包名 cranibox-node；AppImage 使用 --appimage-extract-and-run。

两包均完成：实际界面选择目标并启用 **15 秒**策略；KWin 关闭原生窗口后进程存活，目标随后新增的登记约在 11 秒后进入 0600 快照，唤回同一 PID 后界面可见；客户端停止期间新增另一登记，再以相同 XDG 启动，打开任务页之前后台已获取新目录，NodeID、策略和程序摘要保持。此处秒数包含检查脚本轮询，不作为协议性能基准。

停止目标节点服务后，后台转 offline；实际界面显示“上次读取的登记”“状态未确认”，保存三条历史登记，三个“委托给此 Agent”按钮全部不可用。目标恢复后后台回 online。随后从界面暂停后台同步：.deb 另重启确认暂停策略保持；AppImage 离开任务页后关闭原生窗口。分别建立无页面读取干扰的基线，目标新增登记后等待 **18 秒**，暂停状态和快照字节均保持，没有同步新增登记。该时间只证明超过一次配置间隔，不代表长时间耐久性验收。

### 辅助脚本失败与范围

保留四份失败动作记录：.deb 离线断言误找不存在的“离线快照”文字；AppImage 的刷新按钮误写成“刷新”，实际名称为“刷新所有 Agent”；并行私有 GUI 导致 .deb 的辅助功能总线不可达；首次 .deb 暂停检查的快照摘要仍被最后一次页面交互读取更新。最后一项中后台 last_attempt 没有继续推进，快照仍为原三条，不能据此宣称后台暂停失效。随后串行运行、修正真实控件名称并排除页面轮询后通过，产品代码不变。

另记录一次 AT-SPI select_child 返回成功但没有触发 React 事件：第一次启用实际保存的仍是 60 秒。改为实际 option 动作并重新保存，核对真实配置确为 15 秒后才继续；未将脚本返回成功当作设置已保存。后台暂停和页面刷新分别控制，页面主动读取仍可更新快照。初次失败保留在 deb-offline.json、deb-pause-final.json、deb-resume-pause.json、appimage-hidden.json；最终阶段记录及 acceptance.json 明确选取通过的动作，未抹掉失败或累计重复用例。

本轮没有重验正常会话总线的托盘、自启动、实际登录、休眠、普通 FUSE、桌面控制/剪贴板、撤权、第二物理设备或 Windows/macOS GUI；各范围仍按前文对应产品版本及 P0/P1/P2 继续。目标是独立真实进程，仍不是多物理设备。

### 清理与交接

两份 GUI/启动器、独立节点目标和所属 XDG 辅助进程均停止；测试包已卸载，本次 AppImage 解压目录已按来源核对后删除，归属检查没有残留进程。日常 Web PID **538744** 与客户端 PID **1403228**、各自程序 SHA-256 及 NodeID **node-82bf96c6f15d4f00923656f7877b8609** 保持。容器 running/healthy、privileged=false、无宿主 Docker socket；3080/3031 匿名 401、认证 200。未升级日常程序、重启 KDE/Selkies、操作文字剪贴板或发布镜像。

私有证据 **/config/validation-artifacts/linux-installer-87f20ee** 保存 artifact.json、package-digests.json、CI 原始日志、acceptance.json、两包阶段 JSON、原生进程/界面树/快照与失败记录、安装/卸载日志、cleanup-audit.json 和 final-health.json；最终 CI 元数据另位于 agent-catalog-sync-20261019/ci-final.json。配置、令牌、截图、辅助脚本与原始日志不提交 Git。本轮仅提交测试结论与交接补记，完整需求/剩余事项见[交接入口](implementation-handoff-2026-10-18.md)。

## 2026-10-19：跨设备协作记录统一视图

本轮在 admin@192.168.217.6:22022 的 cranibox-validation、abc 下完成运行验证，本地只编辑、静态审阅、核对与 Git 操作。Web/Tauri 共用按来源的任务、问答/结果、消息和协作 Run 只读列表；CLI 可直接查询本机或远端。新 activity 权限默认关闭、独立于 messages，明确包含其他发送方正文；它不授予投递、修改或执行。操作、协议与容量见[协作记录文档](federated-activity.md)。

来源保留权威，不合并同 ID 的多来源报告，不改来源账本。摘要排除读取时间，只用于确定内容是否相同，不是签名或全局版本。响应严格核对来源、参与者方向、内容结构、摘要与容量；请求结束后重新核对路径与凭据绑定。页面默认折叠，显式选来源，每 10 秒最多两路并发；隐藏、收起或暂停停止自动读取，手动刷新独立。失败、撤权、路由改变清除该来源旧记录，移除后的迟到响应不能恢复记录。没有浏览器存储、后台协作缓存、离线写入或复制协议。

| 验证 | 最终结果 | 范围与限制 |
| --- | --- | --- |
| Rust 工作区 | 185 通过、4 忽略；20 个目标 | 两项新增摘要/结构单元测试包含其中，默认忽略项未重跑 |
| 原生传输专项 | 4/4 通过 | desktop,custom-protocol；ACK/断开/超时，不是实际窗口 |
| 后端联合回归 | 34/34 通过，无跳过；287.849 秒 | 协作记录 6、任务 12、消息 11、Run 5；真实独立节点与 HTTP/CLI，部分受控来源 |
| Web 界面全量 | 134/134 通过；3.8 分钟 | 最终生成资源与实际 Rust Web；新增界面用可控来源响应验证 |
| Tauri 界面全量 | 51/51 通过；3.5 分钟 | 模拟 IPC；新增无 relay 的本机/远端来源读取、撤权清除、纯文本、窄屏和不存正文 |
| 构建与静态检查 | 通过 | 两端 TypeScript/Vite、生成资源、工作区和原生 debug 构建、fmt、两类 all-targets Clippy -D warnings、actionlint |

新增后端六项包括：messages 不能读第三方记录、activity 单独授权可读但不能投递、GET 只读接口、CLI/Web 结果一致；任务问答与结果、重复读取摘要稳定、不改账本、Run 修订/归档和私有字段排除；错误目标/撤权、损坏账本保留；错误 owner、未知字段、改写内容/摘要及重复 ID 拒绝；同文件凭据轮换或移除路径后的迟到响应拒绝；132 条 8192 字节消息的响应超过通用 1 MiB 限制仍在专用 24 MiB 上限内。三方身份仍同容器，不是三台物理设备。

### 本轮失败与修正

- 新集成初轮 6 项因测试重复授权夹具失败。明确撤销旧授权、使用新输出文件并更新调用方凭据后继续，保留产品拒绝覆盖规则。第二轮 3 通过、3 失败：发现 Config 的非空能力检查漏掉 activity，已修复并在现有权限单元测试中覆盖 activity-only；同时修正测试 JSON 的可省略空字段与 CLI 凭据文件参数。
- Rust 工作区首轮编译暴露两处真实 EasyTier 测试的 Permissions 显式字面量缺少新增字段，逐处补 activity=false 后最终全量与严格检查通过；编译失败不计作测试通过。
- 首次后端联合回归 33 通过、1 错误，282.980 秒：大数据用例通过真实接口准备消息时发生既有 8 秒 HTTP 超时，当时并行运行工作区编译和界面回归。结束并行构建后串行联合回归 34/34；未修改超时或校验规则。串行通过不能证明性能根因已修复，规模/负载稳定性仍需继续观察。
- Web 首次全量 133 通过、1 失败，7.3 分钟：旧 Browser Use 用例写死的权限对象缺少 activity=false，已更新该精确断言；其他消息和原生授权断言同步保留新权限默认关闭检查。首次失败 trace 单独保留，再运行最终全量。

专项 Web 32/32 与新增原生 1/1 已通过，与全量重复的范围不累加。已有 Vite 大包、依赖 use client 提示保留，未新增依赖或放宽 Rust 警告检查。原始失败和重跑日志保留，不覆盖成通过。

### 验收范围与交接

提交前阶段没有新功能的实际 Linux 安装包窗口验收，后续补验见本文末节；系统登录、休眠、Windows/macOS GUI、第二物理设备或真实模型调用仍未验收，历史结论继续绑定各自完整提交。没有升级日常 Web/客户端、重启 KDE/Selkies、操作桌面输入/剪贴板、安装系统 Tailscale 或发布镜像。完整账本复制、Agent 检查点/运行进程迁移仍未实现，后续优先级见[交接入口](implementation-handoff-2026-10-18.md#下一阶段优先级与验收条件)。

27 份本地待提交产品源码、测试、工作流及生成资源与远端实际快照逐项 SHA-256 一致，source-binaries.json 另保存五个 Rust 程序摘要。375/768/1440 像素、明暗主题检查无横向溢出，来源选择行至少 44 像素，另审阅 375/1440 的四份元素截图。元素截图保留固定顶栏覆盖，不代替整页首屏或原生窗口验收。

最终容器 running/healthy、privileged=false、无宿主 Docker socket；3080/3031 匿名 401、认证 200。日常 Web PID 538744、客户端 PID 1403228、程序摘要与 NodeID node-82bf96c6f15d4f00923656f7877b8609 保持；plasmashell 484、kwin_wayland 475、Selkies 306、Xwayland 488 仍在运行，无测试预览、节点、MCP 进程或测试安装包残留。日常程序没有升级。

私有证据 /config/validation-artifacts/federated-activity-20261019 保存各轮 backend 日志、Rust/原生测试和构建/Clippy 日志、两端界面日志、web-first-failure trace、四份截图、source-binaries.json 和 final-health.json。配置、令牌、截图、正文和原始日志不提交。后续接手应按本轮产品提交核对 CI 与安装包，不以旧 87f20ee 的成功矩阵代替。

### 本轮提交与 CI 交接

产品提交 **41b8ca6487e32837b25315e5c12bd06e54763c67** 已推送 origin/main；提交前全部 **33 份文件**与远端 SHA-256 一致，final-source-manifest.json 包含产品、测试、工作流、生成资源和六份文档。之后 CI 补记仅改文档。

2026-10-19 **01:43:21 UTC** 核对相同完整 headSha：矩阵 **39186579637 in_progress**，三平台 Rust、三平台 Tauri 及 Sway/wayvnc 七项均运行中，**尚未通过**；Release **39186579662 completed/success**（README 链接和脚本检查，不代表安装包完成或公开镜像发布）；GitCode **39186579640 completed/success**。三项创建时间均为 2026-10-19T01:42:32Z。私有 ci-handoff.json 保存当时完整作业/步骤；下一位维护者继续核对矩阵终态与同提交安装包，再按交接入口 P0/P1/P2 继续。

### CI 直连授权回归修正

矩阵 39186579637 的 Linux native 作业在 test_web_direct_peers.py 第 683 行失败：旧权限对象精确断言漏掉新增的 activity=false。保留独立权限默认关闭检查，补齐该字段；产品实现、依赖和生成资源不变。指定远端整组直连回归最终 16/16，无跳过，14.825 秒。首轮远端辅助脚本漏配已安装的 /config/bin/herdr，14 通过、2 项缺少 Herdr；设置 CRANIBOX_HERDR_BIN 后重跑通过，不改产品终端行为。失败日志、首轮记录和最终回归结果位于私有 linux-installer-41b8ca6。原矩阵与修正提交的最终结果见下文，不能将原矩阵记为全绿。

## 2026-10-19：协作记录 Linux 发布包原生补验

原矩阵 **39186579637 completed/failure**，最终更新时间 2026-10-19T02:01:41Z：三平台 Tauri、Windows/macOS Rust 与 Sway/wayvnc 六项成功，仅 Linux Rust 的旧权限精确断言失败。**984b7af89c22bc7fa6f105d54fa2df660777831f** 修正该断言并补文档；从产品提交 **41b8ca6487e32837b25315e5c12bd06e54763c67** 起产品源码、生成资源与依赖未变。此节真实包绑定 41b8ca6，不把原失败矩阵当作整体通过，也不把后续测试修正写成重新构建了这两包。

### 包来源

下载原矩阵的 artifact **12158731694**（cranibox-node-desktop-ubuntu-latest），workflow_run.head_sha 为完整 41b8ca6，Linux 客户端作业 **117850162837 success**；API 归档摘要 sha256:e0c598cd0e80ed3cf3f179552da315ea60a1aae4710fffc72a7d49b4c2e3927b，归档 96,409,844 字节。本地和指定远端均核对实际包大小与 SHA-256：

| 包 | 字节 | SHA-256 |
| --- | --- | --- |
| Cranibox Node_0.1.0_amd64.deb | 9,442,210 | 3baa9a4a4a49367e1f6a7da39d67b9abbae10782559d5da2f2223c26195b9854 |
| Cranibox Node_0.1.0_amd64.AppImage | 87,607,800 | 03b04f400b4c40889c657c049309906e07b54a0ad91bee9bc6c34594b0f4ce81 |

.deb 实际 dpkg 安装后的程序 SHA-256 为 377f47481afbdc187b1465d2de1946c68893e41fb1a71f8f9f143efec6e6a6f2；AppImage 使用 --appimage-extract-and-run，内部程序为 6675ff7728301755855dfe9059afedd6597a2bcc0a79576e29afaf05d30b213a。没有验证普通 FUSE 启动。

### 实际窗口与结果

两包均在 admin@192.168.217.6:22022 的 cranibox-validation、abc、现有 KDE 中运行，真实发布窗口、IPC 和 Rust 节点宿主。分别使用新 XDG/NodeID 与私有 D-Bus，目标和第三方发送者是同容器独立身份的真实 Rust 节点，经回环 HTTP；没有使用模拟 IPC、Web 页面或网关替代原生动作。

两包都从“协作任务”进入“查看协作记录”，取消本机并选远端来源，读取任务进度、问题、回答和结果，以及第三方发送的设备消息。消息中的类 HTML 内容按文字显示。类型选“消息”后实际活动面板为一条匹配且无任务行；面板外原有本机任务列表仍保留。随后目标撤销调用方授权，点击“刷新协作记录”，确认该来源的任务和消息正文均清除，界面显示零条匹配和授权说明。任务报告由脚本 worker 通过真实接口提交，不代表模型或实际 Agent 进程执行。

.deb 的两次原生搜索输入尝试均失败：WebKit search entry 未提供 org.a11y.atspi.EditableText，首次按接口过滤找不到对象，第二次直接调用也返回 No such interface。因此搜索不计原生通过；未操作全局键盘/剪贴板绕过。浏览器 134/134 与模拟 IPC 51/51 的搜索覆盖仍成立。截图审阅可见类型筛选后的消息和纯文本正文，但原生 select 弹出菜单仍未关闭；实际 React 列表已按内容断言，不把截图当作普通鼠标/键盘关闭菜单、移动端或完整首屏验收。

本次未重验原生 Run 详情/分页、托盘/自启、真实登录、休眠、普通 FUSE、桌面控制/剪贴板、物理节点网络、Windows/macOS GUI。对应历史版本验收和后续 P0/P1/P2 保持边界。

### 失败保留与清理

最初两次 .deb 数据准备失败都发生在 GUI 前：第一次在问题同步前提交回答，第二次等待辅助脚本过早访问可省略的 questions 字段。改为等待真实报告、用可选字段读取后通过，失败身份与日志保留，所属节点进程均已停止。没有修改产品或手改账本。原生搜索的失败动作另在 deb-records.json、deb-filters.json 中保留；通过动作分别保存在 deb-third.json、deb-type-filter.json、appimage-exercise.json，acceptance.json 不将失败计入通过。

两包及独立目标、发送者、启动器和所属辅助进程已退出；按程序摘要核对后卸载测试 cranibox-node 包，按来源与摘要移除本次 AppImage 解压目录。归属审计 owned_processes=[]、test_package_absent=true。日常 Web PID **538744**、客户端 PID **1403228**、各自程序摘要及 NodeID **node-82bf96c6f15d4f00923656f7877b8609** 保持；plasmashell 484、kwin_wayland 475、Selkies 306、Xwayland 488 未重启。容器 running/healthy、privileged=false、无宿主 Docker socket；3080/3031 匿名 401、认证 200。没有升级日常程序或发布公开镜像。

私有证据 **/config/validation-artifacts/linux-installer-41b8ca6** 保存 artifact.json、package-digests.json、acceptance.json、两包阶段 JSON/失败、界面树/截图、CI 原始日志、直连回归首轮/最终结果、cleanup-audit.json 和 final-health.json。配置、令牌、正文、截图、辅助脚本和原始日志不提交 Git；仓库只保留结论、版本、摘要、失败解释与后续入口。

### 修正提交的最终 CI

完整来源 **984b7af89c22bc7fa6f105d54fa2df660777831f**，矩阵 **39187721392 completed/success，7/7**：三个平台 Rust、三个平台 Tauri 与独立 Sway/wayvnc；创建于 2026-10-19T01:57:56Z，最终更新于 **2026-10-19T02:25:01Z**。相同来源的 Release **39187721472**、GitCode **39187721368** 均 completed/success；Release 不发布公开空间镜像。原矩阵失败和进行中快照保留，未覆盖为成功。

下载最终 Linux Rust、Linux 客户端、Wayland 三份作业日志后确认：

| 检查 | 最终 CI 结果 | 范围 |
| --- | --- | --- |
| Rust 工作区 | 185 通过、4 默认忽略 | 默认忽略项不计入 185；下列专项显式执行 |
| Web 界面全量 | 134/134，3.6 分钟 | 实际 Rust Web；新界面部分来源可控 |
| 客户端界面全量 | 51/51，32.9 秒 | 模拟 IPC，不是 GUI 包验收 |
| 直连授权回归 | 16/16，8.889 秒 | 修正旧权限精确断言的完整回归 |
| 新协作记录后端 | 6/6，108.945 秒 | 实际独立 Rust 节点与 HTTP/CLI，部分受控来源 |
| 原生传输、GIO | 4/4、1/1 | 真实 GIO 启动器仍不等于系统登录 |
| EasyTier 用户态组网 | 2/2，16.77 秒 | CI 内真实软件/节点、重启和入口切换，非物理网络 |
| 真实浏览器、X11 VNC、Wayland | 各 1/1 | Chromium/官方 MCP、像素/认证/输入与恢复，各自隔离场景 |
| 资源测量 | 通过、报告已上传 | 显式执行默认忽略的传输测量；不代表规模或长时耐久性 |

其他既有 Linux 服务、目录/文件复制、后台同步、任务/worker/依赖、Computer/Browser Use、桌面/空间集成也均通过；不与远端已有结果累计。Windows/macOS 原生构建和打包成功不代表 GUI、系统权限或实际多物理设备验收。

最终 API 共 **10 份 artifact**：三平台 cranibox 服务端、cranibox-node CLI、cranibox-node-desktop 安装包及 Linux 资源报告。新 Linux 桌面 artifact **12159265408**，未将本节实际使用的旧产品包 **12158731694** 改写为新包。私有 linux-installer-41b8ca6/ci-final.json、ci-correction-artifacts.json、ci-test-summary.json 和三份 ci-correction-*.log 保留完整来源、结果与测试摘要，ci-followup.json 仍保留此前进行中状态；后续仅补交接文档。

## 2026-10-19：Windows/macOS 桌面准备向导收尾

本轮基于 **336df91baed4d6a89b4cc5de8b71f36f8efafde9**，新增 Web/Tauri 共用的三步准备向导：官方说明、回环 RFB 检查、配置核对，再由原有表单明确保存并开启共享。Windows 引导 UltraVNC Server，macOS 引导系统屏幕共享；Tauri 只接受固定文档标识，由 Rust 打开对应官方地址。没有引入新依赖，也没有自动安装提供者、修改系统监听/防火墙或代替用户授予权限。

全部构建、浏览器及运行测试位于指定远端 admin@192.168.217.6:22022 的 cranibox-validation、abc 用户。测试使用隔离配置、端口和进程；本机仅编辑、静态审阅、文件传输和 Git 操作。Web/Tauri 构建、Web 生成资源、cranibox 与原生 debug 构建、fmt 及原生 desktop,custom-protocol all-targets Clippy -D warnings 通过。既有 Vite 大包和依赖 use client 提示保留，未放宽检查。

| 检查 | 最终结果 | 范围 |
| --- | --- | --- |
| Web 全量 | **137/137，229.045 秒** | 真实 Rust Web；新向导仅模拟平台状态，RFB 检查使用实际回环夹具 |
| Tauri 界面全量 | **53/53，117.344 秒** | 模拟 IPC，不是 Windows/macOS GUI 或系统打开链接验收 |
| 桌面服务后端 | **11/11，无跳过，17.023 秒** | 真实 Rust 节点、隔离 X11/wayvnc 夹具 |
| 原生传输专项 | **4/4** | desktop,custom-protocol，未打开桌面查看器 |

新增 Web 专项 3/3、客户端专项 2/2 另已通过，范围包含在上述全量，不累加。向导覆盖 IPv4/IPv6 回环地址校验、无效地址不发请求、连接失败保留输入、重试、异步离页丢弃、文档打开错误、核对不启用与保存前再次检查、启用后授权入口。只读取 RFB greeting，不代表 VNC 认证、画面或输入已验证；没有保存 VNC 密码。

### 失败与界面检查

首轮 Web 全量 **136 通过、1 失败，296.436 秒**：旧平台用例仍查找替换前的 Windows/macOS 提示文案。将断言改为新向导标题，同时保留 Linux Wayland 提示和手动配置断言；最终全量 137/137。首轮日志和 web-initial-test-results 的 trace 单独保留，没有被重跑清除。首次元素截图上方被固定顶栏覆盖，修正截图前滚动位置后重新生成；该问题来自截取区域，不作为产品通过证据。

检查 375×1000、812×375、1440×1000 与明暗主题无横向溢出，文档操作高度至少 44 像素；覆盖 reduced-motion 与步骤焦点。保留两平台 × 375/1440 × 明暗主题的八份截图；人工审阅窄屏 Windows 亮色、最终 macOS 桌面暗色截图，后者确认顶栏不再遮挡。它们是 Linux 浏览器中的平台夹具，不能作为实机 OS 验收。

### 版本、清理与后续

本轮未重新运行全部 Rust 工作区，也没有重装 Linux 发布包；前节的 185 项、7/7 CI 与安装包结论继续绑定各自旧提交。新提交 CI 必须核对新的完整 headSha。Windows/macOS 提供者自动安装和托管、实际桌面权限/认证/画面/输入、自启/休眠，以及跨物理设备仍待验收；WSL2 不代表原生 Windows。原始需求、实现边界与 P0/P1/P2 在[交接入口](implementation-handoff-2026-10-18.md)汇总。

最终远端容器 running/healthy、privileged=false、无宿主 Docker socket；3080/3031 匿名 401、认证 200。日常 Web PID **538744**、客户端 PID **1403228**、原程序 SHA-256 与 NodeID **node-82bf96c6f15d4f00923656f7877b8609** 均保持；plasmashell 484、kwin_wayland 475、Selkies 306、Xwayland 488 未重启。owned_processes=[]、test_package_absent=true；没有升级日常程序或操作全局键盘/剪贴板。

私有证据 **/config/validation-artifacts/desktop-platform-guide-20261019** 保存各轮测试/构建日志、结果 JSON、首次失败 trace、八份平台截图、最终文件与四个程序 SHA-256 清单 final-source-manifest.json，以及 final-health.json。源码、测试、生成资源和交接文档在提交前与远端快照逐项核对；配置、凭据、截图、辅助脚本和原始日志不提交 Git。

## 2026-10-19：多架构镜像验收门槛

基于 **f49341d107e64309f4e04d16c3acef0ff95f2e73** 补齐镜像工作流：仅验证模式默认分别使用 amd64/arm64 原生 runner，可选择单架构；发布模式始终检查两个架构，阿里云镜像独立执行相同门槛。检查器显式选择不可变 index 中唯一匹配的 manifest，按摘要拉取并核对实际架构；预置与正式测试容器均明确指定平台。任一架构失败时，该作业不更新公开标签。arm64 发布验收在 QEMU 下执行，不等于原生设备验收。

在指定远端容器执行检查器回归 **22/22，0.041 秒**，actionlint 通过。新增八项覆盖错误架构拒绝、arm64 匹配、manifest 精确选择、缺失/重复/损坏 index 拒绝、显式平台/不可变摘要要求、默认不联网及预置/正式容器的平台参数。回归模拟 Docker，不能作为真实 arm64 或仓库拉取结果。

同一远端宿主机执行实际 Docker 验收，镜像 **cranibox-image-release:18c3216-xfce**，完整来源 **18c321654dcc21cdb8c9eb1c5686f7ed73d4d6c4**，实际 linux/amd64。先请求 linux/arm64，按预期退出 1，在 requested-platform 检查拒绝，未创建任何容器/卷。随后请求 linux/amd64，**44/44，93.121 秒**：空配置和预置授权的首次启动、节点执行、令牌拒绝、重启及身份/文件保留均通过。新增平台检查使原来的 43 项变为 44 项；不是新增一轮产品功能全量。两个容器和两个命名卷均按归属标签成功删除。

本轮未修改 Rust/React、嵌入资源或依赖，没有重跑旧产品全量或安装包。真实 arm64 的完整镜像构建/启动、新产品镜像、registry 拉取与正式发布仍未验收；不能将旧镜像通过改写为当前提交发布。默认公开镜像不兼容问题保留在 P0。证据统一保存于 **/config/validation-artifacts/image-platform-gate-20261019**；凭据、原始日志、报告和辅助脚本不提交 Git。

最终六份待提交文件与远端快照 SHA-256 一致，保存在 final-source-manifest.json。final-health.json 确认日常 Web PID 538744、客户端 PID 1403228、原程序摘要及 NodeID 保持；KDE/Selkies 未重启。容器 running/healthy、非 privileged、无宿主 Docker socket，3080/3031 匿名 401、认证 200，owned_processes=[]、test_package_absent=true。只清理本轮带归属标签的验收容器与卷，没有升级日常程序。

### 交接时补记：产品 CI 最终成功，镜像验证仍排队

Windows/macOS 桌面准备向导的完整产品提交 **f49341d107e64309f4e04d16c3acef0ff95f2e73**，矩阵 **39192461791** 已 completed/success，**7/7**；创建于 **2026-10-19T02:58:37Z**，最终更新于 **03:20:50Z**。三个平台 Rust、三个平台 Tauri 及独立 Sway/wayvnc 全部成功。同来源 Release **39192461685** 与 GitCode **39192461704** 成功；Release 只是脚本检查，不发布公开产品镜像。

核对最终 Linux Rust、Linux Tauri 和 Wayland 作业日志：Rust 工作区 **185 通过、4 默认忽略**；Web **137/137，2.8 分钟**；Tauri 模拟 IPC **53/53，33.9 秒**；原生传输 **4/4**、GIO **1/1**；真实 EasyTier **2/2，16.22 秒**；真实 Chromium/官方 MCP、X11 VNC、Sway/wayvnc 各 **1/1**，资源测量与既有 Linux 服务、目录/文件、后台同步、任务和桌面/空间集成均成功。显式专项不与默认工作区或远端轮次重复累计；Windows/macOS 构建成功不代表系统 GUI、权限或多物理设备验收。

API 确认 **10 份 artifact**，包含三平台服务端、独立节点 CLI、桌面安装包及 Linux 资源报告。最新 Linux 桌面包 artifact **12161225440**，未安装到日常环境；此前真实窗口和剪贴板结果继续绑定文内各自旧提交。Actions 中打开运行 **39192461791** 即可下载本次 Windows NSIS、macOS DMG、Linux DEB/AppImage。

多架构门槛提交 **8566f59998b4b6884f95ed82be58d95e9de0faf5** 已推送 origin/main。手动运行 **39193919686** 参数 desktop=xfce、architecture=all、publish=false；**2026-10-19T03:21:17Z** 的最后快照为 queued，amd64/arm64 两作业均未执行。同来源 Release **39193888500** 和 GitCode **39193888594** 也 queued。仅触发不计通过，未使用发布模式或替换公开标签；下一位维护者继续核对当前运行终态。

私有 **/config/validation-artifacts/image-platform-gate-20261019** 保存 product-ci-final.json、product-artifacts.json、linux-native.log、linux-desktop.log、wayland.log 与 ci-handoff.json；原始失败和此前进行中证据保持。最后收尾只更新交接和验证文档，未重跑产品测试、安装发布包或修改日常服务。功能状态和剩余 P0/P1/P2 仍以[交接入口](implementation-handoff-2026-10-18.md)为准。

## 2026-10-19：VNC 安装生命周期与最终交接

基于 **37718debd6e1881bbb6977e00a8e624bc0f1ae89** 修复 Linux 安装生命周期：此前包安装使用 kill_on_drop，节点退出或三十分钟监控超时可能主动终止安装子进程，且重启后没有状态记录。现在安装进程关闭 kill_on_drop，监控与系统事务分开；执行前私有写入 desktop-setup.json，写失败不执行；上次运行中、记录损坏或超过 8 KiB 时仅显示中断待确认，不自动重放。系统关机、服务管理器和权限代理仍可能影响事务，不能保证安装完成。

安装要求先查看方案并提交当前 plan_id，绑定提供者、有效期十分钟、只能使用一次；替换方案、重启或过期后重新准备。Web/Tauri 共用安装核对、中断说明、失败清除旧方案与焦点恢复；安装与启动共享始终分开。新前端遇到旧节点缺少方案 ID 时提示更新。协议和私有记录边界已补入[远程桌面文档](remote-desktops.md)。

所有构建、socket、浏览器和运行测试均在指定远端 admin@192.168.217.6:22022、cranibox-validation、abc 用户执行；本机仅编辑、静态审阅、传输与 Git。未执行实际 pkexec/APT，不安装系统包，不使用当前登录桌面输入。

| 检查 | 最终结果 | 范围 |
| --- | --- | --- |
| Rust 工作区 | **184 通过、3 默认忽略，126.077 秒** | cargo test --offline --workspace --exclude cranibox-node-desktop，18 个目标；不包含 Tauri Rust 测试，忽略项不计通过 |
| VNC Rust 专项 | **7/7，51.666 秒含编译** | 包含在工作区；新四项覆盖私有记录、一次性方案、失败恢复和独立 shell 事务，不累加 |
| 桌面服务后端 | **12/12，无跳过，29.692 秒** | 真实 Rust Web/节点，隔离 X11/Wayland 提供者；新重启/坏记录用例证明不重放、不开启共享或写入凭据 |
| Web 桌面专项 | **18/18，109.066 秒** | 真实 Rust Web、RFB/JPEG 与可控安装响应；过期后重新准备、正确 ID、焦点恢复及不自动启动 |
| Tauri 界面全量 | **54/54，139.032 秒** | 模拟 IPC；新增中断恢复和当前方案 ID 提交，非原生窗口/系统安装验收 |
| Web / 客户端前端构建 | **通过，10.195 / 10.034 秒** | TypeScript 与 Vite；Web 静态资源由远端生成并取回提交，已知 use client 和大包提示未放宽 |
| Rust 四程序构建 | **通过，37.334 秒** | cranibox、cranibox-node、workspace-runtime、workspace-web，独立目标目录 |
| fmt / 原生 Clippy | **通过，1.400 / 12.283 秒** | fmt --all --check；desktop,custom-protocol --all-targets -D warnings |

初次提交前未重跑全部 Web 界面和其余 Python 集成；提交后的 Web 全量补验见下节。历史完整矩阵继续绑定 f49341d，不能改写成本次新提交已通过跨平台 CI。Windows/macOS 实机、真实系统安装认证/取消、物理多设备和公开兼容镜像仍待验收。镜像只验证运行 **39193919686** 在此前查询时 queued，后续运行状态见下节，未重复触发、发布或替换公开标签。

布局检查覆盖 375/768/1440、明暗主题无横向溢出；六份面板截图由浏览器在远端生成。人工审阅 375 亮色与 1440 暗色，恢复说明、命令和确认操作可读；元素截图顶部仍受固定顶栏遮盖，不能作为完整首屏或真实手机验收。没有以截图裁剪问题计作产品通过。

私有 **/config/validation-artifacts/vnc-installation-lifecycle-20261019** 保存每条命令的结果、耗时、源码 SHA-256、完整日志、布局截图及 final-health.json；初始构建记录的测试文件与后续记录不同，是补入原生 IPC 用例和保留不支持平台的安装请求校验，产品源码摘要一致。功能需求和剩余 P0/P1/P2 统一在[交接入口](implementation-handoff-2026-10-18.md)。配置、凭据、截图、原始日志与辅助脚本不入 Git。

收尾健康核对：容器 running/healthy、privileged=false、无宿主 Docker socket；3080/3031 匿名 401、认证 200。日常 Web PID **538744**、客户端 PID **1403228**、程序摘要与 NodeID **node-82bf96c6f15d4f00923656f7877b8609** 保持；plasmashell 484、kwin_wayland 475、Selkies 306、Xwayland 488 未重启。owned_processes=[]、test_package_absent=true，无所属浏览器或服务残留，未升级日常程序。

### 55fbe2a 提交后：完整 Web 回归与 CI 补验

产品提交 **55fbe2a74eb61e20eb8050992a99e9f34aaaaa87** 已推送 origin/main。指定远端容器的同一源码快照补验完整 Web 界面：在 addons/control-center/ui 执行 `npm test`，**138/138，231.891 秒，退出 0**。没有修改产品/测试、放宽期限或重跑失败。包含设备授权、项目/Agent 目录、消息/任务/协作记录、后台策略、浮动终端和重连、桌面/剪贴板、空间引导、导航焦点及中英文响应式页面；18 项桌面专项也在这 138 项内，不重复累计。模拟提供者与真实 Rust 服务的边界不变，不能代替跨物理设备或 Windows/macOS GUI 验收。

原始日志和结果位于私有 **/config/validation-artifacts/vnc-installation-lifecycle-20261019/web-all.{log,json}**。结果中的 source_base 是补丁基线 37718de；六项关键源码摘要与 55fbe2a 一致。补验后单独核对容器、3080/3031、日常 Web/客户端 PID、程序摘要、NodeID 与 KDE/Selkies，均保持；owned_processes=[]、test_package_absent=true，保存为同目录 **web-all-followup/final-health.json**，没有覆盖首次收尾健康记录。

**2026-10-19 03:57 UTC** 的 GitHub API 快照：55fbe2a 的三平台矩阵 **39196468666**、Release **39196468706** 与 GitCode **39196468659** 已触发，矩阵仍 queued。旧镜像门槛提交 8566f59 的 **39193919686** 已 in_progress：amd64 构建完成，正在首次启动/授权/重启验收；arm64 正在构建。未记录为成功，也未重复触发或更新公开标签。后续直接核对现有运行及其完整 headSha。

随后镜像运行 **39193919686** 于 **2026-10-19T03:58:35Z** 更新为 completed/success，完整 headSha **8566f59998b4b6884f95ed82be58d95e9de0faf5**；参数 desktop=xfce、architecture=all、publish=false。amd64 作业 **117872437694** 与 arm64 作业 **117872437850** 在原生 runner 上构建完整镜像并分别通过 **44/44** 启动验收，检查耗时 **67.995 / 63.271 秒**。已实际下载两个 artifact（**12162630233 / 12162455868**），核对报告的 expected_revision/revision、expected_platform/platform、所有检查通过、unready_checks=[]、四项所属容器/卷全部 removed=true。

验收覆盖空配置与预置空间身份的首次启动和重启、Rust/Web/静态资源、服务与私有凭据、桌面登录保护、节点授权和错误令牌、真实命令执行及持久数据。amd64 镜像 ID **sha256:d8db47e287689a42cb238736a4bf6f9e1776ed5ba49168cad628d87bcb2acfa3**；arm64 为 **sha256:63e5881e5e8b4d09639d73103ca06760333f0cda74301806b020c48cfa3fb0c7**。Docker Hub 登录、标签提升和 Aliyun 均跳过，没有发布或改变默认公开镜像；不能计作仓库拉取、实际桌面输入、完整空间 UI 或 55fbe2a 新 VNC 修复的镜像验收。

两份启动报告和完整运行元数据/核对摘要保存在私有 **/config/validation-artifacts/image-platform-gate-20261019/native-matrix-final-39193919686**，不覆盖此前队列记录。本轮收尾查询 55fbe2a 的三平台矩阵、Release 与 GitCode 仍 queued，因此其最终结果继续留待核对，没有沿用旧来源的成功结论。

## 2026-10-19：虚拟空间安装生命周期与交接整理

基于 **d5beb23bb9eb7a0e31adfa2845c74a91a8841b22** 修复虚拟空间的运行环境安装：此前与普通 Docker 命令共用 kill_on_drop 进程管理，节点退出或超时可能终止安装，确认也未绑定经过审阅的方案。现在安装进程与监控分离、使用空输入/输出流，避免父进程退出导致管道断开；WSL 安装脚本随命令参数传入，不依赖节点 stdin。普通 Docker 操作仍保持原有超时清理。

安装方案私有持久化，只保留当前一份，绑定操作/命令并在十分钟后过期；执行前复查，原子保存消耗方案与安装中状态，再启动系统命令。分开的 CLI 调用可使用同一未过期方案，已使用方案重启后也不能重放。旧调用方没有 plan_id 时明确拒绝，不执行安装。Web/Tauri 共用连续点击保护、提交后清除旧确认、响应丢失后读取状态、错误/核对/取消焦点以及中断提示。安装结束需重新检测，不自动创建空间。协议、兼容性和 CLI 示例见[虚拟空间](virtual-spaces.md#安装确认与中断恢复)。

全部构建、socket、浏览器与运行测试仍在指定远端 admin@192.168.217.6:22022、cranibox-validation、abc 用户执行；本机只编辑、静态审阅、传输与 Git。系统包安装、权限确认、Windows/macOS 原生安装器均未实际运行，未升级日常程序。

| 检查 | 最终结果 | 范围 |
| --- | --- | --- |
| Rust 工作区 | **187 通过、3 默认忽略，169.390 秒** | cargo test --offline --workspace --exclude cranibox-node-desktop；18 个目标，未运行 Tauri Rust 测试 |
| Rust 空间专项 | **10/10，43.267 秒含编译** | 包含在工作区，不累加；新三项验证方案与重启、超时/取消后独立 shell 继续 |
| 空间后端集成 | **9/9，无跳过，58.917 秒** | python3 -m unittest discover -s tests -p test_virtual_spaces.py -v；真实 Rust 服务、受限 Docker 夹具，不连宿主 Docker |
| Web 界面全量 | **140/140，372.595 秒** | npm test，addons/control-center/ui；真实 Rust Web，新安装场景为可控响应 |
| Tauri 界面全量 | **55/55，190.777 秒** | npm test -- --workers=1，apps/node-desktop；模拟 IPC，非真实系统 GUI |
| 两端前端构建 | **通过，18.694 / 18.302 秒** | Web 生成资源与原生 TypeScript/Vite；既有 use client 和大包提示保留 |
| Rust 四程序构建 | **通过，55.608 秒** | cranibox、cranibox-node、workspace-runtime、workspace-web |
| fmt / 原生 Clippy | **通过，3.129 / 15.561 秒** | cargo fmt --all -- --check；desktop,custom-protocol --all-targets -D warnings |

首轮 Rust 专项编译未通过，因为应用测试 Catalog 夹具遗漏新字段；补齐后专项及工作区通过。首轮客户端界面为 **54 通过、1 失败，89.765 秒**，同区域另有设备目录错误，新增测试的 alert 定位不唯一；只将测试定位限定为安装错误，最终 55/55。初始 rust-unit、native-ui 日志与最终 rust-unit-final、native-ui-final 分开保留，没有放宽超时或删除失败记录。前端构建时测试文件尚未限定 alert，产品源码与生成资源未因此变化。

Web 新两项包含在 140 项内，覆盖过期后重新核对、一次事件循环内重复点击、已接受但响应丢失、重载不重放、安装中限制操作及中断后重新检查。布局检查 375/768/1440、明暗主题、reduced-motion、无横向溢出与确认/取消至少 44 像素；六份完整页面截图及摘要保存在私有证据目录。人工审阅 375 亮色与 1440 暗色，顶栏未遮挡正文，命令换行、核对与取消操作可读；不是手机实机验收，也不替代上一轮 VNC 完整首屏待验范围。

节点退出或监控超时不主动终止已启动的安装器，不保证系统事务完成；关机、服务管理器、权限代理仍可中断，多步安装也可能尚未执行后续命令。当前不保存系统安装输出，只报告启动错误、退出码和超时；应检查系统安装器再重新检测环境。真实三平台依赖安装、Windows/macOS 桌面提供者/权限、跨物理设备以及公开兼容镜像发布仍未完成，按[需求与 P0/P1/P2](implementation-handoff-2026-10-18.md)继续，不把本轮交接当作全部原始需求实现。

**2026-10-19 04:28 UTC** 查询：55fbe2a 的矩阵 **39196468666** 总状态 queued，独立 Wayland 成功、Linux Tauri 运行中、其他五项 queued。d5beb23 的 XFCE 双架构只验证运行 **39197807627** 两项 queued，publish=false；它含 VNC 修复，不含本轮空间修复。未重复触发旧运行、发布镜像或改变公开标签。当前提交推送后须核对新 CI 的完整 headSha，旧 8566f59 镜像的 44/44 或 f49341d 的三平台成功不能沿用为新版本通过。

最终健康检查：容器 running/healthy、非 privileged、无宿主 Docker socket，3080/3031 匿名 401、认证 200。日常 Web PID **538744**、客户端 PID **1403228**、程序摘要及 NodeID 保持；plasmashell 484、kwin_wayland 475、Selkies 306、Xwayland 488 未重启。owned_processes=[]、test_package_absent=true，没有所属浏览器/服务残留，也未操作全局键鼠或剪贴板。

私有证据 **/config/validation-artifacts/space-setup-lifecycle-20261019** 保存各轮命令、耗时、日志、源码摘要、六份截图和 final-health.json；最终待提交源码、测试、生成资源及文档与远端快照逐项核对，清单为 final-source-manifest.json。配置、凭据、截图、原始日志与辅助脚本不入 Git。
