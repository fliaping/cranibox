# Cranibox 功能、验证与后续交接

整理日期：2026-10-18。本文是下一位维护者的接手入口，汇总本次连续开发的最终状态。设计文档描述目标，专项验证记录按发生顺序保留历史；判断目前完成范围时优先看本文及对应功能文档。

最新功能为下文的节点 Computer Use：独立授权、回环桥接适配、短期输入控制权、目标绑定 MCP，以及共用 Web/Tauri 管理界面。功能提交 973f1c8、默认权限断言修复 3c1a38d 已推送；本轮 411 项不同自动化测试通过，3c1a38d 的 CI 矩阵最终 7/7 成功，初次失败与重跑边界见下文。此前功能提交 6396b93 修复 Linux AppImage 的 MCP 与登录启动项；Selkies 接入为 afaa137，Tailscale 安装引导为 b6ed81e。各次 CI 只覆盖对应提交，不能用旧绿色结果覆盖新代码。

交接范围：收尾现有实现、记录需求与验证、提交仓库；没有把所有产品需求标为完成。真实 Linux/KDE 桌面已通过 Selkies 经节点中继接通；3c1a38d 的最新 Linux .deb 和无 FUSE 兼容模式 AppImage 已补验真实 Computer Use、原生管理界面与 MCP，旧版完整托盘/自启流程仍绑定 6396b93，下一阶段优先完成 Windows/macOS 桌面与安装。远端日常 Web 仍运行 71f2397，本轮未停止或升级日常客户端，原版本、原身份和配置保持；没有把新功能部署到日常主服务，也未安装系统 Tailscale 或重启合成器。

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
| EasyTier 与 Tailscale 同时使用 | EasyTier 固定版本安装、用户态网络、节点发现、端口转发、多个入网地址、短期邀请与丢响应恢复；系统 Tailscale 安装/启动方案审阅与确认、后台进度、官方登录、持续发现与授权连接 | Tailscale 是系统服务，不由节点长期托管；自动安装有平台前置条件，容器转宿主/已有服务指引；真实系统安装与 Tailnet 跨设备验收未完成 |
| 同一设备多条路径 | 保存并排序显式路径，新连接按可用性选择，认证/身份错误不继续回退 | 已建立连接不无缝迁移；桌面直连与网关通道仍显式选择 |
| 浏览器和原生端统一体验 | 项目、设备、网络、消息、任务、终端、VNC 与空间共用 React 组件，分别适配 Web 和 Tauri IPC | 不是全部管理页面完全一致，也不是节点数据自动全量复制 |
| 常驻托盘与登录自启动 | Tauri 窗口关闭到托盘、重连、明确退出、用户登录后自启动配置 | 6396b93 的 Linux .deb/AppImage 通过实际界面、托盘、XDG 自启开关、单实例；AppImage 生成启动项经 GIO 在无 FUSE、无手动兼容变量时静默启动并可唤回。最新 3c1a38d 仅补验私有 D-Bus 下的窗口与 Computer Use IPC，未重验托盘。实际登录、休眠、网关断线恢复及 Windows/macOS 仍待实测 |
| 持久终端与浮动窗口 | 可变大小、最小化到状态栏、多个会话、重新附着；服务端使用独立 Herdr，客户端使用原生 PTY | 6396b93 .deb 关闭浮动终端、收起原生窗口及操作托盘重连后，Shell PID 与启动时间不变；此前 afaa137 另验证导出变量保留。没有制造真实网关网络故障，客户端进程或系统退出后不能保留该 Shell |
| 桌面互访 | 共用 VNC/noVNC 与 Selkies/JPEG 查看器；本机、直连及出站 WebSocket 中继；目标桌面单独授权、单控制者、停止与撤权取消 | KDE 容器 Selkies 的真实画面、完整文字输入、重连和撤权通过；Windows/macOS 实机及 KDE/GNOME 的自动安装与托管未实现 |
| 内置 VNC 托管 | Linux X11 使用 x11vnc，兼容 wlroots 的 Wayland 使用 wayvnc；守护、失败退避、节点重启恢复 | 不是一个纯 Rust 跨平台捕获实现；KDE 容器实测屏幕捕获失败，Selkies 可用，见下文 |
| 虚拟空间 | 检测、安装计划、明确执行、镜像策略、创建/停止/启动/归档/恢复、资源限制、独立身份与持久卷；桌面/编辑器独立应用入口 | 真实 Docker 生命周期与浏览器流程已测；完整 Linux amd64 KDE 镜像的干净构建、空配置首次启动及重启保留数据已通过，公开产品镜像拉取与各 OS 系统依赖安装仍未验收 |
| 统一项目与会话目录 | ProjectID、节点工作副本、Shell 关联、网关/直连汇总；增量目录读取、删除处理、只读离线快照 | 各节点保留本地权威；不是全网数据库复制，不支持离线修改后自动合并 |
| 关闭页面后的目录同步 | 本轮新增 Rust 节点后台同步，明确选择 1–32 个已接入设备，15/30/60/300 秒，最多四路并发；暂停和重启恢复 | 默认关闭；只同步项目目录和会话关联，不复制工作文件；凭据/连接变更后需要重新选择 |
| 项目迁移与接续 | 本机与直连双向、两个远端之间快照复制；清单确认、分块校验、暂停/恢复、持久接收回执；保留 ProjectID 与接续说明 | 当前是显式快照复制，非持续文件同步；上限 2 GiB/单文件 1 GiB，实际集成传输约 20 MiB，未做上限压力测试 |
| 多 Agent 通信与任务 | 持久消息、线程回复、确认、离线投递、任务委托/领取/进度/问题/回复/结果/合作取消，CLI/MCP 与共享界面 | 不能由普通消息自动启动进程；同一容器中的独立身份验证不等于多台物理设备验收 |
| Agent 会话与后台执行 | 主动登记/续期、汇总授权会话、定向委托；Codex/Claude Code 非交互适配、启动检查、持久输出与停止；可选独立执行 MCP | 模型调用与 CLI 登录未实测；不支持运行中追加交互输入、模型检查点恢复或进程迁移 |
| 多设备协作与自动接单 | 持久协作 Run、任务汇总、统一停止、核对归档、最多八项前置任务；接收端可明确启用按可信发送方和固定目录的后台 worker | 默认关闭；不是通用复杂编排平台；人核对模式与显式自动提案模式分开，退出码不直接等于任务成功 |
| 各节点 Agent 共享 Computer Use | 本轮新增独立 computer 授权、目标配置的回环桥接、直连 API、短期输入控制权、持久停止、独立目标绑定 MCP；Web/Tauri 共用配置与停止界面 | 复用现有 bridge，不安装跨平台驱动；只走直连。12 项后端专项使用动作夹具；3c1a38d Linux CI 包另通过真实 KDE 截图、输入、MCP、重启保留停止和两包原生 IPC。多物理设备、模型调用及 Windows/macOS 提供者待验收 |
| 各节点 Agent 共享 Browser Use | 容器保留现有浏览器集成，也可通过授权命令使用目标已有工具 | 尚无独立的跨节点 Browser Use 协议、授权与提供者；不能以 Computer Use、VNC 或 Shell 代替 |
| agent-desktop-bridge 是否可删除 | 现有桥接继续保留，并作为本轮 Computer Use 的明确配置提供者；VNC 是桌面传输通道 | cua-driver 替代方案仍需独立核对接口、平台和权限，当前不能删除桥接 |

