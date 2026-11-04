# 远程节点（已下线的 Workspace 网关）

旧版 `workspace-runtime` WebSocket 网关、`runtime.json` 凭据、节点注册/设备组与
`workspacectl runtime` 已移除。所有设备（包括 Docker 工作区）运行同一个 Cranibox 节点，
彼此经 iroh 对等配对；Agent 通过 `cranibox nodes|exec|job|cancel|push|pull|mcp`（或容器内的
`workspacectl nodes` / `workspacectl node exec|job|cancel|push|pull|mcp`）访问已配对与同一身份的设备。

## 传文件和文件夹

```bash
cranibox push --node <device-id> ./dist /opt/app/dist          # 本机 → 设备，文件或整个文件夹
cranibox pull --node <device-id> /var/log/app ./app-logs       # 设备 → 本机
cranibox push --node <device-id> build.zip 'C:\Users\me\build.zip' --overwrite
```

- 目标路径必须是对方设备上的绝对路径；文件夹会连同缺少的上级目录一起创建。
- 按 4 MiB 分块传输，单个文件最大 4 GiB，整体用 SHA-256 校验；文件全部到达并校验通过后才替换，
  中途失败不会留下半个文件。已存在的文件默认不覆盖，需要 `--overwrite`。
- 符号链接不会跟随（跳过并在结果的 `skipped` 里列出）；推送时空文件夹不会创建，拉取时会。
- 需要对方「终端与命令」里的执行命令权限，和 `exec` 相同；每个文件都会记入对方的操作记录。
- MCP 工具：`workspace_push`、`workspace_pull`（参数 `node`、`local`、`remote`、`overwrite`）。
- 推送文件夹、拉取文件夹需要对方也是这个版本或更新；单个文件对旧版本照常可用。

- 组网、relay、局域网发现与身份：见 [iroh 设备组网](iroh-network.md)。
- 旧网关登记的设备：在设备上安装新版 Cranibox，然后在「设备与网络」重新配对。
