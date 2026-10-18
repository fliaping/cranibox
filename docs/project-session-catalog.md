# 项目与会话：第一阶段实现

浏览器与 Tauri 节点客户端共用 React 项目页面，汇总 Workspace 网关目录与已配置直连节点的本地目录。Web 服务所在设备（包括 Docker 部署）和 Tauri 本机都拥有持久 NodeID，并在项目、设备和浮动终端中使用同一身份。Tauri 不配置网关也可使用本机目录与 Shell；未配置的网关不会显示为读取失败。网关不可达时，仍可管理本机和已授权的直连目录、关联与接回原 Shell。已读取的直连目录另有只读离线快照，工作文件可在本机与直连节点之间双向复制，也可在两个直连节点之间经清单确认后复制。这是[多设备工作空间方案](distributed-agent-workspace-design.md)阶段 A/B/D 的部分实现；已配置直连来源支持按项目内容摘要增量拉取及断线后的新增、修改和删除补齐；尚未实现全网目录自动分发、离线编辑、Agent 编排和运行会话迁移。

## 使用

1. 启动更新后的 Cranibox 服务，在 Web 左侧打开“项目与会话”（`/#/projects`），或在更新后的 Tauri 客户端选择同名页面。
2. 注册一个已有项目，选择设备并填写该设备上的绝对路径。注册只保存目录信息，不创建目录、不复制文件；创建 Shell 时由目标设备检查路径。
3. 在目录中创建 Shell，或选择该设备上的已有会话进行关联。相同项目可添加多个设备上的工作副本，文件内容不会自动同步。
4. 从项目重新打开关联会话，接入原节点上的原 Shell。解除关联或移除目录记录只修改目录信息，不结束 Shell、不删除工作文件。移除最后一个副本时项目记录也会移除。

直连设备需要目标额外授予“读取和管理此设备的项目目录”（CLI `peer grant --projects`）；旧 Token 不会自动获得此能力。关联和使用 Shell 还需要 `--terminals`。页面显示“目录来源”、最近读取时间及失败原因；部分来源失败时，其余目录仍可使用。连接中断时可查看上次成功读取的直连目录，明确标记“离线快照”和同步时间，编辑与 Shell 操作禁用。确认撤权、身份不符或协议响应无效后会清除快照；无法连接时不能即时得知远端撤权。失败不代表源节点上的记录或 Shell 已删除。

从现有项目点击“添加副本”，会将相同 ProjectID 写到所选节点，因此多个独立目录可汇总为同一项目。注册另一个同名目录不会自动视为同一项目。CLI 也可通过 `projects register --project-id UUID` 显式关联。

Web 与 Tauri 都在当前界面打开共用的 React 浮动终端，支持尺寸调整、最小化和重新附着。Tauri 通过本机 Rust IPC 操作独立运行的本机节点，网关与直连目标由 Rust 网络适配器连接，不需要另开浏览器或浏览器登录；远程凭据只放在原生连接的 Authorization 请求头中。项目与终端页面已经共用实际组件，其余客户端页面尚未全部统一。

关闭页面、关闭浮动窗口、网络中断或网关重启均不会主动结束节点进程持有的 Shell。Tauri 修改网关地址、网关节点别名或 Token 时只替换网关连接，本机持久 NodeID、目录和 Shell 保持。退出节点进程或重启设备会丢失原生 Shell；项目关联仍保留，并提示原会话已不存在，不自动创建替代会话。Workspace 主机沿用独立 herdr 引擎，Web 与其直连入口使用同一批 Shell，Web 服务重启后仍可接回；普通节点客户端继续使用原生 PTY，不打包 herdr。

## 模型与存储

