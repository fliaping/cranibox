# 独立 Web 控制中心

无需加载 code-server，即可管理 Agent、托管浏览器、MCP / Skills、服务、
桌面、网络、诊断和远程节点。浏览器界面使用 React、TypeScript、React Router 与 Vite，
由 Rust 后端提供页面和 API。支持中英文、
明暗主题、移动端布局、操作日志和刷新后的任务恢复。

## 安装与访问

镜像默认包含 Web 控制中心和节点网关，首次启动自动初始化并启用；无需另行安装。
既有旧环境补齐或修复时可运行：

```bash
workspacectl install web
workspacectl web status
workspacectl web url
```

默认仅监听容器内 `127.0.0.1:9080`，没有独立账号系统。远程访问必须通过
带认证的网关或 SSH 隧道，不能直接公开管理接口。普通 Docker 端口映射
无法连接只监听容器回环地址的服务；SSH 转发需要终止在该容器内。

使用自定义域名时，先在外层网关配置 TLS 和认证，再配置可信入口与 Caddy：

```bash
workspacectl web configure \
  --public-url https://workspace.example.com \
  --workspace-url https://code.example.com \
  --desktop-url https://desktop.example.com
workspacectl proxy add workspace.example.com 127.0.0.1:9090
```

统一 Rust 服务端在 9080 与兼容的 9090 同时接收 WebSocket 和 HTTP，页面和节点网关
由同一进程提供。详见 [浏览器终端与统一服务端](web-terminal-and-unified-server.md)。
Web 入口必须使用独立域名根路径，不支持挂载到 `/workspace/` 等子路径。
网关应保留公开入口的 Host 和 Origin；仅显式配置的域名与容器本地入口
可以访问。会话 Cookie、CSRF 与来源检查用于防御跨站请求，不能代替网关认证。

## 使用边界

- 常规管理操作直接在 Web 完成；「终端」页可操作本机或远程节点的交互式终端，
  也可以从远程节点页直接进入。Agent 启动入口仍提供 code-server 和可复制命令。
- 切换中英文复用已有语言配置流程；同步 code-server 前会要求确认重启。
- 页面先加载，再异步获取状态；多标签页共享状态缓存，避免重复扫描所有服务。
- 安装任务、对话框与输出在页面刷新后可恢复。对话框和会话只保存在内存中，
  操作历史另存到 /config/.local/state/cranibox/web-history，最近 128 条记录、保留输出
  和结果在重启后仍可读取。未完成记录标记为 interrupted，不自动重试。
- Web 最多保留 64 个浏览器会话；达到上限时回收最久未使用且没有活动请求、
  运行操作、待回答对话框或实时连接的会话。终端和桌面连接会保留所属会话。
  被回收的页面重新访问时建立新会话，持久操作历史和节点上的 Shell 不受影响。
- 实时连接使用带 CSRF 的 POST /api/events。首帧同步状态和输出，此后按 UTF-8 字节
  游标只传新增日志；状态快照仅在缓存更新时推送。每会话最多 4 个并发流，空闲保活
  15 秒。连接失败会退回轮询，隐藏页面暂停连接，重新可见时同步状态。
- 「资源与性能」显示 Web 主进程 RSS/CPU、状态采集耗时、缓存复用和 SSE 传输统计。
  进程统计目前支持 Linux，其他平台显示不可用；统一部署包含 Web 与节点网关，不计入 CTL 子进程或业务服务。
- 「远程节点」页注册和撤销节点、下载一次性凭据，并提交程序和参数执行。
  远程任务在统一服务端网关模块运行，详见 [远程节点说明](rust-remote-nodes.md)。
- Computer Use 紧急停止不受当前安装任务或对话框阻塞。
- Agents 页的「更新 Gateway」由 Agent Workspace 管理 OpenClaw：先把新版安装到
  `/config/opt/openclaw/releases`，再切换并验证 Gateway；失败会恢复原用户服务。
  OpenClaw 自带网页更新仍依赖真正的 systemd 用户服务，容器内请使用此入口或
  `workspacectl agent update openclaw`。新安装的 Gateway 会设置
  `OPENCLAW_SUPERVISOR_MODE=external`，标明服务由 Workspace 管理。

## 前端开发

前端源码位于 addons/control-center/ui/src。构建后的 JS、CSS 和中英文 HTML
嵌入 Rust 二进制，运行时不需要 Node。IDE 扩展保留自己的界面。
页面按业务拆分到 src/pages，components.tsx 维护复用组件，workspace.tsx
管理会话、实时连接与轮询回退。TypeScript 开启 strict 和未使用符号检查。
设计依据见 [UI/UX Pro Max 规范](../design-system/cranibox/MASTER.md) 与
[控制中心调整](../design-system/cranibox/pages/dashboard.md)。

修改前端时先构建静态资源，再构建或安装 Rust 服务：

~~~bash
npm ci --prefix addons/control-center/ui
node scripts/build-control-center-ui.cjs
cargo build --locked -p cranibox
# 浏览器回归测试使用合成数据和临时 Rust 服务，不操作实际服务。
npx --prefix addons/control-center/ui playwright install chromium
npm test --prefix addons/control-center/ui
python3 -m unittest discover -s tests -p test_workspace_web_operations.py -v
workspacectl install web
~~~

## 维护

配置：`/config/.config/agent-workspace/web.json`；程序：
`/config/bin/cranibox`（原 Web 程序路径是兼容别名，运行时不依赖 Node）；服务：`agent-workspace-web.service`。
更新项目源码后重新运行 `workspacectl install web` 更新已安装的程序。

```bash
workspacectl web restart
workspacectl logs agent-workspace-web.service
curl http://127.0.0.1:9080/healthz
node scripts/check-control-center-webview.mjs
node scripts/check-control-center-web.mjs
```

Rust 服务管理和启动任务迁移见 [实施说明](native-services-and-web.md)。


增量接口与历史输出使用绝对 UTF-8 字节游标。输出窗口滚动导致游标过旧或处于字节
边界之外时，响应中的 *Reset 为 true，客户端替换对应输出。请使用返回的 *Cursor，
不要从字符数量估算字节位置。历史列表不返回输出；详情接口按需读取。持久化目录
为 0700、记录为 0600，单条记录最大 512 KiB，输出保留 128 KiB。运行日志每 2 秒保存，
强制结束服务可能丢失最近约 2 秒输出；记录恢复不会执行其中的命令。

可复现资源测量：python3 scripts/measure-workspace-resources.py --report target/resource-baseline.json。
脚本使用临时目录和临时服务，分别测量 Bash/s6 监督进程、实际 Rust API 载荷及
真实 Web/网关/节点。详细口径和本机结果见 [资源基线](resource-baseline.md)。
