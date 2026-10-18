# Cranibox 功能、验证与后续交接

整理日期：2026-10-19。本文是下一位维护者的接手入口；需求状态、当前测试和待办集中在这里，历次成功、失败与诊断保留在[远端验证记录](remote-workspace-validation-2026-10-18.md)。日期沿用原文件名，不表示内容仅到 10 月 18 日。

本轮补齐“已有桌面服务接入”：Web/Tauri 共用主动检测入口，Rust 只检查常用回环端口和已保存地址；选择结果后先核对协议、地址，再明确保存并开启共享。检测、核对和取消不改共享配置。补充空结果、错误重试、加载状态、离页清理、键盘焦点、窄屏及明暗主题，开启/停止共享同步更新列表标记。同产品提交 84454e6 的 CI Linux .deb/AppImage 已在隔离 KDE 真实窗口通过检测、核对、取消、保存与停止流程。它简化已有 VNC/Selkies 的接入，不代表完成各系统的提供者自动安装。

上一轮已修复 Selkies 慢查看器阻塞控制消息的问题：Rust 持续读取上游，安全合并已被新画面完全覆盖的待发送 JPEG 区域，并优先发送文字剪贴板；前端完整收到文字后不再等待 JPEG 解码。实际 KDE/Tauri debug 窗口连续五段新文字通过后，进一步下载同一产品提交 2c95dd4 的 CI Linux .deb/AppImage，两包真实窗口的中文、emoji、多行双向剪贴板、等待后更新及断开重连均通过。此前 a8a97f5 的失败记录仍保留。**上述剪贴板包验收绑定旧产品提交，使用隔离 XDG、私有 D-Bus；不能代替本轮新界面、正常登录桌面或另一物理设备的验收**。

