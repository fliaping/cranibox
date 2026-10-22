# 远程节点（已下线的 Workspace 网关）

旧版 `workspace-runtime` WebSocket 网关、`runtime.json` 凭据、节点注册/设备组与
`workspacectl runtime` 已移除。所有设备（包括 Docker 工作区）运行同一个 Cranibox 节点，
彼此经 iroh 对等配对；Agent 通过 `cranibox nodes|exec|job|cancel|mcp`（或容器内的
`workspacectl nodes` / `workspacectl node exec|job|cancel|mcp`）访问已配对与同一身份的设备。

- 组网、relay、局域网发现与身份：见 [iroh 设备组网](iroh-network.md)。
- 旧网关登记的设备：在设备上安装新版 Cranibox，然后在「设备与网络」重新配对。
