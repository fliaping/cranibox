# 节点 BASE / UPPER 拆分评估

> **更正（2026-10-24）**：herdr 是用户自行安装的应用层工具（多用于 Vibe Coding），不属于底座，也不加入我们的依赖。
> 本文早先把它当作“重启后找回 Shell”的最小改动，并据此写了“Windows 不支持”，这两点都不成立。
> 终端由我们自己的 Native 引擎提供（`crates/workspace-pty/src/persistent.rs`，`portable-pty`，Windows 为 ConPTY，默认 `COMSPEC`/cmd.exe，PowerShell 同样可用），三个平台走同一套接口。
> 让终端在上层更新时保持，不需要单独的终端进程，直接把这个引擎放进底座进程。底座 = 网络 + 授权 + 终端 + 远程桌面，共一个进程；上层是另一个进程。文中凡涉及 herdr 的内容仅作旁证。


评估日期：2026-10-24；对象：当前 worktree 源码。只读检查，未构建、未运行测试、未连接设备。行号指本次源码；表中行数包含注释与测试，目录递归计数，同名 `.rs` 与目录分别统计。BASE / UPPER 是建议归属，SPLIT 表示现有条目需拆职责，不代表已有进程边界。

## 1. 结论与边界

当前 `LocalNode` 同时拥有网络、终端、桌面、业务 worker 和共享停止信号；`shutdown` 一起结束它们（`crates/workspace-node/src/local.rs:15`、`:257`）。不是调整重启顺序就能得到两层节点。

建议渐进采用方案 (b)：BASE 持有 mesh HTTP 入口、身份与 roster、Noise、网络、PTY、桌面及最小恢复页面；业务 handler 移至 UPPER，经方案 (a) 的认证本地 IPC 转发。必须覆盖目标端、浏览器接入端和必要中继的整条连接链。UPPER UI/API 可以停机，BASE 恢复页面不能依赖 Models、Agent 或业务前端加载成功。

区分三个承诺：

- **UPPER 重启不断线**：BASE 保留 socket、流任务、PTY/VNC owner、浏览器会话和撤权监控。
- **节点进程重启后可重连原 Shell/桌面**：终端引擎在底座进程内；外部桌面服务继续运行；旧 WS、Noise 会话仍会断。
- **BASE 更新**：独立低频操作，允许连接中断，依靠本地 guard 回滚；本仓库未见已实现的完整升级事务。

## 2. 模块与入口清单

### `crates/workspace-node/src`

| 顶层条目 | 约行数 | 目的 | 归属与理由 |
| --- | ---: | --- | --- |
| `agent.rs` | 148 | CLI 子命令、MCP/exec/space 分发 | SPLIT：设备/配对调用 BASE，业务命令 UPPER |
| `app.rs` | 1561 | App 服务、设备列表、配对、授权、模型与应用操作 | SPLIT：配对/grant/网络状态属于 BASE，聚合 API 属于 UPPER |
| `app/` | 1351 | capabilities、已知应用、偏好 | SPLIT：终端/桌面/CUA 的控制接口留 BASE，其余业务 UPPER |
| `app_mcp.rs` | 209 | App MCP 服务适配 | UPPER：调用 BASE 能力接口，不能拥有设备运行时 |
| `cli.rs` | 165 | 桌面与 CLI 共用入口、配置路径、命令执行 | SPLIT：启动 BASE / UPPER 与客户端调用需分流 |
| `desktop_service.rs` | 650 | 管理 VNC 子进程、共享配置与恢复 | BASE：桌面兜底不能随业务停止 |
| `desktop_service/` | 639 | 探测、provider 校验、安装/设置 | BASE：服务生命周期和桌面接入前置条件 |
| `device_info.rs` | 130 | 主机信息、设备摘要 | BASE：离线业务时仍需最小身份/能力信息 |
| `lib.rs` | 22 | crate 模块导出 | SPLIT：重组依赖，避免 BASE 链接全部业务 |
| `local.rs` | 475 | 所有服务的容器、启动/关闭、目录与终端调用 | SPLIT：当前总 owner，需要两个独立生命周期 |
| `login_path.rs` | 110 | GUI 启动时补齐登录 Shell 的 PATH | SPLIT：启动辅助可共享，不能由 UPPER 改 BASE 进程环境 |
| `messages.rs` | 36 | 消息后台处理服务 | UPPER：业务 worker |
| `models.rs` | 1035 | roster、owner trust、vault、模型路由与代理 | SPLIT：信任与 roster 留 BASE，LLM/账户业务移 UPPER |
| `network/` | 3981 | EasyTier/Tailscale 配置、发现、安装、转发、邀请 | BASE：设备互联与已授权连接的维护 |
| `node.rs` | 584 | 有界 exec、超时/取消、Unix 进程组和 Windows Job | UPPER：普通命令执行，不是网络内核或持久终端 |
| `pair_url.rs` | 319 | 配对链接、请求及有效期 | BASE：恢复访问所需配对入口 |
| `peer/` | 6358 | CLI、iroh、mesh 服务、各业务路由及 hook | SPLIT：传输/授权/恢复通道留 BASE，业务 handler 移出 |
| `project_sync.rs` | 67 | 项目同步后台调度 | UPPER：业务状态机 |
| `settings.rs` | 16 | 平台配置路径选择 | BASE：唯一身份目录的发现；可共享纯路径工具 |
| `spaces/` | 2933 | 容器空间、镜像、应用、provision | UPPER：空间管理业务 |
| `supervisor.rs` | 293 | 后台 execution 恢复与进程监督 | UPPER：不能因恢复业务记录重启 BASE |
| `task_worker.rs` | 36 | 任务 worker 启停 | UPPER：频繁更新业务 |
| `web/` | 2976 | 共享 Web API、终端/桌面 WS、页面、standalone/relay | SPLIT：登录与恢复通道 BASE，完整控制台和业务 API UPPER |

