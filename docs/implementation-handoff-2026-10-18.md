# Cranibox 功能、验证与后续交接

整理日期：2026-10-18。本文是下一位维护者的接手入口，汇总本次连续开发的最终状态。设计文档描述目标，专项验证记录按发生顺序保留历史；判断目前完成范围时优先看本文及对应功能文档。

## 用户已经确定的方向

- 产品用于维护者自己的多设备日常工作：从任意入口看到授权设备、项目与 Agent，发送任务、回复问题、接续工作。
- Windows、macOS、Linux 与 Docker Workspace 是同等节点；节点独立运行，不要求存在中心服务器或选主。Workspace 可以兼任长期在线的 Web 入口与可选中继。
- 服务端使用 Rust，浏览器使用 React。独立客户端使用 Rust 节点引擎与 Tauri；不把客户端合并成服务端，也不在客户端打包 Herdr。
- 保留浏览器端；Tauri 提供托盘、自启动与本机系统接入，两者尽量共用业务组件。
- 自部署网络优先 EasyTier，同时可使用已有 Tailscale。网络可达和能力授权必须分开；希望用地址与短期 Token 引导接入。
- 客户端优先支持授权桌面互访，采用 VNC/noVNC，Linux 容器继续可用 Selkies。没有引入完整 RustDesk。
- 虚拟空间是支持 Docker 生态的环境中运行的完整 Workspace 容器，提供安装引导、创建与生命周期管理。
- 项目和 Agent 接续必须区分目录记录、文件复制、任务上下文及真实运行进程，不能把文件复制称为运行会话迁移。
- 只将项目提供或集成的五项服务切到 Rust 管理入口；第三方业务本身及用户自定义服务不要求重写。

## 功能需求与最终实现状态

“已实现”表示代码存在并有相应验证，不表示所有平台与生产场景均已验收。