五项服务是 agent-desktop-bridge、deepseek-harness、openclaw-gateway、proxy-caddy、tailscaled-workspace。其监督方式迁移不代表业务健康：原记录中桌面桥接 health 为 503、Tailscale 为 NeedsLogin，均不能计作迁移修复。见 [Rust 服务与迁移记录](native-services-and-web.md)。

## 测试情况与可信范围

### 最新收尾：Computer Use

目标用户配置已经运行的本机桥接，并独立授予 computer 权限；旧授权保持关闭。Agent 从明确绑定目标的 CLI/MCP 获取截图与 1–300 秒输入控制权。它和 VNC/Selkies 查看器共用一个控制名额，撤权、到期、重配、监听停止及节点重启使控制权失效。紧急停止先保存暂停再通知桥接，桥接离线时仍禁止重新取得输入控制权；恢复只允许目标管理者明确操作。

当前只接入现有 bridge 的回环 HTTP 服务，使用直连路由；不是 Windows/macOS 原生驱动，也没有实现 Browser Use 或旧网关转发。未知字段、远端恢复、任意目的 URL、持续按住输入均拒绝，输入失败不自动重发。详细操作、边界和源码见 [Computer Use](computer-use.md)。

下表由本会话直接执行的构建与测试均在指定远端容器 cranibox-validation，使用隔离身份；后述 CI 在 GitHub 执行并单列。表内真实 Rust 节点、Python 桥接、Web 后端参与测试；底层桌面动作是可控 HTTP 夹具，Tauri 界面使用模拟 IPC。后续真实桌面和原生安装包验收另列，不计入自动化数量。

| 验证 | 结果 | 可信范围 |
| --- | --- | --- |
| 全 Rust 工作区单元测试 | 174 通过、4 忽略 | 常规 cargo test 忽略资源测量、2 项真实 EasyTier 与 GIO 图形用例；后续 CI 的显式专项执行单列，不加入本表 411 项 |
| 后端专项及相关回归 | 70 通过 | 包含 12 项 Computer Use：旧授权默认、租约/竞争/撤销、VNC 互斥、重启保留停止、桥接离线、输入不重放、PNG/MCP、慢状态期间停止；另含节点、桌面与现有 MCP 回归 |
| Web 全部界面 | 122 通过 | 含 Computer Use 2 项、直连、桌面、网络、项目、协作与浮动终端；覆盖中英文、明暗和小屏。使用真实 Rust 预览服务，部分业务响应模拟；桌面画面来自 RFB/JPEG 夹具 |
| Tauri 全部界面 | 45 通过 | 含新增 Computer Use IPC；属于浏览器模拟原生 IPC，不代替真实安装包验收 |
| 构建与静态检查 | Rust 工作区 fmt/build/clippy、Web/Tauri TypeScript 与生产资源、原生 desktop Clippy 和 desktop,custom-protocol 构建通过 | 发布 Web 资源已更新，未部署至日常主服务 |

初次较大后端回归 69 项中 1 项失败，原因是旧响应断言缺少新增的 computer:false。更新默认关闭断言后，12 项 Computer Use 和该回归共 13 项复测通过；最终对包含新增 Web 并发用例的 70 项完整复测，全部通过。新增的真实 Web 并发用例验证慢桥接状态不持有宿主锁阻碍紧急停止。原始初测、复测、Rust/UI 日志及隔离页面截图在 /config/validation-artifacts/computer-use-20261018；不与历史测试或重复运行相加。

本轮共 411 项不同自动化测试通过，另有 4 项 Rust 测试忽略（174 Rust + 70 后端 + 122 Web + 45 Tauri UI）。此前 20 项 Web 相关测试已被完整 122 项覆盖，不重复计数。原生构建与静态检查不计作测试数。验收摘要为该证据目录的 acceptance.json，source-snapshot.json 保留功能提交的 25 个文件摘要，source-snapshot-final.json 将修复提交的 26 个变更文件逐一核对为一致；entrypoint-checks.json、daily-preserved.json 保留早期检查，final-health.json、post-full-web-processes.json 记录完整 Web 复测后的最终健康与进程检查。容器 running / healthy，未增加 privileged 或 Docker socket；3080/3031 仍为匿名 401 / 认证 200，日常 Web PID 538744、客户端 PID 767446、二进制摘要和 NodeID 不变，没有残留测试节点。

功能提交为 973f1c814bd814745b513d20e2e7f636be06c424，权限断言修复提交为 3c1a38d19c66d3b9050f3003705f47a5c7187ace，均已推送 origin/main。首次矩阵 39116645947 的 Linux Web 完整测试为 121/122：旧消息授权测试的严格对象断言遗漏 computer:false。修复只补齐默认关闭断言，没有放宽授权；该旧运行保留失败结论。

修复后远端 Web 完整 122 项均通过，日志为 web-ui-full.log。**矩阵 39117202769 最终 completed / success，7/7 作业成功**，已核对完整 headSha 为 3c1a38d19c66d3b9050f3003705f47a5c7187ace：三平台原生检查、测试和发布构建，三平台 Tauri 安装包，以及独立 Sway/wayvnc。Linux 同时执行完整 Web、后端、真实 X11 VNC、用户态 EasyTier 重启/入口切换、真实 GIO 启动器和资源测量专项；这些 CI 结果单列，不再累加到 411 项。