关键目录内的实际切点：`peer/transport_iroh.rs`、`peer/secure_ws.rs` 留 BASE；`peer/server.rs` 拆 `App`、监控与 handler；`peer/hook.rs` 消息注入、`peer/browser_proxy.rs` 浏览器业务归 UPPER。`web/terminal.rs`、`desktop.rs`、`desktop_page.rs`、所需 `novnc_files.rs`、`relay.rs` 留 BASE，`standalone.rs` / `mod.rs` 拆主机生命周期和业务挂载。`network/` 的后台维护与控制接口同属 BASE；可将安装向导 UI 放 UPPER，但不能连带停止已建立网络。

### `crates/workspace-runtime/src`

| 顶层条目 | 约行数 | 目的 | 归属与理由 |
| --- | ---: | --- | --- |
| `client.rs` | 1372 | 远端 API、终端、直连/网关、MCP 客户端 | SPLIT：出站身份/Noise/流由 BASE 持有，业务适配 UPPER |
| `config.rs` | 152 | 凭据锁、私有文件原子替换、比较工具 | SPLIT：纯工具可共享，身份配置唯一写者必须 BASE |
| `journal.rs` | 183 | 有界操作记录与中断恢复 | UPPER：业务记录，重启不自动重放命令 |
| `lib.rs` | 22 | 模块导出、shutdown signal | SPLIT：信号和停止域须分开 |
| `peers/` | 19670 | 身份授权、传输及全部设备业务 | SPLIT：详见下表，不能整体移入一层 |
| `projects.rs` | 646 | 本地项目/副本/终端绑定目录 | UPPER：保存 terminal ID，校验经 BASE API |
| `projects/` | 2205 | copy、handoff、sync 与支持模块 | UPPER：项目数据和传输业务 |
| `protocol.rs` | 210 | exec/节点等共享 DTO、限制 | SPLIT：提取版本化契约，执行状态在 UPPER |

`peers/` 补充展开（单文件与同名目录行数分别列出）：

| 条目 | 约行数 | 目的 | 归属与理由 |
| --- | ---: | --- | --- |
| `mod.rs` | 978 | Store/命令聚合 | SPLIT：目前一个 Store 混合授权与业务 |
| `activity.rs` / `activity_snapshots.rs` | 277 / 200 | 活动与快照 | UPPER：业务观察 |
| `agent_sessions.rs` / `agent_snapshots.rs` | 503 / 182 | Agent 会话与快照 | UPPER：Agent 业务 |
| `browser.rs` / `browser/` | 424 / 781 | 浏览器控制与 manager | UPPER：自动化业务；最低键鼠救援由 desktop/CUA 提供 |
| `computer.rs` / `computer/` | 647 / 1476 | CUA、截图/输入、驱动与平台适配 | BASE：最小操作、权限检测、租约与暂停控制应常驻 |
| `config.rs` | 997 | grants/routes/desktop/computer/browser/copy 配置 | SPLIT：拆业务配置，签名信任配置只由 BASE 修改 |
| `copies.rs` / `copies/` | 645 / 477 | 远程副本传输与状态 | UPPER：业务数据 |
| `desktop.rs` / `desktop/` | 267 / 657 | VNC/Selkies 流适配 | BASE：持续桌面通道 |
| `discovery.rs` | 402 | 对端发现/探测 | BASE：设备可达性 |
| `e2e.rs` / `e2e/` | 626 / 302 | Noise、会话、帧协议 | BASE：身份与活跃流密钥 |
| `executions.rs` / `executions/` | 1029 / 80 | 后台命令与记录 | UPPER：普通命令生命周期 |
| `files.rs` | 236 | 文件读写 | UPPER：业务能力 |
| `identity.rs` | 413 | 私钥、签名、配置迁移下限 | BASE：唯一身份 owner |
| `local_forward.rs` | 188 | loopback forward lease | BASE：活跃转发不可随 UPPER 退出 |
| `messages.rs` / `messages/` | 517 / 704 | 消息、投递及策略 | UPPER：业务；grant 决策从 BASE 获取 |
| `paths.rs` | 299 | 对端业务状态路径与绑定 | SPLIT：路径工具可共享，状态写者按能力划分 |
| `projects.rs` / `project_sync.rs` | 397 / 503 | 远程项目目录与同步 | UPPER：业务，不能直接修改 BASE 配置 |
| `roster.rs` / `roster/` | 570 / 163 | 签名成员表、owner、持久化 | BASE：身份组信任，独立于 LLM 存活 |
| `space_catalog.rs` | 161 | 空间目录 | UPPER：空间业务 |
| `tasks/` | 4233 | 任务队列、worker、执行、回执 | UPPER：任务业务 |
| `terminal.rs` | 147 | 终端协议/输入校验 | BASE：稳定流契约 |
| `transport.rs` | 189 | HTTP/WS 传输抽象 | BASE：出站恢复连接所需 |

