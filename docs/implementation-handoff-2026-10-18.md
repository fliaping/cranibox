# Cranibox 功能、验证与后续交接

整理日期：2026-10-19，文件名沿用原日期。本文是下一个模型的接手入口：用户确定的方向、功能状态、按版本区分的测试、未完成项及复现方法。历次成功/失败与诊断见[远端验证记录](remote-workspace-validation-2026-10-18.md)及 Git 历史；设计提案不等于功能完成。

本次基于 **a31164505abc476db04145a4a2480161810f299c**，保留并行合入的端到端通道、入站策略和审批队列，修复私钥缺失、自签名替换、v1 降级绕过和 Noise 公私钥不一致四个问题。当前 Rust 工作区 **209 通过、3 默认忽略**，七组协议集成 **61/61，无跳过**，fmt、Clippy 与五个程序构建通过。前端和 Linux 安装包的历史验收仍绑定其原版本。

原始需求尚未全部完成，优先剩余项见 P0/P1/P2。日常 Web/客户端与 KDE/Selkies 未升级或重启；升级前须备份配置和密钥，详见[节点身份与恢复说明](node-identity-and-e2e.md)。

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

## 功能需求与当前实现状态

“已实现”表示代码存在并有相应验证，不表示所有平台与生产场景均已验收。