| 需求 | 当前实现 | 边界与后续 |
| --- | --- | --- |
| Rust 控制工具与常驻服务管理 | Rust workspacectl；Rust 服务兼容入口配合 s6 监督；五项项目服务已有范围受限的切换记录 | 第三方业务未改写；init/custom-services/systemctl-services 使用短时 oneshot；服务切换记录来自此前开发容器，不等于所有部署实例已迁移 |
| 合并服务端应用、独立发布客户端 | cranibox 合并 Web、节点网关与本机节点；保留独立 cranibox-node 与 Tauri 应用，兼容原入口 | 一份身份配置只允许一个节点宿主进程；GUI 程序兼容 CLI/MCP 子命令 |
| WebSocket 远程命令与 Agent 接入 | 节点出站连接、组授权、CLI/MCP、网关转发、重新连接 | 网络重连不重放普通命令；网关路径是可选通道 |
| 无中心的节点互访 | 持久 NodeID、目标签发凭据、独立 HTTP/WebSocket、能力分别授权；Workspace 停止后仍可直连 | 当前是随机 NodeID 与 Bearer 授权，尚无公钥配对、签名目录或 mTLS |
| EasyTier 与 Tailscale 同时使用 | EasyTier 固定版本安装、用户态网络、节点发现、端口转发、多个入网地址、短期邀请与丢响应恢复；系统 Tailscale 状态、官方登录、持续发现与授权连接 | Tailscale 依赖系统已有客户端；没有实现其安装与系统服务托管，也未完成真实 Tailnet 跨设备验收 |
| 同一设备多条路径 | 保存并排序显式路径，新连接按可用性选择，认证/身份错误不继续回退 | 已建立连接不无缝迁移；桌面直连与网关通道仍显式选择 |
| 浏览器和原生端统一体验 | 项目、设备、网络、消息、任务、终端、VNC 与空间共用 React 组件，分别适配 Web 和 Tauri IPC | 不是全部管理页面完全一致，也不是节点数据自动全量复制 |
| 常驻托盘与登录自启动 | Tauri 窗口关闭到托盘、重连、明确退出、用户登录后自启动配置 | Linux 实机确认窗口隐藏、进程保留及托盘注册；完整托盘菜单、自启动、休眠恢复及 Windows/macOS 仍待实测 |
| 持久终端与浮动窗口 | 可变大小、最小化到状态栏、多个会话、重新附着；服务端使用独立 Herdr，客户端使用原生 PTY | 页面关闭/网络断开后保留节点内 Shell；客户端进程或系统退出后不能保留该 Shell |
| 桌面互访 | 共用 noVNC；本机、直连及出站 WebSocket 中继；目标桌面单独授权、单控制者、停止与撤权取消 | 已有 VNC 可接入多平台；Windows/macOS 及 KDE/GNOME 的自动安装与托管未实现 |
| 内置 VNC 托管 | Linux X11 使用 x11vnc，兼容 wlroots 的 Wayland 使用 wayvnc；守护、失败退避、节点重启恢复 | 不是一个纯 Rust 跨平台捕获实现；KDE 容器实测屏幕捕获失败，Selkies 可用，见下文 |
| 虚拟空间 | 检测、安装计划、明确执行、镜像策略、创建/停止/启动/归档/恢复、资源限制、独立身份与持久卷；桌面/编辑器独立应用入口 | 真实 Docker 生命周期与浏览器流程已测；完整 Linux amd64 KDE 镜像的干净构建、空配置首次启动及重启保留数据已通过，公开产品镜像拉取与各 OS 系统依赖安装仍未验收 |
| 统一项目与会话目录 | ProjectID、节点工作副本、Shell 关联、网关/直连汇总；增量目录读取、删除处理、只读离线快照 | 各节点保留本地权威；不是全网数据库复制，不支持离线修改后自动合并 |
| 关闭页面后的目录同步 | 本轮新增 Rust 节点后台同步，明确选择 1–32 个已接入设备，15/30/60/300 秒，最多四路并发；暂停和重启恢复 | 默认关闭；只同步项目目录和会话关联，不复制工作文件；凭据/连接变更后需要重新选择 |
| 项目迁移与接续 | 本机与直连双向、两个远端之间快照复制；清单确认、分块校验、暂停/恢复、持久接收回执；保留 ProjectID 与接续说明 | 当前是显式快照复制，非持续文件同步；上限 2 GiB/单文件 1 GiB，实际集成传输约 20 MiB，未做上限压力测试 |
| 多 Agent 通信与任务 | 持久消息、线程回复、确认、离线投递、任务委托/领取/进度/问题/回复/结果/合作取消，CLI/MCP 与共享界面 | 不能由普通消息自动启动进程；同一容器中的独立身份验证不等于多台物理设备验收 |
| Agent 会话与后台执行 | 主动登记/续期、汇总授权会话、定向委托；Codex/Claude Code 非交互适配、启动检查、持久输出与停止；可选独立执行 MCP | 模型调用与 CLI 登录未实测；不支持运行中追加交互输入、模型检查点恢复或进程迁移 |
| 多设备协作与自动接单 | 持久协作 Run、任务汇总、统一停止、核对归档、最多八项前置任务；接收端可明确启用按可信发送方和固定目录的后台 worker | 默认关闭；不是通用复杂编排平台；人核对模式与显式自动提案模式分开，退出码不直接等于任务成功 |
| 各节点 Agent 共享 Computer Use / Browser Use | 可通过授权命令、终端与桌面使用远端能力，容器保留现有浏览器与桌面集成 | 尚无完整的跨平台能力提供者注册、统一 Browser/Computer Use API 与端到端验收；不能以 VNC 查看器代替此需求 |
| agent-desktop-bridge 是否可删除 | 现有桥接继续保留；VNC 是桌面传输通道，不能证明已覆盖原自动化接口 | cua-driver 替代方案仍需独立核对接口、平台和权限，当前不能删除桥接 |

五项服务是 agent-desktop-bridge、deepseek-harness、openclaw-gateway、proxy-caddy、tailscaled-workspace。其监督方式迁移不代表业务健康：原记录中桌面桥接 health 为 503、Tailscale 为 NeedsLogin，均不能计作迁移修复。见 [Rust 服务与迁移记录](native-services-and-web.md)。

## 测试情况与可信范围

### 本轮收尾验证

所有构建、服务、套接字、浏览器与原生运行均在 admin@192.168.217.6:22022 的远端容器执行。本机只编辑、检查 Git、计算摘要与同步文件。