### `crates/cranibox-net/src`

全部归 BASE；归属描述不意味着各个 optional transport 必须开启。

| 顶层条目 | 约行数 | 目的与 BASE 理由 |
| --- | ---: | --- |
| `addr.rs` | 91 | Endpoint 地址转换/校验，网络基础 |
| `allow.rs` | 61 | 可热更新 Allowlist，入站身份边界 |
| `forward.rs` | 96 | iroh 到 loopback 转发，活跃连接 owner |
| `lan.rs` | 161 | 已知设备签名 LAN 发现，网络恢复 |
| `lib.rs` | 69 | ALPN/导出/传输类型，稳定网络契约 |
| `mesh.rs` | 133 | QUIC 上 HTTP 与 upgrade，mesh 连接 owner |
| `node.rs` | 376 | iroh endpoint、连接池，常驻网络 |
| `pair.rs` | 734 | 邀请、配对与核对流程，身份建立 |
| `path.rs` | 22 | RTT/path 信息，网络诊断 |
| `relay.rs` | 315 | relay HTTP/准入/诊断，恢复链路可能依赖 |
| `ticket.rs` | 130 | ticket、链接、二维码，恢复配对 |
| `transport.rs` | 125 | `IrohTransport`，稳定出站通道 |

### `crates/workspace-pty/src`

| 顶层条目 | 约行数 | 目的 | 归属与理由 |
| --- | ---: | --- | --- |
| `engine.rs` | 88 | Native 与可选外部适配的统一 API | BASE：Shell owner 的选择和生命周期 |
| `herdr.rs` | 389 | 可选外部终端工具 herdr 的适配（用户自行安装） | 不属于 BASE，也不是依赖；见文首更正 |
| `lib.rs` | 388 | PTY/ConPTY、读写、进程树清理 | BASE：句柄和 Job 必须活得比 UPPER 长 |
| `persistent.rs` | 415 | 内存会话表、重附着、输出回放 | BASE：只持久于连接，不持久于 owner 进程 |

### 装配与路由

| 文件 | 约行数 | 当前装配 | 建议 |
| --- | ---: | --- | --- |
| `crates/cranibox/src/main.rs` | 42 | worker 特殊入口；workspace 功能分派；最终共用 CLI | SPLIT：保持用户命令兼容，显式启动两层 |
| `crates/workspace-node/src/cli.rs` | 165 | `run_program` 归一化 `node`，serve/default 启动 standalone | SPLIT：调用 BASE 客户端或启动 UPPER，禁止隐式再建 LocalNode |
| `crates/workspace-node/src/local.rs` | 475 | Engine、catalog、网络、业务、单实例锁 | SPLIT：提取 `BaseNode` / 业务 host |
| `crates/workspace-node/src/node.rs` | 584 | exec 的 ProcessTree 与 Job | UPPER：保留取消/超时/进程树清理 |
| `crates/workspace-node/src/peer/server.rs` | 1136 | 一个 App、一套 mesh router、监控全部能力 | SPLIT：BASE 接入与 UPPER handler 分离 |

`peer/server.rs:839` 的路由在 `/api/v1/mesh` 下挂载（`:898`）；划分如下。`LLM_MOUNT` / `VAULT_MOUNT` 目前通过共享常量挂载，不能因前缀外置绕过 owner 检查。

| 路由族（省略 mesh 前缀） | 建议 | 原因 |
| --- | --- | --- |
| `/nodes` | SPLIT | BASE 提供身份/基本能力，UPPER 能力按在线状态汇总 |
| `/nodes/{node}/terminal/sessions`、`terminal/ws` | BASE | 会话管理、SecureSocket 与 Engine 必须同寿命 |
| `/nodes/{node}/desktop/ws`、`desktop/page` | BASE | VNC/Selkies 流与页面隧道保持在线 |
| `/nodes/{node}/computer` | BASE | 最小 CUA、租约、暂停/撤权是救援能力 |
| `/keys`、`/nodes/{node}/e2e/handshake`、`e2e/call` | BASE | 解密后仍需进入同一授权/转发路径 |
| 可选 `/identity` | BASE | 网络内只读身份发现，不授予能力 |
| `/nodes/{node}/exec`、`/jobs/{id}`、`/jobs/{id}/cancel` | UPPER | 有界命令与业务 job |
| `/nodes/{node}/browser`、`browser/ws` | UPPER | 浏览器自动化，可单独重启 |
| `/nodes/{node}/projects`、`projects/sync` | UPPER | 目录与同步 |
| `projects/export`、`import`、`copy-status`、`transfer` | UPPER | 副本传输 |
| `/nodes/{node}/messages`、`messages/{id}`、`activity` | UPPER | 消息与业务观察 |
| `/nodes/{node}/tasks`、`tasks/{id}/{dispatch}` | UPPER | 任务与回执 |
| `/nodes/{node}/agent-sessions`、`executions`、`spaces` | UPPER | Agent、后台执行与空间 |
| `/nodes/{node}/files/read`、`files/write` | UPPER | 文件业务 |
| `LLM_MOUNT`、`VAULT_MOUNT` | SPLIT | LLM/账户业务 UPPER；roster 安装/读取与成员授权 BASE |