该矩阵首次 Wayland 作业在两个无效凭据用例后的合法 ARD 认证响应处发生 5 秒套接字超时；六个其他作业成功。整轮结束后仅重跑失败作业，第二次 Wayland 成功，未修改产品代码、跳过错误凭据用例或放宽超时。初次失败保留在 ci-attempt-1.json、ci-wayland-attempt-1.log，最终状态及十项产物元数据在 ci-final.json，成功日志为 ci-wayland-final.log；重跑成功不代表已定位或修复超时原因。通过 gh run view 39117202769 --repo fliaping/cranibox 查看。对应 Release 39117202772、Sync to GitCode 39117202930 也均 completed / success；main 的 Release 不代表发布了产品镜像。

本轮 Artifacts 共十项：三平台 cranibox、cranibox-node、cranibox-node-desktop，以及 resource-baseline-linux。桌面分别提供 Linux .deb/AppImage、Windows NSIS 和 macOS .dmg；应按运行 39117202769 与提交 3c1a38d 下载，不能只按 0.1.0 文件名判断版本。Linux 两包已完成下述新增能力人工验收；Windows/macOS 尚未实机安装，下文 6396b93 的较完整 Linux 流程只覆盖对应旧包。

资源报告 resource-baseline.json 也绑定 3c1a38d。在隔离 Linux CI 的四秒混合 HTTP 轮询与 SSE 场景中，合并服务端 RSS 约 17.4 MiB，独立节点约 10.3 MiB；网关与 Web 属于同一进程，不重复相加。六个相同测试服务的监督开销由 16628 KiB 降至 13136 KiB，降低约 21%，目前仍由 s6 配合 Rust 管理入口监督。以上不是日常空闲、完整 KDE/Agent/业务进程总内存或跨平台资源承诺。

### 最新补验：3c1a38d Linux 包与真实 Computer Use

下载运行 39117202769 的 Linux 桌面 Artifact 12133315868，归档及两包摘要均与元数据一致。实际安装 .deb，再以 --appimage-extract-and-run 运行同一产物的 AppImage；未设置手动兼容变量。两包使用同一隔离 NodeID，日常客户端始终运行。

- .deb 的真实节点/桥接接入 KDE/Selkies PixelFlux，完成 1440×1050 截图、窗口查找/聚焦、短期控制权、文字与 Return；kdialog 实际输出与完整测试文字一致，MCP 返回真实 PNG。
- 无控制权输入拒绝；紧急停止后旧输入与新控制权拒绝，节点重启仍暂停，本机明确恢复后重新取得控制权；撤权后接口拒绝且 MCP 工具列表为空。
- 两包的可见 Tauri 窗口与真实 IPC 完成桥接状态、紧急停止、取消恢复与确认恢复，暂停字段实际写盘。AppImage 自身的 computer-mcp 从 AppImage GUI 宿主取得真实桌面截图。

这轮是同一远端容器内的独立身份人工验收，不增加 411 项自动化数量，也不构成多物理设备或真实模型调用。GUI 为避免单实例插件唤回日常旧包，使用私有 D-Bus，会话中的 KDE 托盘不属于正常桌面集成；因此不重验或声称新包托盘、自启、旧版完整业务、普通 FUSE、登录/休眠与真实网络故障已通过。首轮脚本误将逐行 MCP JSON 当成一个 JSON，修正检查脚本后完整通过，未修改产品；原始失败记录保留。