交接不代表全部产品需求完成。没有升级日常 Web/客户端、重启 KDE/Selkies 或安装系统 Tailscale；日常身份和配置保持。后续先完成安装包及跨平台日用流程，再推进更完整的数据同步与 Agent 接续。

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
| 常驻托盘与登录自启动 | Tauri 窗口关闭到托盘、重连、明确退出、用户登录后自启动配置 | 6396b93 的 Linux .deb/AppImage 通过实际界面、托盘、XDG 自启开关、单实例；AppImage 生成启动项经 GIO 在无 FUSE、无手动兼容变量时静默启动并可唤回。a8a97f5 两包补验私有 D-Bus 下的窗口和桌面 IPC，未重验正常托盘。实际登录、休眠、网关断线恢复及 Windows/macOS 仍待实测 |
| 持久终端与浮动窗口 | 可变大小、最小化到状态栏、多个会话、重新附着；服务端使用独立 Herdr，客户端使用原生 PTY | 6396b93 .deb 关闭浮动终端、收起原生窗口及操作托盘重连后，Shell PID 与启动时间不变；此前 afaa137 另验证导出变量保留。没有制造真实网关网络故障，客户端进程或系统退出后不能保留该 Shell |
| 桌面互访 | 共用 VNC/noVNC 与 Selkies/JPEG 查看器；已有本机服务的 IPv4/IPv6 检测、配置核对及保存前复查；本机、直连及出站 WebSocket 中继；独立授权、单控制者、停止与撤权取消；文字剪贴板支持主动发送/复制、64 KiB 限制和只读 | KDE 容器 Selkies 的真实画面、完整文字输入、重连和撤权通过；Web 双向剪贴板通过；a8a97f5 原生反向获取失败，2026-10-19 修复后的 debug 及同提交 CI .deb/AppImage 的真实双向文字、持续更新、重连清空均通过。两包为私有 D-Bus 验收，未重验正常托盘与撤权。剪贴板不自动粘贴，不传图片/文件，旧 VNC 的 Unicode 取决于服务端。Windows/macOS 实机及 KDE/GNOME 的自动安装与托管未实现 |
| 内置 VNC 托管 | Linux X11 使用 x11vnc，兼容 wlroots 的 Wayland 使用 wayvnc；守护、失败退避、节点重启恢复 | 不是一个纯 Rust 跨平台捕获实现；KDE 容器实测 VNC 屏幕捕获失败，Selkies 可用，见远程桌面文档 |
| 虚拟空间 | 检测、安装计划、明确执行、镜像策略、创建/停止/启动/归档/恢复、资源限制、独立身份与持久卷；桌面/编辑器独立应用入口 | 真实 Docker 生命周期与浏览器流程已测；完整 Linux amd64 KDE 镜像的干净构建、空配置首次启动及重启保留数据已通过。默认公开标签的 amd64 镜像已确认缺少空间协议标签，当前无法用它新建空间；需发布兼容镜像并验收，各 OS 系统依赖安装也未验收 |
| 统一项目与会话目录 | ProjectID、节点工作副本、Shell 关联、网关/直连汇总；增量目录读取、删除处理、只读离线快照 | 各节点保留本地权威；不是全网数据库复制，不支持离线修改后自动合并 |
| 关闭页面后的目录同步 | Rust 节点后台同步，明确选择 1–32 个已接入设备，15/30/60/300 秒，最多四路并发；暂停和重启恢复 | 默认关闭；只同步项目目录和会话关联，不复制工作文件；凭据/连接变更后需要重新选择 |
| 项目迁移与接续 | 本机与直连双向、两个远端之间快照复制；清单确认、分块校验、暂停/恢复、持久接收回执；保留 ProjectID 与接续说明 | 当前是显式快照复制，非持续文件同步；上限 2 GiB/单文件 1 GiB，实际集成传输约 20 MiB，未做上限压力测试 |
| 多 Agent 通信与任务 | 持久消息、线程回复、确认、离线投递、任务委托/领取/进度/问题/回复/结果/合作取消，CLI/MCP 与共享界面 | 不能由普通消息自动启动进程；同一容器中的独立身份验证不等于多台物理设备验收 |
| Agent 会话与后台执行 | 主动登记/续期、汇总授权会话、定向委托；Codex/Claude Code 非交互适配、启动检查、持久输出与停止；可选独立执行 MCP | 模型调用与 CLI 登录未实测；不支持运行中追加交互输入、模型检查点恢复或进程迁移 |
| 多设备协作与自动接单 | 持久协作 Run、任务汇总、统一停止、核对归档、最多八项前置任务；接收端可明确启用按可信发送方和固定目录的后台 worker | 默认关闭；不是通用复杂编排平台；人核对模式与显式自动提案模式分开，退出码不直接等于任务成功 |
| 各节点 Agent 共享 Computer Use | 独立 computer 授权、目标配置的回环桥接、直连 API、短期输入控制权、持久停止、独立目标绑定 MCP；Web/Tauri 共用配置与停止界面 | 复用现有 bridge，不安装跨平台驱动；只走直连。12 项后端专项使用动作夹具；a8a97f5 .deb 另通过真实 KDE 截图、输入、MCP、重启保留停止，AppImage 通过 GUI 节点宿主及真实 MCP 截图；管理面板完整停止/恢复 IPC 的两包验收仍绑定 3c1a38d。多物理设备、模型调用及 Windows/macOS 提供者待验收 |
| 各节点 Agent 共享 Browser Use | 独立 browser 授权、目标回环 Chromium CDP、鉴权 WebSocket、单远程控制者、跨重启保留停止；调用方经临时回环代理运行固定版本官方 MCP，Web/Tauri 共用管理 | 真实 Linux Chromium 的打开页面、快照、填表、提交、PNG 与停止通过；只走直连。授权覆盖共享浏览器全部标签页和会话；无自动安装/启动、域名隔离或旧网关转发。Windows/macOS、跨物理设备及真实模型待验收 |
| agent-desktop-bridge 是否可删除 | 现有桥接继续保留，并作为 Computer Use 的明确配置提供者；VNC 是桌面传输通道 | cua-driver 替代方案仍需独立核对接口、平台和权限，当前不能删除桥接 |

五项服务是 agent-desktop-bridge、deepseek-harness、openclaw-gateway、proxy-caddy、tailscaled-workspace。其监督方式迁移不代表业务健康：原记录中桌面桥接 health 为 503、Tailscale 为 NeedsLogin，均不能计作迁移修复。见 [Rust 服务与迁移记录](native-services-and-web.md)。

## 测试情况与可信范围

### 当前：已有桌面服务接入（2026-10-19）

所有运行验证仍在指定远端容器。本轮新增 Rust `discover` 命令及 Web/Tauri 共用页面，保留已有手动配置、安装和托管入口；未新增依赖。协议探测最多六个回环地址并发，每项上限三秒；只读取握手，不认证、不发送 Selkies SETTINGS，不修改共享、凭据或进程配置。选择结果后填写表单，保存前再次检查。具体边界见[桌面文档](remote-desktops.md)。