Tauri 的参数入口调用同一个 CLI（`apps/node-desktop/src-tauri/src/main.rs:255`），注册 GUI 单实例插件（`:275`），直接 `LocalNode::start`（`:344`）并关闭内核（`:108`）。因此桌面应用也必须改为连接/管理 BASE；单独拆命令行二进制不够。Workspace 容器通过 `crates/workspace-web/src/lib.rs:69` 嵌入 `LocalNode::start_with`，同样需改接 BASE。

## 3. 耦合、状态与锁

以下为现有事实；“需要”指拆分后的设计要求。

| 耦合及源码证据 | 两进程会破坏什么；需要什么 |
| --- | --- |
| `local.rs:15` 的共享 owner、`:42` 的 `service_lock`、`:49` 的 peer 装配、`:76` 的 CUA、`:81` / `:84` 的网络/iroh | 不能分别启动两个 LocalNode；仅 BASE 持有 identity lock，UPPER 不启动网络/Engine |
| `peer/server.rs:80` 的 App 同时持有 Engine、catalog、jobs、semaphores、computer lease、Noise sessions、Models OnceCell；`:949` 同一 monitor 清理会话并取消 jobs | Arc/Mutex/OnceCell 不能跨进程共享；拆 BASE 连接/安全监控与 UPPER job 监控，拆 stop/connections/tasks/slots |
| `app.rs:236` 直接取得 iroh Handle；`:388` 改 Config；`:461` 读 roster；`:545` 查询 peer path；`app/capabilities.rs:60` manage_peers、`:928` 改桌面配置 | UPPER 的 App facade 必须经 BASE API 管理配对/grant/path/信任/共享；业务配置不再混写身份文件 |
| `models.rs:167` 读 Config/Identity，`:223` 初始化 roster，`:275` vault 使用 roster，`:280` DirectTransport，`:599` 改 grants/routes，`:643` / `:671` trust_members 调用 ensure_roster_peer | 不能整体把 Models 移走；先提取 trust service，BASE 持有 roster/身份，模型与账户层订阅公开视图/受限操作 |
| `models.rs:798` install_roster、`:822` 同步撤销 grants/routes；`peer/transport_iroh.rs:458` 进程内 LIVE，`:465` ensure_roster_peer | roster 安装、撤权与 iroh allow/forward 更新要由 BASE 串行协调；文件更新无法代替进程内 LIVE 注册 |
| `peer/server/llm.rs:25` authorize 检查加密、grant、verified、磁盘 roster，`:56` 又启动 Models；`vault.rs:25` 复用它，`:38` / `:82` 读/装 roster | BASE 路由当前依赖 UPPER 状态；需抽出不启动 Models 的 owner 授权与 roster endpoint，再转交账户/LLM 请求 |
| `peer/server/e2e.rs:84` handshake、`:134` TOFU 配置更新、`:156` sessions；`:169` call 解密，`:241` 注入 caller，`:242` `Dispatch.api.oneshot` | BASE 解密和封装结果；fallback 既要接普通请求，也要接解密后的 inner request；不能只代理外层密文 URL |
| `peer/server.rs:125` auth / `:149` 读取进程内 Authenticated；`:165` sealed 验证；`:109` 再读 grant | 受信任 Axum Extension 不会自动变为 IPC 身份；禁止接受客户端伪造的 caller header，转发必须带经认证的上下文 |
| `workspace-runtime/src/client.rs:89` / `:95` 的会话/legacy 缓存；`peers/config.rs:512` read 会在 `:538` 加载私有 Identity | UPPER 继续直用现有 client/Config::read 就仍拥有私钥；要把出站 Noise/配对/签名也移至 BASE，提供脱敏视图，不能只改监听端口 |
| `workspace-runtime/src/peers/executions.rs:284`、`copies.rs:83` / `:102` / `:226`、`project_sync.rs:288` / `:375` / `:410` 读 Config；copies `:31` 有静态 slots | 业务直接依赖 grant/route 快照；改 BASE 查询/撤权事件与出站 API，业务并发限制留 UPPER |
| `local.rs:204` projects 调 `projects_in`；`:310` 绑定前向 Engine 列会话 | 项目目录属 UPPER，但 terminal ID 校验调用 BASE；项目恢复不应成为 BASE 启动条件 |
| `web/mod.rs:32` SessionStore，`:43` Services；`web/terminal.rs:75` ticket、`:97` upgrade、`:193` run；`web/desktop.rs:21` ticket 持有 Stream | 浏览器 cookie/CSRF、30 秒单次 ticket、alive/closing、WS 任务均需 BASE owner；UPPER 退出不能注销恢复会话 |
| `web/standalone.rs:189` 启动 LocalNode、`:191` 主动启动 Models；`local.rs:119` App 初始化也依赖 Models | 拆除“最小恢复界面依赖 Models”的装配；容器宿主的 SessionStore 也需拆，不能只改目标 mesh |
| `peer/server/desktop.rs:12` 校验配置/epoch，`:25` stop/connections，`:43` 控制者许可，`:88` / `:129` WS/SecureSocket | VNC 活着不等于 Cranibox 通道活着；BASE 持有两端代理、许可、加密帧计数与实时撤权 |