| 需求 | 当前实现 | 边界与后续 |
| --- | --- | --- |
| Rust 控制工具与常驻服务管理 | Rust workspacectl；Rust 服务兼容入口配合 s6 监督；五项项目服务已有范围受限的切换记录 | 第三方业务未改写；init/custom-services/systemctl-services 使用短时 oneshot；服务切换记录来自此前开发容器，不等于所有部署实例已迁移 |
| 合并服务端应用、独立发布客户端 | cranibox 合并 Web、节点网关与本机节点；保留独立 cranibox-node 与 Tauri 应用，兼容原入口 | 一份身份配置只允许一个节点宿主进程；GUI 程序兼容 CLI/MCP 子命令 |
| WebSocket 远程命令与 Agent 接入 | 节点出站连接、组授权、CLI/MCP、网关转发、重新连接 | 网络重连不重放普通命令；网关路径是可选通道 |
| 无中心的节点互访 | 持久 NodeID、目标签发凭据、独立 HTTP/WebSocket、能力分别授权；Workspace 停止后仍可直连；持久 Ed25519/X25519 身份、新 ID 公钥派生、旧 ID 保留和授权摘要/签名；本次修复私钥锚定、降级绕过和 Noise 一致性 | 后端单元与隔离节点协议回归见下节，不是跨物理设备验收；本机授权签名不等于全配置签名或签名目录 |
| 节点加密与配对 | a311645 合入 Noise IK HTTP 通道、双向 TOFU 公钥固定、CLI 配对码核对、可选 require-verified，以及终端/桌面/浏览器的加密 WebSocket 数据帧 | 旧目标回退、兼容 bearer 与旧网关流保留；首次 TOFU、静态密钥泄露及元数据有边界，需核对严格授权和所有实际入口，见身份文档；不是外部安全审计 |
| Agent 入站策略与审批 | 新授权默认 hold；accept/hold/refuse、限速、循环保护、不可信正文封装；本机 CLI/workspacectl/Web 放行与拒绝，MCP 不能审批 | 旧授权保持 accept；同一系统用户的本机恶意进程不在隔离范围。最新审批 UI 的真实桌面验收未完成 |
| EasyTier 与 Tailscale 同时使用 | EasyTier 固定版本安装、用户态网络、节点发现、端口转发、多个入网地址、短期邀请与丢响应恢复；系统 Tailscale 安装/启动方案审阅与确认、后台进度、官方登录、持续发现与授权连接 | Tailscale 是系统服务，不由节点长期托管；自动安装有平台前置条件，容器转宿主/已有服务指引；真实系统安装与 Tailnet 跨设备验收未完成 |
| 同一设备多条路径 | 保存并排序显式路径，新连接按可用性选择，认证/身份错误不继续回退 | 已建立连接不无缝迁移；桌面直连与网关通道仍显式选择 |
| 浏览器和原生端统一体验 | 项目、设备、网络、消息、任务、终端、VNC 与空间共用 React 组件，分别适配 Web 和 Tauri IPC | 不是全部管理页面完全一致，也不是节点数据自动全量复制 |
| 常驻托盘与登录自启动 | Tauri 窗口关闭到托盘、重连、明确退出、用户登录后自启动配置 | 84454e6 的 Linux .deb/AppImage 已通过正常 KDE D-Bus 的托盘注册/唤回、Wayland 窗口关闭到托盘、单实例、退出确认、自启开关及 GIO 启动项静默启动，重启身份保持；配置使用隔离 XDG，AppImage 使用无 FUSE 兼容模式。实际登录、休眠、网关断线恢复及 Windows/macOS 仍待实测 |
| 持久终端与浮动窗口 | 可变大小、最小化到状态栏、多个会话、重新附着；服务端使用独立 Herdr，客户端使用原生 PTY | 6396b93 .deb 关闭浮动终端、收起原生窗口及操作托盘重连后，Shell PID 与启动时间不变；此前 afaa137 另验证导出变量保留。没有制造真实网关网络故障，客户端进程或系统退出后不能保留该 Shell |
| 桌面互访 | 共用 VNC/noVNC 与 Selkies/JPEG 查看器；已有本机服务的 IPv4/IPv6 检测、配置核对及保存前复查；本机、直连及出站 WebSocket 中继；独立授权、单控制者、停止与撤权取消；文字剪贴板支持主动发送/复制、64 KiB 限制和只读 | KDE 容器 Selkies 的真实画面、完整文字输入、重连和撤权通过；Web 双向剪贴板通过；a8a97f5 原生反向获取失败，2026-10-19 修复后的 debug 及同提交 CI .deb/AppImage 的真实双向文字、持续更新、重连清空均通过。两包为私有 D-Bus 验收，未重验正常托盘与撤权。剪贴板不自动粘贴，不传图片/文件，旧 VNC 的 Unicode 取决于服务端。Windows/macOS 实机及 KDE/GNOME 的自动安装与托管未实现 |
| 内置 VNC 托管 | Linux X11 使用 x11vnc，兼容 wlroots 的 Wayland 使用 wayvnc；守护、失败退避、节点重启恢复；Ubuntu/Debian 主机安装方案一次性确认、私有状态记录和重启中断提示 | 系统安装与共享分开；退出/超时不主动杀安装进程，不保证系统事务完成，真实认证与包安装尚待验收。不是纯 Rust 跨平台捕获；KDE 容器实测 VNC 屏幕捕获失败，Selkies 可用 |
| Windows/macOS 桌面准备 | Web/Tauri 共用三步向导；UltraVNC Server / macOS 屏幕共享的官方入口、回环端口检查、配置核对和授权跳转；失败重试、IPv6、焦点和窄屏 | 安装和系统权限仍由用户手动完成；只检查 RFB 握手，不验证认证、画面或输入。WSL2 不代表 Windows 原生环境；macOS 系统服务的局域网访问需单独限制，Cranibox 不修改监听或防火墙 |
| 虚拟空间 | 检测、十分钟一次性安装方案、命令复查、退出/超时中断提示、镜像策略、创建/停止/启动/归档/恢复、资源限制、独立身份与持久卷；失败/中断/归档后可修改镜像并重试，已有容器需先归档；桌面/编辑器独立应用入口 | 真实 Docker 生命周期与浏览器流程已有历史验证；新增恢复场景使用真实 Rust 服务与 Docker 夹具。981ff86 的六组完整镜像均通过首次启动、授权与重启。默认公开标签的两架构缺少空间协议标签，不能用于新建空间；需发布兼容镜像并验收，各 OS 系统依赖安装也未验收 |
| 产品镜像多架构发布 | 默认只验证；amd64/arm64 原生验证可分别选择；正式发布与阿里云镜像均要求两个架构的首次启动、授权及重启通过；按候选 index 与平台 manifest 摘要绑定 | 981ff86 六组只验证运行 39208494283 已 completed/success；六份报告全部下载审计，各 44/44、无未就绪项、四项清理通过。公开发布与真实拉取尚未验收；只读 Release 修正已在 CI 成功 |
| 统一项目与会话目录 | ProjectID、节点工作副本、Shell 关联、网关/直连汇总；增量目录读取、删除处理、只读离线快照 | 各节点保留本地权威；不是全网数据库复制，不支持离线修改后自动合并 |
| 关闭页面后的目录同步 | Rust 节点后台同步项目目录/会话关联与公开 Agent 登记；两套独立设置，明确选择 1–32 个已接入设备，15/30/60/300 秒，共用四路后台并发；暂停和重启恢复 | 默认关闭；只读离线快照不授予执行或当前 Agent 身份，不复制工作文件；凭据/连接变更后需要重新选择 |
| 项目迁移与接续 | 本机与直连双向、两个远端之间快照复制；清单确认、分块校验、暂停/恢复、持久接收回执；保留 ProjectID 与接续说明 | 当前是显式快照复制，非持续文件同步；上限 2 GiB/单文件 1 GiB，实际集成传输约 20 MiB，未做上限压力测试 |
| 多 Agent 通信与任务 | 持久消息、线程回复、确认、离线投递、任务委托/领取/进度/问题/回复/结果/合作取消，CLI/MCP 与共享界面 | 不能由普通消息自动启动进程；同一容器中的独立身份验证不等于多台物理设备验收 |
| Agent 会话与后台执行 | 主动登记/续期、汇总授权会话、后台目录同步与离线标签、定向委托；Codex/Claude Code 非交互适配、启动检查、持久输出与停止；可选独立执行 MCP | 离线登记不代表当前状态，不能据此选择执行 Agent；模型调用与 CLI 登录未实测；不支持运行中追加交互输入、模型检查点恢复或进程迁移 |
| 多设备协作与自动接单 | 持久协作 Run、任务汇总、统一停止、核对归档、最多八项前置任务；接收端可明确启用按可信发送方和固定目录的后台 worker | 默认关闭；不是通用复杂编排平台；人核对模式与显式自动提案模式分开，退出码不直接等于任务成功 |
| 跨设备协作记录统一视图 | Web/Tauri 共用任务/报告/问答/结果、消息与 Run 来源视图；独立 activity 只读授权；可见单选筛选、搜索/分页及默认关闭的后台同步、私有离线快照、暂停/清空 | 页面最多选择 16 个来源，后台策略最多 32 台；包含其他发送方正文，需明确选择才缓存。快照绑定连接和凭据，拒绝/无效响应后清除；离线期间无法立即知道撤权。来源报告不合成全局状态，无权威账本复制或新增 MCP。9fd1b25 的 GTK debug 的类型选择、Run 与分页通过，原生搜索输入未通过验证；历史安装包结果不覆盖新控件和后台设置 |
| 各节点 Agent 共享 Computer Use | 独立 computer 授权、目标配置的回环桥接、直连 API、短期输入控制权、持久停止、独立目标绑定 MCP；Web/Tauri 共用配置与停止界面 | 复用现有 bridge，不安装跨平台驱动；只走直连。12 项后端专项使用动作夹具；a8a97f5 .deb 另通过真实 KDE 截图、输入、MCP、重启保留停止，AppImage 通过 GUI 节点宿主及真实 MCP 截图；管理面板完整停止/恢复 IPC 的两包验收仍绑定 3c1a38d。多物理设备、模型调用及 Windows/macOS 提供者待验收 |
| 各节点 Agent 共享 Browser Use | 独立 browser 授权、目标回环 Chromium CDP、鉴权 WebSocket、单远程控制者、跨重启保留停止；调用方经临时回环代理运行固定版本官方 MCP，Web/Tauri 共用管理 | 真实 Linux Chromium 的打开页面、快照、填表、提交、PNG 与停止通过；只走直连。授权覆盖共享浏览器全部标签页和会话；无自动安装/启动、域名隔离或旧网关转发。Windows/macOS、跨物理设备及真实模型待验收 |
| agent-desktop-bridge 是否可删除 | 现有桥接继续保留，并作为 Computer Use 的明确配置提供者；VNC 是桌面传输通道 | cua-driver 替代方案仍需独立核对接口、平台和权限，当前不能删除桥接 |

