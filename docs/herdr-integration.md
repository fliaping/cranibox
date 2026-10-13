# herdr 持久终端对接

Cranibox 使用独立 herdr 程序，不添加 Cargo 依赖、不复制 herdr 引擎源码，也不实现 herdr 内部客户端协议。当前适配版本为 **0.9.3**；参考 devswha/herdr-web-ui 的桥接方式，保留 React + xterm.js 和 Rust WebSocket 后端。

## 网页工作台

终端页面适配 herdr-web-ui 的会话侧栏、标签导航、终端工具和输入法处理，使用现有 Cranibox 会话接口。本次参考提交为 `b76aebefded791d3416d1df64337e2b76543b217`；MIT 版权说明位于 `docs/licenses/herdr-web-ui.txt`，构建脚本也将它保留在分发的 `web.js` 中。页面设计约定见 `design-system/cranibox/pages/terminal.md`。

终端通过全局浮动窗口打开，总览、Agent 和远程节点上的终端按钮直接在当前页面打开它。桌面支持拖动标题栏、拖动边角调整大小、最大化、预设尺寸与位置；标题栏和右下角也支持方向键操作。窗口最小化到右上角状态栏，显示打开的标签数量，点击恢复。最小化及切换页面保持原有连接与显示内容；关闭窗口仅脱离，后台会话继续运行。桌面尺寸与位置保存到本机浏览器，刷新网页恢复当前浏览器标签的窗口状态和会话。旧的 `/#/terminal?node=…&session=…` 链接仍可选择节点和会话并打开浮动窗口。

桌面端侧栏选择会话后直接接入，新建会话自动打开。最多同时打开 4 个标签；每个标签保留独立连接和显示内容，支持同时查看两个终端，也能跨节点保留连接。关闭标签仅脱离，结束会话需另行确认。Agent 启动按钮创建相应命名的会话，并将启动命令填入命令输入栏，用户回车即可执行。

手机端会话列表改为抽屉，命令使用原生文本框编辑后发送，兼容中文输入法的候选确认；Shift+Enter 换行，另提供 Esc、Tab、Ctrl+C、方向键等触摸快捷键。界面随浏览器可视区域变化，减少软键盘遮挡。手机上的并排会话通过标签切换显示。

xterm 的 DOM 渲染器动态生成字符尺寸与 ANSI 颜色样式，公开 API 不支持设置样式 nonce。因此网页 CSP 对样式允许 `unsafe-inline`；脚本继续限制为同源资源与每次请求生成的 nonce。浏览器回归检查缩放后的实际可见字符、样式策略错误及脚本限制。

这次移植的是终端工作台交互；上游的原生工作区/标签树、Agent 聊天、文件浏览和审批流程尚未对接。每个 Cranibox 持久会话仍映射一个 herdr 工作区及根终端，管理与鉴权继续由现有 Rust 服务承担。

## 部署

新 Docker 镜像构建时下载并验证官方 release SHA-256，初始化时将 herdr 安装到 `/config/bin/herdr`，保留已安装版本。源码部署或远程 Linux/macOS 节点可以运行：

~~~bash
python3 scripts/install-herdr.py --output-directory "$HOME/bin"
~~~

也可以安装官方 herdr 并加入 PATH，或给 `cranibox` / `cranibox-node` 设置 `CRANIBOX_HERDR_BIN=/absolute/path/to/herdr`。查找顺序为该环境变量、执行机器 HOME/config-root 下的 `bin/herdr`、PATH。没有 herdr 时 Web 显示安装提示，原有命令执行仍可使用。浏览器无需连接另一个 Web 服务或保存节点凭据。

herdr 0.9.3 的直接终端接入仅在 Unix 提供。Windows 节点不声明持久终端能力，Web 显示说明；Windows 远程命令照常使用。macOS 有原生发布资产和 CI 构建，本次持久终端实际运行验证在 Linux，不能据此承诺 macOS 交互行为已经实测。

## 接口和进程

管理使用公开 CLI JSON：`workspace create/list/rename/close`、`api snapshot` 和 `server stop`。每个 Cranibox 会话对应一个 herdr 工作区及根终端。接入执行 `herdr terminal attach <terminal-id>`；需要接管时追加 `--takeover`。临时客户端 PTY 通过现有 WebSocket 通道把 ANSI 显示流转发到 xterm.js。

第一次创建会话启动独立 headless 引擎。Linux/macOS 引擎在独立进程会话运行，不属于接入客户端的清理范围。断开只终止客户端；Shell、后台任务归 herdr 所有。结束最后一个工作区才停止空闲引擎。没有终端会话的机器不会启动额外引擎。接入等待客户端启用 raw mode；Unix 写入端关闭不发送隐含 Ctrl+D，避免断开时退出 Shell。

本地目录为 `<config-root>/.local/state/cranibox/terminal`；远程目录为节点 token 文件旁的 `.cranibox-herdr-<node-id>`。包含 `sessions.json`、`config.toml`、`engine.log`、`engine.pid` 和本地 Socket。每个目录映射独立 `HERDR_SESSION` 名称，避免与个人 herdr 或其他节点共享恢复记录。herdr 自己的日志和恢复文件仍位于执行账户的 `~/.config/herdr/sessions/cranibox-<hash>`。

会话目录保存不可猜测的 UUID、名称、目录、herdr 工作区及终端 ID。列举和接入校验归属，同时匹配工作区与终端 ID，防止引擎重启复用工作区编号。操作复用登录、CSRF、Origin 和一次性接入票据。每个引擎最多 64 条会话记录、Web 最多 8 个接入、远程节点最多 4 个接入。

## 升级

默认版本和已核验发布哈希在 `scripts/install-herdr.py`。可单独安装指定版本：

~~~bash
python3 scripts/install-herdr.py --version 0.9.3 --output-directory "$HOME/bin"
~~~

其他版本从 GitHub release API 获取 SHA-256；没有官方 digest 或校验失败就拒绝替换。旧二进制备份为 `herdr.previous`。替换文件不会自动升级已运行的引擎：先结束该机器上的全部会话，再替换，新建会话会启动新引擎。升级后应运行终端回归；公开 CLI/JSON 契约兼容时无需重新编译 Cranibox。还原备份可回退二进制，但不能恢复已结束的进程。

网页断开、节点和服务端重启后的存活依赖 herdr 引擎及执行机器持续运行。引擎或系统重启、移动目录、清理账户文件均不保证旧 Shell 存活。终端显示流是客户端屏幕渲染结果，不是 Shell 原始 stdout；需要精确输出和退出码的 Agent 自动化继续使用原有远程命令 API。

## 验证

~~~bash
python3 scripts/install-herdr.py --output-directory target/herdr
cargo build --locked
CRANIBOX_HERDR_BIN="$PWD/target/herdr/herdr" python3 -m unittest discover -s tests -p test_cranibox_terminal.py -v
CRANIBOX_HERDR_BIN="$PWD/target/herdr/herdr" npm test --prefix addons/control-center/ui
~~~

回归使用隔离服务、节点和 herdr，引擎在测试结束后停止。