表中未写全路径的节点文件均以 `crates/workspace-node/src/` 为根。

roster 的切割还包含密码操作：`models.rs:611` 的 `member_secret` 使用 `vault_private` 与成员 X25519 公钥派生成对授权凭据；`:798` 的 roster 安装同时处理 sealed owner key 和 vault 轮换。必须把维持设备信任所需的最小密钥/派生/owner 操作一起抽入 BASE，或设计不依赖 UPPER 在线的受限密码服务；不能只移动 `RosterStore` 类型并把私钥复制到两边。账户数据/LLM 可在 UPPER，相关密钥操作接口仍需另行定义。

锁和单写者边界：

- 常驻锁：`peer/mod.rs:986` 创建 `<peers>.service.lock`，`:996` `try_lock_exclusive`，只允许同一身份一个常驻服务；`local.rs:284` 显式解锁。不要删除锁，也不要给 UPPER 换配置目录制造第二个 NodeID。
- 配置写锁：`crates/workspace-runtime/src/config.rs:19` / `:36` 的 `<path>.lock`；`peers/config.rs:592` 锁内读改签名，`:608` 替换配置，`:609` 写身份 checkpoint。锁能序列化文件写入，不能同步两个进程的缓存；这两份文件也不是一个原子事务。
- 转发 lease：`peers/local_forward.rs:8` / `:23` 的 `.network-{node}.lock`、`:54` 活性检测；`peer/transport_iroh.rs:130` 的 `.iroh-{node}.lock`。新 UPPER 不得重建 listener/lease；保存的 loopback 地址本身不证明有活跃 owner。
- roster：`peers/roster.rs:79` 的 RwLock/watch 是进程内状态；`:187` 安装、`:198` 文件锁不能更新另一个进程的内存。BASE 发布带 generation 的视图/撤销事件，断开订阅时 UPPER 应拒绝依赖过期授权的新请求。
- 恢复副作用：`local.rs:46` claim_host、`supervisor.rs:25` recover；双重实例会重复恢复/调度。UPPER 单写业务记录，BASE 单写身份/网络/终端状态。锁不能当成跨进程 RPC。

现有 forward ALPN 的注释明确：loopback 不继承 iroh 身份（`crates/cranibox-net/src/lib.rs:40`）。复用 forward 可减少传输改动，但不能把 loopback 当作认证，更不是完整 BASE 控制 API。

## 4. 候选切线

以下都是设计建议，尚未实现。S/M/L 是相对工程量，不是排期承诺。

### (a) 两进程，认证 loopback HTTP / Unix socket API

- **改动**：提取 BASE binary/service 与版本化 IPC 契约；拆 LocalNode/App、Config/Identity/Roster、出站 client；UPPER 的 App/MCP/worker 通过 IPC 调 BASE；CLI、Tauri、Workspace 改装配。Unix socket 用私有目录权限及 peer 校验；跨平台可先统一 loopback HTTP，使用仅限本机的认证通道。Windows 如采用 named pipe，另需 ACL/客户端身份实现。上述 IPC 均非现成接口。
- **保留**：iroh、EasyTier/Tailscale、既有 NodeID/grants、Noise 协议、PTY 与桌面 provider；业务协议尽量兼容。BASE 同时提供最小浏览器登录/终端/桌面界面。
- **不断线条件**：UPPER 无权触发 BASE stop；恢复 WS、Noise、底层 PTY/VNC 和接入端转发一直由 BASE 持有。若浏览器 socket 仍接 UPPER，只能断线后重连。
- **工程量 L**：身份读取/签名深入 runtime client；roster 藏在 Models；三个宿主入口与浏览器 SessionStore 都要拆。
- **主要风险**：IPC 版本不匹配、caller 伪造、授权快照过期、流量背压、撤权与业务执行竞争、同用户进程仍可访问原配置文件。逻辑分层不等于 OS 权限隔离。

发布也需分层：UPPER 的 dev build 使用独立可替换产物/版本路径，不能通过重启当前 Tauri 可执行文件间接退出 BASE。BASE、UPPER、guard 的安装与自启动关系需要显式定义；保持 `cranibox` 命令兼容不等于继续共用同一 resident 进程。

### (b) BASE 保留整个 mesh HTTP 入口，未命中 BASE 的路由反向代理 UPPER