| 验证 | 最终结果 | 范围与限制 |
| --- | --- | --- |
| Rust 工作区 | 183 通过、4 忽略 | 20 个目标汇总；忽略项不计通过，内容见下方历史说明 |
| 原生传输专项 | 4/4 通过 | desktop,custom-protocol；不代表原生 GUI 验收 |
| 构建与静态检查 | 通过 | fmt、工作区及原生 all-targets 严格 Clippy、工作区构建、两端 TypeScript/Vite、实际嵌入最新资源的 Web 与原生 debug 程序 |
| 桌面服务与传输集成 | 11/11、7/7 通过，无跳过 | 真实 Rust 与隔离提供者；三项新增用例覆盖 IPv4/IPv6、自定义 Selkies、错误/无响应服务、重试、不接管和配置不变 |
| Web 全量 | 128/128 通过；215.26 秒 | 桌面 14 项已包含其中；检测响应可控，保存检查、配置与连接使用实际 Rust 服务 |
| 客户端界面全量 | 49/49 通过；103.99 秒 | 浏览器模拟 Tauri IPC；新增检测、核对、取消、保存和停止后标记更新 |
| Web API 检查 | 通过 | 访问保护、嵌入资源、快照缓存、表单、后台操作、刷新恢复和紧急停止 |

全量通过后，最终截图检查只追加了平台指引文案和表单与托管说明之间的 20 像素间距；两端资源、实际 Web 和原生 debug 程序重新构建通过，受影响的 Web 桌面 14/14、客户端相关 3/3 通过，并重新运行真实 Selkies 检测、核对和取消流程。这些专项与全量范围重叠，不累加计数；最后两处调整后没有再次运行全部 128/49 项。

首次客户端为 48 通过、1 失败：旧 MCP 加载用例依赖 250 毫秒延迟，断言执行时响应已完成，已改为明确释放响应的同步方式。首轮 Web 全量因辅助脚本漏配 CRANIBOX_HERDR_BIN，119 通过、9 个终端相关用例失败；补齐已安装 Herdr 的路径后，全量 128/128。构建过程中也修复了本轮 optional protocol 的 TypeScript 类型错误，并更正辅助脚本的 Cargo 包名和 API 检查工作目录。主要失败日志与辅助脚本修正记录保留，不记作通过，也没有为环境问题修改产品终端行为。

真实 KDE 的浏览器流程检测到日常 Selkies 8082，选择后正确填写协议和地址并聚焦输入；核对与取消后配置逐字节不变，未开启共享或打开查看器。这段浏览器流程不计为新的桌面控制或原生安装包验收；后续同提交 Linux 包验收另见下节。首轮截图暴露移动端核对表单与结果距离过远，已将表单移到结果后并增加标题；最终布局重新检查明暗主题、375/1440 像素截图。自动化另覆盖 768/1024 像素及 812 像素横屏、减少动画、错误重试与离页迟到结果。

最终核对 12 份产品源码及资源与远端构建快照一致，另记录五个程序摘要。测试清理后容器 running/healthy、非 privileged、无宿主 Docker socket；3080/3031 均匿名 401、认证 200；日常 Web 与客户端的 PID、程序摘要和 NodeID 保持，测试预览/MCP 进程无残留。未升级日常程序，也未操作桌面输入或剪贴板。

证据目录为远端 /config/validation-artifacts/desktop-discovery-20261019：首轮 checks.json 和日志、final 的 Rust 测试、ui-final 的失败日志与追踪、verified 的全量测试及 API 检查、post-review 的最终资源构建/专项/真实服务检查，以及 source-binaries.json 和 final-health.json。凭据、截图、原始配置与辅助脚本不提交。日常服务和此前安装包验收均有独立版本边界，下方历史结果不代替本轮检查。本轮 CI 与提交绑定信息见下方补记。

### 本轮提交与 CI 交接

已有服务检测的产品提交为 **84454e695d915157d245630c4ad3b8b77091c0b6**（feat: discover and review existing desktop services），已推送 origin/main。后续补记只改文档；以下运行全部绑定该完整产品提交：

| 工作流 | 运行编号 | 最终结果与范围 |
| --- | --- | --- |
| Cranibox server and independent node client | 39169225796 | 7/7 成功：三平台 Rust 服务端/节点、三平台 Tauri 安装包，以及独立 Sway/wayvnc 作业 |
| Release | 39169225825 | 成功；脚本、Dockerfile、Compose 检查及脚本地址维护，ShellCheck 非阻断；不发布产品镜像 |
| Sync to GitCode | 39169225719 | 成功 |

