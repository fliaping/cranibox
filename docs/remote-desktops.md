# 跨设备桌面

Web 的“远程桌面”和 Tauri 的“桌面”共用 React 查看器及本机服务管理页面：VNC 使用 noVNC，已有 Selkies 服务使用 JPEG 查看器。浏览器通过同源 Rust 后端连接目标；桌面客户端通过本机 Rust IPC 连接目标。节点令牌留在 Rust 侧，查看器输入的 VNC 密码不写入浏览器存储或节点配置。托管服务生成的密码保存在目标设备的私有文件中，只在主动请求时返回。

## 开启共享

1. Linux/X11 可以在“本机 VNC 服务”启动托管的 x11vnc；兼容 wlroots 的 Wayland 登录会话可使用 wayvnc 0.8 或以上版本。Ubuntu/Debian 桌面主机还提供安装命令预览及系统认证后的包安装。Workspace 的 KDE/Selkies 容器可选择下文的 Selkies 接入；其他不兼容 wayvnc 的会话以及 Windows/macOS 可接入已有 VNC 服务，限制监听到 `127.0.0.1` 或 `::1`。这些环境的自动安装与进程托管尚未实现。
2. 使用已有服务时，在该设备的“桌面 → 共享这台设备的桌面”填写 `127.0.0.1:5900` 或 `[::1]:5900`，保存并开启共享。页面先检查 VNC 握手，连接失败时不启用共享。这里只接受非零端口的回环地址，不能用它转发其他主机。托管服务启动成功时已完成此步骤。
3. 在“设备网络 → 入站授权”为调用方 NodeID 单独勾选桌面权限。已有执行、终端或项目授权不会自动获得桌面权限。浏览器调用方是运行 Web 的设备，页面会显示需要授权的 NodeID；Tauri 调用方是客户端所在设备。
4. 选择连接路径：两台设备接入同一 Workspace 时，桌面页会列出中继设备，无需开放目标入站端口；节点凭据调用还要求双方属于同一设备组。也可以配置目标入站监听，在调用方保存直连地址、NodeID 和目标签发的 Token；私网明文入口需明确允许可信私网 HTTP。
5. 进入“桌面”，选择目标并点击“连接桌面”。同时有直连和中继时，同一设备只显示一次，可在“连接方式”切换；切换会断开查看器，下次连接重新检查授权。VNC 要求认证时，查看器提示输入凭据。

Web 中“本机”指运行 Rust Web 服务的设备，包括 Docker 容器，不是浏览器所在电脑。Tauri 中“本机”指安装客户端的电脑。

CLI 使用相同配置：

~~~sh
cranibox-node peer desktop --address 127.0.0.1:5900
# 新建授权时按需附加 --exec、--terminals、--projects
cranibox-node peer grant --node NODE_A --output-token ./node-a.token --desktop
# 不带 address 表示停止共享
cranibox-node peer desktop
~~~

已有授权需要先撤销再重新签发，才能改变权限。其他直连配置见[节点直连与目标本地授权](direct-node-peers.md)。

## 已有 Selkies 服务

KDE 容器不一定支持 x11vnc/wayvnc 的捕获方式。如果已有可用 Selkies，在“配置共享”选择 Selkies，填写该设备内部的 WebSocket 地址，然后保存。当前验证镜像的地址为 `127.0.0.1:8082`；它不是浏览器入口 3000/3001 或宿主映射的 3031。端口取决于部署配置，应先核对监听。检查只等待 `MODE websockets`，不发送 SETTINGS，因此不会接管现有桌面。

~~~sh
cranibox-node peer desktop --protocol selkies --address 127.0.0.1:8082
~~~

这条路径使用现有桌面授权、单控制者、本机/直连/中继及 Tauri IPC，不另外开放远端 Selkies 端口。连接时选择 primary 显示器、JPEG 编码、30 FPS 和质量 65，不请求修改桌面分辨率；支持画面、键鼠、移动端输入框、只读、重连和撤权断开。一次粘贴产生的键盘命令合并为不超过 64 KiB 的传输块，避免小消息挤满节点输入队列。

当前只接入运行中的、支持 JPEG 和 `MODE websockets` 的 Selkies。服务须无额外 token 握手，并将内部接口限制在可信容器/回环网络；没有实现 Selkies token 认证、安装或进程托管。连接会接管上游主显示器并使原有 Selkies 查看器断开，其他 Selkies 客户端仍可反过来接管。Cranibox 的单控制者限制只覆盖自身通道。支持下述纯文本剪贴板，暂不支持音频、文件传输、视频编码或多显示器。没有复制 Selkies Web UI 或引入其运行时库。