- Project：稳定 UUID、名称和工作副本列表。
- Replica：稳定 UUID、所属节点、绝对路径和关联会话。Workspace 主机使用持久 NodeID；旧目录中的 `node: null` 在启动时一次迁移到此身份，保留项目、副本和关联 ID。浏览器所在设备不会被推断为执行目标。
- Session binding：稳定关联 UUID 与终端引擎的会话 UUID。它是 Shell 的关联记录，尚不是 Agent harness 会话或可迁移检查点。
- Catalog：递增 revision 与项目列表。幂等注册和重复关联不会增加 revision；同设备的同一目录只注册一次。

目录文件位于配置根目录下的 `.config/agent-workspace/projects.json`，集成服务与其网关共用此文件；独立 runtime 则使用凭据文件所在目录下的 `projects.json`。写入使用文件锁和原子替换，文件权限沿用私有凭据文件策略（Unix 为 0600）。损坏文件会返回错误，不自动覆盖。

普通节点的直连目录保存在 `peers.json` 同目录的 `node-projects.json`，只允许写入目标持久 NodeID 所属的副本。Tauri 的“这台设备”和目标的直连入口读写同一目录并使用同一 Shell 引擎；已授权的远程修改可在本机刷新后看到。Web 主机的直连入口复用 `.config/agent-workspace/projects.json`，按持久 NodeID 过滤，仅返回主机拥有的副本，其他设备的路径不会随主机授权泄露。Web 不额外创建另一份 `node-projects.json`。

普通节点的旧网关注册可以显式迁移到持久 NodeID，网关项目副本随之使用同一身份；网关目录与直连目录仍是独立来源，不会自动合并文件或会话记录。节点或 Web 服务重启后目录与关联 ID 保留；节点进程退出后原生 Shell 本身不会保留。

上限为 512 个工作副本，每副本最多关联 64 个会话，目录文件最多 1 MiB。路径校验接受 Unix、Windows 驱动器和 UNC 绝对路径；不会在服务端解析远端符号链接或推测真实目录是否相同。

## 迁移旧网关身份

新设备注册时，使用 Tauri“这台设备”或节点 CLI 的 `cranibox-node peer identity` 显示的持久 NodeID。已有 laptop 等旧注册可在 Web“远程节点”中点击对应设备的“迁移身份”，填写同一设备的持久 NodeID；管理员也可通过运行中的网关操作：

```bash
cranibox runtime migrate-identity laptop node-YOUR_PERSISTENT_ID --url http://127.0.0.1:9090
```

迁移保留节点 Token、设备组、ProjectID、ReplicaID 和会话关联 ID，项目 revision 只在实际改写副本时增加。节点会重连，浮动终端自动更新目标并重新附着到原 Shell，Shell 的 PID、环境变量和工作目录保留，不创建替代 Shell 或重复执行命令。集成 Web 服务通过节点连接事件更新在线状态。服务端保留旧 ID 的兼容映射，所以现有客户端设置、API 路径和已打开的终端仍可使用；Tauri 不会自动覆盖已保存的旧注册。

服务端不会依据主机名、IP 或客户端自报身份自动合并设备。正在执行的网关任务或尚未完成的终端管理操作会阻止实际迁移；已完成映射的重复确认不会打断新任务或现有终端连接。目标已注册、属于其他设备的旧 ID、Workspace 主机身份或已有项目副本时也会拒绝。迁移仅允许管理员执行，普通节点无此权限。

旧 ID 的映射随凭据持久化，并在撤销设备后继续保留，避免同名新注册取得历史路径含义。轮换凭据、分组和撤销操作解析到当前身份；重新注册应使用当前 NodeID。需要回退时，可再次显式迁移到本设备拥有的旧 ID，Token 不变。别名最多 512 个，不支持链式映射。

## API 与权限