- **改动**：保留 `mesh_router` 的稳定入口、auth、E2E inner dispatch 和 BASE 路由；将现有 UPPER handler 替换为 proxy，并为未来业务路径加受控 fallback。BASE 内不再持有 catalog/jobs/Models，只保留连接、安全与恢复状态。代理目的地址由本地可信管理接口指定，不来自请求参数。
- **保留**：远端 URL、iroh `MeshTarget::Router`、已建立连接；Tauri/浏览器可继续使用稳定 BASE 地址。配对/grant/roster 更新和恢复页仍需拆成 BASE 服务，不能因“保留 router”继续依赖 Models。
- **不断线条件**：终端/桌面 handler 在 BASE 本地执行；它们不经过 UPPER，也不共享业务 stop/JoinSet。UPPER 的 WS/SSE 会随其重启中断，这不影响 BASE 恢复通道。
- **工程量 M（首批业务外置）/ L（完整边界）**：少改远端调用方，便于逐路由迁移；完整拆分仍有 (a) 的身份、roster、宿主和出站调用工作。
- **主要风险**：“unknown route”不等于“已授权能力”；当前 `route_layer(auth)` 只作用于匹配路由，不能假定自动保护新 fallback（`peer/server.rs:895`）。明确能力映射/拒绝规则，保留 body limit、target、owner、verified、加密要求；proxy 必须支持 upgrade、流式返回、取消和背压。

两进程方案都需要受保护 IPC 上的 caller/target/capability/generation 上下文，剥离外来同名 header；BASE 执行当前 grant 校验，UPPER 执行业务校验，长流继续响应撤权。不要把 BASE 私钥或全量 grants 复制给 UPPER。健康检查/版本不兼容时返回明确不可用；不自动重放 exec、输入事件等有副作用请求。

### (c) 底座进程内含终端引擎与 VNC 监督，上层另起进程

- **改动**：底座进程直接持有自有 Native Engine / PTY（`persistent.rs`，`portable-pty`，三个平台同一份代码）；托管 VNC 改独立监督与可验证接管。区分 detach、节点 restart 与用户 End/Stop。服务启动时读取持久记录重连。
- **保留**：大部分 LocalNode/router/App/业务结构、现有 URL 和授权。无需立即拆全部 Models/runtime 依赖。
- **存活范围**：Shell、桌面服务及 OS 会话可继续；退出的节点拥有全部 TCP/WS/Noise 状态，现有连接必断。只能提供重连恢复，**不满足“普通更新只重启 UPPER，已打开连接保持在线”**。
- **工程量**：终端引擎并入底座为 L（一份代码覆盖 Linux、macOS、Windows ConPTY，但各平台的进程与 Job 归属要分别验证）；VNC 独立监督/接管为 M。
- **主要风险**：孤儿进程/遗失记录、服务管理器清理整组、误接管同端口服务、失去及时撤权；只去掉 `kill_on_drop` 会留下不受控服务，并不能保住 PTY 句柄。

## 5. 终端与桌面存活的具体条件

### 当前终端实际保证

`Engine::new` 默认 Native（`crates/workspace-pty/src/engine.rs:17`）。`persistent.rs` 的会话表是内存 Arc/Mutex，detach 后继续读取、保存最近 256 KiB；`:213` 的 shutdown 会 drain 并 End 所有 Session。因此文件名 persistent 指断连后可附着，不是节点重启持久化。

