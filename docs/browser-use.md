# 授权节点的 Browser Use

2026-10-18：Agent 可以通过独立的 browser 授权操作另一节点明确共享的 Chromium 浏览器。Rust 节点负责身份、授权、WebSocket 转发和停止；调用端运行固定版本的官方 Chrome DevTools MCP，不在 Rust 项目内复制其代码。Web 与 Tauri 共用配置、授权、状态和停止界面。

## 能力范围

- 目标必须已有支持 CDP 的 Chromium 浏览器，并明确配置回环 IP:port。节点不会自动安装、启动浏览器或修改日常用户目录。
- browser 权限默认关闭，旧授权保持关闭；desktop、computer、exec 和 terminals 均不隐含此权限。允许 browser 不要求允许命令执行。
- 授权覆盖共享浏览器的全部标签页、登录会话、脚本执行和网络能力，没有按域名或标签页隔离。建议使用专门的浏览器用户目录。它也不受 Computer Use 的输入租约或桌面停止按钮管理。
- 使用已保存的节点直连路由，可经现有 EasyTier/Tailscale 网络访问；当前不支持旧 Workspace 出站网关转发。调用端和目标端需要可达，浏览器 CDP 端口本身不需要暴露到网络。
- 每个目标节点仅允许一个远程 CDP 控制连接。其他本地 CDP 工具仍可能访问同一浏览器，不在这个远程名额内。

## 目标设备配置

在目标 B 上启动专用 Chromium，以下为 Linux 示例，命令名称及路径按系统调整：

~~~sh
chromium --remote-debugging-address=127.0.0.1 --remote-debugging-port=9222 \
  --user-data-dir=/path/to/cranibox-browser-profile

cranibox-node peer browser --address 127.0.0.1:9222
cranibox-node peer browser-owner --command '{"action":"status"}'
cranibox-node peer grant --node NODE_A --browser --output-token ./node-a.token
~~~

不要添加 --no-sandbox 到日常浏览器。地址只接受回环 IP 和非零端口，例如 127.0.0.1:9222 或 [::1]:9222；localhost 不作为配置地址。Web 的配置对应 Web 服务所在设备，Tauri 的配置对应运行客户端的设备，不一定是当前网页所在电脑。

Token 文件只供对应调用方保存，不提交仓库。监听器、NodeID、凭据交付和连接方法见[节点直连](direct-node-peers.md)。已经运行 Web/Tauri 节点宿主时，通过其界面启用监听器，不要为同一身份再启动一个 peer serve。配置是持久的，浏览器本身仍由设备用户管理。

设备界面的“Agent 浏览器操作”面板可保存或关闭共享、检查浏览器状态及紧急停止；“授权设备接入”中单独勾选浏览器能力。清空地址保存或执行 cranibox-node peer browser 可关闭共享。修改配置会断开已有远程控制连接，保存地址不会清除已持久化的紧急停止。

## 调用设备配置

在 A 保存 B 颁发的凭据并建立到 B 的路由后：

~~~sh
cranibox-node peer call --node NODE_B browser
cranibox-node peer call --node NODE_B browser-mcp
~~~

第一条检查目标状态；第二条是供 Agent 配置的独立 stdio MCP 进程，明确绑定 NODE_B。自定义 settings.json 时在 peer 前添加 --config /path/to/settings.json。安装桌面客户端的用户也可将可执行文件替换成实际安装的 cranibox-node-desktop；AppImage 应使用实际持久路径并添加 --appimage-extract-and-run，不使用临时挂载路径。

调用设备需要 Node.js 与 npx。当前固定启动 chrome-devtools-mcp@1.11.0，要求 Node ^20.19.0、^22.12.0 或 >=23；首次启动需要 npm 下载，后续可使用其缓存。默认 MCP 配置不会自动添加这一目标，也不会自动安装 Node.js。进程将 --wsEndpoint 指向 Rust 创建的临时回环代理，并关闭使用统计和 CrUX 查询；目标 Token 不会写入 MCP 参数或传给子进程。工具及 pageId 等参数遵循该固定版本的官方 MCP schema，Agent 应先读取 tools/list。