| 入口 | 路径 | 身份 |
| --- | --- | --- |
| 浏览器 | POST /api/projects | 现有 Web 会话、同源及 CSRF 校验 |
| 浏览器身份迁移 | POST /api/nodes/identity | 现有 Web 会话、同源及 CSRF 校验；请求为 id、target |
| 原生管理员身份迁移 | POST /api/v1/nodes/{node}/identity | controller bearer，拒绝浏览器 Origin；请求为 target |
| 原生管理员 | POST /api/v1/projects | controller bearer，拒绝浏览器 Origin |
| 节点客户端 | POST /api/v1/mesh/projects | 节点 bearer，拒绝浏览器 Origin |
| 节点终端管理 | POST /api/v1/mesh/nodes/{node}/terminal/sessions | 节点 bearer，目标设备组校验 |
| 节点终端附着 | GET /api/v1/mesh/nodes/{node}/terminal/ws | 节点 bearer，目标设备组校验，拒绝浏览器 Origin |
| 原生管理员终端附着 | GET /api/v1/nodes/{node}/terminal/ws | controller bearer，拒绝浏览器 Origin |
| 直连节点目录 | POST /api/v1/mesh/nodes/{node}/projects | 目标颁发的 bearer、X-Cranibox-Target、projects 能力，拒绝浏览器 Origin |
| Tauri 本机目录 | local_projects IPC | 桌面宿主直接访问本机持久 NodeID 的目录 |
| Tauri 本机会话 | local_sessions / terminal_open IPC | 桌面宿主直接访问本机 Shell 引擎，附着校验本机 NodeID |

各目录入口使用相同 JSON 命令；本机 IPC 的 register 可省略 node，由本机节点补齐：

```json
{"operation":"list"}
{"operation":"register","name":"My project","node":"laptop","path":"/home/me/project"}
{"operation":"register","project_id":"PROJECT_UUID","name":"My project","node":"desktop","path":"C:/Projects/project"}
{"operation":"bind","replica_id":"REPLICA_UUID","terminal_id":"TERMINAL_UUID"}
{"operation":"unbind","replica_id":"REPLICA_UUID","terminal_id":"TERMINAL_UUID"}
{"operation":"forget","replica_id":"REPLICA_UUID"}
```

每次请求重新读取设备组权限。节点只可读写自己或同组设备的副本，项目中不可访问的副本、路径和关联会话会被移除，空项目不返回；Workspace 主机的记录只对管理员入口可见。网关目录仍沿用设备分组授权，尚未改为目标本地逐项目审批。

上述设备组规则适用于网关目录。直连目录使用目标本地授权：`projects` 允许读写该节点的全部项目元数据（路径和会话关联），不自动授予文件读取或执行能力；`bind` 额外要求 `terminals` 并验证终端存在。原始 `register.node` 必须等于目标 NodeID。Web 的 `POST /api/peers` 与 Tauri 的 `direct_peers` IPC 使用 `{operation: "projects", node, command}` 调用同一 Rust 适配器，返回 owner、revision、projects 和当前能力。MCP 的 `workspace_projects` 也支持上述命令；绑定直连目标时可省略 register.node，由适配器补齐，指定其他目标会报错。

原生终端附着要求 session UUID，可选 cols、rows 和 takeover；不会隐式创建 Shell。数据格式与直连终端一致：input/output 使用 Base64，resize 使用尺寸，detach 只脱离附着。连接期间持续检查调用方令牌与目标设备组，撤销权限或轮换调用方令牌会关闭附着并保留后台 Shell。浏览器继续使用同源 ticket 通道，不能直接调用原生 bearer 接口。

Web 与 Tauri 都通过服务读取来源目录，前台每 10 秒刷新并在恢复焦点时刷新；并发写入由 Rust 存储层串行化。网关与直连来源并行查询，写入只发送给所选副本的所属来源。列表刷新不会覆盖之后完成的写入。各入口汇总自己配置的来源，并可查看这些直连来源的本地离线快照；目录和凭据均不写入浏览器存储。没有自动分发路由或离线编辑；支持向已授权直连来源增量拉取，并在重连后补齐目录删除。网关停用不影响直连 API；浏览器仍需要其 Web 后端运行。全网身份配对和目录自动分发仍按设计阶段 B 实现。

## 离线目录快照