## Linux/X11 托管服务

在目标设备的图形登录会话中运行节点或 Tauri 客户端，选择 X11 显示器（例如 `:0`）及本机端口，再点击“启动并共享桌面”。服务继承节点的 `DISPLAY`、`XAUTHORITY`，共享已有桌面，不创建新的登录会话。启动成功并收到 VNC 握手后才启用节点共享；端口占用或显示器不可访问时保留输入并显示错误。若之前接入了另一个 VNC 服务，先停止其共享。

Rust 节点负责保持 x11vnc 运行，关闭查看器、切换页面或断开网关不会终止它。节点正常退出会停止子进程；重启节点后按已保存配置恢复。异常退出采用有上限的退避重试，连续五次失败后可检查环境并手动重试。“停止托管服务”或“停止共享”会禁用恢复并关闭托管进程。关闭 Tauri 窗口到托盘仍保持节点运行；操作系统注销或关闭设备后不保证图形会话继续存在。

使用相同配置运行独立的 `cranibox-node peer serve`，也会恢复已启用的托管服务，不要求连接网关。节点仍须有权访问原来的 X11 登录会话；“无界面运行节点”不代表能创建不存在的图形桌面。

服务只监听 IPv4 回环地址，关闭 x11vnc 远程管理接口，并使用随机的 8 位 VNC 密码；此认证应始终配合 Cranibox 的授权通道及可信私网或加密传输。配置旁的 `desktop-service.json` 和 `desktop-vnc.password` 为当前用户私有文件（Unix 权限 `0600`），状态及命令参数不包含明文密码。查看密码是单独操作，页面不会持久保存它。容器中不提供宿主机 VNC 包安装，也不会因此共享宿主机桌面。

## Linux/Wayland 托管服务

在已有的兼容 wlroots 图形会话中运行节点或 Tauri 客户端，例如 Sway。页面根据节点环境选择 wayvnc，填写本机 Wayland socket 名称（例如 `wayland-0`），可选指定输出名称（例如 `DP-1`，留空使用默认输出），再启动共享。节点继承有效的 `XDG_RUNTIME_DIR`，通过 `WAYLAND_DISPLAY` 连接所选会话，不创建新的图形登录。GNOME、KDE/Plasma 和 Weston 不支持这条路径，页面会说明原因，并保留接入已有 VNC 服务的入口。

Selkies 镜像中的后台节点可能只继承 `DISPLAY` 和 `PIXELFLUX_WAYLAND=true`，没有登录会话的 Wayland 变量。节点会识别这一环境，避免把 XWayland 误判成可以使用 x11vnc 的 X11 桌面；未选择 Wayland 会话时也不提供无效的安装和启动操作。页面提示在下方选择 Selkies 或接入兼容 VNC 服务。显式选择的独立 X11 会话仍按 X11 处理。Workspace Web 的“打开桌面”按镜像运行时是否存在显示，不要求先安装编辑器扩展。

wayvnc 需要 0.8 或以上版本。Ubuntu/Debian 桌面主机可先查看安装命令，经系统认证安装 wayvnc 后单独启动；容器不提供宿主包安装。启动失败保留显示器、输出和端口输入，成功后再启用共享。异常重试、节点重启恢复、停止共享和退出清理沿用上述托管服务生命周期；节点恢复时仍需访问原 Wayland 会话。

服务监听 `127.0.0.1`，启用用户名 `cranibox` 和随机密码认证。noVNC 需要用户名时会同时提示用户名和密码。私有配置 `desktop-wayvnc.conf` 以 Unix `0600` 权限保存，每次启动重新生成；控制 socket 位于当前用户的私有临时目录，进程停止后清理。认证凭据不进入命令参数或浏览器持久存储。为兼容 noVNC，回环段允许不使用 TLS 的 RFB 认证；跨设备仍须使用 Cranibox 授权通道及可信私网或 WSS，不应把原始 VNC 端口公开到网络。

## 查看器与连接

支持画面缩放、鼠标和键盘、全屏、只读查看、移动端输入框及断线后重新连接。只读是当前查看器的交互选项，不能代替目标授权。每个目标同时只接受一个桌面控制连接，本机和远程查看器共用这个名额。关闭页面、窗口或查看器只断开画面连接，不结束目标桌面。