已下载并核对原始日志：Linux CI 的 Web 全量 128/128（3.1 分钟）、客户端界面 49/49（22.9 秒）、真实 Chromium/官方 MCP、X11 VNC、Sway/wayvnc、真实 EasyTier 2/2、GIO 启动器均通过。CI 全量覆盖最终文案/间距后的完整产品提交，补上前述远端只重跑专项的范围；客户端界面全量仍是浏览器模拟 IPC，原生窗口另见下节。与远端或历史重叠的测试不累计。

本轮上传 10 项 Actions Artifacts：三平台各自的 cranibox、cranibox-node、cranibox-node-desktop，以及 resource-baseline-linux。Linux 客户端为 .deb/AppImage、Windows 为 NSIS .exe、macOS 为 .dmg；Windows/macOS 的构建成功不等于实机安装或桌面权限验收。下载含新检测界面的包请使用本轮矩阵并核对 headSha，下文旧版 2c95dd4 仍保留独立证据。私有证据 linux-installer-84454e6/ci-final.json 保存完整提交、七项作业、其他工作流与产物元数据，另有 ci-native-linux.log、ci-desktop-linux.log、ci-wayland.log。

### 本轮 CI Linux 安装包真实窗口验收

从矩阵 39169225796 下载 artifact 12151304011（cranibox-node-desktop-ubuntu-latest），核对完整产品提交 84454e695d915157d245630c4ad3b8b77091c0b6、本机与远端文件大小及 SHA-256。Linux 客户端作业已成功。

| 包 | SHA-256 | 真实结果 |
| --- | --- | --- |
| .deb，9,316,726 字节 | b4cdd7157696f74fed8af6add38ccc7c5f9ec181cfe6877c58604718bf9147eb | dpkg 安装后运行；完整检测/核对/取消/保存/停止通过 |
| AppImage，87,484,920 字节 | 26869e475df7538bc7211c8e598637089993a3982d63f758228adffe534a364a | 使用 --appimage-extract-and-run；同一完整流程通过 |

两包使用各自全新的 NodeID、隔离 XDG 和私有 D-Bus，在指定远端现有 KDE 内运行真实 Tauri 窗口与 IPC。检测到实际 Selkies 127.0.0.1:8082，核对时协议、地址和输入焦点正确；取消后配置逐字节不变，没有托管配置或密码。再次核对并明确保存后，实际配置为 Selkies、列表显示“当前共享配置”；点击停止后共享为空、标记和停止按钮消失，NodeID 保持，未生成托管配置或密码。截图已检查。

首轮管理脚本将 WebKit 的 button 角色误写为 push button，第二次只按独立文本名称找内联标记，均与实际 AT-SPI 控件不符。保留两份失败记录，按真实角色/列表项文本修正脚本后两包通过，没有为此修改产品代码。两次均在脚本匹配处中止，不作为产品通过或产品缺陷的证据。

本轮只验收发现与配置，不打开查看器，不发送桌面输入、不改剪贴板；未重验正常登录、托盘、自启、普通 FUSE、休眠、跨物理设备、撤权或 Windows/macOS 原生窗口。两包通过不替代旧版本双向剪贴板的独立证据。测试后退出独立 GUI/辅助进程、卸载测试 .deb、删除本次 AppImage 解压目录；容器健康、访问保护、日常 PID/摘要/NodeID 均保持。私有证据为 /config/validation-artifacts/linux-installer-84454e6 的 artifact.json、package-digests.json、各包 exercise.json/状态/截图/清理记录、acceptance.json 和 final-health.json。

### 默认公开空间镜像的实际阻塞

在指定远端仅通过 Docker Registry API 读取默认镜像 xuping/agent-workspace:ubuntu-xfce 的索引、Linux amd64 manifest 和配置，没有拉取镜像层或启动容器。该公开版本确认缺少 io.cranibox.space-protocol 标签，因此会被当前空间创建流程在数据卷创建前拒绝，不应再只列为“尚未验收”。

| 核对项 | 结果 |
| --- | --- |
| 公开标签索引 | sha256:f2e58c4fffd5652e6b3736f6af432a81ededd3349f04c53195d87169f6eb06fa |
| Linux amd64 manifest | sha256:4d60b036e39d8194457f8d2c12b340101e00163606b20aa3fe7ac4b304b4ad97 |
| 镜像来源提交 | c56508ce5008180e067a9c489f4527aa30b80b31 |
| 镜像配置创建时间 | 2026-10-13T10:40:40.352550645Z |
| 空间协议标签 | 缺失；当前要求值为 1 |