Web 的 `/api/peers` 与 Tauri 的 `direct_peers` 共用 Rust 本地存储。成功取得并校验目标目录后，在 `peers.json.project-snapshots/` 保存派生快照，文件名为目标 NodeID 的 SHA-256；Unix 文件权限为 0600。快照绑定调用节点身份、目标身份、连接地址、连接选项及实际凭据内容，不包含原始 Token。Web 重启或客户端重新打开页面后仍可读取。目标返回的同名 `sync` 字段不会被当成本地同步状态。

仅列表读取在连接失败、超时、502/503/504 或本地 EasyTier 转发未运行时允许退回快照。离线结果标记同步时间，并禁用执行、终端和桌面能力。页面仍能显示工作路径与会话关联，但会话当前状态未知；不会因列表刷新而重建 Shell。写入失败直接返回错误，不用快照伪装成功，不排队、重放或自动重试。远端写入已完成而本地快照保存失败时，返回成功结果和本地保存警告，避免重复执行远端操作。

明确认证失败、撤权、目标不符或无效响应会清除旧快照；移除连接及显式更换 Token 也会清除。直接修改凭据文件后，原快照不再匹配。请求按目标串行化，网络返回后重新检查当前路由与凭据，防止移除连接或轮换凭据后的旧响应恢复快照。损坏的派生快照不会覆盖源目录，只能由下一次有效在线响应更新。

这是当前入口的只读元数据缓存，不传输项目文件，不给其他节点分发记录，也不提供离线编辑。配置来源重新可达时，下一次页面刷新会从所有者补齐元数据变化。网关目录尚无持久离线快照；CLI/MCP 直接访问目标 API 的路径不经过此 Web/Tauri 缓存。

## 授权直连目录增量拉取

Web 和 Tauri 共用 Rust 适配器，在首次完整读取后保存所有者声明的 `sync_protocol: "project-index-v1"`。后续页面刷新将私有快照的 ProjectID 与内容 SHA-256 索引发送到目标的 `POST /api/v1/mesh/nodes/{node}/projects/sync`。请求只含 UUID 和摘要，不含缓存的路径、会话、Token 或待写入记录。每次请求仍检查原有 bearer、目标身份、Origin 和 projects 能力；索引从不授予权限。

所有者先按自己的持久 NodeID 过滤目录，再比较记录摘要，返回变更的完整项目、已移除的项目 ID、当前顺序、修订号和完整目录校验值。删除一个工作副本但保留其他副本时，返回更新后的项目；最后一个副本删除时才移除整个项目。读取方核对本次基准、所有者、协议、当前权限、ID 唯一性、所属节点和重建后的摘要，再原子替换派生快照。删除记录不会删除工作文件、结束 Shell 或创建替代会话。

摘要由共享 Rust 模型的紧凑 UTF-8 JSON 序列化计算；Catalog 字段顺序为 revision/projects，Project 为 id/name/replicas，Replica 为 id/node/path/sessions，Session 为 id/terminal_id，数组保留原顺序。这不是对任意 JSON 文本直接求摘要。改变模型或摘要编码时必须声明新的同步协议，不能继续沿用 project-index-v1。

索引按内容比较，不依赖设备时间、连续事件历史或单调增长的远端修订号；离线多次修改、跨入口修改、节点重启及所有者恢复旧备份都可在下一次成功刷新时补齐当前目录。读取期间路由或凭据变化仍拒绝旧响应。明确撤权、无效协议或不完整响应会清除缓存并显示错误；不会退回旧快照掩盖错误。转发停止或全部路径不可达时才使用已有只读快照，备用路径的明确拒绝仍优先于离线判断。

旧节点未声明该协议时继续完整读取；已声明协议的节点回退到不含新端点的版本时，仅明确 HTTP 404 允许重新完整读取。认证失败、400 或校验失败不会触发此兼容回退。损坏的本地派生快照由一次新的完整读取修复。写入仍直接发送给所有者，不排队、重放或自动重试。

