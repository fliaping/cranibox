# Rust 服务管理与浏览器控制中心

实施日期：2026-10-12。Web 只提供浏览器入口，不引入 Tauri 或桌面应用。

## 用户服务管理

原先每个自动重启的用户服务都有一个 Bash 监督进程。现在使用
workspace-services 兼容命令，加上一个用户级 s6-svscan 和每服务的 s6-supervise。
服务启动辅助程序最终 exec 成业务进程；Rust 不会为了等待正常运行的服务而常驻。
重启延迟期间的 Rust finish 辅助程序会短暂存在。

~~~bash
bash scripts/install-workspace-services.sh
systemctl --user status agent-workspace-web
systemctl --user restart agent-workspace-web
~~~

安装更新 /config/bin/systemctl 和 ~/.local/bin/systemctl，系统级调用仍
委托 /usr/bin/systemctl.py。旧镜像的 /usr/local/bin/systemctl 可以通过新的
薄启动脚本寻找持久化二进制；构建新镜像时会自动安装这个脚本。

默认安装不会批量重启用户服务。已有 Bash 监督服务在下一次主动重启时切换到 s6。
需要主动批量切换时，先生成只读计划，再应用；同一服务定义和启用链接保持原样：

~~~bash
workspace-services --user migration-plan
workspace-services --user migration-plan --exclude=temporarily-unhealthy
workspace-services --user migrate --apply \
  --rollback-manager=/config/opt/agent-workspace-services/user-systemctl-legacy.sh
# 安装程序也支持预检通过后切换：
bash scripts/install-workspace-services.sh --migrate
~~~

预检覆盖 s6 工具、已运行的旧监督进程身份、支持的 unit 指令、主程序和工作目录。
不支持的候选服务会使整批操作在重启前失败。应用时串行切换，检查业务进程连续存活
以及 s6 监督器；失败即停止后续服务。有旧管理器时尝试回退，报告中的
rollback_started / rollback_verified 分别表示启动请求成功与进程存活验证通过，
不代表 HTTP 等应用层健康检查通过。报告保存在 ~/.local/run/user-systemd/migration.json
（0600）。--exclude 只排除本次操作，不更改启用状态。原本停用的服务不会被启动。
原有 service 定义、启用链接、PID 和日志位置继续使用。新增监督目录位于
~/.local/run/workspace-services/services，日志仍在 ~/.local/log/user-systemd。

支持 start/stop/restart/enable/disable/status/show/is-active/is-enabled/list-unit-files。
支持 Type=simple/exec、ExecStart、ExecStartPre、WorkingDirectory、Environment、
EnvironmentFile、Restart、RestartSec、SuccessExitStatus 和 TimeoutStopSec；
支持 SIGTERM 及当前使用的 control-group/mixed 停止策略。
这是容器兼容层，不是完整 systemd：没有依赖调度、cgroup 隔离、socket activation
或完整 unit 语法。停止时按进程启动时间检查身份并清理可追踪的子孙进程，
不等同于真正的 systemd cgroup；已脱离并重新归属其他父进程的后代不受保证。
不支持的 ExecStartPost/ExecStop、User/Group 或 service 类型明确报错。

环境快照使用 NUL 分隔的数据文件；兼容读取旧 export 快照，但不执行其中的脚本。
env -i 命令只补入允许的语言、时区、代理和证书变量，命令中的显式变量优先。
ExecStart 中无选项的前置 setsid 在 s6 下直接执行其后程序：s6 已提供独立进程组，
再次 setsid 会派生进程，使监督器只跟踪已退出的启动器。原 unit 无需修改。
失败迁移留下 s6 目录、但服务已回退到旧 Bash 监督器时，stop 会同时处理两种
监督器身份，避免旧监督器继续重启业务进程。

## 浏览器控制中心

workspace-web 使用 Rust/Axum 实现 HTTP、会话、状态缓存、动作分派、配置
对话框和输出流。默认仍是 127.0.0.1:9080，由现有认证网关提供外部访问。
本地管理操作通过固定的 workspacectl 参数执行。远程节点任务通过独立的
鉴权网关 API 传递程序和参数，显式选择 Shell 时可以执行 Shell 命令。

浏览器界面使用 React、TypeScript、React Router 和 Vite，JS、CSS 与英文/中文
HTML 在源码中预构建并嵌入二进制。Node 仅用于构建前端；运行 Web 二进制不需要 Node。
IDE 扩展仍使用自己的 JavaScript 动作实现，不随这次后端迁移改写。

~~~bash
workspacectl web install
workspacectl web status
curl http://127.0.0.1:9080/healthz
# 仅更新程序和 unit，不启停服务：
bash scripts/install-workspace-web.sh --no-start
# 修改 React 前端后重新构建静态资源，再构建 Rust 程序：
npm ci --prefix addons/control-center/ui
node scripts/build-control-center-ui.cjs
cargo build --locked -p agent-workspace-web
~~~

预编译安装分别使用 AGENT_WORKSPACE_SERVICES_BIN 和 AGENT_WORKSPACE_WEB_BIN。
构建使用 Cargo.lock。Web 会话上限 64，操作日志每会话最多 128 KiB，事件最多
256 条；状态采集共享 15 秒缓存。会话在重启后清空，操作历史独立落盘，最近
128 条记录和保留的输出可以从「操作历史」页面读取。重启前仍运行的记录变为
interrupted，不会自动重试或重放命令。开始任务前和任务完成时落盘，运行日志
每 2 秒批量刷新；强制杀进程可能丢失最近约 2 秒输出。