证据位于远端 /config/validation-artifacts/public-image-review-20261019/metadata.json，另校验了配置 blob 的 SHA-256。以上只覆盖该时点的 Linux amd64 公开版本，未检测 arm64、镜像站或运行期行为。后续需发布兼容镜像并完成真实拉取、首次启动及重启；本轮没有触发镜像发布或改变默认镜像引用，不能删除协议检查来兼容旧版。

镜像发布入口是 .github/workflows/build-webtop.yml，由 v* 标签或手动 workflow_dispatch 触发，包含三个桌面变体和 amd64/arm64；手动指定 version 仍会更新 ubuntu-{desktop} 浮动标签。普通 main 提交触发的 Release 工作流只做脚本、Dockerfile、Compose 检查和脚本地址维护，不代表已经构建或发布产品镜像；其中 ShellCheck 允许失败，也不能当作全部警告已修复。

### 上一轮 Selkies 修复的代码验证：2026-10-19

全部运行验证位于指定远端 admin@192.168.217.6:22022 的 cranibox-validation 容器；本机仅编辑、静态审阅和 Git 操作。以下仅为上一轮 Selkies 修复源码的结果，专项与全量中的重叠测试不累计。

| 验证 | 结果 | 范围与限制 |
| --- | --- | --- |
| 工作区 Rust 测试 | 183 通过、4 忽略 | 20 个测试目标的汇总；忽略项不计通过。包含桌面专项 10 项及新增的 5 项队列/慢查看器回归 |
| 原生桌面传输测试 | 4/4 通过 | desktop,custom-protocol；ACK、超时、提供者断开和主动关闭，不能代替原生 GUI |
| Rust 构建与静态检查 | 通过 | 工作区构建、fmt；工作区及原生 desktop,custom-protocol 的 all-targets Clippy，-D warnings |
| 两端资源与原生程序构建 | 通过 | TypeScript/Vite、生成并嵌入 Web 资源、实际原生 debug 构建；已有 Vite 大包提示仍存在 |
| Tauri 界面 | 48/48 通过 | Playwright 模拟 IPC；扩展原有用例，证明 JPEG 解码挂起时新剪贴板仍及时呈现；不另累计为第 49 项 |
| Web 桌面 | 12/12 通过 | 新生成资源、实际 Rust 服务、受控 RFB/JPEG 提供者；不是当前 KDE 原生窗口 |
| 桌面后端集成 | 22/22 通过 | 直连、中继、权限撤销、单控制者、服务守护和生命周期；使用隔离提供者 |
| 真实 KDE Web 中继 | 1/1 通过 | 真实画面、完整键盘、双向文字剪贴板、重连和撤权；18.726 秒，不是原生包验收 |
| 真实 KDE 原生窗口 | 五轮反向更新通过 | 无插桩、无代理、真实 Tauri IPC 与日常 Selkies；48/66/63/50/62 字节新标记逐字节匹配，重连后接收栏先清空 |

工作区默认忽略的是离线传输测量、两项需真实 EasyTier 二进制/网络权限的检查，以及需 Linux GIO 的启动器检查。本次 CI 已显式运行后两类：真实 EasyTier 2/2、GIO 启动器 1/1 通过；不与上述默认测试重复累计。

原生程序 SHA-256 为 372bb5eea65f5314a50d512d757d07de3c8a861e95e0e41702ded3757b7f4162。实际画面可见且非黑屏；两次 TCP 样本没有上游发送积压，但这不是受控性能基准。每轮管理脚本耗时包含 SSH 与辅助进程启动，不能写成协议响应延迟。

最终清理核对原剪贴板逐字节恢复、测试 GUI/辅助进程停止；容器 running/healthy、非 privileged、无宿主 Docker socket；3080/3031 均匿名 401/认证 200，日常 Web/客户端 PID、摘要和 NodeID 保持。

上述 debug 轮次的发送自动化在准备文本框输入时未成功，未执行“发送文字”，不计为产品通过或产品失败。后续同提交 CI 安装包使用 AT-SPI 聚焦、Selkies 键盘粘贴准备输入，逐字节检查完整草稿；再将系统剪贴板改成另一段文字，确认目标与草稿不同后才点击原生“发送文字”。.deb 的 41 字节及 AppImage 的 46 字节中文、emoji、多行文字均逐字节到达目标，避免把准备阶段写剪贴板误当作产品发送成功。