| 验证项 | 结果 | 覆盖 |
| --- | --- | --- |
| 后台目录同步真实服务集成 | 9 项通过 | 默认关闭、无页面定时更新、Web 重启、独立 headless 节点、暂停恢复、交互读取锁、离线保留/拒绝清除、连接重建重新确认、配置冲突与损坏保护、最多四路并发 |
| 原有目录增量同步集成回归 | 7 项通过 | 增量/删除、重启、离线、授权、坏数据、旧版本与内嵌节点 |
| Web Playwright | 9 项通过 | 新同步设置 4 项、联合目录 3 项、项目 Shell 2 项；保存/暂停、冲突保留草稿、错误身份、移动与明暗主题 |
| Tauri 前端 Playwright | 9 项通过 | 新同步设置 IPC、快照恢复、项目交接与文件复制、共享 ProjectID、网关离线、原生本机项目与离线快照 |
| Rust 格式与静态检查 | 通过 | 全工作区 fmt；runtime/node 所有 target 严格 Clippy（-D warnings） |
| 构建 | 通过 | Web/Tauri 的 TypeScript 与生产资源；cranibox、cranibox-node、runtime、web 与原生客户端，原生启用 desktop,custom-protocol |

本轮合计 34 项不同的后端/界面测试通过，重跑不重复计数。完整 Linux、Windows、macOS CI 矩阵不是上述 34 项的含义。