五项服务是 agent-desktop-bridge、deepseek-harness、openclaw-gateway、proxy-caddy、tailscaled-workspace。其监督方式迁移不代表业务健康：原记录中桌面桥接 health 为 503、Tailscale 为 NeedsLogin，均不能计作迁移修复。见 [Rust 服务与迁移记录](native-services-and-web.md)。

## 测试情况与可信范围

### 当前源码：a311645 加本次身份修复（2026-10-19）

全部运行在指定 **admin@192.168.217.6:22022 / cranibox-validation / abc**；独立源码 **/config/validation-sources/identity-a311645**。本机只编辑、静态核对、传输和 Git。私有证据 **/config/validation-artifacts/identity-handoff-20261019/merged-a311645** 保存命令、耗时、日志及开始/结束源码摘要。

| 检查 | 结果 | 范围 |
| --- | --- | --- |
| Rust 工作区 | **209 通过、3 默认忽略，75.671 秒** | cargo test --locked --offline --workspace --exclude cranibox-node-desktop；18 个目标，包含 runtime 130 项，不重复相加 |
| fmt / Clippy | **通过，1.418 / 29.843 秒** | fmt --all --check；工作区排除 Tauri、all-targets、-D warnings |
| Rust 程序构建 | **通过，72.367 + 27.879 秒** | cranibox、cranibox-node、workspace-runtime、workspace-web；另外构建同版本 workspacectl |
| 七组协议集成 | **61/61，无跳过，199.355 秒** | 节点、任务、消息/审批、本地与远程 Agent 执行、后台协作记录、E2E；同容器真实服务 |

首次协议集成为 **60 通过、1 失败，193.958 秒**：新审批测试调用了共享目标目录中旧版 workspacectl，报 Unknown command: messages。构建同版本工具并明确 WORKSPACE_CTL_BIN 后完整重跑 **61/61**，不改产品或测试断言，原失败日志保留。四项 E2E 集成覆盖中继只见密文、双方固定密钥、严格配对/拒绝明文、公钥更换、篡改/重放/未知会话；WebSocket 加密帧另有 Rust 单元测试，不等于所有入口强制加密或外部安全审计。

三项默认忽略为两项真实 EasyTier 与传输资源测量，本轮未显式执行。未重跑 Tauri Rust 测试、全前端、最新审批 UI 的浏览器/真实窗口、旧二进制历史账本逐类升级、跨物理设备及 Windows/macOS 实机。此前的绿色结果只证明相应版本，不作当前安装包或跨平台验收。

2026-10-19 **07:55:24 UTC** 最终健康检查：容器 healthy、非 privileged、无宿主 Docker socket；3080/3031 匿名 401、认证 200。日常 Web/客户端的 PID、程序摘要与 NodeID 保持，KDE/Selkies 未重启；本轮夹具进程扫描为空，无测试安装包。详见验证记录最新章节。

提交前 a311645 的 Release **39217247654** 已成功，产品矩阵 **39217247582**、GitCode **39217247378** 仍排队，均不含本次身份修复。推送后用最新完整提交查询已有运行：gh run list --repo fliaping/cranibox --commit <完整提交>；CI 未完成不能记为通过，也无需重复派发。下方旧 CI 表仅作历史索引。

### 合并前基线：节点身份与迁移补验（基于 79f3e4a，2026-10-19）

全部运行在指定 **admin@192.168.217.6:22022 / cranibox-validation / abc**。从 79f3e4a 归档到独立 **/config/validation-sources/identity-79f3e4a**，只覆盖本次两个 Rust 文件与一个 Python 回归文件；没有改写日常身份或升级日常程序。

| 检查 | 结果 | 范围 |
| --- | --- | --- |
| 修复前回归 | **116 通过、4 失败，12.739 秒** | 五个新增用例叠加原生产代码；四项失败分别证实缺失私钥、自签名替换、v1 降级绕过和 Noise 不匹配，中断迁移用例已通过；原始日志保留 |
| runtime 单元测试 | **120/120，31.550 秒** | cargo test --locked --offline -p agent-workspace-runtime --lib；包含五个新增用例 |
| Rust 工作区 | **199 通过、3 默认忽略，140.552 秒** | cargo test --locked --offline --workspace --exclude cranibox-node-desktop；18 个目标，包含上述 runtime，不重复相加 |
| fmt / Clippy | **通过，1.031 / 30.999 秒** | fmt --all --check；工作区排除 Tauri、all-targets、-D warnings |
| Rust 四程序构建 | **通过，20.064 秒** | cranibox、cranibox-node、workspace-runtime、workspace-web；后续协议测试使用这些新程序 |
| 六组协议集成 | **55/55，无跳过，185.969 秒** | test_node_peers、test_device_tasks、test_device_messages、test_agent_executions、test_remote_agent_executions、test_background_activity_sync |

隔离节点启动时真实迁移 v1、沿用旧令牌执行命令，私钥丢失后 HTTP 503、CLI 失败且不生成替代文件，恢复原密钥后可访问；降回 v1 同样拒绝，恢复签名配置并重启后身份与执行保持。单元测试另覆盖有效自签名替换、Noise 不匹配，以及中断于生成密钥后、保存旧配置前仍可沿用原密钥完成迁移。首版无检查点的密钥文件可兼容。成功保存 v2 后才记录最低配置版本，两个文件并非同一原子事务；检查点保存前中断的边界及有效旧 v2 记录重放限制见身份文档。

私有证据 **/config/validation-artifacts/identity-handoff-20261019** 保存命令、失败/成功日志、耗时和源码摘要。该轮明确不含后来 a311645 的协议和 UI；旧二进制历史账本的逐类回放、原生三平台安装包与跨物理设备仍待补验。

### 此前：协作记录筛选与原生补验（基于 95cde28，2026-10-19）