### 同提交 CI Linux 安装包实测

下载矩阵 39162097500 的 cranibox-node-desktop-ubuntu-latest，核对完整产品提交 2c95dd49d7a11a6a6c6aedf87cf67dc769e9bd6a 及文件 SHA-256。两包均在指定远端现有 KDE 中运行真实原生窗口、真实 IPC，直接连接日常 Selkies 8082，不经过诊断代理。

| 包 | 实测结果 | 范围 |
| --- | --- | --- |
| .deb | 实际 dpkg 安装；初次 48 字节、连接数分钟后 62 字节、重连后 55 字节、重连超过一分钟后 63 字节均逐字节匹配；主动发送 41 字节通过 | 系统包名 cranibox-node；测试后卸载，未替换日常客户端 |
| AppImage | 使用 --appimage-extract-and-run；初次 48 字节、第二段 56 字节、连接超过一分钟后 57 字节、重连后 55 字节、重连超过两分钟后 63 字节均逐字节匹配；主动发送 46 字节通过 | 未重验普通 FUSE 启动 |

两包均确认实际画面可见、重连前接收栏移除、重连后先清空再接收新文字。隔离 XDG 与私有 D-Bus 避免占用日常单实例身份；这不等于正常登录、托盘、自启、休眠、另一物理设备或 Windows/macOS 验收。新包未额外制造超时、断网或撤权，不能挪用 Web/夹具结果补齐这些范围。

