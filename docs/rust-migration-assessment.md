# 现有服务与 Rust 迁移评估

检查日期：2026-10-12。依据本机服务定义、进程、监听端口及仓库源码。
以下服务与内存数据是迁移前的检查快照，当时仅检查并记录。

后续已按用户确定的范围，仅迁移 `workspacectl` 到 Rust，保留 Web 后端、桌面桥、
manager 和第三方服务。[实际迁移与验证](workspacectl-rust.md)记录当前实现。
以下建议是当时的候选方案，不代表本次要迁移所有服务。

## 判断

值得把项目自有的控制后端迁入 Rust：本地状态采集、Web API、作业管理、节点调度。
主要收益是统一接口、减少重复启动解释器、方便跨平台分发和改善故障隔离；
不能把第三方 Agent、浏览器和桌面的占用算成这次迁移可以消除的占用。
不建议重写所有第三方服务，也不建议为了语言一致性优先重写启动脚本。

## 项目自有组件

| 组件 | 当前实现/职责 | 运行方式 | 建议 |
| --- | --- | --- | --- |
| Rust runtime | Rust；HTTP/WS、节点连接、远程作业、CLI/MCP | 二进制已构建，本机未安装服务、未监听 9090 | 作为后续控制核心 |
| Web 控制中心 | Node.js；9080；会话、CSRF、状态缓存、动作、提示与日志 | 常驻 agent-workspace-web.service | 高优先级迁移后端；保留 JS/CSS 前端 |
| workspacectl 状态采集 | Python；服务、端口、路由、Agent 和资源的 JSON 快照 | 按需子进程，Web 更新快照时调用 | 高优先级；做 Rust 原生采集与缓存 |
| 本地管理动作 | Bash/Python；服务、网络、代理配置、浏览器、语言、时区、Skills/MCP | 按需命令 | 分批迁移；先明确输入、输出及兼容测试 |
| 用户服务管理器 | 519 行 Bash；PID、停止、重启、环境传递、Restart 策略 | 每服务可有常驻 Bash 监督进程 | 中优先级；可做 Linux 适配，但继续使用 s6 或宿主原生管理器 |
| 桌面桥与桌面 MCP | Python；截图/输入转发、坐标转换、会话、急停、窗口操作 | 桥常驻；MCP 随 Agent 按需启动 | 中优先级；先迁移协议与急停层，保留现有桌面上游 |
| manager / 安装更新 | Python/Bash；下载、安装、更新、配置、扩展注册 | 安装时运行 | 低优先级；分发简化有价值，日常内存收益有限 |
| 启动、s6 注册、Deb 恢复 | Bash 等；启动集成、恢复持久化应用 | 启动时或轻量等待 | 低优先级；保留平台启动适配即可 |
| VS Code 扩展、Web UI | JavaScript/CSS；呈现界面、调用控制命令 | 依托 IDE/浏览器 | 保留；迁移其调用的后端 |

状态采集实测连续两次约 5 秒后失败，安装入口与仓库入口均报告：
`[Errno 8] Exec format error: '/config/.npm-global/bin/claude'`。
这项迁移前检查没有修复 Claude 文件本身。后续 Rust CTL 已增加单项失败隔离：
一个 Agent 探测失败返回该项错误，其他状态仍可返回；真实 Workspace 快照已验证成功。

## 当前环境内的其他服务

本机有 17 个用户 service 定义，其中 13 个启用。启用不等于正在运行，
PID 文件缺失也不等于没有进程：Hermes/OpenCodex 存在另行启动的实例。

| 定义 | 当前观察 | 是否属于本项目迁移范围 |
| --- | --- | --- |
| agent-workspace-web | 运行，9080 | 是 |
| agent-desktop-bridge | 运行，8766（当前配置） | 是 |
| code-server | 运行，8443 | 集成的第三方 IDE；保留 |
| deepseek-harness | 运行，3080/3081 | 第三方 Agent；保留，管理适配可迁移 |
| openclaw-gateway | 运行，18789 | 第三方 Agent；保留 |
| orca-serve | 运行，6768 等 | 第三方开发工具；保留 |
| hermes-dashboard | 启用；观察到 dashboard 进程，9119 | 第三方 Agent UI；保留 |
| hermes-gateway | 定义禁用；另有 gateway 进程，8644 | 第三方 Agent；保留 |
| ocx-proxy | 启用；另有 OpenCodex/Bun 进程，10100 等 | 外部代理工具；按业务需要优化，不作为 Workspace 整体迁移项 |
| proxy-caddy | 运行，80/2019 | 第三方反向代理；保留，迁移配置管理层 |
| tailscaled-workspace | 运行，1055 等 | 第三方网络工具；保留 |
| nat-tunnel-server | 运行，18081/2222/30000/8765 | 另装隧道服务；先厘清与 WS 节点的职责，暂不重写 |
| audio-download | 运行，9127 | 另装下载服务；体量小，低优先级 |
| shop-ops-experiment | 启用，未发现对应活 PID 记录 | 独立业务任务；不作为 Workspace 核心迁移项 |
| FTNN | 禁用 | 另装应用 |
| mihomo-test | 禁用 | 独立测试服务 |
| proxy-sub-forward | 禁用 | 独立转发任务 |