按 UI/UX Pro Max 规则将四种类型直接展示为原生 radio/fieldset，保留可见焦点、至少 44px 的标签点击区域、窄屏换行和减少动画设置。Web/Tauri 共用实现，修正原生全局表单样式对布局和 radio 尺寸的影响。没有新增依赖或修改 Rust 协议。

| 检查 | 结果 | 范围 |
| --- | --- | --- |
| Web 资源 / 原生前端 | **通过，12.660 / 12.265 秒** | 生成受版本控制的 web.js/web.css；原生 Vite 资源用于随后构建 |
| Rust Web / 原生程序构建 | **通过，10.511 / 82.291 秒** | cranibox；cranibox-node-desktop 的 desktop,custom-protocol debug 构建，后者包含等待 Cargo 锁 |
| Web tasks.spec.ts | **19/19，39.775 秒** | 搜索、报告、类型切换、空格/方向键、分页重置；375/768/1440 深浅主题的宽度和 44px 断言 |
| Tauri client.spec.ts | **56/56，70.764 秒** | Chromium 模拟 Tauri IPC，新增类型方向键和搜索回归，不是 GTK 键盘实测 |
| 真实 GTK 窗口 | **限定流程通过** | 私有 D-Bus、隔离 XDG、KDE 的 Xwayland；通过 AT-SPI action 选择任务/Run，读取任务问答/结果与 Run 目标/结果，24 条真实协议记录按 20/4 分页，筛选后重置 |
| 真实 GTK 搜索输入 | **未通过验证** | 两次定向键盘尝试后输入框仍空白；不能判断为搜索逻辑已坏，也不能写成原生搜索可用 |

窗口读取同容器内另一独立节点的 1 个任务、22 条消息、1 个已关闭 Run；数据通过真实 Rust CLI/HTTP 创建，worker 为脚本，未调用模型。GUI 程序摘要为 **46e04fb2ba74442ce4e6989a14314e06e6fb32db2a475c65c5d34510b03e5000**。这是 debug 程序，不是最新 CI 安装包，也不是正常登录/托盘或跨物理设备验收。原生类型选择由可访问性 action 完成，不能替代普通键鼠验收。

四个源码文件在全部六轮构建/回归开始与结束时摘要保持不变，并与当时本机一致。9fd1b25 的受测 web.js SHA-256 为 **cbf4f2f98842996f16ffe803f96d69d31ef9ac3ef7565ab3bb63d5906fdabf82**，web.css 为 **add1e00311a5135b32dc8fe0bf221c47be7330833065166dc49b35fca226f626**，已取回提交；32522b6 随后改变 web.js，不能沿用该摘要为最新资源证明。此次没有重复全 Web 143 项或未改动的 Rust 测试。依赖的 use client 忽略与 Vite 大包提示仍存在。

私有证据为 **/config/validation-artifacts/activity-kind-controls-20261019**：六份构建/测试报告、native-steps 的成功和失败记录、截图、原生配置与最终健康报告。deb-native 仅沿用辅助脚本的目录名，本轮没有安装 .deb。分页最初因 AT-SPI 不暴露内联页码而断言失败，另一次 Run 计数断言误用了文案；改用实际 article 数和正确文案确认通过，原失败保留。测试 GUI、私有辅助进程与目标已清理；3080/3031、日常 Web/客户端及 KDE/Selkies 进程、程序摘要和 NodeID 均保持，无测试安装包。

### 上次 Rust 验证：Selkies 慢接收者测试同步（5af2b53，2026-10-19）

旧夹具收到 cr 后就通知测试恢复读取，此时 clipboard 响应尚未发送；在途视频帧因此也被计入剪贴板排队，2299b45 的 Linux CI 曾计到 12 帧。现在先发送剪贴板和带标记的 Ping，收到适配器返回的匹配 Pong 后才恢复输出消费。WebSocket 消息有序，适配器必须在输出仍阻塞时读取响应及后续 Ping；如果上游不再被排空，此测试仍会超时。生产代码、500 帧负载、<10 帧断言与原三秒期限均未改变，没有增加固定等待。

| 检查 | 结果 | 范围 |
| --- | --- | --- |
| Rust 工作区 | **187 通过、3 默认忽略，39.939 秒** | 18 个目标，--locked --offline --workspace --exclude cranibox-node-desktop；包含修正用例 |
| 慢接收者重复验证 | **30/30，37.413 秒** | 同一精确用例串行运行 30 次，均实际执行一项；不计为 30 个不同功能测试 |
| 格式检查 | **通过，0.983 秒** | cargo fmt --all --check |
| 工作区 Clippy | **通过，20.537 秒** | --locked --offline --workspace --exclude cranibox-node-desktop --all-targets -- -D warnings |

修正文件 SHA-256 为 d8d9a9860ab248df940a53517df21e184d55eba3e1722a1ca947cfb94b991b82。四项报告均记录关键源码开始/结束摘要，source_unchanged=true，本机与远端逐项一致；静态核对生产部分与 66edf11 相同。最初 fmt 因长 if 的换行失败（1.056 秒），仅按提示换行，原失败报告保留。未重复执行未变化的 Web/Tauri 界面、系统安装或真实桌面测试；既有功能回归仍绑定下节来源。私有证据为 /config/validation-artifacts/selkies-ordering-20261019，最终报告在 final/。

### 镜像与 CI 交接（截至 2026-10-19 07:06:51 UTC）