当前最多 512 个索引项；索引和顺序列表仍与项目数成正比，减少的是未变化项目详情的重复传输。此协议不提供文件同步、全网广播、信任传播或离线多方合并；网关旧目录入口和直接 CLI/MCP 请求继续完整读取。元数据按页面现有的前台 10 秒刷新及恢复焦点刷新更新；另可按下节显式启用节点后台同步。默认关闭，不自动轮询全网设备。

## 节点后台目录同步

Web 与 Tauri 的“项目与会话 → 后台目录同步”共用设置。默认关闭；选择 1–32 台已保存直连连接的设备，再启用。运行当前界面的 Rust 节点会在网页关闭后继续读取，节点重启或独立 peer serve 启动后恢复；无须 Workspace 网关在线。每个入口保留自己的选择，不把设置传播给其他节点。

间隔可选 15 秒、30 秒、1 分钟或 5 分钟，推荐 1 分钟；每轮完成后等待该间隔再读取同一设备，最多同时读取 4 台。后台沿用上述增量目录协议与完整读取兼容路径，只拉取项目记录、路径和会话关联，不复制工作文件，不发现或启动 Shell / Agent。页面显示所属节点、最近尝试、最近成功、项目数量和离线、拒绝或连接变更状态。手动目录请求持有锁时后台让出，稍后重试；暂停不再开始新读取，已发出的只读请求会结束。

设置保存在 peers.json 同目录的 project-sync.json，派生状态在 project-sync-state.json，Unix 权限为 0600。修改要求当前 revision，多个入口同时编辑时拒绝过期覆盖；页面保留修改并要求核对最新版本。配置或状态损坏、所属节点不符时停止后台读取，原文件保留。配置绑定接入节点、目标 NodeID、主要地址、HTTP 策略、实际 Token 和凭据文件；移除后重新添加同一连接、轮换凭据或变更主要地址均需重新选择。添加备用路径仍沿用当前授权。

目标仍须授予 projects 权限。断网时保留明确标记的只读快照；认证失败、身份或协议无效时清除相应快照。后台状态不返回 Token、绑定摘要、凭据路径或原始远端错误正文。该功能只同步当前节点已选来源，不提供全网广播、多方离线编辑或项目文件同步。

独立节点也可通过本机 CLI 管理，JSON 与 Web/Tauri 的本机入口相同：

```bash
cranibox-node peer project-sync
cranibox-node peer project-sync --command '{"operation":"configure","revision":0,"nodes":["node-TARGET"],"interval_seconds":60}'
cranibox-node peer project-sync --command '{"operation":"pause","revision":1}'
cranibox-node peer serve
```

以上 revision 只是首次保存示例；修改前读取实际状态。入站 peer API 与任务 MCP 不提供同步设置修改能力。

## 经确认的项目文件复制

Web 与 Tauri 共用“复制项目到设备”面板。本机与直连远端之间可以双向复制，也支持两个直连远端之间复制；Web 所在节点使用现有项目目录，Tauri 使用本机项目目录，本机不需要开启监听、授权自己或连接网关。首先在目标设备的“远程节点 → 入站授权”（Tauri 的设备页面）指定**已经存在的绝对接收目录**；清空目录可关闭接收。只有目标本机可以修改此设置。当前入口必须已配置远端的直连连接，远端均须向入口授予 projects 和 exec 权限；单独的 projects 权限只允许元数据访问。仅有网关连接的远端暂不支持文件复制。

选中来源工作副本后，选择目标并“生成并预览快照”。来源会读取当前工作文件，包括未提交和未跟踪的修改，Rust 入口保存固定快照，页面只显示文件、空目录、跳过项、大小和校验值。用户检查清单并勾选确认后才发送。准备后对来源文件的修改不会改变已经审阅的快照；需要新内容时重新准备。目标创建接收目录下的 cranibox-UUID/project，不覆盖已有目录，保留 ProjectID、生成新的 ReplicaID，不复制 Shell 关联、不执行安装或构建，也不删除来源。

