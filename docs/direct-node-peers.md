# 节点直连与目标本地授权

2026-10-16：阶段 B 的直连 CLI/MCP、共享设备管理页面和浮动终端。`cranibox-node` 可以单独提供 HTTP/WebSocket API，另一个设备使用目标颁发的凭据直接访问它。Workspace 中继无需启动。原网关连接保留兼容；两条入口在同一个节点进程中共用原生 PTY 和普通命令的四个执行名额。

这还不是完整的去中心化工作空间：公钥身份、配对码核对与节点间端到端加密已实现（[节点身份与端到端加密](node-identity-and-e2e.md)），全网目录自动分发和既有连接的无缝迁移尚未完成；已支持同一设备的多条显式路径和新连接选路。节点已支持[明确启动的本机后台执行与 Codex/Claude Code 适配](agent-executions.md)、[默认关闭的接收端自动接单](device-tasks.md#本机自动接单)；运行会话迁移仍待实现。EasyTier 托管、节点发现、系统 Tailscale 登录与按设备接入见[设备网络](device-networks.md)；持久收发箱、回复、收件确认及独立 MCP 见[设备消息](device-messages.md)。Web/Tauri 的共享项目页面已汇总网关与已授权直连节点的目录，支持明确启用的后台目录增量同步；设备页通过同源后端/Tauri IPC 管理入站授权、出站连接、普通命令任务和终端。两个入口共用 React 浮动终端，分别由 Web 后端和本机 Rust 连接目标。Web 服务所在设备（包括 Docker Workspace）启动时自动获得持久 NodeID，能够以相同的直连协议授权其他设备使用自己的能力。

## 身份与授权

`cranibox-node peer identity` 创建并显示本机持久 NodeID。配置保存在节点 `settings.json` 同目录的 `peers.json`；所有命令支持 `--config /path/to/settings.json` 选择隔离目录，即使该 settings 文件不存在也可以使用 `peer`。NodeID 与 IP、主机名无关，删除配置会丢失身份和授权。

每个节点首次运行时生成持久 Ed25519 签名密钥和独立的 X25519 Noise 密钥（私钥保存在 `peers.json` 旁的 `node-identity.json`，`0600`，不会被覆盖或导出）。新节点的 NodeID 由公钥派生（`node-` + 公钥哈希前 128 位，与旧格式同形），已有随机 NodeID 保持不变并由同一密钥自签名绑定。`peer identity` 显示公钥指纹。`peers.json` 只保存 bearer 凭据的加盐 SHA-256 摘要，授权记录由目标节点签名；手工改写权限或去掉签名会被拒绝。远端请求用目标颁发的 256 位凭据授权；新版节点之间所有 mesh 调用自动走 Noise IK 端到端加密通道，凭据只在加密握手内传输，并在首次连接时互相固定公钥。两端 `peer identity` 显示相同的 12 位配对码，核对后执行 `peer verify --node NODE --code CODE`；新授权默认要求核对完成且拒绝明文请求（`--allow-unverified` 显式退出，旧授权保持不变）。终端、桌面与浏览器 WebSocket 的数据帧也逐帧端到端加密。身份、迁移和威胁模型详见[节点身份与端到端加密](node-identity-and-e2e.md)。

新设备的网关注册默认使用持久 NodeID；显式保存的 `settings.id`/`--id` 继续兼容。旧注册可在 Web“远程节点”或 `cranibox runtime migrate-identity` 中显式迁移到同一设备的 NodeID，保留 Token、设备组、项目和 Shell 关联，原 ID 作为兼容别名保留。运行中的网关任务会阻止实际迁移。网关与直连共用同一节点进程中的 Shell；两边的项目目录仍是独立来源，身份迁移不会自动合并目录或复制项目文件。详见[迁移旧网关身份](project-session-catalog.md#迁移旧网关身份)。

目标在本地分别授予：

- `--exec`：启动普通命令，查询和取消该凭据自己创建的作业。
- `--terminals`：管理、附着本节点的原生 Shell。此权限能执行任意 Shell 命令，也能接管本节点其他原生会话，不是只读权限或按项目隔离的权限。
- `--projects`：读写本节点全部项目目录元数据，包括路径和会话关联，不授予文件访问或执行权限。关联已有会话还需要 `--terminals`。此权限默认关闭，已有 Token 不会自动升级。
- `--computer`：通过目标明确配置的桌面桥接使用截图、短期输入控制权与窗口动作；默认关闭，不能远端恢复紧急停止。见 [Computer Use](computer-use.md)。
- `--browser`：操作目标明确共享的本机 Chromium CDP，包括所有标签页和登录状态；独立授权，默认关闭，持久紧急停止只能由目标恢复。见 [Browser Use](browser-use.md)。
- `--desktop`：访问目标已开启的本机 VNC 共享，可查看完整屏幕并发送鼠标和键盘输入；与其他能力分别授权，默认关闭。详见[跨设备桌面](remote-desktops.md)。

命令与终端能力以节点进程的操作系统账户运行。仅在目标执行 `grant` 才获得权限；知道 NodeID、加入 VPN 或持有原网关令牌不会自动获得直连权限。输出的身份视图不包含凭据；Unix 上配置和新令牌文件使用 `0600`，Windows 上继承所在目录的 ACL，应使用本人的私有配置目录。

## 两台设备的最小操作

先在 A 和 B 各运行一次，记录各自输出的 `node_id`：

~~~sh
cranibox-node peer identity
~~~

在目标 B 上，将下面的 `NODE_A` 替换成 A 的完整 NodeID：

~~~sh
cranibox-node peer grant --node NODE_A --output-token ./node-a.token --exec --terminals --projects
~~~

通过已有安全方式将该令牌文件交给 A。不要把令牌内容放进 URL、命令参数或代码仓库。重复授权必须先撤销旧授权；输出令牌文件采用不覆盖写入。

在 B 配置监听地址。仅本机测试或同机 TLS 反向代理可使用：

~~~sh
cranibox-node peer listen --address 127.0.0.1:9140
~~~

跨设备时使用已有私网地址；例如 B 在现有 EasyTier/Tailscale/其他可信私网中的地址是 `10.42.0.2`：

~~~sh
cranibox-node peer listen --address 10.42.0.2:9140 --allow-private-http
~~~

节点监听器本身提供 HTTP/WS。默认拒绝非回环明文监听和远端 HTTP 地址；`--allow-private-http` 表示显式信任底层私网加密和访问控制，程序不负责判断该接口是否确实属于 VPN。公网使用经过可信证书验证的 HTTPS/WSS 反向代理，后端保持回环监听，保留 Authorization、X-Cranibox-Target 与 WebSocket Upgrade 头。

启动 B 按宿主选择：

- 已使用普通 CLI 常驻服务：通过 CLI 配置后重启该服务。重启会结束原来由该进程持有的 Shell，应先结束当前工作。托盘应用可直接在“设备网络 → 入站授权 → 配置监听”中保存并启用，无需重启。
- Web 服务所在设备：直接在“远程节点 → 入站授权 → 配置监听”中启用。Web 内嵌节点复用原有项目目录与 herdr Shell，无需另外运行 `peer serve`。也可以用 Web 配置根目录中的 `.config/cranibox-node/settings.json` 作为 CLI 的 `--config` 参数配置，下次 Web 启动时应用。
- 无需 Workspace 连接：运行下面的独立模式，不需要创建 `settings.json` 或填写服务端连接。

~~~sh
cranibox-node peer serve
~~~

同一份身份只能有一个节点服务进程，包括未开启直连监听器的 Web/桌面宿主；不要对同一配置目录再运行 `peer serve`。`listen` 和 `disable` 在下次服务启动时生效，授权增删则实时读取。例如，Web 使用配置根目录 `/path/to/config-root` 时，可运行：

~~~sh
cranibox-node --config /path/to/config-root/.config/cranibox-node/settings.json peer identity
cranibox-node --config /path/to/config-root/.config/cranibox-node/settings.json peer listen --address 127.0.0.1:9140
~~~

在 A 上，将 `NODE_B` 替换成 B 的完整 NodeID，注册目标地址和收到的令牌文件：

~~~sh
cranibox-node peer connect --node NODE_B --url http://10.42.0.2:9140 --credential-file ./node-a.token --allow-private-http
cranibox-node peer call --node NODE_B nodes
cranibox-node peer call --node NODE_B exec -- hostname
cranibox-node peer call --node NODE_B sessions create --name work
cranibox-node peer call --node NODE_B sessions list
cranibox-node peer call --node NODE_B projects register --name app --path /home/me/app
cranibox-node peer call --node NODE_B projects list
~~~

HTTPS 地址不需要 `--allow-private-http`。`connect` 只保存并验证配置，首次调用才验证实际网络和凭据。每个目标可保存多条显式路径，使用同一份目标授权；新连接可在允许的网络故障下选择其他路径，身份或认证错误不会继续回退，不自动切到网关并重跑命令。

上述授权是 A → B 单向访问。需要 B → A 时，在 A 上为 B 单独授权并让 B 注册 A 的地址。设备之间没有主从关系，也不需要选主。

## Web 与 Tauri 设备页

“入站监听”显示实际运行状态和绑定地址，可保存并启用、切换端口、关闭入站接入。修改即时生效；新端口先绑定、配置成功保存后才替换原监听，失败保留原连接入口。切换或关闭监听会断开入站 WebSocket 连接，Shell、运行中的命令、任务记录和授权仍由节点保留。后台命令不会因为关闭端口而取消；如需停止工作，应先取消任务或撤销对应授权。端口为 0 时显示系统实际分配的端口。启动时端口被占用也不妨碍 Web/桌面宿主管理本机，可在页面查看错误并换端口恢复。

API 的 listener 操作由运行中的本机节点处理，不允许仅写配置后返回启用成功。设备列表和授权操作返回的 listener_runtime 包含 active、address、applied 和启动错误；applied 可区分 CLI 另行修改的配置与当前实际状态。普通 CLI 的 peer listen / peer disable 仍在下一次服务启动时生效。

Web 的“远程节点”页面、Tauri 的“设备网络”页面共用直连设备组件，支持创建当前接入端身份、创建/查看/撤销入站授权、保存/编辑/移除出站连接、检查目标及授权，以及提交命令、查看 stdout/stderr、取消任务。Tauri 尚未配置 Workspace 中继也可以使用此页面。浏览器访问的 Web 服务仍需运行；Workspace 中继可以离线。

Web 与 Tauri 启动时会创建直连身份，页面可查看 NodeID。在目标设备的“入站授权”中填写调用方的直连 ID，按需勾选命令、终端、项目目录、只读空间目录、桌面、Agent 桌面操作或消息与协作任务权限，点击“创建授权 Token”。权限默认不勾选；命令和终端均以节点的系统账户运行，终端权限允许任意 Shell 命令及该账户所有终端会话。Token 仅在创建成功时展示，关闭弹窗后无法再次读取；丢失时撤销旧授权再创建。复制失败会选中 Token，支持手动复制。页面和浏览器存储不会保存已签发 Token。

把 Token、目标设备 ID 和可达地址填入调用方的“添加直连设备”。出站连接与目标的入站授权分别管理；删除其中一个不会删除另一个。目标监听器可在目标的 Web/Tauri 页面配置并启用；保存授权不会自行开启端口或安装 VPN。页面显示实际监听状态，并区分尚未应用的配置。Web 上授权的是 Web 服务所在设备；Tauri 上授权的是客户端所在设备。

撤销授权会停止该调用方运行中的直连命令并关闭终端附着，后台 Shell 仍保留。授权列表只包含设备 ID 和能力，不返回 Token。原 CLI 的 peer grant/revoke 继续兼容。

添加时填写目标 NodeID、可达的 HTTP(S) 源地址及目标颁发的 Token。地址不接受路径、查询或内嵌凭据；非回环 HTTP 需要显式勾选可信私网选项。保存后点击“检查连接”，页面显示目标实时返回的系统及命令、终端、项目目录权限；检查通过且有终端权限时，可点击“打开终端”进入共用浮动窗口，列出并接回已有 Shell，也可创建、改名或显式结束会话。最小化保留当前附着；关闭窗口仅断开附着，重新打开仍可接回同一个 Shell。

Web 保存到其配置根目录下的 `.config/cranibox-node/peers.json`；Tauri 使用节点默认 `settings.json` 同目录的 `peers.json`，与 CLI 共用格式。Web 上的身份指 Web 服务所在的接入端，不是访问浏览器所在的电脑。一个 Web 实例当前仍是单用户管理边界。

Token 经 HTTPS/受信本地访问提交后单独写入私有文件，不返回列表或自动填入编辑表单。编辑时留空可保留原凭据，变更目标地址必须重新提供对应凭据。填写新 Token 会清除备用路径，避免把新授权自动发送到旧地址；留空编辑保留原凭据与路径。移除只删除本接入端保存的路由及页面管理的凭据文件，不撤销目标授权，不停止目标任务或 Shell；CLI 引用的外部凭据文件不会被删除。

命令采用“程序＋字符串 JSON 参数数组”，不会隐式经 Shell 执行。提交立即返回 JobID，页面只轮询已提交任务，不自动重试执行。刷新后可继续查询当前标签页最近 20 个任务；sessionStorage 只保留 NodeID/JobID，不保存输出或 Token。关闭页面不会取消命令，但超时、目标节点退出或授权撤销仍会结束普通命令。节点重启不保留作业，完成记录的保留规则见下文。

统一管理入口：Web `POST /api/peers` 使用已有登录会话、Origin 和 CSRF 校验；Tauri `direct_peers` IPC 调用同一本机节点与共享 Store。支持 `list / identity / listener / grant / revoke / connect / add_path / prefer_path / remove_path / forget / probe / exec / job / cancel / sessions / projects`、`computer_sharing / computer_owner / computer` 及 `copy_root / copy_prepare / copy_commit / copy_status / copy_list / copy_discard` 操作，不需要 Workspace 网关转发。listener 由运行中的本机节点应用。grant 返回 {catalog, node, token}，其他配置变更返回不含秘密的目录视图，computer_owner 与 computer 返回相应操作结果；所有 Web 响应保留 no-store。目录来源、ProjectID、关联会话及经确认的工作文件复制见[项目与会话](project-session-catalog.md)。

浮动终端共用 React 会话管理与交互组件。浏览器通过 `POST /api/terminal/sessions`、终端 ticket 和同源 WebSocket 访问 Web 后端，使用物理 NodeID 与 `direct: true` 选择直连路由；目标令牌留在后端。Tauri 通过 `terminal_open/input/close` IPC 和 Channel 访问本机 Rust 适配器，`direct: true` 选择本机保存的直连配置，false 使用已配置网关。两个适配器共用 Rust 的目标绑定、输入校验、鉴权与 WebSocket 传输；界面里的 `direct:` 前缀仅用于区分目标，不会当作节点身份发给目标。连接成功以实际收到 ready 为准，断线只重新附着，不自动新建 Shell 或重复执行命令。

## 同一设备的多条连接路径

一个 NodeID 最多保存 4 条有序路径，例如局域网、EasyTier 托管转发和 Tailscale 地址。每条路径共用该目标颁发的同一份授权，不创建重复设备。Web 与 Tauri 在设备详情的“连接路径”中添加、设为首选和移除路径；保留至少一条，否则应移除整个设备连接。添加前只检查目标身份与授权，不执行命令；失败不改配置并保留表单，成功后清除输入的 Token。

CLI 与 Agent 使用相同路径配置：

~~~sh
cranibox-node peer paths add --node NODE_B --url http://100.64.0.2:9140 --credential-file ./node-a.token --allow-private-http
cranibox-node peer paths prefer --node NODE_B --url http://100.64.0.2:9140
cranibox-node peer call --node NODE_B nodes
cranibox-node peer paths remove --node NODE_B --url http://100.64.0.2:9140
~~~

配置多条路径时，每次新调用或终端/桌面附着先按顺序进行只读鉴权与 NodeID 检查，每条路径限 2 秒、4 KiB；无法建立连接、超时、502–504 或已停止的本机 EasyTier 转发才尝试下一条。401/403、身份不匹配、无效协议等立即报错，不借备用路径绕过。托管端口失去本机锁后不发送目标凭据。

实际命令和 WebSocket 握手只发送一次；响应丢失不换路径重发。主路径中断后可重新附着同一目标进程持有的 Shell，新附着重新选路；Shell 的存活依赖目标进程。主路径恢复后，新连接重新优先使用它。已有 TCP/WebSocket 不跨地址无缝迁移，正在运行的终端不会因调整顺序而主动断开。

`probe` 与 CLI `nodes` 响应额外返回 `connection: {url, fallback, path_count}`。页面标记上次检查实际使用的地址；修改路径后清除旧检查结果，这不是持续在线监测。旧单地址配置继续读取；公开目录返回 `paths`，不返回 Token 文件或托管锁文件路径。

## Agent 桌面操作

节点可将已经运行的回环桌面桥接提供给明确授权的调用方，独立 computer 权限默认关闭。Web/Tauri 共用桥接配置、状态、授权、紧急停止及本机确认恢复；CLI/MCP 可截图、取得短期输入控制权和操作键鼠/窗口。它与桌面查看器共用一个控制名额，暂只走直连路由。具体操作及平台、测试边界见 [Computer Use](computer-use.md)。

## Agent 使用 MCP

在 A 的 MCP 配置中启动：

~~~sh
cranibox-node peer call --node NODE_B mcp
~~~

如使用自定义配置，在参数中加入 `--config /path/to/settings.json`。每个 MCP 进程绑定一个已配置目标；多个目标可注册多个 MCP 实例。提供 `workspace_nodes`、`workspace_exec`、`workspace_job`、`workspace_cancel` 和 `workspace_projects`。执行参数里的 `node` 必须为此目标，不能借这个端口转发到任意第三方节点。`workspace_projects` 接收 `{command: {operation: "list"}}` 等目录命令，直连注册时自动补齐目标 NodeID。

普通命令默认等待退出；`exec --detach` 返回作业 ID，可用 `peer call --node NODE_B job ID` 和 `cancel ID` 查询或取消。CLI 退出不结束已经提交的直连作业，目标节点进程仍需存活。普通命令最长一小时，不能替代独立的[本机后台执行](agent-executions.md)。

设备间纯文本通信另使用 `cranibox-node peer messages mcp`，不需要网关设置，提供本机持久收发箱、回复、重试、归档和明确收件确认；目标需单独授予 `messages` 权限。见[设备消息](device-messages.md)。

## HTTP 与终端 WebSocket

所有直连请求都需要：

~~~text
Authorization: Bearer <目标为调用方颁发的令牌>
X-Cranibox-Target: <目标持久 NodeID>
~~~

服务拒绝带 Origin 的浏览器请求及跨站请求，不提供浏览器 CORS 放行；Web 通过同源后端转发，Tauri 通过 IPC 调用共享 Rust 逻辑，浏览器不直接连接目标或持有已保存的令牌。目标头不匹配时在执行前返回 409。路由如下：

| 方法 | 路径 | 作用 |
| --- | --- | --- |
| GET | `/api/v1/mesh/nodes` | 此目标及授予当前凭据的能力 |
| POST | `/api/v1/mesh/nodes/{NodeID}/computer` | 独立 computer 授权；状态、短期控制权、桌面动作与紧急停止 |
| POST | `/api/v1/mesh/nodes/{NodeID}/exec` | 提交命令；请求体沿用 ExecRequest |
| POST | `/api/v1/mesh/nodes/{NodeID}/messages` | 保存授权发送方的消息，返回持久回执 |
| GET | `/api/v1/mesh/nodes/{NodeID}/messages/{MessageID}` | 查询此发送方的收件/明确确认回执 |
| GET | `/api/v1/mesh/jobs/{JobID}` | 自己作业的输出和结果 |
| POST | `/api/v1/mesh/jobs/{JobID}/cancel` | 请求取消自己的作业 |
| POST | `/api/v1/mesh/nodes/{NodeID}/terminal/sessions` | list/create/rename/end/remove，沿用原生会话请求 |
| GET/Upgrade | `/api/v1/mesh/nodes/{NodeID}/terminal/ws?session={SessionID}&cols=80&rows=24` | 附着已有 Shell，可显式加 `takeover=true` |
| GET/Upgrade | `/api/v1/mesh/nodes/{NodeID}/desktop/ws` | 经桌面权限校验后转发本机 VNC 的二进制 RFB 字节 |

直连 WebSocket 客户端消息使用 JSON：

~~~json
{"type":"input","data":"<标准 Base64 字节>"}
{"type":"resize","cols":100,"rows":30}
{"type":"detach"}
~~~

服务发送 `{"type":"output","data":"<标准 Base64 字节>"}` 与 `{"type":"exit","code":0}`。共享 Rust 适配器将界面的 UTF-8 输入转换为目标协议的 Base64 字节，并将输出还原为终端字节流；浏览器无需直接实现目标协议。

断开附件、浏览器或网关不终止 Shell；目标节点进程持有 Shell 并保留有界回放。节点进程退出或重启会结束它自己的 Shell，作业和会话目录也不会跨节点进程重启恢复。持久身份、授权、路由与这些运行态分开保存。直连最多同时四个终端附件，原生会话目录仍采用现有引擎上限。

## 撤销与结果边界

在 B 撤销 A：

~~~sh
cranibox-node peer revoke --node NODE_A
~~~

新请求立即拒绝；活动作业和终端附件约每 250 毫秒重新检查授权，并启动取消或断开。普通命令沿用进程树清理；终端附件断开后 Shell 继续存在，已在 Shell 中启动的后台任务不会因撤销授权而自动结束。需要结束会话时由目标用户或另一个已授权管理者显式 end。此撤销不修改旧网关的设备组权限。

重新授权使用新令牌。旧令牌不能重新连接，也不能用新凭据读取旧作业；其他调用方同样看不到这些作业。配置缺失、损坏或运行中身份变化会拒绝新请求并撤销活动直连执行，不自动重建身份或覆盖损坏文件。

普通作业输出 stdout/stderr 合计最多 64 KiB，返回 Base64 原始字节和文本展示以及 `truncated`。每节点最多保留 128 个内存作业；新提交时清理已完成超过十分钟的记录。不能依赖它作为持久审计日志。

HTTP 提交可带 UUID `Idempotency-Key`。同一凭据、同一规范化请求在记录尚保留时返回原作业，参数不同返回 409；记录过期或节点重启后不提供去重保证。CLI/MCP 没有自动重试提交。提交后连接丢失且未收到 JobID 时应先核实副作用，不应直接认为命令未执行。

## 验证与后续

`tests/test_node_peers.py` 使用真实 HTTP/WebSocket、CLI/MCP 和 PTY 验证独立运行、目标绑定、能力权限、作业隔离、重复请求、容量、输出上限、取消/超时、实时撤销、重启/损坏，以及停止 Workspace 后接回同一 Shell 和继续新建调用。测试只创建临时配置与随机回环端口。`tests/test_web_direct_peers.py` 进一步验证 Web 中继离线、Web 重启后恢复路由及原任务查询、取消、身份绑定、撤销、CSRF、私有凭据和损坏配置处理。网页签发的入站授权另由真实节点验证终端允许、命令拒绝、撤销即时失效与重启保留。共享页面与 Tauri IPC 有浏览器回归，覆盖刷新不重复提交、能力拒绝、错误恢复、一次展示 Token、键盘关闭、复制失败回退和移动布局。

本轮真实 Web/节点测试还覆盖中继停止后创建与接回直连 Shell、环境变量和进程号保持、改名、尺寸调整、撤销附着及后台 Shell 保留；既有本地和中继终端回归继续通过。浏览器测试覆盖 Web 适配器及模拟 Tauri IPC 下的共享终端、输入、最小化、关闭和重新附着。容器缺少 Tauri 的 Linux 系统开发依赖，尚未完成桌面原生构建和三平台实际运行验证，IPC 模拟不能替代这些验证。

阶段 B 已实现 EasyTier 托管、多入口与短期邀请，及系统 Tailscale 登录、按设备查找节点和保存目标授权，见[设备网络](device-networks.md)。共享项目目录、明确启用的后台目录增量同步、旧网关身份显式迁移、离线只读快照、本机/直连节点间经确认的分块工作文件复制，以及[持久设备消息与独立 MCP](device-messages.md)也已实现。新连接支持显式多路径选路；公钥身份、配对码核对与端到端加密已实现（见[节点身份与端到端加密](node-identity-and-e2e.md)），WebSocket 数据帧也已逐帧加密；仍需全网目录自动分发及既有连接无缝迁移。本机后台执行、会话寻址、Task/Dispatch 和默认关闭的自动接单已有明确入口，运行会话接续见[交付设计](distributed-agent-workspace-design.md)。

空间管理节点还可单独授予 --spaces，只共享基本空间目录，不授予 Docker 操作、空间登录凭据或空间节点访问。Web/Tauri 会在“虚拟空间”中汇总已连接节点的授权目录；CLI 使用 peer call --node NODE spaces。详见[跨设备空间目录](virtual-spaces.md#跨设备空间目录)。
