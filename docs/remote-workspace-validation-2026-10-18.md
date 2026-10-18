# Linux 远端 Workspace 验证环境

本次按用户要求使用 `admin@192.168.217.6:22022`，在远端 Docker 中启动独立验证容器，不使用本机图形桌面验证客户端。

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

证据位于远端容器 /config/validation-artifacts/ci-gates-__f09ral/ci-gates-validation.json 与同目录私有日志。仅进行了构建检查及 scratch 上下文验证，没有完整构建产品镜像、创建新产品实例或改变部署。

### 真实 EasyTier 测试的授权恢复

首次矩阵的 Linux 作业 117578716627 在真实 EasyTier 测试处出现 1 项通过、1 项失败。两节点测试先保存错误令牌，验证探测被拒绝，随后错误地通过 easy_tier_connect 直接换成有效令牌。多路径功能已要求新增路径使用同一个目标签发令牌，不能隐式替换现有授权，因而被正常拒绝。三节点入口故障切换测试已通过。

本次仅更新两节点测试的恢复步骤：断言替换令牌被拒绝、原错误连接仍不能探测，然后通过公开 Forget 接口删除错误连接，再使用有效令牌重新连接。没有放宽生产授权检查，也没有删去错误凭据的反向测试。

远端容器下载官方 EasyTier v2.6.4 Linux x86_64 归档，验证 SHA-256 为 61b659eaedba658fa66fe47d17e1426cdd77e5d02fa15fed447bb4357c09dfd6，仅解出两个二进制至 /config/validation-tools/easytier-2.6.4。隔离测试使用独立临时配置与 NodeID，未变更主节点网络：

- 全工作区 cargo fmt --all --check 通过。
- cranibox-node 的所有 target 严格 Clippy（-D warnings）通过。
- 两项真实 EasyTier 测试均通过，0 失败；覆盖错误授权、显式恢复、远程命令、持久 PTY、节点重启、入口故障及备用入口接续。测试执行约 17 秒，不含编译。

命令与 CI 相同：CRANIBOX_TEST_EASYTIER_DIR 指向上述目录，执行 cargo test --locked -p cranibox-node network::tests::real_ -- --ignored --nocapture --test-threads=1。证据在 /config/validation-artifacts/ci-easytier-d_4dykd5/result.json、clippy.log 与 real-easytier.log。它们是同一远端容器内的真实独立网络进程，不替代跨物理设备 NAT、真实 Tailnet 或休眠恢复验收。