两端节点均声明 `project_copy_protocol: 2` 时，使用磁盘暂存和每块最多 1 MiB 的顺序传输，上限为总计 2 GiB、单文件 1 GiB、32768 个文件及目录条目、8 MiB 清单。来源和入口按文件及分块校验 SHA-256，目标写入完成后再次核对整个快照，随后发布并登记。文件内容不再整体放入内存或传入浏览器。任一远端未声明新版能力时，继续使用原协议：总计 16 MiB、单文件 4 MiB、4096 项；面板会显示旧版限制。快照均有效 24 小时。

跳过 Git 历史、常见依赖和构建目录、缓存、常见凭据名及密钥扩展名；符号链接跳过，Unix 硬链接拒绝读取。不兼容跨平台的路径、大小写冲突、越界路径及校验失败均拒绝传输。过滤不能识别所有自定义凭据文件，确认前必须检查清单。超过 200 条的清单可按路径筛选并分批展开。这仍是显式确认的文件快照复制，尚不提供目录增量同步、Git 仓库迁移或 Agent 检查点迁移。

入口在 peers.json 同目录的 project-copy-jobs 保存私有清单、分块缓存和进度；新版来源在 project-copy-exports 保存最多 8 份固定快照，入口准备完成后释放来源快照，新增导出时回收过期快照。新版目标在 project-copy-stream-receipts 保存回执及清单，旧版目标继续使用 project-copy-receipts。Unix 私有文件权限为 0600。入口最多保留 32 条记录；更换连接地址、身份或实际凭据后，旧记录不会继续显示远端路径，也不能继续发送。不可用记录仅显示本机 UUID，可显式移除并释放暂存空间。新版目标最多保存 256 份回执；旧版仍按回执文件和锁文件合计最多 256 项限制，达到上限后拒绝新增；当前没有自动回收目标回执。文件内容不写入浏览器存储。

面板显示准备和传输的字节进度，打开时每 1.5 秒读取精简进度，不自动重复写请求。关闭页面后，仍在运行的入口继续处理；进程退出或网络中断后，可重新打开同一记录。准备阶段暂停时，使用“继续生成快照”，从来源原快照和入口已有分块继续，仍需检查完整清单并明确确认。传输阶段优先“查询目标回执”，再显式“使用原 ID 继续”；新版按目标持久化的文件和字节位置接续，不重发已确认的分块。

写入成功但回包丢失不会自动切换地址重发。目标复用原回执，恢复发布或登记中断，已完成的重复确认保留目标目录后续修改。目标只在自己的未发布临时目录内修复未确认的分块尾部；校验失败、接收目录变化或撤权均停止处理。无法安全恢复时保留目录供检查，不伪装完成。移除本机记录会删除暂存快照，保留目标文件，也不会取消已发出的请求。复制完成后入口释放文件缓存，保留记录及回执。

CLI 使用同一 Rust 实现，适合由本机 Agent 操作；prepare 返回清单，commit 必须使用该清单的 digest：

```bash
cranibox-node peer copy-root --path /home/me/received
cranibox-node peer copy prepare --source laptop --target desktop --replica-id REPLICA_UUID
# 仅在新版快照准备中断后，沿用原 ID 继续准备
cranibox-node peer copy prepare-resume COPY_UUID
cranibox-node peer copy commit COPY_UUID --digest SHA256
cranibox-node peer copy status COPY_UUID
cranibox-node peer copy list
cranibox-node peer copy discard COPY_UUID
# 关闭本机接收
cranibox-node peer copy-root
```

来源或目标也可以填写本机持久 NodeID；独立节点 CLI 使用 peers.json 同目录的 node-projects.json。Web 嵌入的本机目录沿用原 projects.json，入口会绑定目录位置，避免把两份目录误认为同一个来源。