私有证据在 /config/validation-artifacts/linux-installer-3c1a38d，包括 acceptance.json、real-computer-acceptance.json、两包原生 IPC/MCP、摘要、清理及最终健康。测试 GUI、私有会话辅助进程与桥接已退出，.deb 卸载，测试授权撤销、失效 Token 文件删除、剪贴板恢复。日常 Web PID 538744、客户端 PID 767446、原摘要与身份保持，容器 healthy，3080/3031 仍为匿名 401 / 认证 200。包的完整摘要、限制和证据见[专项补验](remote-workspace-validation-2026-10-18.md#3c1a38d-linux-包及真实-computer-use-补验)。

### 此前补验：6396b93 Linux CI 安装包

已下载运行 39110808540 的 Linux 桌面 Artifact 12130851033，归档摘要与 GitHub 元数据一致，实际安装 .deb 并运行同一产物的 AppImage，使用独立 profile 和相同 NodeID。通过真实 KDE 原生窗口与 PID 对应的托盘操作，没有模拟 Tauri IPC。

- 两个安装包的默认两个 MCP 入口、明确启用后的三个入口与 XDG 自启开关通过；启动文件权限 0600，关闭后两处目录均无条目。
- GUI 退出后按界面实际复制的命令运行，两种包的 devices/messages/runs 均完成 initialize 与 tools/list，分别返回 5/3/2 个工具，退出码均 0；只用隔离测试网关设置，不声称远程命令验收。
- AppImage 的命令及生成启动项自带 --appimage-extract-and-run；无 /dev/fuse、无手动兼容环境变量时，GIO 实际启动后业务进程只收到 --background，窗口隐藏、托盘存在，单实例和托盘唤回通过，NodeID 不变。
- .deb 真实创建的 Shell 在浮动窗口关闭、原生窗口收起、托盘重连与重新附着后，PID 761420 和进程启动时间不变；取消退出不结束 Shell，确认退出后 Shell 消失。远程桌面输入转发干扰了命令标记输入，因此本轮不宣称导出变量测试通过，也不替代真实网络故障测试。

证据在 /config/validation-artifacts/linux-installer-6396b93，包括 acceptance.json、产物摘要、两个包的复制配置/自启记录、六组 MCP 原始输出、GIO/PTY/单实例记录、恢复与健康检查，以及 16 份私有截图。测试包、Shell、AppImage 和浏览器均已退出或卸载，剪贴板恢复，日常客户端恢复为 PID 767446、原二进制与原 NodeID；主 Web PID 538744 未变，容器 healthy，3080/3031 未认证 401、认证 200。

这补齐了此前“新源码与旧 AppImage 组合验证”留下的新 Linux 包验收缺口；普通 FUSE、实际系统登录/休眠、网关断网、Windows/macOS 和真实 Tailnet 仍未覆盖。详情与摘要见[最新安装包补验](remote-workspace-validation-2026-10-18.md#6396b93-linux-ci-安装包补验)。

### 此前收尾：Linux 启动可靠性

针对此前真实 AppImage 验收发现的缺陷，抽出可独立测试的启动模块；仅 Linux 替换原自启动插件的路径写入逻辑，Windows/macOS 仍使用原插件。未新增前端交互。AppImage MCP 与自启自动加入 --appimage-extract-and-run；该参数由运行时移除，不传入业务 CLI。普通安装程序不加此参数，后台运行工具仍需用户明确选择。

所有执行仍在指定远端 Linux 容器，本轮结果单列，不累加到此前 186 项：

| 验证 | 结果 | 范围 |
| --- | --- | --- |
| 启动模块单元测试 | 5 通过 | MCP 默认/可选入口、持久路径与参数、无效路径拒绝、XDG 与旧目录迁移、关闭清理及隐藏条目 |
| 真实 GIO 启动器 | 1 通过 | 包含空格、引号、反斜杠、美元符号、反引号和 %f 的实际文件名，接收的参数完整且准确；显式执行忽略用例，已加入 Linux CI |
| Rust 检查和原生构建 | 通过 | 工作区格式；客户端 desktop 所有 target 严格 Clippy；desktop,custom-protocol 构建 |
| 新源码的真实原生 IPC | 通过 | 独立 profile；实际打开/关闭自启，确认写入/删除 XDG 目录且 HOME 未新增条目；复制默认两个入口及明确启用后的三个入口 |
| 三个 AppImage MCP 入口 | 通过 | GUI 退出后，实际执行所复制命令；无 APPIMAGE_EXTRACT_AND_RUN 环境变量；initialize 与 tools/list 分别返回 5/3/2 个工具 |
| 新启动项运行已有 AppImage | 通过 | GIO 真实启动含空格的 afaa137 AppImage；无 /dev/fuse、无手动兼容变量；静默进入托盘，托盘可打开窗口，NodeID 保持 |

后两项执行的 AppImage 仍是已校验的 afaa137 CI 产物；新源码使用实际构建的原生程序，并指定该持久 AppImage 作为启动器，以分别验证新 IPC 输出和已发布运行时。设备 MCP 使用隔离的测试设置，只做协议初始化和工具枚举，不声称访问了真实网关。没有把这项组合验证称为新安装包验收，也没有真实注销/登录或休眠。

首次 GIO 测试真实失败：GIO 在展开 %% 字段码之前检查可执行文件存在，导致文件名含百分号时无法加载。修复为仅此类路径经 /usr/bin/env -- 传入，保留正确参数转义；最终真实用例通过，初次失败日志保留。代码与证据在远端 /config/validation-artifacts/linux-launcher-20261018，包含单元/真实启动器/Clippy/构建日志、复制配置、MCP 原始输出、自启项与恢复记录、私有截图。自启恢复禁用、剪贴板恢复、测试 GUI 和浏览器退出，日常原生程序恢复原字节和 NodeID；主 Web 未替换。

**该阶段功能提交 6396b93 的完整 CI 已完成，矩阵运行 39110808540 为 7/7 成功**，已核对完整 headSha 为 6396b93ed53a2f6a820a67fb3af0f0f0d8188859：三平台 Rust 检查、测试与发布构建，三平台 Tauri 安装包，以及独立 Sway/wayvnc 作业均成功。Linux 同时通过真实 GIO 启动器、完整 Web/后端回归、真实 X11 VNC、用户态 EasyTier 重启/入口切换与资源报告。Release 39110808411、Sync to GitCode 39110808393 也均 completed / success；此次 main Release 没有发布产品镜像。

该运行提供十项 Artifacts：三平台 cranibox、cranibox-node、cranibox-node-desktop，以及 resource-baseline-linux。客户端分别包含 Linux .deb/AppImage、Windows NSIS、macOS .dmg；下载时选择运行 39110808540，不能仅凭相同的 0.1.0 文件名区分版本。Linux 新包随后已补做实际验收，见上节；这里的原生 IPC 与旧 afaa137 AppImage 组合验证仍单列。完整 CI 元数据、作业步骤和产物摘要保存在本节证据目录的 ci-final.json。

### 此前收尾：Selkies 节点桌面接入

KDE 容器原有 VNC 捕获失败没有被绕过或标为成功。新增路径连接已运行 Selkies 的内部 WebSocket，走既有目标签发授权、单控制者、直连/中继和 Tauri IPC；Web/Tauri 按首包选择 noVNC 或 JPEG 查看器。配置和 CLI 显式选择 protocol，旧 VNC 配置保持兼容；只探测 MODE，不主动接管桌面。连续文字产生的键盘事件按批传输，修复真实输入时挤满节点有界队列的断开。

全部执行仍在指定远端容器，最新专项验证如下；同一用例重跑不累计，模拟与实机分开：

| 验证 | 结果 | 范围 |
| --- | --- | --- |
| runtime peers Rust 单元测试 | 61 通过 | 含协议兼容、分片/合并、大小限制、输入白名单、探测不接管、慢消费者撤权与控制名额释放 |
| cranibox-node Rust 单元测试 | 47 通过，2 忽略 | 原有节点/网络/服务回归；两个真实 EasyTier 用例本轮未跑 |
| 桌面 API/直连/中继/服务集成 | 22 通过 | remote_desktops 7、desktop_relay 7、desktop_service 8；含 Selkies 128 KiB 输出分块、输入、授权、停止与撤销 |
| Web 桌面界面 | 11 通过 | 真实 Rust 通道、合成 RFB/JPEG；连续文字、鼠标、只读、重连、取消待连接、认证、服务引导及明暗/小屏 |
| Tauri 前端全套 | 44 通过 | 浏览器模拟 IPC；新增 Selkies 首包/画面分片、逐帧确认、完整连续输入和关闭清理，兼容原 RFB |
| 真实 KDE/Selkies | 1 通过 | 出站节点经 Workspace 中继，1024×768、113 种采样颜色；整段 31 字符送达自身 kdialog，重连、撤权及零页面异常 |

本轮共 186 项不同测试通过，2 项忽略；不与此前 118 项等历史轮次累加。全工作区格式、严格 Clippy、Web/Tauri TypeScript 与生产资源、Rust 工作区和 desktop,custom-protocol 原生构建另列检查项。375px 明色与 1440px 暗色截图已查看，测试两种主题均无横向溢出。真实 KDE 测试使用自身对话框并清理，不修改主服务或用户数据。

接入地址在本镜像中是 127.0.0.1:8082；第一次误用 8081 的连接失败、连续输入修复前的真实断开，以及 Web 初始端口冲突/旧文案断言/预览二进制环境遗漏均保留在证据中。最终 11 项 Web 与真实 KDE 重测通过；取消连接用例单独复测及最终整套均通过，没有据此修改取消逻辑。

证据目录为远端 /config/validation-artifacts/selkies-desktop-20261018，命令及日志见[专项记录](remote-workspace-validation-2026-10-18.md#selkies-节点桌面收尾)。Selkies 需要已运行的无 token 握手服务；连接会接管主显示器并断开其原查看器，外部 Selkies 查看器也能反向接管。当前不支持音频、剪贴板、文件传输、视频编码或自动托管 Selkies，详见[接入限制](remote-desktops.md#已有-selkies-服务)。

上述 186 项验证不能代替 Windows/macOS 实机、完整镜像重新构建或真实多物理设备互访；afaa137 Linux 安装包的独立人工验收见下节，后续 6396b93 补验见前文。

**本次功能提交 afaa137 的完整 CI 已完成，矩阵运行 39106262547 为 7/7 成功**。已核对完整 headSha 为 afaa137ce78c1847cb893e97d1527e3b06b23d6f：Linux、Windows、macOS 原生检查/测试与发布构建，三平台 Tauri 安装包，以及独立 Sway/wayvnc 作业均成功。Linux 同时通过完整 Web、后端集成、真实 X11 VNC、用户态 EasyTier 重启/入口切换与资源报告检查。Release 39106262625 和 Sync to GitCode 39106262578 也均 completed / success。最终元数据与产物清单保存在上述远端证据目录的 ci-final.json；此次 main 分支 Release 没有发布产品镜像。

该矩阵运行的 Artifacts 包含 cranibox-node-desktop-ubuntu-latest（.deb、AppImage）、cranibox-node-desktop-windows-latest（NSIS）和 cranibox-node-desktop-macos-latest（.dmg）。服务端与独立节点分别为 cranibox-平台-latest、cranibox-node-平台-latest，资源报告为 resource-baseline-linux。Linux 桌面包已在下述真实环境验收；Windows/macOS 仍需下载该运行对应的新包实际安装，不能以 CI 构建或旧包证据代替。

### 此前：afaa137 Linux CI 安装包与 Shell 重新附着

从上述运行 39106262547 下载 cranibox-node-desktop-ubuntu-latest，Artifact ID 12128354180，归档 SHA-256 为 2999969618d43206a225553e6bc4d218828780018181c0843d3dd4fa6df6ad2a，与 GitHub 元数据及远端接收文件一致。所有执行均在指定远端 Ubuntu 26.04 amd64 / KDE Wayland 容器，使用独立配置与身份。此为人工集成验收，不计入 186 项自动化测试。

| 产物/场景 | 实测结果 | 限制 |
| --- | --- | --- |
| Cranibox Node_0.1.0_amd64.deb | 实际安装 release 程序、可见原生界面、真实 IPC 复制 MCP、与进程对应的托盘、自启配置开关、退出取消/确认均通过；--background 隐藏启动、单实例唤回、重启保持 NodeID 通过 | 未做系统注销/登录、休眠或全部业务流程 |
| .deb 本机终端 | 关闭浮动终端、关闭原生窗口、操作托盘重连/打开后，重新附着的 Shell PID 仍为 713970，导出变量仍为 afaa137-pty-persist；结束时显式退出测试 Shell | 未配置网关；重连菜单操作不构成真实网络断开/恢复验收 |
| AppImage 默认运行 | 退出码 127，日志为缺少 /dev/fuse，不能挂载 | 该容器的默认启动失败，未验收具备 FUSE 的普通桌面启动 |
| AppImage 无 FUSE 兼容模式 | 设置 APPIMAGE_EXTRACT_AND_RUN=1 后，版本命令、真实原生界面、托盘、IPC、退出及复用 .deb 的 NodeID 均通过；复制的 MCP 指向持久 AppImage 路径，补充同一环境变量后消息 MCP 初始化和三个工具枚举通过 | 复制配置没有自动包含兼容变量，需手动给对应 MCP 条目添加 env；未测试 AppImage 自启动 |

已卸载测试 .deb，恢复原生客户端的原字节、原 NodeID 和配置；原程序旧 inode 已被先前构建替换，因此从备份恢复至稳定路径 /config/validation-client/bin/cranibox-node-desktop，未覆盖当前构建产物。自启恢复为禁用，剪贴板恢复、私有剪贴板备份删除，临时浏览器和测试 Shell 已退出。主 Web PID 538744 未变；容器 running / healthy、非 privileged、无 Docker socket，Web 3080 与桌面 3031 均为未认证 401 / 认证 200。

证据目录为远端 /config/validation-artifacts/linux-installer-afaa137；acceptance.json 状态是 completed_with_documented_limits，记录包摘要、同 PID/变量、AppImage 默认失败与兼容模式、MCP 环境限制、清理恢复和最终健康。安装/卸载日志、托盘/MCP/PTY 明细及 22 份私有截图均保留，真实桌面截图不提交 Git。详细记录见[最新 Linux 安装包验收](remote-workspace-validation-2026-10-18.md#最新-linux-ci-安装包与-shell-重新附着验收)。同时下载并校验了服务端产物，但没有运行或部署该 release 服务端。

### 此前：Tailscale 系统安装引导

新增 Web/Tauri 共用的方案审阅、实际命令与脚本展示、确认、过期提示、失败恢复及后台安装状态。Rust 节点使用十分钟一次性方案，执行前重新验证系统条件，先持久保存状态再启动；关闭页面不影响任务，节点重启不重放安装。Linux 使用 PolicyKit/APT，macOS 使用 Homebrew 下载与系统安装器授权，Windows 使用 winget/UAC。容器明确转宿主或已有服务指引，已有可用服务不修改，登录另行触发。详见[平台限制](device-networks.md#安装与系统服务引导)。

全部执行位于上述远端验证容器，使用隔离配置、安装器夹具或模拟 IPC，没有安装真实系统 Tailscale、登录账号或修改主机网络：

| 验证 | 结果 | 范围 |
| --- | --- | --- |
| cranibox-node Rust 单元测试 | 47 通过，2 忽略 | 含新增 4 项方案/状态测试：未确认不执行、错 ID/过期/状态变更拒绝、单次执行、私有记录、失败停止、写盘失败不启动、重启不重放；两个真实 EasyTier 用例本轮未跑 |
| 网络 API 集成 | 17 通过 | 会话/CSRF/同源、拒绝任意程序和下载地址、未审阅拒绝、容器不执行、已有服务不修改，及原网络/邀请/发现回归 |
| Web 网络界面 | 11 通过 | 新增 3 项安装交互；确认、错误保留/重试、刷新恢复、独立登录、容器说明、过期、375/768/1440px、横屏、明暗、减弱动画与 200% 字号无横向溢出 |
| Tauri 前端全套 | 43 通过 | 含新增安装 IPC、官方下载由原生打开、页面切换后台状态；均为浏览器模拟 IPC，非系统安装验收 |

合计 118 项不同自动化测试通过，不与历史轮次相加。严格 Rust Clippy、格式、Web/Tauri TypeScript 与生产资源、工作区及 desktop,custom-protocol 构建另记录为检查项。Web 首次回归 3 项失败是辅助构建仅生成 ui/dist、遗漏发布资源复制所致；按 scripts/build-control-center-ui.cjs 重新生成嵌入资源后 11/11 通过，初始失败日志保留。发布的 web.js/web.css 已随源码更新。截图检查覆盖真实内置脚本在小屏的明暗展示。

证据目录为远端 /config/validation-artifacts/tailscale-setup-20261018；日志、命令、退出码和资源摘要见[专项记录](remote-workspace-validation-2026-10-18.md#tailscale-安装引导收尾)。真实各系统安装、管理员弹窗、macOS 网络扩展、Tailscale 账号与跨物理设备网络仍未验收。远端专项验证没有重跑完整 KDE 镜像构建、Linux 安装包安装或 KDE 桌面捕获，不能沿用旧证据宣称新版本完成这些验收。

**该阶段功能提交 b6ed81e 的完整 CI 已完成，矩阵运行 39101805562 为 7/7 成功**：Linux、Windows、macOS 原生检查与发布构建，三平台 Tauri 安装包，以及独立 Sway/wayvnc 作业。Linux 作业同时通过完整 Web 与后端回归、真实 X11 VNC 和用户态 EasyTier 验证。Release 运行 39101805607 与 Sync to GitCode 运行 39101805611 也均 completed / success，三条运行均核对完整 headSha。元数据保存在上述证据目录的 ci-final.json；这覆盖该阶段代码的自动化检查，不代替 Tailscale 系统安装、目标 OS 交互和 KDE 捕获验收。此次 main 分支 Release 没有发布产品镜像。

### 此前：后台目录同步收尾验证

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

**此前功能代码 71f239755f9a95f528151ddbebf8ec758a866087 的矩阵运行 39096427553 已完成，7/7 作业成功**：Linux、Windows、macOS 原生检查，三平台 Tauri 安装包，以及独立 Sway/wayvnc 桌面验证。对应 Release 运行 39096427579 与 Sync to GitCode 运行 39096427547 也均已完成并成功。Release 已执行严格 Dockerfile/Compose 检查，但此次 main 分支运行没有发布完整产品镜像。最终运行元数据保存在远端 /config/validation-artifacts/full-image-71f2397/ci-final.json；以上结论绑定功能代码 71f2397，不包含此次新增的 Tailscale 安装引导。CI 构建成功仍不等于目标系统安装、托盘、权限与桌面交互实机验收。

### 完整产品镜像与新实例

本轮另在同一远端主机使用 71f2397 的 git archive、独立 BuildKit 构建器和完整 Dockerfile 完成 Linux amd64 KDE 镜像构建，没有叠加旧验证镜像。使用国际软件源，构建、导出和载入成功，约耗时 17 分 50 秒。镜像 cranibox-full-validation:71f2397 的 ID 为 sha256:571d20538e6bae77fbc70cc0209246b4f4c8ca770a6f6f5f96772b8b566a3a04，大小 6,656,198,462 字节，约 6.2 GiB，包含 KDE 和完整开发工具；这不是空闲内存或 CPU 基准。

用空的独立 /config 启动新容器 cranibox-full-firstboot-71f2397，验证默认 Rust 服务 enabled/active、容器健康、Web 与网关、认证后的节点目录 API、内置命令和桌面入口认证均通过，无需手动安装 runtime。再实际重启容器，复验服务，并确认 NodeID、私有运行凭据和项目文件保持不变。初版临时检查脚本误用了独立 Web 的健康响应格式；根据合并服务端的实际协议修正脚本后通过，初始记录保留，没有修改生产代码。

测试容器和专用构建器均已停止，镜像、缓存、配置与证据保留；主验证容器仍 running / healthy，镜像、身份与端口未改动。本项只验证 HTTP/API、服务恢复和数据持久化，没有验证新镜像中的真实桌面画面或输入，也不覆盖其他桌面、架构、镜像源、公开产品镜像发布/拉取及 Windows/macOS 安装。完整命令和证据见[完整镜像构建与首次启动](remote-workspace-validation-2026-10-18.md#完整镜像构建与首次启动)。

### 此前：71f2397 Linux CI 安装包实测

已下载并实际安装功能代码 71f2397、矩阵运行 39096427553 的 Linux 客户端产物 cranibox-node-desktop-ubuntu-latest（Artifact ID 12125116753）。归档 SHA-256 为 0401d3af4ebb9b2192c82def68defe8874317cfc28766ef5d4f5d7f7da7ca929，与 GitHub 元数据一致。在远端 cranibox-validation 的 Ubuntu 26.04 amd64 / KDE Wayland 环境中实际安装 Cranibox Node_0.1.0_amd64.deb，核对 cranibox-node 0.1.0、已安装二进制及实际运行进程；此次运行的是安装包的 release 程序。

使用独立配置和 NodeID，通过真实 Selkies 桌面操作原生窗口，验证界面可见、真实 IPC 复制的 MCP 配置指向已安装程序、关闭窗口后进程保留、托盘重新打开窗口、退出确认的取消与确认、自启动配置启用/禁用。再手动以 --background 重启，验证窗口隐藏、身份持久化及第二次启动唤回原进程。托盘菜单通过与该进程关联的真实 D-Bus 菜单触发；未测试重连菜单。自启文件实际位于 HOME 下的 /config/.config/autostart，而非独立 XDG_CONFIG_HOME，结束时已恢复为禁用。

安装包随后卸载，原验证客户端及原 NodeID 已恢复，临时浏览器、自启项与剪贴板备份已清理；主容器保持 healthy，Web 和桌面认证边界不变。最终 acceptance.json 为 passed: true，证据保存在 /config/validation-artifacts/linux-installer-71f2397。此次是人工集成验收，不累计到上文 34 项自动化测试；未执行 AppImage、实际系统注销/登录、休眠恢复、安装包 PTY/全部业务流程，也未修复 KDE VNC。详见[Linux CI 安装包与原生交互验收](remote-workspace-validation-2026-10-18.md#linux-ci-安装包与原生交互验收)。

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
| 实际桌面 | 真实 X11/Xvfb+x11vnc、隔离 Sway+wayvnc，以及本轮 KDE/Selkies 节点中继的画面、完整输入、重连和撤权 | KDE 的 VNC 捕获仍失败；Windows/macOS 原生桌面未验收 |
| 原生壳 | 6396b93 三平台安装包 CI 成功；同提交 Linux .deb/AppImage 实际运行、MCP 与 XDG 自启开关、.deb PTY 保留、AppImage GIO 后台启动和单实例通过 | 无 FUSE 兼容参数已在新包实测；普通 FUSE、实际系统登录/休眠、真实网关故障及 Windows/macOS 仍待实测 |

### 明确未通过或尚未验证

- **KDE Wayland VNC 未通过**：w0vncserver 密码和 portal 授权成功，但 KDE 屏幕捕获报 failed to start streaming / 不支持的合成类型，在 ServerInit 前断开；隔离 KWin 回退 QPainter，没有可用 OpenGL 画面。临时进程与密码已清理。当前继续用 Selkies，不得标记 VNC 已完成。
  - 收尾时再次只读检查：当前 KWin 明确报告 Compositing Type: QPainter，容器没有 /dev/dri；内层 wayland-0 与 Selkies 外层 wayland-1 都未公布 wlr screencopy 或 ext image-copy-capture 协议。外层仅有虚拟键盘、没有虚拟指针，不能直接改用 wayvnc 共享当前桌面。该检查没有修改或重启合成器，也没有重跑 VNC；原始协议清单及 KWin 信息保存在 /config/validation-artifacts/kde-capture-followup-20261018。本轮已独立实现并验证 Selkies 节点接入；若继续做 KDE VNC，仍需具备可用捕获后端的图形环境，不要放宽检测。
- Windows 原生与 macOS 的安装包、托盘、登录自启、权限提示、VNC、进程树、断网/休眠恢复、容器安装向导尚未实机验收。WSL2 不能替代 Windows 原生。
- 真实 Tailscale ACL、两台物理设备互访及日常网络切换未验收；本轮没有连接真实 Tailnet。
- 完整 Linux amd64 KDE 镜像构建及新实例首次启动、重启已通过；公开产品镜像发布/拉取、其他桌面/架构/镜像源，以及该新实例的实际桌面画面与输入尚未验证。
- 未调用真实 Codex/Claude 模型，未验证订阅登录；后台执行适配器仍需端到端实际工作任务验收。
- 浏览器与 IPC 测试中的模拟响应、进程替身和独立 RFB 服务，均在各专项记录中注明，不能提升为实机验收。
- 本轮 3c1a38d 的完整 CI 最终 7/7 通过，Linux .deb/AppImage 的真实 Computer Use、原生 IPC 与 MCP 已人工补验；较完整的托盘、自启与终端流程仍绑定此前 6396b93。最新包的正常桌面集成、普通 FUSE、Linux 实际系统登录/休眠、真实网关网络故障及 Windows/macOS 安装与交互仍未验收。

## 下一阶段优先级与验收条件

| 顺序 | 工作 | 完成判据 |
| --- | --- | --- |
| P0 | 桌面互访可日用 | KDE/Selkies 已通过节点中继验收；继续完成 Windows/macOS 的安装、录屏/输入权限、启停、退出清理，并从另一个物理设备真正看见画面、输入及撤权断开 |
| P0 | 跨平台安装与产品镜像发布验收 | 3c1a38d 的 7 项 CI 通过，Linux 两包新增 Computer Use、真实原生 IPC/MCP 已补验，托盘/自启等较完整流程仍绑定 6396b93。继续 Windows/macOS、最新包正常桌面集成、普通 FUSE、实际登录/休眠及网关故障。完整 KDE 镜像与新实例启动/重启仍绑定 71f2397，公开镜像发布/拉取待完成 |
| P1 | 维护者的真实三设备流程 | Mac 委托 Linux/Windows Agent，浏览器关闭后继续，换入口回复并核对结果；覆盖断网、休眠、重启，不依赖同容器替身或人工改账本 |
| P1 | Tailscale 安装与登录实机验收 | 在干净的 Windows、macOS、Debian/Ubuntu 图形主机上审阅方案，检查管理员授权、取消/失败恢复、系统服务启动、独立登录及真实 Tailnet 互访；不以单元夹具和模拟 IPC 代替 |
| P1 | 完整统一视图与数据同步 | 明确设备/项目/Agent/任务各数据的权威、版本、删除与冲突协议；另一入口无需逐一手配才能看同一授权范围；保留设备撤权语义 |
| P1 | 设备信任与能力接入 | 定义公钥配对、签名身份、凭据轮换、加入/撤销与跨平台提供者协议；Computer Use 已有 Linux 真实桌面证据，继续多物理设备、模型流程与 Windows/macOS 提供者，并独立实现 Browser Use |
| P2 | 真正 Agent 会话接续与迁移 | 针对具体 harness 设计可恢复检查点、项目文件/上下文一致性与目标启动；源端停止和目标继续有明确状态，不能声称移动原进程 |
| P2 | 交互和规模优化 | 在真实用户流程上检查导航、状态密度、长清单和错误恢复；评估大量设备与文件的资源消耗；复杂编排按日常需求推进 |
| P2 | Wayland CI 认证超时诊断 | 保留 wayvnc/neatvnc 与客户端握手诊断，定位本轮 5 秒等待超时；不跳过错误凭据测试或将重跑成功表述为时序问题已修复 |

暂不需要为了节点身份等同引入选主；目前数据由所属节点持有，界面连接任一运行入口。若未来要复制协作协调状态，先定义故障与冲突模型，再决定是否需要一致性协议。

## 接手与复现

### 阅读顺序

1. 本文与 [远端验证记录](remote-workspace-validation-2026-10-18.md)：先了解完成与失败边界。
2. [产品与分布式设计](distributed-agent-workspace-design.md)、[组网设计](decentralized-node-network-design.md)：目标架构，不将提案当成现有功能。
3. [节点直连](direct-node-peers.md)、[设备网络](device-networks.md)、[客户端](node-desktop-and-device-mesh.md)。
4. [远程桌面](remote-desktops.md)、[Computer Use](computer-use.md)、[虚拟空间](virtual-spaces.md)、[项目目录与复制](project-session-catalog.md)。
5. [消息](device-messages.md)、[任务与自动接单](device-tasks.md)、[会话](agent-sessions.md)、[后台执行](agent-executions.md)。

### 源码入口

| 位置 | 内容 |
| --- | --- |
| crates/cranibox | 合并服务端入口 |
| crates/workspacectl、crates/workspace-services | Rust 控制工具与服务兼容管理 |
| crates/workspace-runtime/src/peers、projects | 连接、目标授权客户端、消息/任务/项目账本、文件复制与后台同步配置 |
| crates/workspace-node/src/{local.rs,peer,network,spaces,supervisor.rs,task_worker.rs,project_sync.rs} | 节点宿主、直连服务、网络/空间、进程、自动接单与后台目录调度 |
| crates/workspace-node/src/network/tailscale_setup.rs、install-tailscale*.sh | Tailscale 十分钟一次性方案、后台状态记录、平台安装/启动；不会自动登录或重放未完成安装 |
| crates/workspace-web | 登录/CSRF、Web API、终端/桌面和空间应用转发 |
| crates/workspace-runtime/src/peers/desktop/selkies.rs；addons/control-center/ui/src/desktop/{openDesktop.ts,SelkiesViewer.ts} | Selkies WebSocket 适配、受限输入、首包协议选择、JPEG 解码与键鼠；真实桌面回归为 tests/test_desktop_real_selkies.py |
| crates/workspace-runtime/src/peers/computer.rs；crates/workspace-node/src/peer/server/computer.rs | 有界桥接、类型化动作、目标 MCP；短期控制权、鉴权复查与停止 |
| addons/control-center/ui/src/devices/ComputerSharingPanel.tsx；tests/test_computer_use.py | Web/Tauri 共用桥接管理；真实节点/桥接/Web 与可控原生动作夹具 |
| addons/control-center/ui/src | React 共用业务组件与浏览器适配器 |
| apps/node-desktop | Tauri 原生宿主、IPC、托盘与客户端适配器 |
| apps/node-desktop/src-tauri/src/launch.rs | 持久启动器、AppImage MCP 参数、Linux 自启动路径与转义；纯 Rust 库单元测试和真实 GIO 用例 |
| tests、两端 UI 的 tests | Python 真实服务集成和 Playwright；模拟与实机条件见各文件 |
| .github/workflows/rust-runtime.yml | 三平台 Rust/安装包、Linux 服务与浏览器、独立 Wayland 验证 |

### 远端环境

- SSH：admin@192.168.217.6，端口 22022，容器 cranibox-validation；不得改用当前本机做运行验证。
- Web：https://192.168.217.6:3080；KDE/Selkies：https://192.168.217.6:3031。入口检查使用上述 LAN 地址，主机 127.0.0.1 的相同端口拒绝连接，不代表服务停止。凭据只在远端私有 container.env，不写入 Git、测试报告或命令行。
- 容器源码 /config/agent-workspace-manager/source；Cargo 目标 /config/validation-build/target；证据 /config/validation-artifacts。源码和验证数据均在远端持久卷。
- 主 Web 与原生客户端是两份独立身份；原生使用 /config/validation-client/{config,data,state}。保持分离，避免两个进程共用 NodeID。
- 最新 Linux 包与真实 Computer Use 证据在 /config/validation-artifacts/linux-installer-3c1a38d；较完整托盘/自启等流程在 linux-installer-6396b93；此前包在 linux-installer-afaa137 与 linux-installer-71f2397，启动修复专项在 linux-launcher-20261018（均位于同一证据根目录）。测试 .deb 已卸载，日常客户端仍为 /config/validation-client/bin/cranibox-node-desktop，PID 767446，原身份和配置不变。6396b93 已验证遵循 XDG_CONFIG_HOME，关闭后两处均无启动项；本轮未改变自启配置。
- 主容器不挂宿主 Docker socket，也不是 privileged。虚拟空间用过独立临时管理容器；它和两个测试空间目前停止，保留数据供复核，不要误删。
- 完整镜像验证另留 cranibox-full-validation:71f2397 镜像、已停止的 cranibox-full-firstboot-71f2397 容器与 cranibox-validation-full-build-20261018 构建器缓存。证据在 /config/validation-artifacts/full-image-71f2397；首次启动环境文件含私有凭据，不要输出或提交。停止后的容器可能仍显示 unhealthy，应以 firstboot.json 中停止前健康检查及重启结果判断本次验收。
- 其余候选设备：Windows/WSL2 192.168.217.226；macOS fliaping@192.168.217.85。macOS 当前拒绝现有 SSH 公钥，Windows 原生登录入口尚未提供；已请求可用账号、端口和密钥文件路径，尚未取得。不得把 WSL2 或 CI 代替原生交互验收，接手时先核对访问与图形登录环境。

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
python3 -m unittest test_computer_use test_node_peers test_remote_desktops test_desktop_relay test_desktop_service test_web_direct_peers test_agent_runs_mcp test_task_worker_mcp -v
~~~

完整界面回归为 npm test --prefix addons/control-center/ui 与 npm test --prefix apps/node-desktop。UI 使用仓库 Playwright 配置与安装好的 Chromium；本轮为复用远端系统 Chromium 临时指定 /usr/bin/chromium 和 --no-sandbox，临时配置不提交。CI 按工作流安装浏览器。Web 最新完整 122 项及客户端完整 45 项已执行；新增专项位于 Web 的 computer-use.spec.ts 与客户端 client.spec.ts；真实 KDE 测试的显式启用和图形会话要求见[远程桌面](remote-desktops.md#验证范围)。此前目录相关用例为 Web 的 project-sync.spec.ts、federated-projects.spec.ts、projects.spec.ts。

先构建两端前端，再构建嵌入静态资源的 Rust Web 与原生程序；直接 Cargo 运行原生客户端必须启用 desktop,custom-protocol，避免误连已经关闭的 Vite 开发服务。完整构建及 CI 命令以功能文档与工作流为准。

后续模型应先阅读本文最新收尾结果、对应提交的 CI 与本表 P0；本轮 Computer Use 的提交与 CI 记录见最新专项。仅更新交接文档的后续提交不重新触发该路径过滤矩阵。如又修改代码，再按新提交复核。选择一个日常流程完成，不要继续无界扩展功能，也不要在没有新证据时把“模拟 IPC 通过”或“进程仍存在”升级为完整产品验收。