| 项目 | 已核对结果 | 后续 |
| --- | --- | --- |
| Release / GitCode，981ff86 | 39208300414 / 39208300409 均 completed/success | 只读检查已落地；不代表公开镜像发布 |
| Release / GitCode，4fb6a41 | 39210815260 / 39210815816 均 completed/success | 包含 5af2b53；之后仅改交接文档 |
| 产品矩阵，4fb6a41 | 39210815263：Linux/Windows 四项和真实 Wayland 成功，macOS 两项运行 | 继续核对七项终态；不含本轮控件修改，推送后核对新来源；不能以构建代替实机验收 |
| 历史产品矩阵，66edf11 | 39207809642：06:40:57 快照为 5 成功、1 运行、1 排队 | 保留历史快照，不含 5af2b53 或本轮控件修改 |
| XFCE linux/amd64，981ff86 | **44/44，65.785 秒**；artifact 12166813629；四项清理通过 | 仅构建与启动验收，无真实 GUI |
| XFCE linux/arm64，981ff86 | **44/44，63.563 秒**；artifact 12166139955；四项清理通过 | 仅构建与启动验收，无真实 GUI |
| LXQt linux/amd64，981ff86 | **44/44，67.300 秒**；artifact 12167462140；四项清理通过 | 仅构建与启动验收，无真实 GUI |
| LXQt linux/arm64，981ff86 | **44/44，63.869 秒**；artifact 12166694684；四项清理通过 | 仅构建与启动验收，无真实 GUI |
| KDE linux/amd64，981ff86 | **44/44，67.488 秒**；artifact 12167750579；四项清理通过 | 仅构建与启动验收，无真实 GUI |
| KDE linux/arm64，981ff86 | **44/44，66.051 秒**；artifact 12167705546；四项清理通过 | 仅构建与启动验收，无真实 GUI |