部署后的真实 Web 另检查默认关闭的后台同步设置、空设备选择不可启用、默认一分钟间隔、375/768/1440px 及横屏的明暗布局、200% 根字号、页面错误与配置保持。Web 与原生程序已更新并保留旧二进制备份；运行程序摘要、入口认证和容器健康结果见 [后台目录同步验证记录](remote-workspace-validation-2026-10-18.md#节点后台项目目录同步)。

### 提交后的 CI 复核

功能与交接提交为 3882dc2，首批 CI 修复提交为 6aa57d4。首次矩阵运行 39094741447 最终失败，暴露的三个跨平台问题已修复并推送：

- 独立客户端只安装自身依赖时，无法从共享 VNC 组件解析 noVNC；将 noVNC 加入客户端 Vite 的依赖去重列表，使其使用客户端自己的依赖。远端另建无 Web UI node_modules 的干净目录，先复现失败，再验证 npm ci、TypeScript 和生产构建通过。
- Windows 严格 Clippy 拒绝两处仅在 Unix 使用的可变绑定；调整 DirBuilder 绑定，保留 Unix 私有目录权限和 Windows 行为。
- macOS 两项复制测试直接使用未解析的临时目录路径，与 /private/var 下的规范路径不同；测试改用生产接口返回的规范根目录，未放宽生产路径安全检查。

修复后远端全工作区格式、全工作区所有 target 的严格 Clippy，以及 9 项项目复制单元测试通过。这些复核独立于上文 34 项功能测试，不将重复运行累计。首次 CI 的真实 Sway/wayvnc 作业通过；Linux 后续在真实 EasyTier 两节点测试失败：旧测试试图通过新增网络路径替换错误令牌，违反多路径的同一授权要求。本次保留拒绝错误凭据的断言，改为显式删除错误连接再接入；远端用校验过的官方 v2.6.4 二进制重新运行，两节点授权/重启和三节点入口故障切换均通过，node 严格 Clippy 与全工作区格式检查亦通过。

6aa57d4 的矩阵运行 **39095261447 已结束**：Windows/macOS 原生作业、三个系统的客户端安装包及真实 Sway/wayvnc 作业均通过；Linux 原生作业在上述旧 EasyTier 测试处失败，其此前步骤通过。此运行不含最新 EasyTier 测试修复，后续验证通过不能改变旧运行的失败结论。

本次同时修正 Release 工作流吞掉 Dockerfile/Compose 检查失败的问题，增加 .dockerignore 防止本地凭据、依赖、编译产物及持久数据进入构建上下文。远端实际 BuildKit 上下文验证、有效与故意损坏的 Dockerfile/Compose 共六项检查通过：11 个必需文件保留，11 个合成排除文件均未进入上下文。旧 Release 的绿色结果不能证明这些失败拦截有效，也不代表完整产品镜像已构建。新增证据与命令见[CI 复核记录](remote-workspace-validation-2026-10-18.md#提交后的-ci-复核)。

**最新功能代码 71f239755f9a95f528151ddbebf8ec758a866087 的矩阵运行 39096427553 已完成，7/7 作业成功**：Linux、Windows、macOS 原生检查，三平台 Tauri 安装包，以及独立 Sway/wayvnc 桌面验证。对应 Release 运行 39096427579 与 Sync to GitCode 运行 39096427547 也均已完成并成功。Release 已执行严格 Dockerfile/Compose 检查，但此次 main 分支运行没有发布完整产品镜像。最终运行元数据保存在远端 /config/validation-artifacts/full-image-71f2397/ci-final.json；本次后续只更新文档，以上结论绑定功能代码 71f2397。CI 构建成功仍不等于目标系统安装、托盘、权限与桌面交互实机验收。

### 完整产品镜像与新实例

本轮另在同一远端主机使用 71f2397 的 git archive、独立 BuildKit 构建器和完整 Dockerfile 完成 Linux amd64 KDE 镜像构建，没有叠加旧验证镜像。使用国际软件源，构建、导出和载入成功，约耗时 17 分 50 秒。镜像 cranibox-full-validation:71f2397 的 ID 为 sha256:571d20538e6bae77fbc70cc0209246b4f4c8ca770a6f6f5f96772b8b566a3a04，大小 6,656,198,462 字节，约 6.2 GiB，包含 KDE 和完整开发工具；这不是空闲内存或 CPU 基准。

用空的独立 /config 启动新容器 cranibox-full-firstboot-71f2397，验证默认 Rust 服务 enabled/active、容器健康、Web 与网关、认证后的节点目录 API、内置命令和桌面入口认证均通过，无需手动安装 runtime。再实际重启容器，复验服务，并确认 NodeID、私有运行凭据和项目文件保持不变。初版临时检查脚本误用了独立 Web 的健康响应格式；根据合并服务端的实际协议修正脚本后通过，初始记录保留，没有修改生产代码。

测试容器和专用构建器均已停止，镜像、缓存、配置与证据保留；主验证容器仍 running / healthy，镜像、身份与端口未改动。本项只验证 HTTP/API、服务恢复和数据持久化，没有验证新镜像中的真实桌面画面或输入，也不覆盖其他桌面、架构、镜像源、公开产品镜像发布/拉取及 Windows/macOS 安装。完整命令和证据见[完整镜像构建与首次启动](remote-workspace-validation-2026-10-18.md#完整镜像构建与首次启动)。

### 此前已经执行的专项验证

下表列出最近一轮对应专项记录，**不是本次全部重跑，也不能将各行数量直接相加**：多轮存在重复回归。详细证据与限制见 [远端验证记录](remote-workspace-validation-2026-10-18.md)。

| 专项 | 已有证据 | 重要限制 |
| --- | --- | --- |
| 自动接单 | 后端 36、Web 16、Tauri 模拟 IPC 6 项；共 58 项不同测试通过 | Codex/Claude 替身，无真实模型与账号登录 |
| 文件分块复制 | Rust 9、后端 13、Web 8、Tauri 模拟 IPC 4 项；共 34 项 | 约 20 MiB 实际传输，不是 2 GiB 压力验证 |
| 邀请中断恢复 | Rust 网络 23、后端 16、Web 8、Tauri 模拟 IPC 2 项；共 49 项 | 网络安装向导使用隔离进程替身；另两项 EasyTier 实网测试在该轮未跑 |
| 协作前置依赖 | Rust 23、后端 34、Web 23、Tauri 模拟 IPC 2 项 | 多个节点是同一远端容器中的独立进程 |
| 实际空间 | 真实 Docker 创建、停止、归档、恢复、身份/文件保留；页面终端及 Selkies/code-server HTTPS 和 WebSocket | 该专项使用远端主机上的旧镜像叠加；完整 Dockerfile 构建另见下一行 |
| 完整产品镜像 | 71f2397 的完整 Dockerfile 构建成功；空配置首次启动、默认服务、认证 API、容器重启与数据保留通过 | Linux amd64 / KDE / 国际源；未发布产品镜像，未验证新实例的实际桌面画面与输入 |
| 实际桌面 | 真实 X11/Xvfb+x11vnc、隔离 Sway+wayvnc 测试及 noVNC；远端 KDE 的 Selkies 与 Tauri 窗口 | X11/Sway 通过不能推导 KDE、Windows、macOS 通过 |
| 原生壳 | 远端 KDE 中原生静态资源加载、真实 IPC、窗口隐藏、托盘注册 | 不能用“进程存在”代替界面可用，也未完成每个业务流程原生实测 |

### 明确未通过或尚未验证

- **KDE Wayland VNC 未通过**：w0vncserver 密码和 portal 授权成功，但 KDE 屏幕捕获报 failed to start streaming / 不支持的合成类型，在 ServerInit 前断开；隔离 KWin 回退 QPainter，没有可用 OpenGL 画面。临时进程与密码已清理。当前继续用 Selkies，不得标记 VNC 已完成。
- Windows 原生与 macOS 的安装包、托盘、登录自启、权限提示、VNC、进程树、断网/休眠恢复、容器安装向导尚未实机验收。WSL2 不能替代 Windows 原生。
- 真实 Tailscale ACL、两台物理设备互访及日常网络切换未验收；本轮没有连接真实 Tailnet。
- 完整 Linux amd64 KDE 镜像构建及新实例首次启动、重启已通过；公开产品镜像发布/拉取、其他桌面/架构/镜像源，以及该新实例的实际桌面画面与输入尚未验证。
- 未调用真实 Codex/Claude 模型，未验证订阅登录；后台执行适配器仍需端到端实际工作任务验收。
- 浏览器与 IPC 测试中的模拟响应、进程替身和独立 RFB 服务，均在各专项记录中注明，不能提升为实机验收。
- 功能代码 71f2397 的完整 CI 已通过；三平台安装包成功生成，仍不能替代目标系统安装和交互实测。

## 下一阶段优先级与验收条件

| 顺序 | 工作 | 完成判据 |
| --- | --- | --- |
| P0 | 桌面互访可日用 | 先在真实 Linux/KDE 主机选择并验证可支持的捕获后端；然后 Windows/macOS 的安装、录屏/输入权限、启停、退出清理均可操作；从另一个节点真正看见画面并输入，撤权断开 |
| P0 | 跨平台安装与产品镜像发布验收 | 71f2397 的 7 项 CI、完整 KDE 镜像构建及新实例启动/重启已通过；下一步在目标 OS 安装实际安装包并验证交互，以及验证公开产品镜像的发布/拉取。不能用旧失败运行或本地镜像标签替代相应结论 |
| P1 | 维护者的真实三设备流程 | Mac 委托 Linux/Windows Agent，浏览器关闭后继续，换入口回复并核对结果；覆盖断网、休眠、重启，不依赖同容器替身或人工改账本 |
| P1 | 完整统一视图与数据同步 | 明确设备/项目/Agent/任务各数据的权威、版本、删除与冲突协议；另一入口无需逐一手配才能看同一授权范围；保留设备撤权语义 |
| P1 | 设备信任与能力接入 | 定义公钥配对、签名身份、凭据轮换、加入/撤销与能力提供者协议；独立验收 Browser Use/Computer Use |
| P2 | 真正 Agent 会话接续与迁移 | 针对具体 harness 设计可恢复检查点、项目文件/上下文一致性与目标启动；源端停止和目标继续有明确状态，不能声称移动原进程 |
| P2 | 交互和规模优化 | 在真实用户流程上检查导航、状态密度、长清单和错误恢复；评估大量设备与文件的资源消耗；复杂编排按日常需求推进 |

暂不需要为了节点身份等同引入选主；目前数据由所属节点持有，界面连接任一运行入口。若未来要复制协作协调状态，先定义故障与冲突模型，再决定是否需要一致性协议。

## 接手与复现

### 阅读顺序

1. 本文与 [远端验证记录](remote-workspace-validation-2026-10-18.md)：先了解完成与失败边界。
2. [产品与分布式设计](distributed-agent-workspace-design.md)、[组网设计](decentralized-node-network-design.md)：目标架构，不将提案当成现有功能。
3. [节点直连](direct-node-peers.md)、[设备网络](device-networks.md)、[客户端](node-desktop-and-device-mesh.md)。
4. [远程桌面](remote-desktops.md)、[虚拟空间](virtual-spaces.md)、[项目目录与复制](project-session-catalog.md)。
5. [消息](device-messages.md)、[任务与自动接单](device-tasks.md)、[会话](agent-sessions.md)、[后台执行](agent-executions.md)。

### 源码入口

| 位置 | 内容 |
| --- | --- |
| crates/cranibox | 合并服务端入口 |
| crates/workspacectl、crates/workspace-services | Rust 控制工具与服务兼容管理 |
| crates/workspace-runtime/src/peers、projects | 连接、目标授权客户端、消息/任务/项目账本、文件复制与后台同步配置 |
| crates/workspace-node/src/{local.rs,peer,network,spaces,supervisor.rs,task_worker.rs,project_sync.rs} | 节点宿主、直连服务、网络/空间、进程、自动接单与后台目录调度 |
| crates/workspace-web | 登录/CSRF、Web API、终端/桌面和空间应用转发 |
| addons/control-center/ui/src | React 共用业务组件与浏览器适配器 |
| apps/node-desktop | Tauri 原生宿主、IPC、托盘与客户端适配器 |
| tests、两端 UI 的 tests | Python 真实服务集成和 Playwright；模拟与实机条件见各文件 |
| .github/workflows/rust-runtime.yml | 三平台 Rust/安装包、Linux 服务与浏览器、独立 Wayland 验证 |

### 远端环境

- SSH：admin@192.168.217.6，端口 22022，容器 cranibox-validation；不得改用当前本机做运行验证。
- Web：https://192.168.217.6:3080；KDE/Selkies：https://192.168.217.6:3031。凭据只在远端私有 container.env，不写入 Git、测试报告或命令行。
- 容器源码 /config/agent-workspace-manager/source；Cargo 目标 /config/validation-build/target；证据 /config/validation-artifacts。源码和验证数据均在远端持久卷。
- 主 Web 与原生客户端是两份独立身份；原生使用 /config/validation-client/{config,data,state}。保持分离，避免两个进程共用 NodeID。
- 主容器不挂宿主 Docker socket，也不是 privileged。虚拟空间用过独立临时管理容器；它和两个测试空间目前停止，保留数据供复核，不要误删。
- 完整镜像验证另留 cranibox-full-validation:71f2397 镜像、已停止的 cranibox-full-firstboot-71f2397 容器与 cranibox-validation-full-build-20261018 构建器缓存。证据在 /config/validation-artifacts/full-image-71f2397；首次启动环境文件含私有凭据，不要输出或提交。停止后的容器可能仍显示 unhealthy，应以 firstboot.json 中停止前健康检查及重启结果判断本次验收。
- 其余候选设备：Windows/WSL2 192.168.217.226；macOS fliaping@192.168.217.85。当前尚未完成这些设备的本轮测试，先核对访问与图形登录环境。

在远端容器源码目录配置以下环境，再运行本轮测试：

~~~sh
export CARGO_HOME=/config/.cargo
export CARGO_TARGET_DIR=/config/validation-build/target
export CARGO_BUILD_JOBS=2
export CRANIBOX_BIN=$CARGO_TARGET_DIR/debug/cranibox
export CRANIBOX_NODE_BIN=$CARGO_TARGET_DIR/debug/cranibox-node
export WORKSPACE_RUNTIME_BIN=$CARGO_TARGET_DIR/debug/workspace-runtime
export WORKSPACE_WEB_BIN=$CARGO_TARGET_DIR/debug/workspace-web
export CRANIBOX_HERDR_BIN=/config/bin/herdr
export PYTHONPATH=tests

cargo fmt --all --check
cargo clippy --locked -p agent-workspace-runtime -p cranibox-node --all-targets -- -D warnings
python3 -m unittest test_background_project_sync test_project_catalog_sync -v
~~~

UI 使用仓库 Playwright 配置与安装好的 Chromium；本轮为复用远端系统 Chromium 临时指定 /usr/bin/chromium 和 --no-sandbox，临时配置不提交。CI 按工作流安装浏览器。相关用例为 Web 的 project-sync.spec.ts、federated-projects.spec.ts、projects.spec.ts，以及客户端 client.spec.ts 中 background catalog sync / resumable snapshots / project 相关用例。

先构建两端前端，再构建嵌入静态资源的 Rust Web 与原生程序；直接 Cargo 运行原生客户端必须启用 desktop,custom-protocol，避免误连已经关闭的 Vite 开发服务。完整构建及 CI 命令以功能文档与工作流为准。

后续模型应先阅读功能代码 71f2397 的最终 CI 与本表 P0；如又修改代码，再按新提交复核。选择一个日常流程完成，不要继续无界扩展功能，也不要在没有新证据时把“模拟 IPC 通过”或“进程仍存在”升级为完整产品验收。
