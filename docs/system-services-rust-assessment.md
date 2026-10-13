# 系统常驻服务与 Rust 迁移评估

检查日期：2026-10-12。依据当前容器的 `/run/service`、用户 service 定义、
实际进程和仓库启动脚本；不包含宿主机未暴露给容器的服务。
本节为迁移前检查记录。后续实现见 [Rust 服务与浏览器控制中心](native-services-and-web.md)，
桌面驱动替代结论见 [cua-driver 评估](cua-driver-assessment.md)。

## 结论

最值得继续改造的是项目自有的用户服务管理层 `scripts/user-systemctl.sh`。
建议由 Rust 实现兼容命令和状态管理，评估将监督职责交给现有 s6；
确需自行监督时，采用一个 Rust 常驻管理进程管理多个服务。
收益主要是进程生命周期管理、故障恢复和减少重复监督进程，
具体内存收益需要同负载实测。

现有基础设施继续使用现有实现。启动完成后仅休眠的任务优先改为
一次性启动任务，避免为语言迁移引入新的常驻进程。
Web 控制后端和桌面桥是另外两个 Rust 候选，需单独确定迁移范围；
此前已完成的 `workspacectl` 迁移不代表这些服务已经迁移。

## 当前管理架构

- PID 1 是 `s6-svscan`，当前 `/run/service` 有 22 个条目。
- `systemctl --user` 使用安装到 `/usr/local/bin/systemctl` 的 Bash 兼容脚本。
  系统级调用再委托 `/usr/bin/systemctl.py`，不能据此认为存在 systemd 常驻管理器。
- 用户目录有 17 个 service 定义、13 个启用链接。启用、PID 文件存在与实际运行
  是不同状态；检查中存在失效 PID 和另行启动的实例。
- 监督服务显示运行不代表对应功能已启动：多个服务当前只运行 `sleep infinity`。

## s6 管理的服务

| 服务 | 当前观察/职责 | 建议 |
| --- | --- | --- |
| s6-svscan、s6-supervise、s6-linux-init-shutdownd、s6rc-oneshot-runner | 容器监督、关闭及一次性任务设施 | 保留；本身已很轻 |
| s6rc-fdholder | 已注册，当前未运行 | 保留基础设施配置 |
| svc-docker | dockerd 正在运行 | 保留 Docker；不重写容器引擎 |
| svc-nginx | Nginx 主进程正在运行 | 保留；配置管理可由 Rust CTL 承担 |
| svc-dbus、svc-pulseaudio | D-Bus 和音频服务正在运行 | 保留 |
| svc-de | Bash 启动桌面，实际有 KDE/Wayland 桌面子进程 | 保留桌面组件，按需求选择是否启动 |
| svc-selkies | Python 远程桌面服务正在运行 | 保留上游；迁移桌面桥不会替代编码与桌面栈 |
| sshd、svc-cron、svc-xorg、svc-xsettingsd、svc-watchdog | 当前处于休眠或脚本等待分支 | 按功能启用条件整理，无需 Rust 重写 |
| init、custom-services、systemctl-services | 初始化、注册服务、启动用户服务；完成后休眠 | 改为一次性任务更合适 |
| deb-native-restore | 恢复持久化应用后运行 s6-pause | 低优先级；复用一次性任务设施 |
| xfce-panel-scale | 当前休眠；脚本有 XFCE 缩放轮询逻辑 | 当前桌面为 KDE，优先按桌面类型启用 |
| gateway-default | 外部 Hermes Python gateway 正在运行 | 第三方 Agent 服务，保留 |
| value-investing | 外部投资平台 Python 服务正在运行 | 独立业务项目，另行评估 |
| futu-opend | 当前休眠 | 外部集成，按使用情况整理 |

仓库还有 `services/workspace-bootstrap/run`，但当前 `/run/service` 中没有对应条目，
不能计入当前正在运行的服务。

## 项目自有常驻组件的迁移优先级

| 组件 | 当前实现 | 优先级与迁移边界 |
| --- | --- | --- |
| 用户服务管理/监督 | 519 行 Bash；当前 11 个独立监督进程 | 第一优先级；Rust 兼容接口配合 s6，或统一 Rust 管理进程 |
| agent-desktop-bridge | Python HTTP 服务；桥接输入、截图、窗口操作和急停 | 中；保留上游桌面服务，迁移协议及控制逻辑 |
| agent-workspace-web | Node.js HTTP 后端，调用共享 JS 动作逻辑 | 中；可并入 Rust runtime，保留 Web 前端 |
| 初始化与服务注册脚本 | Bash；主要在启动时工作 | 低；先调整任务生命周期 |
| XFCE 缩放监听 | Bash；启用时每 5 秒检测一次 | 低；当前没有执行轮询，先按平台条件启动 |