六份启动报告的 revision/expected_revision、platform/expected_platform 与完整来源一致，检查名 44 项不重复、passed=true、unready_checks=[]；每份的两个容器与两个卷均 removed=true。作业的 Docker Hub 登录与公开标签推广均 skipped，push-aliyun 跳过。原报告摘要见[六组镜像最终审计](remote-workspace-validation-2026-10-18.md#2026-10-19六组镜像最终审计)。快照与报告存入同一私有交接证据目录；当前公开默认镜像的兼容性、正式发布与拉取仍为 P0。

### 功能回归：协作记录后台同步（66edf11，2026-10-19）

针对关页面后仍需查看各设备任务、消息与 Run 的需求，新增独立 activity_sync 策略。设备侧必须允许 activity 只读能力；messages 授权不会自动扩大。后台最多选择 32 台设备，与项目及 Agent 同步交错调度，共享四路并发。Web/Tauri 共用设置：默认折叠、明确告知缓存完整正文，支持间隔、设备、暂停和清空；CLI 为 `cranibox peer activity-sync`。协议及命令见[协作记录](federated-activity.md)。

暂停保留设备和快照，停止新的后台读取；已选设备的手动读取仍可更新快照。清空会停止同步并删除快照；移除设备、忘记连接、变更路径或凭据也会删除对应缓存。读取前后校验策略版本与连接绑定，避免清空、忘记或令牌轮换期间的迟到响应重新写入。授权拒绝、身份不匹配、摘要或内容校验失败不使用旧缓存；网络故障才允许显示明确标记的离线副本。离线期间不能即时知道远端撤权，快照不授予执行能力。

| 检查 | 最终结果 | 范围与限制 |
| --- | --- | --- |
| Rust 工作区 | **187 通过、3 默认忽略，33.623 秒** | 最终 Rust 源码；18 个目标，排除 cranibox-node-desktop Rust 测试 |
| 新增后台协作记录集成 | **8/8，无跳过，85.474 秒** | 最终四程序；默认关闭、页面/网关退出、独立节点重启、暂停/清空、离线、拒绝/篡改及迟到响应 |
| 三类后台同步集成 | **25/25，无跳过，260.631 秒** | test_background_*_sync.py；包含新增八项，共享四路并发；等价 Clippy 修正前程序 |
| 协作记录协议集成 | **6/6，无跳过，285.556 秒** | test_federated_activity.py；真实 Rust、授权、限额与大目录；等价 Clippy 修正前程序 |
| Web 界面全量 | **143/143，347.936 秒** | 真实 Rust Web 与可控响应，含新增两项设置/离线回归；最终前端资源，等价 Clippy 修正前程序 |
| Tauri 界面全量 | **56/56，132.133 秒** | 模拟 IPC；扩展协作记录的设置、暂停、清空和离线状态，不是原生 GUI |
| 两端前端构建 | **通过，15.752 / 15.421 秒** | TypeScript/Vite；Web 生成资源取回提交，无新依赖 |
| Rust 四程序构建 | **通过，27.468 秒** | 最终源码与前端资源；cranibox、cranibox-node、workspace-runtime、workspace-web |
| fmt / 原生 Clippy | **通过，1.127 / 18.996 秒** | fmt --all --check；desktop,custom-protocol --all-targets -D warnings |

全量界面、25 项同步和六项协议之后仅将一处嵌套 if 改为 Clippy 要求的等价 let-chain，未改变行为、测试或前端资源。修正前源码和程序摘要保存在各自报告；最终 Rust、四程序、新增八项、fmt/Clippy 另行复验，不把两批程序视为同一个二进制。各报告均记录开始/结束源码与开始时程序、资源摘要，source_unchanged=true。

首轮界面专项为 **4 通过、2 失败，42.851 秒**：预览启动时 Rust 仍在编译，加载的是旧内嵌资源；等待最终资源嵌入后，全量 143 项通过，没有更改断言或超时。首次原生 Clippy 报 collapsible_if，改为等价 let-chain 后复验；两次失败的日志与 trace 均保留。

布局覆盖 375/768/1440 像素、明暗主题、无横向溢出与至少 44 像素操作控件。人工审阅 375 亮色与 1440 暗色的设置截图；这是浏览器响应式检查，不是手机或新原生包的真实窗口验收。私有证据目录为 **/config/validation-artifacts/background-activity-sync-20261019**，不提交凭据、原始日志、截图或辅助脚本。

收尾健康通过：容器 running/healthy、非 privileged、无宿主 Docker socket；3080/3031 匿名 401、认证 200，所属进程为空、无测试安装包。日常 Web PID 538744、客户端 PID 1403228、程序摘要和 NodeID 保持，KDE/Selkies 未重启。环境与本机/远端源码、生成资源及测试报告的核对记录为同目录 final-health.json、final-source-manifest.json。当前生产实现的六组完整镜像另已完成上述 CI 启动审计；三平台实机和物理组网仍待补验，不沿用历史版本成功。先处理下方 P0，完整账本复制、公钥信任及真实 Agent 接续继续留在 P1/P2。

### 历史验证索引

历史绿色结果只属于表中版本，不能代替本次修改或跨平台实机验收。逐条证据、包摘要、失败和重跑记录在[远端验证记录](remote-workspace-validation-2026-10-18.md)及对应 Git 历史。

| 版本/记录 | 已验证内容 | 尚未覆盖 |
| --- | --- | --- |
| 2312b64，矩阵 39151770009，7/7 成功 | 三平台 Rust 与 Tauri 打包，Web 126 项、客户端 48 项，真实 Sway/wayvnc、X11 VNC、Chromium/MCP、EasyTier | 当时 KDE 原生反向剪贴板仍失败；不是本次修复的 CI |
| fa1d401、84fdf88 | 原生错误原因保留、ACK/队列/关闭回归 | 不代表剪贴板当时已修复 |
| a8a97f5 Linux .deb/AppImage | 真实 KDE 画面、Computer Use、Chromium/MCP；.deb 原生文字发送 | 原生反向获取失败；两包私有 D-Bus 验收未覆盖正常托盘 |
| 6396b93 Linux .deb/AppImage | 窗口、托盘、XDG 自启开关、单实例；无 FUSE 的 AppImage 启动项经 GIO 静默启动；PTY PID/启动时间保持 | 普通 FUSE、实际登录/休眠、真实网关断网、Windows/macOS |
| 71f2397 KDE 产品镜像 | Linux amd64 完整构建、空配置首次启动、重启和数据保留 | 公开产品镜像发布/拉取及所有 OS 依赖安装 |
| 3c1a38d 资源与面板记录 | 当时空闲服务端约 17.4 MiB、节点约 10.3 MiB；s6/Rust 服务管理对照约下降 21%；Computer Use 管理 IPC | 不是完整 KDE 容器资源，也不是本次新性能测量；Wayland 首次认证超时曾单项重跑成功，根因未宣布修复 |

## 下一阶段优先级与验收条件

| 顺序 | 工作 | 完成判据 |
| --- | --- | --- |
| P0 | 正常登录及跨设备的持续桌面流程 | 2c95dd4 的 CI .deb/AppImage 已通过隔离 KDE 双向文字、等待与重连；84454e6 两包另通过桌面服务配置及正常 KDE 总线托盘/窗口/启动项。继续在实际登录及另一物理设备验证连续双向中文/emoji/多行、超时/恢复、撤权与断网，不以隔离 XDG、debug 或模拟 IPC 替代完整日用验收 |
| P0 | Windows/macOS 桌面互访与安装 | 三步提供者引导已有代码与回归；继续完成原生安装、真实提供者兼容性、录屏/输入权限、托盘/自启、真实登录和休眠恢复、网关断网及退出清理；从另一设备看到画面、输入和撤权断开。自动安装/托管仍未实现，WSL2 不是 Windows 原生验收 |
| P0 | 发布兼容的公开产品镜像并验收 | 2026-10-19 04:41 UTC 的默认公开标签两个架构均缺少空间协议标记，无法新建空间；已有空间现在可显式更换兼容镜像。981ff86 的六组只验证运行 39208494283 及六份启动报告已全部审计通过；下一步固定待发布来源，正式发布并完成公开镜像真实拉取/启动验收。只验证成功不会移动公开标签，不删除协议保护 |
| P1 | 维护者真实三设备流程 | Mac 委托 Linux/Windows Agent，关页面后继续，换入口回复和核对结果，覆盖断网、休眠、重启，不靠同容器替身或手改账本 |
| P1 | Tailscale 系统安装/登录与 EasyTier 多设备 | 干净的三平台主机验证方案确认、管理员授权、取消/失败恢复、系统服务、Tailnet 互访与目标授权；不能以夹具代替系统安装 |
| P1 | 完整统一视图与同步协议 | 项目、公开 Agent 目录及任务/消息/Run 均已有默认关闭的后台只读快照；协作记录需独立 activity 授权和明确选择。继续定义权威账本复制的版本、删除、冲突及撤权规则，评估增量传输、存储容量和多物理设备恢复；不能把摘要当签名或时钟、来源报告当全局状态，也不能扩大旧 messages 权限 |
| P1 | 设备信任与能力提供者 | 持久身份、授权签名和摘要兼容已有后端回归，并修复本机密钥锚定、降级绕过和 Noise 一致性；a311645 新增 TOFU/配对码和加密 HTTP/WebSocket。继续核对新 CI、旧二进制历史账本、跨设备严格配对、轮换/撤销、有效旧签名记录重放及所有入口的加密边界。Computer Use、Browser Use 的多物理设备和 Windows/macOS、依赖引导仍待验收。cua-driver 替代需独立评估 |
| P2 | Agent 会话接续与运行交互 | 实测具体 CLI 登录和模型调用；检查点、项目文件/上下文一致性、目标启动及源端停止有明确状态。当前不支持移动原进程或向运行中 Agent 追加交互输入 |
| P2 | 交互、规模与测试稳定性 | 原生搜索两次键盘尝试未生效，需定位焦点/输入链路，再验普通键鼠选择和最新安装包的日常流程；9fd1b25 的 GTK debug 的 AT-SPI 类型选择、Run 报告、20/4 分页与重置已通过，不重复计作待实现功能。评估长清单、错误恢复、大量设备/文件，定位大数据准备并行负载下的 8 秒 HTTP 超时。Selkies 夹具同步已在 5af2b53 修正，远端全量及连续 30 次通过，4fb6a41 的 Linux CI 已成功；仍需真实桌面连续响应及 Wayland/KDE 输入故障复现 |

暂不需要为节点身份等同引入选主。当前数据由所属节点持有，界面连接任一运行入口；若要复制协作协调状态，先定义故障与冲突模型，再决定一致性协议。

## 接手与复现

先读本文与[验证记录](remote-workspace-validation-2026-10-18.md)，再看[分布式设计](distributed-agent-workspace-design.md)、[组网设计](decentralized-node-network-design.md)、[节点直连](direct-node-peers.md)和[设备网络](device-networks.md)。功能细节见[客户端](node-desktop-and-device-mesh.md)、[桌面](remote-desktops.md)、[Computer Use](computer-use.md)、[Browser Use](browser-use.md)、[虚拟空间](virtual-spaces.md)、[项目目录/复制](project-session-catalog.md)、[消息](device-messages.md)、[任务](device-tasks.md)、[协作记录](federated-activity.md)、[会话](agent-sessions.md)与[后台执行](agent-executions.md)。设计文档包含尚未实现的提案，以本文状态与源码为准。

### 源码入口

| 位置 | 内容 |
| --- | --- |
| crates/cranibox | 合并服务端入口 |
| crates/workspace-runtime/src/peers/{identity.rs,config.rs,e2e.rs,e2e/stream.rs,e2e/socket.rs}；tests/test_node_e2e.py | 持久身份、授权签名、迁移检查点、TOFU/配对、Noise 与加密流；新增身份回归在 test_node_peers.py |
| addons/control-center/ui/src/messages；tests/test_device_messages.py | Agent 入站策略、所有者审批队列与 workspacectl messages；最新 UI 尚未实机验收 |
| crates/workspacectl、crates/workspace-services | Rust 控制工具与服务兼容管理 |
| crates/workspace-runtime/src/peers、projects | 连接、目标授权客户端、消息/任务/项目账本、文件复制与后台同步配置 |
| crates/workspace-node/src/{local.rs,peer,network,spaces,supervisor.rs,task_worker.rs,project_sync.rs} | 节点宿主、直连服务、网络/空间、进程、自动接单与后台目录调度 |
| crates/workspace-runtime/src/peers/{project_sync.rs,agent_snapshots.rs,activity_snapshots.rs}；addons/control-center/ui/src/projects/ProjectSync.tsx | 项目/Agent/协作记录三套独立同步策略，共享四个后台并发；凭据绑定只读快照与两端配置/状态；专项 tests/test_background_{project,agent,activity}_sync.py |
| crates/workspace-runtime/src/peers/activity.rs；crates/workspace-node/src/peer/server/activity.rs；addons/control-center/ui/src/tasks/FederatedActivity.tsx | 只读协作记录协议、目标独立授权与请求后连接/策略复查、Web/Tauri 来源和离线标识；专项 tests/test_federated_activity.py |
| crates/workspace-node/src/network/tailscale_setup.rs、install-tailscale*.sh | Tailscale 十分钟一次性方案、后台状态记录、平台安装/启动；不会自动登录或重放未完成安装 |
| crates/workspace-web | 登录/CSRF、Web API、终端/桌面和空间应用转发 |
| crates/workspace-runtime/src/peers/desktop/{selkies.rs,selkies/output.rs}；addons/control-center/ui/src/desktop/{openDesktop.ts,SelkiesViewer.ts} | Selkies WebSocket 适配、有界画面合并、受限输入、首包协议选择、JPEG 解码与键鼠；真实桌面回归为 tests/test_desktop_real_selkies.py |
| crates/workspace-node/src/desktop_service/{discovery.rs,provider.rs}；addons/control-center/ui/src/desktop/{ExistingDesktopServices.tsx,RemoteDesktopsView.tsx} | 常用回环地址和已保存服务的有界检测、提供者兼容性；共用配置核对、取消与保存前协议检查 |
| crates/workspace-node/src/desktop_service/setup.rs；addons/control-center/ui/src/desktop/DesktopServicePanel.tsx | VNC 一次性安装方案、私有安装记录、监控退出/超时与重启中断提示；两端共用核对/错误恢复，专项 tests/test_desktop_service.py |
| crates/workspace-node/src/spaces/{mod.rs,engine.rs}；addons/control-center/ui/src/spaces/SpacesView.tsx | 空间安装方案私有持久化、命令复查与原子消费、安装进程/监控分离；容器不存在时修改镜像并保留身份、凭据与卷；两端响应丢失恢复与焦点；专项 tests/test_virtual_spaces.py 和 spaces.spec.ts |
| addons/control-center/ui/src/desktop/DesktopPlatformGuide.tsx；apps/node-desktop/src-tauri/src/main.rs | Windows/macOS 三步准备向导；Tauri 通过固定文档标识打开官方页面，复用原有服务检查和保存入口；Web desktops.spec.ts / 原生 client.spec.ts 覆盖引导流程 |
| addons/control-center/ui/src/desktop/{DesktopClipboard.tsx,clipboard.ts,VncViewer.tsx} | Web/Tauri 共用纯文本剪贴板面板、UTF-8 限制、明确复制及连接生命周期；Web desktops.spec.ts、原生 client.spec.ts 为夹具/模拟 IPC 回归 |
| crates/workspace-runtime/src/peers/computer.rs；crates/workspace-node/src/peer/server/computer.rs | 有界桥接、类型化动作、目标 MCP；短期控制权、鉴权复查与停止 |
| addons/control-center/ui/src/devices/ComputerSharingPanel.tsx；tests/test_computer_use.py | Web/Tauri 共用桥接管理；真实节点/桥接/Web 与可控原生动作夹具 |
| crates/workspace-runtime/src/peers/browser.rs；crates/workspace-node/src/peer/{server/browser.rs,browser_proxy.rs} | Chromium CDP 提供者、独立授权、状态/停止、WebSocket 与调用端官方 MCP 进程生命周期 |
| addons/control-center/ui/src/devices/Browser{Sharing,Target}Panel.tsx；tests/test_browser{_use,_real_chromium}.py | Web/Tauri 共用浏览器管理；真实节点/CDP 夹具以及显式启用的真实 Chromium/官方 MCP 验收 |
| addons/control-center/ui/src | React 共用业务组件与浏览器适配器 |
| apps/node-desktop | Tauri 原生宿主、IPC、托盘与客户端适配器 |
| apps/node-desktop/src-tauri/src/launch.rs | 持久启动器、AppImage MCP 参数、Linux 自启动路径与转义；纯 Rust 库单元测试和真实 GIO 用例 |
| tests、两端 UI 的 tests | Python 真实服务集成和 Playwright；模拟与实机条件见各文件 |
| .github/workflows/rust-runtime.yml | 三平台 Rust/安装包、Linux 服务与浏览器、独立 Wayland 验证 |
| scripts/verify-workspace-image.py、.github/workflows/build-webtop.yml；tests/test_workspace_image_check.py | 原生双架构验证、候选 index/平台 manifest 摘要选择、首次启动、节点接入及重启；两架构均过才发布，普通 Release 运行检查器回归 |

### 远端环境

- SSH：admin@192.168.217.6，端口 22022，容器 cranibox-validation，用户 abc。运行验证继续使用这个环境，不改用当前本机。
- Web：https://192.168.217.6:3080；KDE/Selkies：https://192.168.217.6:3031。核对入口使用 LAN 地址。凭据只保存在远端私有 container.env，不能输出或提交。
- 既有容器源码 /config/agent-workspace-manager/source；本次身份补验独立源码 /config/validation-sources/identity-79f3e4a，合并后另用 /config/validation-sources/identity-a311645。前者未覆盖为新版本；各快照不含本地 Git。Cargo 目标 /config/validation-build/target；证据 /config/validation-artifacts。
- 日常 Web PID 538744、原生客户端当前 PID 1403228；均未升级。原客户端在正常桌面补验时暂停后恢复，原 NodeID 和身份文件字节保持；历史记录中的 767446 已退出。两者独立身份，客户端使用 /config/validation-client/{config,data,state}；不得使两个宿主进程共用 NodeID。最新托盘补验使用正常 KDE 总线与隔离 XDG，其他私有 D-Bus 记录保持各自边界。
- 容器非 privileged、无宿主 Docker socket。虚拟空间曾用独立临时管理容器，两个测试空间与管理容器已停止且保留数据，不要误删。
- Mac fliaping@192.168.217.85 拒绝现有 SSH 公钥；Windows 地址 192.168.217.226 是 WSL2，原生图形账号/入口尚缺。不能将 CI 或 WSL2 写成原生 GUI 已验收。
- 既有 linux-installer-{a8a97f5,6396b93,3c1a38d} 等证据仍保留；测试 .deb 已卸载，日常自启配置未改变。

### 构建与测试命令

在远端容器源码根目录、普通用户下运行；离线缓存已齐全时可附加 --offline，CI 使用 --locked。先生成资源，再构建实际嵌入这些资源的程序：

~~~sh
export CARGO_HOME=/config/.cargo
export CARGO_TARGET_DIR=/config/validation-build/target
export CARGO_BUILD_JOBS=2
export CRANIBOX_BIN=$CARGO_TARGET_DIR/debug/cranibox
export CRANIBOX_NODE_BIN=$CARGO_TARGET_DIR/debug/cranibox-node
export WORKSPACE_RUNTIME_BIN=$CARGO_TARGET_DIR/debug/workspace-runtime
export WORKSPACE_WEB_BIN=$CARGO_TARGET_DIR/debug/workspace-web
export CRANIBOX_HERDR_BIN=/config/bin/herdr
export WORKSPACE_CTL_BIN=$CARGO_TARGET_DIR/debug/workspacectl
export PYTHONPATH=tests

node scripts/build-control-center-ui.cjs
npm run build --prefix apps/node-desktop
cargo build --locked --workspace
cargo build --locked -p cranibox-node-desktop --features desktop,custom-protocol
cargo fmt --all -- --check
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked -p cranibox-node-desktop --features desktop,custom-protocol --bin cranibox-node-desktop desktops::tests
cargo clippy --locked -p cranibox-node-desktop --features desktop,custom-protocol --all-targets -- -D warnings
python3 -m unittest test_remote_desktops test_desktop_relay test_desktop_service -v
python3 -m unittest test_background_activity_sync test_background_agent_sync test_background_project_sync test_project_catalog_sync test_device_tasks test_task_worker_mcp -v
python3 -m unittest test_federated_activity test_device_tasks test_device_messages test_collaboration_runs -v
python3 -m unittest test_node_peers test_device_tasks test_device_messages test_agent_executions test_remote_agent_executions test_background_activity_sync test_node_e2e -v
npm test --prefix apps/node-desktop
npm test --prefix addons/control-center/ui
npm test --prefix addons/control-center/ui -- tests/desktops.spec.ts --workers=1
~~~

Web 的 npm run build 只生成 ui/dist；scripts/build-control-center-ui.cjs 另生成带许可证的 web.js、web.css 与两份 HTML，Rust 服务嵌入后者。共用 React 修改后必须重新生成资源并重建 cranibox。直接 Cargo 运行原生程序需 desktop,custom-protocol，避免误连已停止的 Vite 开发服务。完整 CI 定义在 .github/workflows/rust-runtime.yml；原生 IPC 模拟测试和包构建不能代替真实窗口。

真实 KDE/Selkies 测试会主动接管现有桌面和文字剪贴板，先确认当前会话，按该容器环境运行；用例检查并恢复原纯文本剪贴板，不输出内容：

~~~sh
DISPLAY=:1 WAYLAND_DISPLAY=wayland-0 XDG_RUNTIME_DIR=/config/.XDG CRANIBOX_TEST_CHROMIUM=/usr/bin/chromium CRANIBOX_TEST_REAL_SELKIES=127.0.0.1:8082 CRANIBOX_TEST_SELKIES_CLIPBOARD=1 python3 -m unittest test_desktop_real_selkies -v
~~~

Browser Use 的 test_browser_real_chromium 需 CRANIBOX_TEST_REAL_BROWSER=1 与 CRANIBOX_TEST_CHROMIUM=/usr/bin/chromium，使用临时 profile/随机端口，不访问日常 9222。Computer Use 为 test_computer_use；更多条件见对应功能文档。下一位模型从 P0 的正常桌面、另一物理设备及跨平台流程开始；历史 Linux 包验收分别绑定文内提交，9fd1b25 的 GTK debug 补验不能替代最新安装包或正常登录。原生搜索输入、普通键鼠等边界仍保留。保留已有日志和版本界限，勿重复已经撤回的诊断方案。