私有证据根目录为 /config/validation-artifacts/linux-installer-2c95dd4：artifact.json、deb-install.json、各包 GUI/操作/发送/截图/清理记录、acceptance.json 与 final-health.json。包摘要见[安装包专项记录](remote-workspace-validation-2026-10-18.md#2026-10-19-同提交-ci-linux-安装包实测)。全部测试结束后恢复原剪贴板、停止测试窗口及辅助进程；日常服务 PID、程序摘要与 NodeID 保持，两个 LAN 入口匿名 401/认证 200，容器健康。辅助脚本、正文、截图及凭据不提交 Git。

### 修复依据与失败记录

- 临时关联追踪中，上游发送 cr 后至 Rust 读到下一条剪贴板消息相隔 44.434 秒；同条消息完整进入前端后约 1 毫秒完成事件派发。协议没有请求编号，也可能主动广播，前者不是逐请求精确响应延迟。诊断代码已全部撤回。
- 第一版有界合并在真实 KDE 触发队列上限：上游反复发送查看器原本忽略的 cursor 消息，阻止了 JPEG 合并。这轮失败保留，不能当作通过。最终版本在缓冲前忽略未实现的光标位图和音频输出，再做区域合并，通过上述真实窗口验收。
- 合并只删除未开始发送、同宽且被更新画面完全覆盖的旧 JPEG 区域，不跨分辨率/控制消息屏障，部分重叠仍保留；剪贴板保持自身顺序，在完整记录边界优先。原生 ACK、5 秒阻塞保护、授权撤销和容量上限均保留。
- 前端回归先在旧实现上失败，再在修复后通过。没有通过扩大队列、延长超时、降低固定帧率或另建控制连接来绕开故障。

私有证据目录位于远端 /config/validation-artifacts：native-desktop-correlated-20261019（关联追踪与前后回归）、native-desktop-coalescing-20261019（首次真实失败）、native-desktop-coalescing-v2-20261019（最终源码、构建、全部新测试、五轮 GUI 和清理）。剪贴板正文、截图、环境与凭据不提交 Git。详细时间线见[本次专项记录](remote-workspace-validation-2026-10-18.md#2026-10-19-selkies-慢查看器与原生剪贴板修复)。

### 上一轮 Selkies 修复的提交与 GitHub CI

产品提交为 **2c95dd49d7a11a6a6c6aedf87cf67dc769e9bd6a**（fix: keep Selkies clipboard responsive for slow viewers），已推送 origin/main。以下结果均核对该完整提交；最后补充 CI 结果的提交仅修改文档，不改变已验收的产品源码。

| 工作流 | 运行编号 | 结果与范围 |
| --- | --- | --- |
| Cranibox server and independent node client | 39162097500 | 7/7 成功；Linux、Windows、macOS 的 Rust 服务端/节点构建及 Tauri 安装包，另有独立 Sway/wayvnc 作业 |
| Release | 39162097404 | 成功，脚本验证及 README 下载入口检查 |
| Sync GitCode | 39162097385 | 成功 |

Linux 日志确认 Web Playwright 126/126、客户端 Playwright 48/48；真实 Chromium/官方 MCP、X11 VNC、Sway/wayvnc、EasyTier 两项及 GIO 启动器均通过。三平台 Rust、原生传输与打包步骤也通过。它们与远端表中的重复范围不累计；后续 Linux 包实测见上节，Windows/macOS 实机仍未验收。

矩阵共上传 10 项 Actions Artifacts：三平台各自的 cranibox、cranibox-node、cranibox-node-desktop，以及 resource-baseline-linux。客户端 Ubuntu 产物含 .deb/AppImage，Windows 为 NSIS .exe，macOS 为 .dmg；在 Actions 中打开运行 39162097500 下载。资源报告仅作为该 CI 环境的产物，不将旧版本的资源数字当成本次测量。

私有证据目录 native-desktop-coalescing-v2-20261019 追加 ci-final.json 与 ci-native-linux.log、ci-desktop-linux.log、ci-wayland.log，保存完整提交、七项作业状态、产物元数据和上述原始日志。下一轮下载包时仍须核对提交，不直接使用其他运行的产物。

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
| P0 | 正常登录及跨设备的持续桌面流程 | 2c95dd4 的 CI .deb/AppImage 已通过隔离 KDE 双向文字、等待与重连；84454e6 的两包另通过真实检测/核对/取消/保存/停止。继续在正常桌面及另一物理设备验证连续双向中文/emoji/多行、超时/恢复、撤权与断网，不以隔离 XDG/私有 D-Bus、debug 或模拟 IPC 替代正常登录验收 |
| P0 | Windows/macOS 桌面互访与安装 | 原生安装、录屏/输入权限、VNC 提供者引导、托盘/自启、真实登录和休眠恢复、网关断网及退出清理；从另一设备看到画面、输入和撤权断开。WSL2 不是 Windows 原生验收 |
| P0 | 发布兼容的公开产品镜像并验收 | 已确认默认公开标签的 Linux amd64 镜像来自旧提交且缺少空间协议标签，当前无法新建空间。发布包含当前代码及协议标签的镜像，核对来源、真实拉取、首次启动和重启；保留 71f2397 本地完整镜像验收的版本边界，不删除协议保护 |
| P1 | 维护者真实三设备流程 | Mac 委托 Linux/Windows Agent，关页面后继续，换入口回复和核对结果，覆盖断网、休眠、重启，不靠同容器替身或手改账本 |
| P1 | Tailscale 系统安装/登录与 EasyTier 多设备 | 干净的三平台主机验证方案确认、管理员授权、取消/失败恢复、系统服务、Tailnet 互访与目标授权；不能以夹具代替系统安装 |
| P1 | 完整统一视图与同步协议 | 明确设备/项目/Agent/任务的权威、版本、删除及冲突；其他入口看到同一授权范围，离线变更有处理规则，撤权有效 |
| P1 | 设备信任与能力提供者 | 公钥配对、签名身份、轮换/撤销；Computer Use、Browser Use 在多物理设备与 Windows/macOS 验收，完善依赖引导。cua-driver 是否替换现有桥接需独立评估 |
| P2 | Agent 会话接续与运行交互 | 实测具体 CLI 登录和模型调用；检查点、项目文件/上下文一致性、目标启动及源端停止有明确状态。当前不支持移动原进程或向运行中 Agent 追加交互输入 |
| P2 | 交互、规模与测试稳定性 | 按日常流程检查长清单和错误恢复，评估大量设备/文件；若 Wayland 认证或 KDE 输入超时重现，定位原故障，不以放宽超时或重跑通过代替修复 |

暂不需要为节点身份等同引入选主。当前数据由所属节点持有，界面连接任一运行入口；若要复制协作协调状态，先定义故障与冲突模型，再决定一致性协议。

## 接手与复现

先读本文与[验证记录](remote-workspace-validation-2026-10-18.md)，再看[分布式设计](distributed-agent-workspace-design.md)、[组网设计](decentralized-node-network-design.md)、[节点直连](direct-node-peers.md)和[设备网络](device-networks.md)。功能细节见[客户端](node-desktop-and-device-mesh.md)、[桌面](remote-desktops.md)、[Computer Use](computer-use.md)、[Browser Use](browser-use.md)、[虚拟空间](virtual-spaces.md)、[项目目录/复制](project-session-catalog.md)、[消息](device-messages.md)、[任务](device-tasks.md)、[会话](agent-sessions.md)与[后台执行](agent-executions.md)。设计文档包含尚未实现的提案，以本文状态与源码为准。

### 源码入口

| 位置 | 内容 |
| --- | --- |
| crates/cranibox | 合并服务端入口 |
| crates/workspacectl、crates/workspace-services | Rust 控制工具与服务兼容管理 |
| crates/workspace-runtime/src/peers、projects | 连接、目标授权客户端、消息/任务/项目账本、文件复制与后台同步配置 |
| crates/workspace-node/src/{local.rs,peer,network,spaces,supervisor.rs,task_worker.rs,project_sync.rs} | 节点宿主、直连服务、网络/空间、进程、自动接单与后台目录调度 |
| crates/workspace-node/src/network/tailscale_setup.rs、install-tailscale*.sh | Tailscale 十分钟一次性方案、后台状态记录、平台安装/启动；不会自动登录或重放未完成安装 |
| crates/workspace-web | 登录/CSRF、Web API、终端/桌面和空间应用转发 |
| crates/workspace-runtime/src/peers/desktop/{selkies.rs,selkies/output.rs}；addons/control-center/ui/src/desktop/{openDesktop.ts,SelkiesViewer.ts} | Selkies WebSocket 适配、有界画面合并、受限输入、首包协议选择、JPEG 解码与键鼠；真实桌面回归为 tests/test_desktop_real_selkies.py |
| crates/workspace-node/src/desktop_service/{discovery.rs,provider.rs}；addons/control-center/ui/src/desktop/{ExistingDesktopServices.tsx,RemoteDesktopsView.tsx} | 常用回环地址和已保存服务的有界检测、提供者兼容性；共用配置核对、取消与保存前协议检查 |
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

### 远端环境

- SSH：admin@192.168.217.6，端口 22022，容器 cranibox-validation，用户 abc。运行验证继续使用这个环境，不改用当前本机。
- Web：https://192.168.217.6:3080；KDE/Selkies：https://192.168.217.6:3031。核对入口使用 LAN 地址。凭据只保存在远端私有 container.env，不能输出或提交。
- 容器源码 /config/agent-workspace-manager/source；Cargo 目标 /config/validation-build/target；证据 /config/validation-artifacts。远端源码为快照，不含本地 Git。
- 日常 Web PID 538744、原生客户端 PID 767446；本轮均未升级。两者独立身份，客户端使用 /config/validation-client/{config,data,state}；不得使两个宿主进程共用 NodeID。测试 GUI 的隔离 XDG、私有 D-Bus 不代表正常桌面集成。
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
npm test --prefix apps/node-desktop
npm test --prefix addons/control-center/ui
npm test --prefix addons/control-center/ui -- tests/desktops.spec.ts --workers=1
~~~

Web 的 npm run build 只生成 ui/dist；scripts/build-control-center-ui.cjs 另生成带许可证的 web.js、web.css 与两份 HTML，Rust 服务嵌入后者。共用 React 修改后必须重新生成资源并重建 cranibox。直接 Cargo 运行原生程序需 desktop,custom-protocol，避免误连已停止的 Vite 开发服务。完整 CI 定义在 .github/workflows/rust-runtime.yml；原生 IPC 模拟测试和包构建不能代替真实窗口。

真实 KDE/Selkies 测试会主动接管现有桌面和文字剪贴板，先确认当前会话，按该容器环境运行；用例检查并恢复原纯文本剪贴板，不输出内容：

~~~sh
DISPLAY=:1 WAYLAND_DISPLAY=wayland-0 XDG_RUNTIME_DIR=/config/.XDG CRANIBOX_TEST_CHROMIUM=/usr/bin/chromium CRANIBOX_TEST_REAL_SELKIES=127.0.0.1:8082 CRANIBOX_TEST_SELKIES_CLIPBOARD=1 python3 -m unittest test_desktop_real_selkies -v
~~~

Browser Use 的 test_browser_real_chromium 需 CRANIBOX_TEST_REAL_BROWSER=1 与 CRANIBOX_TEST_CHROMIUM=/usr/bin/chromium，使用临时 profile/随机端口，不访问日常 9222。Computer Use 为 test_computer_use；更多条件见对应功能文档。下一位模型从 P0 的正常桌面、另一物理设备及跨平台流程开始；同提交 Linux 包的隔离 KDE 验收已完成，保留已有日志和版本界限，勿重复已经撤回的诊断方案。