s6 中还注册了 Selkies、桌面/Xorg、音频、D-Bus、Nginx、SSH、Docker、cron、watchdog，
以及自定义服务注册、用户服务启动、Deb 恢复、gateway-default、futu-opend、
value-investing、xfce-panel-scale 等。注册列表包含基础设施和启动任务，
不能把每个条目都算作需要重写的应用服务。
投资平台 8768 等独立业务应用也不属于本仓库控制核心。

## 内存快照与收益边界

以下为本次检查的主进程快照，单位 MiB。RSS 含共享驻留页；PSS 分摊共享页，
SwapPss 为已换出的分摊页。三列不可直接当作独占常驻内存；
启动不久的 Rust 空闲进程与运行很久的已换出进程也不是严格对照实验。

| 主进程 | RSS | PSS | SwapPss |
| --- | ---: | ---: | ---: |
| Web 控制中心 | 43.5 | 4.7 | 26.9 |
| 桌面桥 | 12.5 | 4.7 | 11.7 |
| OpenClaw | 571.4 | 526.1 | 164.5 |
| Hermes gateway | 346.5 | 329.6 | 61.5 |
| OpenCodex proxy | 377.5 | 353.2 | 46.7 |
| Orca 主进程 | 199.8 | 168.1 | 46.6 |
| Selkies | 58.1 | 43.5 | 268.7 |
| code-server 监听进程 | 18.8 | 12.0 | 110.7 |

另外，采样到 29 个 Chromium 进程，RSS 合计约 1939 MiB，PSS 合计约 1064 MiB；
Orca 子进程树包含工具、Agent、MCP 等，不能把全部子进程归为 Orca 自身固定成本。
不同进程组可能重叠，不能将各组数字简单相加。

自有 Web 后端与桌面桥只有几十 MiB 量级的优化空间，收益需要相同负载下实测。
要显著降低整机占用，应同时检查不需要的 Agent 常驻实例、重复 MCP、浏览器标签页、
桌面与 headless 运行模式。是否关闭这些实例应按实际工作需求决定。

## 推荐迁移顺序与验收

1. 本地状态采集与基础管理 API：固定既有 JSON schema，隔离超时/不可执行/缺失组件错误，
   避免频繁重复外部探测；保留兼容 workspacectl 入口。
2. Web 控制后端并入 Rust runtime：复用作业管理；迁移会话、CSRF、Host/Origin、
   提示交互、日志、取消与状态事件，保留浏览器和 Agent 两套鉴权边界。
3. 本地与远程统一调度：让 Agent 使用一致的发现/执行/查询接口，明确本地与各节点的
   平台能力；不能把现有 Linux 桌面能力直接视为 Windows/macOS 已支持。
4. 桌面桥/MCP与用户服务适配：保留急停、暂停持久化、坐标和窗口操作语义；
   Linux 对接 s6/systemd，macOS/Windows 使用各自服务设施。
5. 安装与更新逻辑：最后处理下载、原子更新、回滚、配置兼容和旧实现移除。

每一步都应比较相同机器、相同 UI/节点数量、相同探测和命令负载下的延迟、
CPU、PSS/SwapPss、进程数及启动时间，并通过行为兼容测试后再替换现有服务。

## 源码定位

- Web 服务：addons/control-center/server.cjs；共享 UI/动作：extensions/control-center/control-center.js。
- 状态采集：scripts/workspacectl-json.py；控制入口：scripts/workspacectl。
- 桌面：addons/desktop-bridge/bin/agent-desktop-bridge、agent-desktop-mcp。
- 用户服务：scripts/user-systemctl.sh、services/systemctl-services/run。
- 安装更新：scripts/agent-workspace-manager；代理配置：scripts/workspacectl-proxy.py。
- Rust 核心：crates/workspace-runtime/src。