Web 的 POST /api/peers 与 Tauri 的 direct_peers IPC 使用 copy_root、copy_prepare、copy_prepare_resume、copy_commit、copy_status、copy_list、copy_activity、copy_discard 命令，共用持久记录。copy_activity 只返回 ID、状态、进度和错误，不重复发送完整清单。新版节点通过 POST /api/v1/mesh/nodes/{node}/projects/transfer 的 export/read/release/begin/write/status/finish 操作处理快照；旧版 export、import、copy-status 端点继续可用。两者使用既有 bearer、目标身份与 Origin 校验，导出和接收同时要求 exec 与 projects，并持续复查撤权。远端 API 不提供修改本机接收目录的能力。当前 MCP 的 workspace_projects 仍只管理目录，文件复制通过上述 CLI 发起。

## 携带 Agent 交接说明并接续

复制面板可选择“携带 Agent 交接说明”，填写目标与约束、当前进展与决定、下一步及验证结果。说明与来源身份写入 `.cranibox/handoffs/COPY_UUID.md`，作为普通 Markdown 文件纳入同一份快照、大小限制和 SHA-256 校验；接收协议版本不变。预览可以检查全部说明，复制本身不启动 Agent。已有同名文件或大小写冲突会拒绝准备，不覆盖用户文件。

复制完成并登记后，可检查目标上的 Codex 或 Claude Code，选择权限、审阅完整接续指令和命令，再明确确认启动。工作目录固定为回执中的目标副本目录，说明固定为已审阅的快照内容；服务重新核对启动校验值，不能由页面另传命令或路径。默认只读，目标需要自行安装 CLI 并完成登录。进程使用目标节点账户的环境及登录，真实模型调用可能产生费用。

每份复制记录只提供一次接续，运行 UUID 与复制 UUID 相同。同 ID、相同内容的重试返回原运行，改变启动内容不会创建第二个进程。结果未知时先“查找已有接续任务”；关闭页面或观察端 Web 重启后，目标节点持有的进程继续运行，重开复制记录可找回输出、手动刷新或停止。输出按 UTF-8 字节偏移分页，每条流最多保存 1 MiB。移除复制记录不结束进程，仍可在目标设备的后台执行列表中管理。授权或连接变化后不能继续使用原记录；普通节点进程退出和设备重启仍不能恢复原进程。

这是用户编写的工作背景和新任务启动，不恢复模型内部上下文、原 Shell、Agent 进程、登录凭据或协作任务的派发状态，也不会停止来源 Agent。进程正常退出不表示原协作任务已完成；须单独检查结果与处理来源工作。完整运行权限与恢复边界见 [Agent 后台执行](agent-executions.md)。

说明的 goal 必填，goal / progress / next_steps / verification 分别最多 2048 / 4096 / 4096 / 2048 个 UTF-8 字节；允许换行和制表，拒绝其他控制字符。内容保存在入口私有复制记录和目标工作副本中，不写入浏览器持久存储。CLI 可读取最多 64 KiB 的 JSON 文件：

```bash
cranibox-node peer copy prepare --source laptop --target desktop --replica-id REPLICA_UUID --handoff-file /home/me/handoff.json
cranibox-node peer copy agent --command '{"operation":"copy_agent_prepare","id":"COPY_UUID","adapter":"codex","access":"read_only"}'
# 仅在完成文件复制并检查上述 review 后启动，digest 使用返回值
cranibox-node peer copy agent --command '{"operation":"copy_agent_start","id":"COPY_UUID","adapter":"codex","access":"read_only","digest":"REVIEW_DIGEST"}'
cranibox-node peer copy agent --command '{"operation":"copy_agent_status","id":"COPY_UUID","stdout_offset":0,"stderr_offset":0}'
cranibox-node peer copy agent --command '{"operation":"copy_agent_stop","id":"COPY_UUID"}'
```