已有 CDP 客户端可运行以下命令获取临时端点：

~~~sh
cranibox-node peer call --node NODE_B browser-proxy
~~~

stdout 输出目标 NodeID 和随机回环 WebSocket 地址。该地址仅在进程存活时有效，不提供 HTTP /json/version；使用输出的完整 WebSocket 地址连接。代理校验 Host、随机路径，拒绝 Origin 与 Sec-Fetch-Site 请求。MCP 关闭 stdin、退出或收到 Ctrl-C 后，代理及其子进程树随之清理。失去连接后应先检查状态，节点不重放浏览器动作。

## 紧急停止与恢复

目标管理界面、目标 CLI 以及获 browser 授权的调用方均可停止：

~~~sh
# 调用方 A
cranibox-node peer call --node NODE_B browser --command '{"operation":"emergency_stop"}'

# 目标所有者 B
cranibox-node peer browser-owner --command '{"action":"emergency_stop"}'
cranibox-node peer browser-owner --command '{"action":"resume"}'
~~~

停止先保存暂停状态，再断开控制连接；浏览器离线和节点重启后仍保持停止。恢复只允许目标的所有者 CLI 或已认证管理界面明确操作，并要求配置的浏览器可用；远端 resume 请求被拒绝。撤权、Token 轮换、关闭监听器、节点重启和重新配置浏览器也会使已有连接失效。状态查询不会占住停止通道。已经发给浏览器的动作无法撤回，停止不会关闭浏览器或中止其已启动的下载、脚本和网络请求。

## 协议边界

远端接口为 POST /api/v1/mesh/nodes/{NodeID}/browser，支持严格类型的 status / emergency_stop；WebSocket 为同路径加 /ws。两者要求 Bearer 与 X-Cranibox-Target，拒绝浏览器 Origin。Web 管理入口 POST /api/peers 与 Tauri direct_peers IPC 共享 browser_sharing、browser_owner、browser 三种操作；Web 继续执行登录、Origin 和 CSRF 校验。网页不是 CDP 客户端，只提供管理控制。

目标只请求所有者指定回环地址的 /json/version，禁用代理和重定向，超时 5 秒、元数据上限 64 KiB。只接受浏览器级 /devtools/browser/{id}，忽略元数据宣称的主机，始终连接已配置的回环地址。远端状态仅返回浏览器版本和可用性，不泄漏 CDP 端点。单条 WebSocket 消息上限 16 MiB；约每 250ms 独立检查撤权/暂停，心跳检测断线。没有跨故障自动重连、动作重试、域名白名单或细粒度审计。

## 验证

指定远端 Linux 容器内的 tests/test_browser_use.py 共 9 项集成测试通过，运行真实 Rust 节点和受控 CDP 服务：旧授权、独立能力、目标身份、消息转发、单控制者、撤权、配置变更、慢状态并发停止、跨重启保留停止，以及回环代理的路径/Origin/Host 限制和退出清理。

tests/test_browser_real_chromium.py 另有 1 项真实 Chromium 与官方 MCP 验收：专用临时用户目录、独立节点、打开测试页、读取快照、填表提交、取得 PNG、紧急停止后工具失败且不重复提交、正常关闭 MCP。它不访问日常 9222，也没有调用真实模型。CI 在 Linux 使用 Playwright 提供的独立 Chromium 显式执行：

~~~sh
CRANIBOX_TEST_REAL_BROWSER=1 \
CRANIBOX_TEST_CHROMIUM=/path/to/chromium \
python3 -m unittest discover -s tests -p test_browser_real_chromium.py -v
~~~

Web 专项 3 项、Tauri IPC 专项 1 项覆盖默认关闭、错误输入保留、恢复确认、慢查询期间停止、选定目标、中英文及小屏明暗主题。界面测试部分模拟业务响应和原生 IPC，不代替真实安装包。Windows/macOS 的浏览器提供者、原生包、跨物理设备及模型实际调用尚未验收；系统依赖安装引导、自动提供者启动和按标签页限制未实现。

本轮完整回归、初测失败、证据目录和提交状态见[功能与验证交接](implementation-handoff-2026-10-18.md)。