`Session::command` 拥有 PTY master/child（`lib.rs:65`）；Unix 特意克隆 writer，防止 portable-pty writer Drop 注入 Ctrl+D（`:74`）。`Session::drop`（`:275`）会杀进程：Unix 同 session 成员和负 PID 进程组（`:281`），Linux 用 `/proc`、其他 Unix 用 `/bin/ps` 枚举后 `getsid`（`:295`）；Windows `Job::new`（`:329`）设置 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`（`:336`）。ConPTY master 在 owner 内（`:106`）。

普通 exec 是另一条路径：`crates/workspace-node/src/node.rs:106` 使用 kill-on-drop，`:111` 建 Unix process group，Windows 使用 Job/挂起后分配再恢复（`:205`）。这些取消/超时清理应留在 UPPER，不应为持久终端而全局禁用。

（旁证，不作为设计依据）herdr 是用户自行安装的应用层工具。其适配 `herdr.rs:159` 优先复用 daemon；`:171` server 设置 `kill_on_drop(false)`，Unix `setsid`（`:182`），Windows `CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS`（`:190`）。这说明“独立进程持有终端”在 Unix 上可行，但我们不依赖它。herdr 对 Windows attach 不支持只是该工具自身的限制，与我们自己的终端无关。

Workspace 镜像默认选择 herdr（`crates/workspace-web/src/lib.rs:69`），普通 CLI/Tauri 的 LocalNode 默认 Native。后续设计以 Native 引擎为准，不把 herdr 作为恢复手段。



### 当前桌面实际保证

`desktop_service.rs:558` 的长期 VNC 子进程 `kill_on_drop(true)`；manager 退出还主动 `self.kill()`（`:160`）。只改一个 flag 无效。wayvnc 私有 control socket 位于 owner 的 TempDir（`:538`、`:575`）；启动前主动拒绝已有端口（`:467`），没有“发现旧服务并接管”的协议。`:497` 的 version probe 是短命探测，不应为持久化而改掉清理。

托管 provider 只有 x11vnc/wayvnc，且显式只支持 Linux（`desktop_service/provider.rs:5`、`:35`）。macOS Screen Sharing 与 Selkies 外部服务的接入方式见 `docs/remote-desktops.md`、`docs/node-desktop-and-device-mesh.md`；它们已独立于 Cranibox，但 `peer/server/desktop.rs:55` 的 TCP 与 `:88` 的 WS 代理仍随节点退出。

### 平台变更矩阵

| 平台/对象 | 两层方案下，UPPER 重启 | 若还要求整个节点/BASE 重启后可恢复 OS 会话 |
| --- | --- | --- |
| Linux terminal | BASE 持有 Native Engine、PTY master、读写/回放任务与终止权限，UPPER 只 RPC | 由底座进程持有；独立 session/进程组，明确 stdio/日志与 owner；仅给 Shell `setsid` 或忽略 SIGHUP 不保 master。broker 保留注册表/重附着 API；显式 End 继续杀整个 session |
| macOS terminal | 同 Linux；PTY owner 不随业务退出 | 由底座进程持有；保留非 Linux 的 session 清理语义。GUI quit 不能隐式结束需要保留的底座进程 |
| Windows terminal | BASE 持有 ConPTY、管道和 kill-on-close Job，Job 不能由 UPPER 关闭 | 独立 helper/service 持有整个 PTY/Job，提供认证 attach/replay；保留 owner 退出清理。仅 `DETACHED_PROCESS` / 新进程组不能转移 ConPTY，也不保证逃离继承 Job；需要核实宿主 Job 策略。默认 shell 为 `COMSPEC`（cmd.exe），PowerShell 同属 ConPTY 接口 |
| Linux managed VNC | manager 与 child 都留 BASE，不调用 BASE 的 shutdown | 分离 StopSharing 与 restart；移除重启路径主动 kill 和 owner Drop kill；独立 supervisor/detached spawn，持久私有 runtime/control socket；记录 PID 加启动身份、provider/display/port，健康验证后接管；保留显式 Stop 的终止能力 |
| Linux Selkies | 保留外部 s6 服务；BASE 持有页面隧道、流与许可 | 不重启桌面服务/容器；新节点重连外部服务；旧 WS 必断。具体服务管理器 kill 范围仍需部署核实 |
| macOS desktop | 保留系统 Screen Sharing；BASE 持有 VNC proxy 与授权 | 系统服务继续，重建代理即可重连；仓库没有托管 macOS VNC child 可直接改 detached。BASE/driver 改打包后 TCC 要单独验收 |
| Windows desktop | 保留外部 VNC 服务；BASE 持有代理 | 外部服务生命周期独立；若未来引入托管 helper，用独立 service/合适 Job 边界，不能套用 Unix setsid。当前没有 Windows 托管 VNC 实现 |

Linux/macOS 的 detached helper 也不能只靠 setsid：它不修改部署管理器的终止范围；需确保 UPPER 的 stop 不覆盖 BASE 与其内的桌面服务。Windows 是否允许 Job breakaway 取决于实际宿主，不能从 creation flags 推定。上述是必须验证的启动契约，不是已验证的平台保证。

VNC 接管还要保持密码文件与配置安全、稳定 desktop epoch、单控制者和撤权逻辑；不得把“端口已占用”视为可信旧实例。现有配置变化会递增 epoch（`desktop_service.rs:297`、`:308`），单纯恢复 owner 不应造成无意义换代。

CUA 也有生命周期耦合：`local.rs:260` 退出时 `stop_daemon`；`crates/workspace-runtime/src/peers/computer/cua/embedded.rs:139` 启动前会停止旧 socket daemon，`:180` 主动 kill/wait。BASE 应持有驱动和 computer lease/动作串行锁，UPPER 重启不走这些路径。macOS embedded 模式检测 `.app/Contents/MacOS/`（`:35`），使用 `com.cranibox.node` host bundle（`:20`）；不能假设换成裸 BASE binary 后原权限归属仍成立。

## 6. 独立 BASE 更新与离线回滚设计

下面是待实现流程。现有 `service_lock`、配置迁移下限、detached 行为可作为约束；没有从源码获得现成 guard、BASE/UPPER 版本协商或完整回滚事务，不能声称现有安装包具备它们。

1. 在 BASE 在线时预置 candidate、上一版可执行文件及本地校验元数据。独立 guard 持有可启动的上一版文件、绝对路径、受限启动参数和本地操作入口；更新期间不替换 guard。Windows 等平台避免原地覆盖正在运行的文件，按版本目录保存。具体签名/分发信任机制待定。
2. 更新前确认 candidate 的 IPC/配置兼容范围和恢复能力。优先保持身份/roster schema 向后兼容；若要迁移，先静止相关写入并制定匹配快照/恢复规则。`peers/config.rs:538` 检查身份迁移下限，`:608` / `:609` 分别写配置与 checkpoint；**仅保留旧 binary 不足以保证旧版本可启动**。
3. guard 接管更新状态后，停止旧 BASE，等待其退出/回收并释放同身份锁，再启动 candidate；不并行启动两个相同身份的 BASE，不删除锁文件绕过排他。guard 与 BASE 的心跳/就绪 IPC 必须在本地独立工作。
4. 就绪检查至少覆盖配置/身份打开、网络服务启动、最小恢复页面、终端/桌面能力状态；外网断开不能被误判成必须回滚。基础能力可明确报告 OS 权限未就绪，不能把 UPPER health 当 BASE health。需要本地超时、故障判定和有限重试策略。
5. 启动失败/本地就绪超时：guard 停止并回收 candidate，确认锁释放，用上一版绝对路径与原身份目录启动。回滚不依赖下载、DNS、对端或远程终端；需有本机可执行的离线恢复入口。
6. 仅对兼容数据回退二进制；若迁移不可逆，不能盲目还原旧 grants/roster 快照，避免复活已撤销授权。需要成对考虑 Config/Identity checkpoint、roster/owner 材料及本次修改的 vault 数据，保留撤销单调性；无法证明兼容时应在停止旧 BASE 前拒绝该自动升级。
7. 新版稳定后才确认并保留上一版。BASE 重启会丢失网络连接和内存 Noise sessions；用户重新握手/附着。底座内的终端和桌面服务只能保 OS 会话，guard 不提供 socket/密钥状态迁移。

## 7. 工作顺序、最小交付与验收

1. **最小有价值改动（后续实施）**：把 Native 持久终端引擎（`persistent.rs`；`portable-pty`，Windows 为 ConPTY，cmd.exe / PowerShell 均可）放进底座进程（与网络、授权、远程桌面同进程），三个平台同一份实现；节点重启后重新附着同一 Shell，分清 detach、restart、End。此步不等于两层拆分，也不承诺 WS 不断。
2. 提取 BASE 生命周期、resident lock、Config/Identity/Roster 与最小恢复界面；移走 Models 启动依赖。冻结终端/桌面/CUA/授权的 IPC 契约、配置写者与撤权通知；分开 base_stop / upper_stop。
3. 采用 (b) 逐项迁移 UPPER handler/worker，使用 (a) 的认证 IPC；先迁移状态少的业务，再迁移 tasks/messages/executions/projects/LLM/vault。改造出站 client，消除 UPPER 对私钥/原始 Config 的运行依赖。
4. 同步改 CLI、Tauri 和 Workspace 宿主、SessionStore 与恢复页面；保证浏览器接入端/中继也在 BASE。验证未启动 UPPER 时仍可配对、授权、开终端、看桌面和最小输入。
5. 补齐托管 VNC 独立监督/接管及 Windows 底座；最后实现 guard、BASE 兼容性门禁和离线回滚。无需为了 UPPER 更新先解决 BASE 热迁移。

后续验收应在授权的测试环境完成：保持终端连续输出/交互和桌面输入流，强制结束/替换 UPPER，确认同一 WS/PTY/桌面流不重建；同时验证撤权/暂停仍立即生效、UPPER 离线明确不可用。另测整个节点重启后的 底座里的终端和 VNC 重附着，明确它与不断线验收不同；分别检查 Unix 管理器、Windows Job 和 macOS 权限。guard 则在无网络条件下注入启动失败、配置不兼容与进程未退出故障。**本次未执行这些测试。**

## 8. unknown（源码不足以确定）

- unknown：真实部署下 systemd/s6/launchd/安装器对进程组、cgroup、Job、注销/关机的处理；setsid/DETACHED 不构成实机存活证明。
- unknown：Windows 上 底座进程能否脱离宿主 Job 而继续持有 ConPTY（取决于启动方式，如计划任务 / 登录项 / 桌面应用）。
- unknown：macOS 将 BASE 移出当前 App 后 TCC/Accessibility/Screen Recording 的归属、签名替换后的授权保持，以及安全桌面/系统密码对话框是否可操作。Windows 安全桌面/UAC 同样未证明。
- unknown：Selkies s6 与系统 Screen Sharing 在各目标设备升级、注销及服务重启时的实际状态；本次未访问设备或外部镜像内容。
- unknown：UPPER/BASE IPC schema、能力版本协商、认证载体和 Windows IPC 最终选型；目前没有这样的完整两进程契约。
- unknown：可验证的 BASE 升级分发/签名机制、guard 打包与离线恢复入口；现有发布说明不等于更新事务实现。
- unknown：未来 schema 的向后兼容范围与迁移回滚策略；现有配置下限检查不能证明旧 binary 兼容新数据。
- unknown：实际浏览器接入端/relay/网关是否与被更新 UPPER 同进程；不同拓扑须逐跳验证，不能只测试目标节点。
- unknown：跨 BASE 重启时 socket、Noise session 或 VNC controller 状态的无缝移交；当前代码未提供，不承诺。
- unknown：实现耗时与生产可靠性数据；S/M/L 仅依据依赖范围估算，未构建、压测或实机验证。
