# Cranibox 资源基线

2026-10-13 已增加统一服务端测量，见 [新记录](validation/2026-10-13-resource-baseline.json)。
同样在临时目录中运行四个 SSE 消费者和 80 次轮询，单个服务端同时提供 Web 与节点网关，
RSS 为 12,716 KiB（12.42 MiB），独立 Linux 节点客户端为 5,356 KiB（5.23 MiB）。
新记录中的 Web/gateway RSS 是同一个 PID 的兼容字段，不能相加；该快照不包含
PTY 活跃负载，也不是内存峰值或跨平台指标。以下保留 2026-10-12 的分离进程历史基线。

测量日期：2026-10-12 09:51 UTC。环境为 Linux x86_64，内核
6.18.18.c1107-trim。使用包含当前改动的工作区和 release 二进制，
并非只使用报告中记录的 Git HEAD。原始数据、二进制大小及 SHA-256
见 [测量记录](validation/2026-10-12-resource-baseline.json)。

## 相同服务的监督开销

临时 HOME 中启动六个相同的 /bin/sleep 服务，先使用旧 Bash 管理器，
再通过原生迁移命令切换到 s6。业务进程单独统计；这里比较监督进程
的总 RSS，各组采样三次，间隔 0.3 秒，取总量中位数。

| 监督方式 | 监督进程数 | 总 RSS |
| --- | ---: | ---: |
| Bash，每服务一个监督器 | 6 | 16,464 KiB（16.08 MiB） |
| s6，一个 svscan 和六个 supervise | 7 | 612 KiB（0.60 MiB） |

这一场景下监督进程 RSS 减少 96.28%。workspace-services 完成管理操作后
退出，不是额外的常驻进程。业务服务的内存没有算作迁移收益。RSS 会重复
计算共享页，因此不是 PSS，也不等于容器实际减少的物理内存。短时空闲采样
没有观察到 CPU tick 增量，不能由此推断长期 CPU 开销为零。

## 状态和日志传输

实际启动隔离的 Rust Web、节点网关和 Linux 节点，在约 4.35 秒的同一个
日志任务中维持四个 SSE 消费者，同时发出 80 次完整轮询。

| 方式 | 接收字节 |
| --- | ---: |
| 80 次完整轮询响应 | 10,544,840 |
| 四条 SSE 流，包含初始同步 | 610,914 |

这一场景下响应正文传输量减少 94.21%。SSE 数值包含帧格式，双方均不计
HTTP 头和 TCP/TLS 开销。初始日志窗口为 128 KiB，测试使用合成状态和日志；
短日志或频繁重新连接时节省比例会不同。

另一个进程内测量直接序列化实际 Rust API 载荷：39 个合成服务、128 KiB
日志窗口、四个标签页和 20 条新增日志。完整轮询为 11,344,872 字节，增量
事件为 561,208 字节，减少 95.05%。这组结果只比较 JSON 载荷，不含网络帧。

真实服务采样中，Web 主进程 RSS 为 8,388 KiB（8.19 MiB），节点网关为
7,128 KiB（6.96 MiB），节点客户端为 6,388 KiB（6.24 MiB）。Web CPU 为
1.61%，包含同时处理轮询与 SSE 的负载，不能作为两种方案的独立 CPU 对比。
状态采集两次，缓存复用 90 次；这次最大采集耗时为 36 ms。
这些数值是本机短时测量，不是跨平台或生产负载的性能保证。

## 复现

需要 Linux、s6、Cargo 依赖及创建本地套接字的权限。受限环境中应在已授权
的沙箱外执行；缺少套接字权限时脚本明确记录网络测量跳过。

~~~bash
cargo build --release --locked
WORKSPACE_SERVICES_BIN=target/release/workspace-services \
WORKSPACE_WEB_BIN=target/release/workspace-web \
WORKSPACE_RUNTIME_BIN=target/release/workspace-runtime \
python3 scripts/measure-workspace-resources.py --report target/resource-baseline.json
~~~

脚本创建临时服务和配置，结束后清理；不迁移或重启现有用户服务。CI 在
Linux 上运行同一测量并上传 resource-baseline-linux 产物。

本次功能验证通过 29 项 Rust 测试、53 项 Python 集成测试和 15 项 Chromium
回归测试；额外的载荷测量通过显式运行的 ignored 测试完成。格式、Clippy、
前端严格类型构建和 Web/IDE 控制中心检查通过。浏览器覆盖保持 Chromium，
未增加 Firefox 或 WebKit。

## 运行环境状态

09:51 UTC 的资源测量完成时，十个旧监督器符合迁移条件，当时尚未批量切换。
同日后续按用户指定范围，将 agent-desktop-bridge、deepseek-harness、
openclaw-gateway、proxy-caddy、tailscaled-workspace 五项切换到 Rust 管理入口
和 s6，并修复 setsid 进程跟踪及失败回退后的停止处理；其余候选服务未迁移。
本报告的资源数值和二进制 SHA-256 对应测量时版本，不是更新后管理器的重新测量。

实际安装的 Web 仍需要重新安装才能包含本次新增页面和接口。当前容器切换记录
见 [实施说明](native-services-and-web.md)，五项服务验证见
[迁移记录](validation/2026-10-12-project-services-migration.json)。