## 启动任务

init、custom-services、systemctl-services 保留短时 Bash 实现，改成原生 s6-rc
oneshot。init 等待基础初始化，custom 注册器等待 init 和 init-services，
systemctl-services 等待注册器。legacy-services 会等待 user bundle 完成，
因此启动任务先于遗留常驻服务执行。自定义服务注册器也会预留镜像服务名称，
避免在遗留服务链接尚未创建时发生覆盖。

这些变更在重建镜像后生效；不会在线替换当前容器的 s6 启动图。收益是去掉任务
结束后的三个 sleep infinity 及对应监督进程，不需要 Rust 重写启动逻辑。
workspace-bootstrap 的独立启动流程这次保留。

## 验证

~~~bash
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
python3 -m unittest discover -s tests -p test_workspace_services.py -v
node scripts/check-control-center-web.mjs
node scripts/check-control-center-webview.mjs
~~~

服务集成测试使用隔离 HOME 和实际 s6，覆盖故障重启、成功退出、并发启动、
停止重启等待、环境传递、启动前命令、后代清理、旧监督器迁移和失效 PID 身份。
另覆盖 setsid 服务的真实业务 PID、迁移后稳定运行和停止后端口释放，以及
迁移失败回退后停止旧监督器；2026-10-12 的服务专项验证为 14 项集成测试和
3 项 Rust 单元测试，格式和 Clippy 检查通过。
Web 集成检查启动真实 Rust HTTP 服务，验证访问限制、缓存、对话框、日志、
刷新恢复、取消任务和紧急停止。内存节省须在相同负载下比较，不能由语言推算。

## 当前容器的切换记录

2026-10-12 首次安装 Rust CTL、用户服务兼容入口和 Web 二进制，将 Web 服务切换
到 s6。9080 仍绑定 127.0.0.1；页面、状态快照及 Rust healthz 验证通过。
首次切换仅涉及 Web。ocx-proxy 与 hermes-dashboard 原有监督器正在自行
重试启动，因此不能用它们变化的业务 PID 判断本次是否重启了其他服务。

访问页面和状态接口后，旧 Node Web 主进程 RSS 采样为 49,568 KiB，新 Rust Web
主进程为 6,392 KiB。这里只比较 Web 主进程，未计入 CTL 子进程或 s6；两者运行
时长不同，是本机观察值，不是可推广的性能基准。

备份在 /config/.local/state/agent-workspace/native-web-backup-20261012T034441Z，
含原 unit、CTL、旧管理入口和切换前后记录。需要恢复旧 Web 时，先用新的
systemctl 停止 agent-workspace-web，恢复备份 unit，再通过备份中的旧管理脚本
启动。原 Node 运行目录保留，当前安装和 unit 不再使用 server.cjs。

同日后续按用户指定范围，将以下五个项目提供或集成的服务切换到 Rust 管理入口
和 s6；业务程序继续使用各自原有实现：

| 服务 | 监督器 | 迁移后业务检查 |
| --- | --- | --- |
| agent-desktop-bridge | s6-supervise | /health 返回 503，与迁移前相同 |
| deepseek-harness | s6-supervise | HTTP 200 |
| openclaw-gateway | s6-supervise | HTTP 200 |
| proxy-caddy | s6-supervise | 本地管理接口 HTTP 200 |
| tailscaled-workspace | s6-supervise | 状态命令成功，NeedsLogin，与迁移前相同 |

迁移前只读预检并备份五份 unit 和旧管理器。使用 --exclude 排除其余所有服务，
串行迁移；未使用安装程序的无范围 --migrate。第一项桥接服务因上述 setsid
问题中止整批，修复并通过专项测试后清理已确认身份的孤立桥接进程，恢复桥接
服务，再继续另外四项。五个旧 Bash 监督器已退出，复查业务与 s6 监督器 PID
保持稳定；所有服务定义和启用状态保持原样，其余服务监督器身份未变。

脱敏验证记录见 [五项服务迁移记录](validation/2026-10-12-project-services-migration.json)。
完整备份和首次失败、恢复、后续迁移报告保存在私有目录
/config/.local/state/cranibox/project-service-migration-20261012T132957Z。
桌面桥接 503 与 Tailscale 未登录需要后续业务排查，本次仅迁移管理方式。

同日更新 Web 到包含远程节点页面的版本，并补齐节点网关为内置服务。镜像构建
增加 Rust 构建阶段，init 在首次启动时准备持久化程序、凭据和默认启用的 Web/网关
服务；启动顺序沿用 init → custom-services → systemctl-services。后续启动保留
已有程序、服务配置和禁用状态。网关安装器支持预编译程序及保留配置的 --ensure，
CTL 根据具体脚本是否存在选择来源，修复旧缓存源码缺少网关安装脚本的问题。

当前容器 Web 9080 和网关 9090 的 healthz 均返回 200，两项用户服务已启用。
浏览器节点页注册按钮可用，已注册节点数为 0；凭据权限为 0600，未输出 token。
隔离首次启动测试 3 项、CTL 集成测试 24 项、Rust 单元测试 29 项通过（另有
1 项显式测量用例默认忽略），release 构建、Clippy、格式和 Web 集成检查通过。
完整 Docker 镜像构建未运行；新镜像的启动准备脚本已用真实 Rust 程序和 s6 验证。
验证记录见 [内置节点网关](validation/2026-10-12-builtin-node-gateway.json)。