用户监督层需保留环境导入、ExecStart 解析、日志、服务状态、
Restart 策略、重启等待期间停止、进程树停止及配置更新行为。
不能简单将现有 Bash 循环逐服务翻译为 Rust 进程，否则仍然保留重复监督开销。

桌面桥需保留坐标转换、会话管理、急停及暂停持久化、Wayland/XWayland
窗口操作与诊断语义。Web 后端需保留会话、Host/Origin 检查、CSRF、
提示交互、日志和取消行为，且迁移范围包含其依赖的共享动作逻辑。

Rust WebSocket runtime 已有代码和构建产物，但当前未作为服务安装运行。
若以后统一 Web、节点接入和本地服务管理，应明确由哪个组件负责监督，
避免同时由 runtime 与 s6 重复拉起同一个服务。

## 占用快照与收益边界

以下是检查时的 Linux `/proc` 快照，单位 MiB；s6 监督器额外列出 KiB。
RSS 包含共享页，PSS 按比例分摊共享页，SwapPss 表示已换出的分摊页。
单个主进程的数据不包含其整个子进程树，也不是迁移前后对照实验。

| 进程/进程组 | RSS | PSS | SwapPss |
| --- | ---: | ---: | ---: |
| 11 个 Bash 用户服务监督进程合计 | 21.56 | 1.96 | 5.07 |
| agent-desktop-bridge 主进程 | 17.37 | 8.52 | 8.45 |
| agent-workspace-web 主进程 | 43.60 | 4.93 | 26.87 |
| dockerd 主进程 | 58.17 | 55.79 | 72.58 |
| Selkies 主进程 | 57.57 | 42.84 | 268.64 |
| Hermes gateway 主进程 | 351.57 | 334.64 | 56.87 |
| OpenClaw gateway 主进程 | 568.79 | 523.68 | 164.51 |
| value-investing 主进程 | 171.96 | 168.52 | 91.12 |

22 个 `s6-supervise` 合计 RSS 176 KiB、PSS 88 KiB；
`s6-svscan` 的 RSS 68 KiB、PSS 64 KiB。这部分无需以 Rust 替换来节省资源。

当前内存压力和换页会影响驻留数据。Bash 监督器的 21.56 MiB RSS
不能全部算成迁移后可回收的独占内存；新 Rust 进程也有自身开销。
主要占用仍来自 Agent、浏览器、桌面和业务应用，改写管理层不会消除这些占用。

## 用户服务中的其他常驻应用

`code-server`、`deepseek-harness`、`openclaw-gateway`、`orca-serve`、
`hermes-dashboard`、`ocx-proxy` 属于第三方工具或 Agent 集成；
`proxy-caddy` 与 `tailscaled-workspace` 是现有网络基础设施，建议保留。
`hermes-gateway` 的用户定义当前禁用，但 s6 的 `gateway-default` 有实际实例。
`nat-tunnel-server` 属于另一仓库，迁移前需核对其 TCP/UDP/SSH 等协议职责，
不能直接用 Workspace WebSocket 节点功能替代。
`audio-download` 是另装的小型 Python 服务，优先级低。
`shop-ops-experiment` 虽启用，当前没有对应活 PID 记录；
`FTNN`、`mihomo-test`、`proxy-sub-forward` 的用户定义当前禁用。

## 下一步建议

先确定用户服务管理接口和后端：优先复用 s6 监督，保留 `systemctl --user`
兼容调用，再由 Rust 接管服务配置和状态。若 s6 难以满足现有用户服务行为，
才实现统一 Rust 监督进程。跨平台客户端另接各平台的服务管理设施。

迁移验收覆盖异常退出自动重启、正常退出策略、等待期间停止、
完整进程树清理、环境传递、日志、容器重启和服务更新后的重启。
随后在相同服务数量与负载下比较 CPU、PSS/SwapPss、启动时间及进程数。

## 源码依据

- [用户服务管理器](../scripts/user-systemctl.sh)
- [用户服务启动集成](../services/systemctl-services/run)
- [初始化](../services/init/run)与[自定义服务注册](../services/custom-services/run)
- [XFCE 缩放监听](../scripts/watch-xfce-panel-scale.sh)
- [桌面桥](../addons/desktop-bridge/bin/agent-desktop-bridge)
- [Web 后端](../addons/control-center/server.cjs)
- [已完成的 Rust CTL 迁移](workspacectl-rust.md)