停止共享或撤销权限会在约 250 毫秒内使目标上的活动转发失效；重新开启共享不会恢复旧连接。Web 退出登录会关闭对应浏览器连接。重连重新校验当前授权，不自动重新开启共享。

目前支持本机、保存的直连路由和 Workspace WebSocket 中继。仅出站连接网关的节点也可转发 VNC；设备列表来自网关目录和已保存的直连连接，路径由用户选择，尚未自动切换。本机、直连和中继共用目标的单控制者名额。中继断开不会重放键鼠输入，重新连接时重新建立通道。[设备网络](device-networks.md)可安装和托管 EasyTier、发现节点并保存授权连接，也可读取系统 Tailscale 状态、打开官方登录，或选择在线设备核对 Cranibox 身份并保存授权。两种网络的连接保存后复用同一直连桌面入口；网络成员身份不会自动授予桌面权限。Docker Workspace 已有的 Selkies 入口继续保留。

中继模式信任所配置的网关认证和断言调用方身份，目标仍需为该 NodeID 显式授予桌面权限；注册或同组成员身份不能代替此授权。中继不是端到端加密通道，网关可接触 RFB 数据，公网使用 WSS。旧式独立 gateway 若没有本机 NodeID，不能代替浏览器主机断言调用身份，需使用统一 Cranibox 服务或节点凭据。

## 文字剪贴板

Web 和 Tauri 查看器连接后可展开“剪贴板”。左侧输入文字并点击“发送文字”，只会更新目标剪贴板；请在远端应用中自行粘贴，不会模拟快捷键或执行命令。“仅查看”模式禁止发送。右侧显示远端复制的文字，检查后点击“复制到本机”，或在浏览器禁止复制时使用已选中的文本手动复制。Selkies 另有“获取远端文字”按钮；旧服务剪贴板为空或关闭时可能不回复，未更新不表示读取成功。

获取期间显示等待状态并禁止重复请求。10 秒内没有收到文字会提示重新获取或重新连接，保留此前接收的文字供核对；这不表示桌面已经断开，也不能判断远端剪贴板是否为空。收到有效响应后结束等待，即使内容与上次相同或为空；迟到的响应也会清除超时提示。无效响应仍按下面的限制拒绝，关闭面板或断开时取消等待计时。此反馈改善不代表已修复下文的原生同屏停滞。

每次最多 64 KiB，按 UTF-8 字节计算；支持多行、中文和 emoji。noVNC 使用公开的 `clipboardPasteFrom` / `clipboard` 接口；旧 VNC 的 Latin-1 回退可能把中文替换为问号，Unicode 需要服务端支持扩展剪贴板。Selkies 使用 `cw` / `cr` 与纯文本通知，兼容独立的 base64 分块；分块限 256 个、10 秒，按声明长度校验后统一解码 UTF-8。非法、超限、图片、文件或明确标记为 secret 的数据会被忽略并清除旧接收内容，桌面连接仍可使用。

查看器不会自动读取或覆盖本机剪贴板，不保存历史、不写浏览器存储；关闭面板清除发送草稿，断开或切换目标清除暂存内容。网络可达不能代替桌面授权；剪贴板沿用当前桌面通道和控制者限制，不增加独立公开端口。VNC/Selkies 服务端仍需允许剪贴板。64 KiB 是本查看器的限制，不是服务端一定接受的保证；发送提示表示命令已提交，并非远端应用已粘贴。

## 协议

目标提供 `GET /api/v1/mesh/nodes/{NodeID}/desktop/ws`，使用与其他直连 API 相同的 Bearer 凭据和 `X-Cranibox-Target` 头，拒绝浏览器 Origin。VNC 连接转发原始二进制 RFB 字节，旧配置省略 protocol 时仍按 VNC 处理；每个消息不超过 64 KiB，队列有界，超时或背压异常会断开连接。

配置 `desktop: {address, protocol: "selkies"}` 时，Rust 仅连接该回环地址的 `/websockets`。授权流先发送 `CRANIBOX-SELKIES/1\n` 标记，后续记录为一个字节的类型（0 文本、1 二进制）、四字节大端长度及原始负载；记录可跨多个传输消息。单条输出记录最多 16 MiB，普通输入文本最多 16 KiB；仅 `cw,BASE64` 可使用容纳 64 KiB UTF-8 的 87,387 字节记录上限，Rust 在转发前校验 base64、UTF-8 和解码长度。输入只允许 JPEG 设置、键鼠、帧确认及纯文本剪贴板等受限命令，仍拒绝文件和音频流。查看器限制解码队列为 256 条/32 MiB、画面最大边长 8192 和总像素 32 Mi，超限显式断开；这些上限不是已做压力验收的性能承诺。

