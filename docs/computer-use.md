# 授权节点的 Computer Use

2026-10-18：节点新增独立的 Computer Use 权限、直连 API、目标绑定 MCP，以及 Web/Tauri 共用的桥接配置和紧急停止。它复用目标上已经运行的 agent-desktop-bridge，不会安装驱动，也没有实现跨平台 Browser Use。

## 范围与权限

目标用户先配置一个回环 IP:port，再为指定调用方单独授予 computer。旧配置、旧授权和新授权表单均默认关闭；desktop、exec、terminals 不会隐式授予此能力。Web 配置的目标是 Web 服务所在设备，不是浏览器所在电脑。

这条能力使用节点直连路由，可经已有 EasyTier/Tailscale 网络访问，暂不支持旧 Workspace 出站网关转发。Windows/macOS 没有随包提供原生桌面驱动；现有桥接的 Linux/Selkies 路线仍须在具体图形会话验证。VNC/桌面查看器与此接口是不同能力。

## 配置与调用

以下命令使用各节点已有的 settings.json；自定义位置时，在 peer 前加 --config /path/to/settings.json。NodeID 和 Token 的创建、保存及监听器说明见[节点直连](direct-node-peers.md)。

在目标 B 上，先运行本项目的桌面桥接，再配置共享：

~~~sh
cranibox-node peer computer --address 127.0.0.1:8765
cranibox-node peer grant --node NODE_A --computer --output-token ./node-a.token
~~~

地址必须是回环 IP，端口不能为零。上面的 8765 是示例，以实际桥接监听端口为准。grant 将 Token 写入新的私有文件 node-a.token；通过已有安全方式交给 A，不提交仓库。已有 Web/Tauri 节点宿主时直接通过其界面启用监听器，不要再启动同身份进程；独立 CLI 的监听和启动方式见直连文档。

在 A 保存 B 颁发的凭据并建立路由后：

~~~sh
cranibox-node peer call --node NODE_B computer
cranibox-node peer call --node NODE_B computer --command '{"operation":"action","action":{"action":"screenshot"}}'
cranibox-node peer call --node NODE_B computer --command '{"operation":"acquire","ttl_secs":60}'
~~~

acquire 返回 lease UUID；将它用于后续输入、续期与释放：

~~~sh
cranibox-node peer call --node NODE_B computer --command '{"operation":"action","lease":"LEASE_UUID","action":{"action":"type","text":"hello"}}'
cranibox-node peer call --node NODE_B computer --command '{"operation":"renew","lease":"LEASE_UUID","ttl_secs":60}'
cranibox-node peer call --node NODE_B computer --command '{"operation":"release","lease":"LEASE_UUID"}'
~~~

输入控制权有效期为 1–300 秒，默认 60 秒，绑定调用方身份及当前 Token。目标同时只允许一个控制者，与 VNC/Selkies 查看器共用名额。截图、区域截图、光标位置与窗口列表不要求输入控制权，仍要求单独授权。控制权到期、撤权、换 Token、重配桥接、监听器停止或节点重启后失效；后台约每 250ms 清理，也会在每个请求前重新检查。断开一次 HTTP 请求不会立即释放控制权，应显式 release 或等待到期。

## 紧急停止与恢复

调用方可发 emergency_stop，目标管理者也可点击 Web/Tauri 的“紧急停止 Agent 输入”。停止先持久保存暂停状态，再通知桥接，并释放控制权。即使桥接离线或节点重启，也不能重新申请输入控制权。慢状态查询或截图不会占住停止通道；已经发给底层桌面的操作无法撤回。

远端没有 resume。恢复只能通过目标的已认证管理界面确认，或由目标所有者执行：

~~~sh
cranibox-node peer computer-owner --command '{"action":"resume"}'
~~~

只有桥接确认恢复且配置未改变，节点才清除持久暂停。桥接的暂停也会影响该桥接的其他本地调用者；VNC 查看器仍按自身权限和生命周期管理，Computer Use 停止不代表关闭整个桌面。留空保存桥接地址，或运行 cranibox-node peer computer，可关闭 Computer Use 共享。

## MCP 与 API

为选定目标明确添加独立 MCP 进程：

~~~sh
cranibox-node peer call --node NODE_B computer-mcp
~~~

该入口提供 device_computer，参数与上述 JSON 相同。每次 tools/list 实时探测指定目标；能力不可用或已撤权时返回空列表。截图返回标准 MCP image/png 内容块及带目标 NodeID 的文字结果。它不会自动出现在默认 MCP 配置，也不能通过参数改成另一台设备。

远端端点是 POST /api/v1/mesh/nodes/{NodeID}/computer，需要 Bearer 和 X-Cranibox-Target，拒绝浏览器 Origin 请求。操作为 status / acquire / renew / release / action / emergency_stop。Web 的 POST /api/peers 和 Tauri direct_peers IPC 共用 computer_sharing、computer_owner、computer 操作；Web 继续使用登录、Origin 和 CSRF 校验。当前界面负责配置、状态、授权和停止，实际 Agent 动作从 CLI/MCP 或 API 调用。

| 动作 | 参数与限制 |
| --- | --- |
| screenshot、cursor_position、window_list | 无附加参数 |
| zoom | region 为 [x1,y1,x2,y2]，像素坐标且区域有序 |
| mouse_move、left_click、right_click、middle_click、double_click | coordinate 为 [x,y]；点击可省略坐标；coordinate_space 为 pixels 或 normalized |
| left_click_drag | start_coordinate、coordinate，与上述坐标空间相同 |
| scroll | scroll_direction 为 up/down/left/right；scroll_amount 为 1–100；可选坐标 |
| type、key | text；输入文本上限 16384 UTF-8 字节，按键名称上限 128 字节 |
| window_focus | 十进制字符串 window_id，最长 20 字符；底层是否支持由实际驱动决定 |

所有请求拒绝未知字段和未列出的动作。像素坐标 0–65535，归一化坐标 0–1，必须有限；不提供持续按住输入、任意 URL、远端恢复。节点只请求目标用户配置的回环 /v1/action，禁用代理、重定向和自动重试，桥接超时 10 秒，响应最多 8 MiB。PNG 检查 Base64、签名、IHDR 及尺寸上限，不能替代完整图像解码校验。输入响应丢失后返回错误，由调用方检查状态，不换路径重放。

## 验证与未完成事项

专项 Python 集成共 12 项，启动真实 Rust 节点、实际 Python 桥接和真实 Web 后端；桌面原生动作使用可控 HTTP 夹具，不操作日常桌面。覆盖默认关闭、授权隔离、控制权竞争/到期/撤销、VNC 互斥、节点及桥接重启后暂停、桥接离线停止、慢查询期间停止、输入不重放、请求与 PNG 边界、目标绑定 MCP。另有 Web 专项 2 项及 Tauri IPC 专项 1 项，后者属于完整客户端界面套件。

真实桌面动作、Windows/macOS 驱动、多台物理设备与模型调用尚未端到端验收。Browser Use、跨平台提供者安装、自动能力发现与公钥信任仍待实现。测试结果和交接入口见[功能与验证交接](implementation-handoff-2026-10-18.md)；原始日志保存在指定远端 /config/validation-artifacts/computer-use-20261018。
