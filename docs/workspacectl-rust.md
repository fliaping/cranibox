# Rust workspacectl

本次迁移仅替换 Workspace 的控制命令 `workspacectl`。它是按需运行的 Rust
二进制，不增加常驻服务。现有 Control Center 的 `schema_version: 1` JSON 接口、
命令入口和持久化配置路径保持兼容。

## 范围

| 能力 | 实现 |
| --- | --- |
| 状态、Agent、Skills/MCP、服务、端口、路由、诊断 | Rust；外部探测并发执行、限时、限制输出；版本缓存按文件变化失效 |
| 服务和 s6、日志、Tailscale、浏览器与桌面控制 | Rust 命令分发；调用现有平台工具、浏览器启动器或桌面桥 |
| 语言、语言包、时区、Web 配置 | Rust；保留既有配置和语言包元数据 |
| Skills/MCP 管理 | Rust；调用 npx skills、MCPM、Agent 原生 CLI，原生生成 DeepSeek Harness 配置 |
| Caddy 路由配置 | Rust；互斥写入、备份、输入校验、失败恢复 |
| OpenClaw 更新控制 | Rust；暂存新版本、切换服务、验证进程和 HTTP，失败恢复旧服务定义 |
| 节点命令 | 转交既有 Rust workspace-runtime，保持命令参数和退出码 |
| 设备消息审批队列 | `messages held/release/reject/inbound` 校验参数后转交 `cranibox peer`（`CRANIBOX_NODE_BIN` 或 `$ROOT/bin/cranibox`），使用 `$ROOT/.config/cranibox` 中与控制中心相同的存储 |
| 软件下载安装、容器启动 | 继续使用 manager / Bash 安装器 |
| Web 控制中心 | 后端为 Rust，浏览器界面和 IDE 扩展保留 JavaScript |
| 桌面桥/MCP、用户服务监督器、第三方 Agent 和浏览器 | 保留现有实现 |

旧 `scripts/workspacectl-*.py` 暂时保留作为兼容参考和旧测试的被测实现，
正式 `workspacectl` 不再调用这些 Python 控制模块。manager 仍需要 Python，
浏览器启动器和系统服务仍需要各自平台工具。

## 构建与安装

构建需要 Rust 1.89 或更新版本；运行已编译的 CLI 不需要 Cargo 或 Python。
Linux Workspace 的安装器只安装控制二进制和兼容链接，不重启服务。

```bash
cargo build --locked -p agent-workspacectl --release
./target/release/workspacectl --help
bash scripts/install-workspacectl.sh

# 使用预编译的本机二进制安装
AGENT_WORKSPACE_CTL_BIN=/path/to/workspacectl bash scripts/install-workspacectl.sh

workspacectl status --json
workspacectl service restart demo
workspacectl nodes --json
```

安装位置默认 `/config/bin/workspacectl`；`/config/.local/bin/workspacectl` 链接到
它。`agent-workspace-manager install workspace-controls` 会自动构建和安装。
安装器按源码指纹跳过重复构建。源码入口 `scripts/workspacectl` 只是兼容启动器，
优先执行构建产物，也可用 `AGENT_WORKSPACE_CTL_BIN` 指定二进制。

根目录由 `AGENT_WORKSPACE_CONFIG_ROOT` 控制，Workspace 路径由
`AGENT_WORKSPACE_ROOT` 控制，部署源码由 `AGENT_WORKSPACE_SOURCE_DIR` 控制。
节点运行时可用 `AGENT_WORKSPACE_RUNTIME_BIN` 指定。

## 平台和资源边界

CI 配置 Linux、Windows、macOS 原生构建和 Rust 单元测试，并输出两个二进制。
Linux 本地控制操作另外有隔离集成测试。systemd、s6、KDE、容器时区等管理能力
属于 Linux Workspace；编译出 Windows/macOS CLI 不意味着这些 Linux 功能可用。
跨平台远程执行由 `workspace-runtime node` 承担，参见 [远程节点](rust-remote-nodes.md)。

迁移减少控制命令启动解释器及重复版本探测的开销；没有重写或关闭浏览器、IDE、
桌面及 Agent，不应据此推算整机内存降幅。需在同一负载下实测 CPU、内存和延迟。

## 验证

```bash
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked
cargo test --locked
# PyYAML 仅用于比较旧版 JSON 契约
python3 -m unittest discover -s tests -p test_workspacectl_rust.py -v
python3 -m unittest discover -s tests -p test_rust_runtime.py -v
```

CTL 测试使用临时配置目录、模拟平台命令和本机测试 HTTP 服务，覆盖 JSON 契约、
探测失败与超时隔离、缓存、参数透传、配置幂等、路由失败恢复、资源管理和二进制安装。
OpenClaw 单元测试验证更新失败时先恢复原始服务定义再尝试恢复进程。

2026-10-12 本机 Linux 验证：21 项 CTL 集成测试（使用 release 二进制）、
12 项节点集成测试、10 项 Rust 单元测试通过，格式和 Clippy 检查通过。
已安装到 `/config/bin/workspacectl`；二进制文件约 8.12 MiB。真实 Workspace
状态查询成功；本次首次查询约 5.28 秒，缓存版本信息后的查询约 0.16 秒。
这些是本机样本，文件大小不代表运行内存，查询速度取决于外部探测和缓存。
本次会话没有在 Windows/macOS 上执行测试；相应原生构建和测试已配置在 CI。