网关提供 `/api/v1/mesh/nodes/{NodeID}/desktop/ws` 接受节点凭据，并提供 `/api/v1/nodes/{NodeID}/desktop/ws` 接受控制凭据。目标通过控制 WebSocket 收到 `desktop_open` 后，用自身凭据及单次 `X-Cranibox-Desktop-Ticket` 头，主动连接 `/ws/desktops/{request-id}`；ticket 绑定目标和连接代次，不放在 URL。画面走独立二进制 WebSocket。网关最多保留 32 个待建立或活动中继，建立等待最多 15 秒。撤销调用凭据、设备组权限、轮换目标凭据或目标重新连接会使旧通道失效。升级响应头协商新增能力；旧节点显示升级提示，新节点连接旧网关时省略新增 Hello 字段。

Web 使用登录会话、Origin 和 CSRF 校验的 `POST /api/desktop/ticket`，请求为 `{node, local, relay}`；后两项默认 false，不能同时开启。签发前检查授权并建立后端连接，让目标错误在连接时直接显示。单次 ticket 有效期 30 秒，绑定登录会话，由同源 `/api/desktop/ws?ticket=...` 消费；不包含节点或 VNC 凭据。离开查看器或主动断开时，前端通过 `POST /api/desktop/cancel`（`{ticket}`）释放尚未消费的连接；其他登录会话不能取消，已消费 ticket 的取消也不会影响活动查看器。未消费 ticket 在到期或退出登录时同样释放连接和控制名额。Tauri 使用 `desktop_open/input/ack/close` 与 Channel，输出逐帧确认，避免无界积压；连接尚未建立就离开页面时，IPC 返回后立即关闭该连接。

原生通道在 10 秒未收到逐帧 ACK、收到无效 ACK 或提供者关闭时显示原因；主动关闭不报错。Rust 传输输入队列已满与提供者断开分别返回错误，若提供者已有失败原因则保留原原因。输出队列仍为 32 个位置，其中一个预留最终错误，慢查看器不会使失败被丢弃或阻塞控制名额释放。Selkies 接收队列阻塞 5 秒同样报告原因；这些改动不自动重发输入，也不将断线后的连接状态视为桌面仍可用。

服务管理通过同样需要登录和 CSRF 的 `POST /api/desktop/service` 或 Tauri `desktop_service` 调用，支持 `status`、`plan`、`install`、`start`、`stop`、`credentials` 及仅限回环地址的握手检查 `check`。`check` 可传 `protocol: "selkies"`，省略时检查 VNC。安装和启动是独立操作，查看安装计划不会执行命令。`start` 接受 `display`、`port`，Wayland 另传 `provider: "wayvnc"` 和可选 `output`；省略 `provider` 保持原有 x11vnc 请求兼容。`status` 返回所选提供者、兼容性说明和输出；只有显式 `credentials` 操作返回密码，wayvnc 同时返回用户名。

## 验证范围

`tests/test_desktop_relay.py` 使用真实出站节点和隔离 VNC 服务验证中继、重连、目标授权、调用身份、设备组与凭据撤销、跨路径单控制者、未消费 ticket 到期释放及登录会话绑定的显式取消。Rust 测试额外验证 rendezvous 的目标凭据、单次 ticket、连接代次绑定及取消清理。

`tests/test_remote_desktops.py` 使用真实 Rust Web、节点和回环 TCP 服务，验证二进制转发、目标绑定、CSRF、ticket、停止共享、撤销和单控制者。`tests/test_desktop_service.py` 使用真实 Rust 节点与隔离的 x11vnc/wayvnc 替身验证进程守护、节点恢复、密码和配置权限、私有控制 socket、配置重新生成、输出恢复、版本与会话兼容性检查、启动失败及主动停止，不安装系统包或访问真实桌面。

`tests/test_desktop_real_vnc.py` 另外运行真实 x11vnc 和独立 Xvfb 显示器，验证密码认证、错误密码拒绝、实际像素、X11 键鼠事件、Web 重启恢复，以及同一配置切换到无界面节点后的远程授权访问、撤销和退出清理。需要系统已有 x11vnc、Xvfb、xev、xsetroot、xdotool、Node 和 Web UI 的 npm 依赖；设置 `CRANIBOX_TEST_REAL_VNC=1` 后运行，Linux CI 已配置此检查。它不使用当前登录桌面，也不会自行安装系统软件。

