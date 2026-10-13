# 浏览器终端与统一 Rust 服务端

2026-10-13：Web 控制中心新增全局浮动终端，使用 React + xterm.js，通过 WebSocket
接入持久终端。Workspace 本机使用 herdr；远程节点由客户端直接持有 PTY 和 Shell。可选择本机或在线且支持持久终端的远程节点。
节点客户端是独立的 `cranibox-node` 应用，服务端是 `cranibox`。

## 嵌入方案选择

| 项目 | 类型与特点 | 本项目采用方式 |
| --- | --- | --- |
| [xterm.js](https://github.com/xtermjs/xterm.js) | MIT；VS Code、Tabby 等使用的浏览器终端组件；支持 ANSI、中文输入、窗口调整 | 采用 `@xterm/xterm` 6.0.0 与 `@xterm/addon-fit` 0.11.0，直接嵌入 React 页面 |
| [ttyd](https://github.com/tsl0922/ttyd) | 成熟的独立 Web 终端服务，后端使用 C/libuv，提供自身认证和进程管理 | 独立部署可用；这里需要复用已有会话和远程节点路由，未引入额外常驻服务 |
| [portable-pty](https://github.com/wez/wezterm/tree/main/pty) | WezTerm 项目的 Rust 跨平台 PTY 库，支持 Unix PTY 与 Windows ConPTY | 服务端和独立节点客户端共用 PTY 库；Windows 需要支持 ConPTY 的系统 |

xterm.js 负责显示和输入，本机命令在独立 herdr 后台引擎的 PTY 中运行，管理调用公开 JSON CLI，接入使用 herdr terminal attach，不引用源码或内部协议。远程节点使用 portable-pty 创建 Shell，客户端内置会话管理器负责创建、列举、重命名、接管和结束会话。前端包已锁定版本；
构建后的 JS/CSS 嵌入服务端程序，运行时不需要 Node.js 或 ttyd。

## 应用与部署

| 应用 | 职责 |
| --- | --- |
| `cranibox serve` | Web 页面/API、节点网关、本机终端；单个常驻服务端进程 |
| `cranibox ctl` | Workspace 控制命令 |
| `cranibox service` | Linux 用户服务管理；监督继续由 s6 执行 |
| `cranibox runtime` | 节点注册、命令查询、MCP；不包含节点执行客户端 |
| `cranibox-node` | Windows/macOS/Linux 独立客户端；远程命令与交互终端 |

Cargo 保留兼容开发入口；正式部署包含服务端和独立客户端两个 Cranibox Rust 二进制，服务端本机终端另外使用独立升级的 herdr。客户端构建产物只包含 `cranibox-node` 或 `cranibox-node.exe`。
`workspacectl`、`workspace-services`、`systemctl`、`workspace-runtime` 和原 Web 程序路径
通过别名调用统一服务端。现有用户服务 unit、自定义服务和节点凭据保留。

```bash
npm ci --prefix addons/control-center/ui
node scripts/build-control-center-ui.cjs
cargo build --release --locked -p cranibox
# 单独构建，客户端不启用服务端依赖
cargo build --release --locked -p cranibox-node
python3 scripts/install-herdr.py --output-directory /config/bin
CRANIBOX_BIN=./target/release/cranibox bash scripts/install-cranibox.sh
```

新镜像默认初始化并启用 `agent-workspace-web.service`。迁移时停止并禁用旧的
`agent-workspace-runtime.service`，其配置文件保留。9080 与兼容的 9090 入口
由同一服务端进程监听，HTTP 和 WebSocket 不再经过服务间转发。
已有 Web unit 的命令参数保留；自定义兼容端口可在 Web unit 中追加
`--gateway-port <port>`，默认 9090。两个入口均限定回环地址，通过现有 TLS 代理访问。

安装备份位于 `/config/.local/state/cranibox/install-*`，记录原文件及服务状态；
安装步骤失败会还原。`--ensure --no-start` 用于启动时补齐缺失文件，保留已有
二进制、凭据、配置与 Web 禁用状态。

在远程机器上只安装匹配系统和 CPU 的节点客户端：

```bash
cranibox-node --server wss://workspace.example.com \
  --id my-node --token-file ./my-node.token
```

旧客户端可继续执行非交互命令；它没有终端能力标识，Web 不会提供远程终端按钮。
Linux/macOS/Windows 更新客户端后即可使用节点终端，token 无需重新注册，也无需安装 herdr。Windows 需要支持 ConPTY 的系统。

## 使用与生命周期

点击侧栏「终端」或右上角终端入口打开浮动窗口，选择本机或远程节点，新建会话后自动接入。总览、Agent 与远程节点页上的终端按钮直接在当前页面打开窗口。桌面支持移动、调整尺寸、最大化与布局预设；最小化后右上角入口显示打开的标签数量，点击即可恢复。刷新网页保留窗口状态、选中节点和会话。手机端窗口使用状态栏下方的可视区域，会话列表改为抽屉，提供 Esc、Tab、Ctrl+C/D、方向键、Enter 按钮，并跟随软键盘调整尺寸。

- 最小化、恢复、切换页面、调整窗口大小：保留原有接入，Shell 和任务继续运行。
- 脱离、关闭窗口或标签、关闭网页、网络中断：只停止接入客户端，Shell 和任务继续运行。
- Cranibox 服务端重启或节点连接中断：远程客户端继续持有 Shell，恢复连接后可接回原会话；本机 herdr 引擎也继续运行。
- 节点客户端退出或重启：客户端持有的终端会话结束，新客户端不恢复旧 Shell。
- 结束会话：停止对应 Shell 和任务、移除记录；本机终端关闭 herdr 工作区，最后一个工作区结束后停止空闲引擎。
- Shell 自行退出：记录显示已结束，可以移除。接入客户端退出码不代表 Shell 命令退出码。
- 引擎退出或机器重启：不保证任意旧进程存活，不能将恢复布局等同于继续运行旧 Shell。

默认目录为执行机器 HOME 下的 Workspace（存在时），否则 HOME。默认一个会话同时允许一个接入客户端；需要时勾选「接管已有连接」。安装、升级及状态目录见 [herdr 对接说明](herdr-integration.md)。

网页连接意外中断后自动重试接入；主动脱离、结束会话和连接被接管时停止自动重试。远程客户端离线期间持续读取 PTY 输出，每个会话缓存最近 256 KiB，接回时重放；最旧输出超出容量后丢弃。输出队列阻塞只解除网页接入，不终止 Shell。

复用现有 Web 会话、Host/Origin 校验和 CSRF。创建终端先获得绑定会话、30 秒有效、
一次性的随机票据；WebSocket 握手还需同一会话与 Origin，票据不携带节点 token。
远程终端由服务端通过现有节点连接转发，浏览器不持有节点凭据。

每个 Web 服务最多 8 个活动接入；每个远程节点最多 4 个接入 PTY；每个引擎最多管理 64 条会话记录。输入、帧和队列
都有上限，心跳检测和输出超时会释放连接，30 分钟无活动只关闭网页接入，不结束后台会话。外层网关仍需认证。
命令权限等于运行该进程的账户权限；PTY 是交互执行环境，不是恶意程序隔离沙箱。

## 验证

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
CRANIBOX_HERDR_BIN="$PWD/target/herdr/herdr" python3 -m unittest discover -s tests -p test_cranibox_terminal.py -v
python3 -m unittest discover -s tests -p test_workspace_bootstrap.py -v
CRANIBOX_HERDR_BIN="$PWD/target/herdr/herdr" npm test --prefix addons/control-center/ui
```

Linux 实测覆盖本机/远程 PTY、中文与 ANSI、尺寸调整、Ctrl+C、客户端退出、
断开后的 Shell/后台任务存活、接回相同 Shell、重命名与结束、服务端重启后远程 Shell 存活、客户端退出时结束会话，以及票据/来源/会话校验、并发限额、旧服务迁移和回滚。远程测试将 herdr 路径指向不存在的文件，验证客户端完全不依赖 herdr。
浏览器回归覆盖 375、768、1024、1440 像素宽度、中英文与真实终端输入。
CI 配置了 Linux、Windows、macOS 原生构建、节点命令测试及客户端 PTY 会话测试；本机 Linux 测试不能代替 Windows/macOS 原生验证。