Web 的 `POST /api/peers` 和 Tauri 的 `direct_peers` IPC 使用同名命令；`copy_prepare` 可额外传入上述 handoff 对象，省略时保持原文件复制行为。新接续入口复用既有节点执行 API，无需目标升级文件接收协议；目标仍需支持 Agent 启动检查及后台执行。

## 验证

- Rust 测试覆盖稳定身份、重复操作、设备范围过滤、显式跨目录 ProjectID、节点所有权、并发写入和损坏文件保护；本机真实 PTY 测试覆盖无网关启动、重复实例拒绝、替换网关连接后保留 Shell 环境变量，以及重启后保留目录但不重建 Shell。
- 真实 WebSocket/PTY 集成测试覆盖 Web/节点 API 一致性、设备组变更、原生与浏览器入口接回相同 Shell、网关重启后 PID 和环境变量保持、尺寸与控制字符、实时撤销、节点退出后缺失会话、认证与非法请求。
- Web 浏览器测试覆盖注册、创建、重新打开、关联/解除关联、移除记录不结束 Shell、无效目录，以及 375/768/1024/1440 像素的深浅色布局。
- Tauri 浏览器测试通过 IPC mock 验证共享页面、本机/网关/直连目标、未配置中继时注册项目和独立设置自启动、应用内终端、断线重新附着、最小化和关闭窗口保留 Shell；Rust 客户端另有真实套接字测试。原生窗口、托盘和三平台实际运行仍需平台构建与设备验证。
- 真实节点 CLI/MCP 与 Web 测试覆盖独立目录、默认拒绝旧授权、目录专用权限、绑定终端权限、重启后保留关联、网关停用和 Web 重启后原 Shell 环境变量保持。共享页面测试覆盖同 ProjectID 合并、部分来源失效和恢复、路由目标及 375/768/1024/1440 像素的目录来源布局。
- `test_web_host_node.py` 使用真实 herdr 和 HTTP/WebSocket，覆盖 Web/直连共用身份、目录与 Shell，Web 重启后 PID 和环境变量保持，旧主机记录幂等迁移，以及主机授权不泄露其他设备路径。显式本机目标不匹配会拒绝请求，不回退或创建 Shell。
- `test_node_identity.py` 使用真实网关、节点、CLI 和 PTY，覆盖身份迁移、冲突保护、撤销后别名保留、重复确认、运行任务保护及网关重启后原 Shell 的 PID 和环境变量保持。
- `test_web_direct_peers.py` 使用真实 Web/节点进程验证离线快照跨 Web 重启保留、关联 ID 保持、损坏快照拒绝与在线修复、撤权及凭据轮换清除、错误响应不返回缓存，以及移除连接后未完成的旧请求不能恢复快照。Web/Tauri 页面测试另覆盖离线只读、重连更新、无浏览器持久路径和四档宽度的深浅色布局。
- 同一集成测试还覆盖工作文件清单与固定快照、超过默认请求体上限的文件、接收目录关闭、撤权、凭据变更、同 ProjectID 新副本与空会话关联、重复接收保留目标修改。代理实际丢弃已落盘请求的 HTTP 回应，再重启 Web 查询回执，验证不重新发送；查询过程中移除连接也不会返回旧路径。本机 Web 与独立节点 CLI 双向复制另验证无需网关或本机监听、本机回执恢复及回执隐私；Web/Tauri 页面覆盖本机作为来源和目标、确认勾选重置与不可用记录清理。
- `test_project_handoffs.py` 通过真实 Web、独立节点和隔离 CLI 替身验证说明文件校验、只预览不启动、校验值不符拒绝、目标目录与标准输入、观察端重启后相同运行去重、中文输出分页、撤权、本机目标与停止、缺失 CLI 及文本边界。浏览器另覆盖显式确认、启动回应丢失后的原运行查询、无浏览器持久说明及四档宽度深浅主题；Tauri 使用共享组件的模拟 IPC 流程。未调用真实模型或验证跨平台 CLI 登录。