`tests/test_desktop_real_wayland.py` 使用真实 wayvnc 和独立的无显示器 Sway 会话，使用 noVNC 的认证实现验证 ARD 用户名/密码、错误用户名和密码拒绝、实际像素、键盘、崩溃重试，以及关闭 Workspace 后独立节点恢复同一桌面、直连授权访问、撤销和退出清理。需要 wayvnc 0.8 或以上、Sway、swaybg、Node 和 Web UI 的 npm 依赖；以普通用户设置 `CRANIBOX_TEST_REAL_WAYLAND=1` 运行。CI 为此单独使用 Ubuntu 26.04 容器，不依赖 runner 的已有桌面；本地验证使用 wayvnc 0.9.1、Sway 1.11，未安装系统包。

浏览器测试使用实际 noVNC 与最小 RFB 服务验证画面、输入、密码及 ARD 用户名/密码认证、拒绝错误凭据和重连，另验证 Wayland 安装预览、表单恢复及明暗主题的移动布局；Tauri 前端测试使用模拟 IPC。隔离的 Sway 检查不能代替其他 Wayland 合成器、Windows/macOS 实机，或系统录屏/辅助功能权限、托盘及原生安装包的人工验收。noVNC 的归属和许可证见 [第三方声明](licenses/novnc.txt)。

Selkies 的 Rust 单元与服务集成覆盖分片/合并、输入白名单、128 KiB 输出的分块、只探测不接管、慢查看器撤权和控制名额释放。Web 使用真实 Rust 通道与合成 JPEG 服务验证画面、连续文字、鼠标、只读及重连；Tauri 使用模拟 IPC 验证分片、逐帧确认、JPEG 和输入合并。`tests/test_desktop_real_selkies.py` 另需明确设置 `CRANIBOX_TEST_REAL_SELKIES=127.0.0.1:8082`，通过出站节点中继访问现有 KDE 桌面、向自身 kdialog 输入完整文字、重连及撤权。它会接管正在使用的主显示器，要求已有 Chromium、Playwright、kdialog 与图形会话环境，不在普通 CI 自动执行。2026-10-18 在指定远端容器中通过；这不等于 KDE 的 VNC 捕获成功或完整产品镜像/安装包已重新验收。

剪贴板回归覆盖 noVNC 的 Latin-1 往返与 64 KiB 上限、Selkies 的中文/emoji、UTF-8 字节限制、跨传输块的输入、跨字符的输出分块、坏数据及 secret 拒绝、只读、主动复制、HTTP 复制受阻、离开页面后迟到的复制结果，以及断开/重连清除暂存内容。Web 使用真实 Rust 通道与 RFB/JPEG 夹具；Tauri 为浏览器模拟 IPC。真实 KDE 剪贴板另加 `CRANIBOX_TEST_SELKIES_CLIPBOARD=1`，要求图形会话与 wl-copy/wl-paste；测试先拒绝非纯文本原内容，临时验证双向文字后恢复原剪贴板并核对字节，不输出原内容。这是主动接管现有桌面的可选检查，不在普通 CI 自动执行。

2026-10-18 另实际运行 a8a97f5 的 Linux CI .deb/AppImage，原生 Selkies 画面通过；.deb 发送中文、emoji、多行文字到真实 Wayland 剪贴板通过，**反向“获取远端文字”仍未更新，原因未定位**。复现于私有 D-Bus 的 KDE 同机查看场景，断开重连后仍未收到新文字；不能用 Web 双向测试或模拟 IPC 通过代替原生双向验收。完整复现、包摘要和后续优先级见[交接文档](implementation-handoff-2026-10-18.md#最新补验a8a97f5-linux-安装包)。

后续直接 Selkies 协议诊断能读取新文字，AppImage 与 debug 原生接收也曾显示新文字，但后续更新仍停滞。诊断代理轮次约 60 秒断线；绕过代理的 debug 对照约三分钟后仍显示已连接且面板可操作，第二段新文字仍未更新，不能判定为同一根因。84fdf88 的前端错误保留与完整 48 项界面回归、本次 Rust 错误/ACK/队列测试均不等于剪贴板故障已解决。真实原生复查、证据与下一步见[交接记录](implementation-handoff-2026-10-18.md)。2026-10-19 新面板的 10 秒超时、恢复获取按钮及保留旧文字已在真实 KDE 原生 GUI 核对，更新停滞仍保留为 P0；最新 48 项客户端与 12 项 Web 桌面自动化通过不替代该未通过项。
