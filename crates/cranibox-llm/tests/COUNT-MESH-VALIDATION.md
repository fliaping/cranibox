# WS2 / WS6 计数集成验收

日期：2026-10-19。CONTRACT-REQUEST：请 WS0 / WS6 评审跨工作流的 RemoteRouter、Coordinated 和 node mesh 入口补丁；共享接口由 PR #23 单独评审，代理核心实现见 PR #24，WS5/6/7 基线为 PR #22 的 `3c33f35`。这些依赖完成评审后再合并。

## 行为

- RemoteRouter 将计数发送到既有 `/nodes/{node}/llm/v1/messages/count_tokens`，保留 provider/model 和 session，移除 stream，不使用普通生成端点。
- Coordinated 装饰器代理 `count_tokens`，计数不再因包装丢失 adapter 能力；刷新仍走既有协调器。
- mesh 入口将 Unsupported 保持为 501，路由器不把不支持某项能力的节点标为离线。否则原入口会转成 502，调用方最终得到 503。
- 复用已有身份、owner 名册、授权和 Noise 通道；不增加监听地址或管理 TCP 接口。保留远端 home_offline / claude_node_offline 的 C8 映射。
- 这里没有实现四家 OAuth 的原生计数，也没有变更 Claude adapter。

## 测试范围

测试全部在 `admin@192.168.217.6:22022` 的 `cranibox-validation` 容器、用户 abc 运行。源码目录 `/config/validation-sources/count-mesh-20261019`，构建目录 `/config/validation-build/ws2-completion`；切换集成源码时仅清理该私有 target 的工作区包。未在本机运行构建、服务或测试。上游和凭据均为 mock。

1. `tests/count_mesh.rs`：专用 mesh 路径、session / model / stream、响应内容；连续三次 501 不改变健康状态；错误协议和非对象 body 不进入 transport。
2. node 的现有三节点测试扩展：**同一进程中三个独立节点实例，通过真实 loopback TCP + Noise 交换**。C 无本地 secret 时经 Hub 计数；Hub 连续三次不支持计数时返回 501，仍为 Remote；启用 B 后停止 Hub，B 本地计数、C 回退到 B 计数均成功；捕获的 TCP 字节不出现计数内容、账号标签或凭据明文。
3. 三节点回归先验证旧错误映射失败（预期 501、实际 503），再验证修复。配对身份与授权在 fixture 中预置，此测试不等于真实设备配对验收，也不等于跨主机网络验收。

## 结果

| 检查 | 结果 |
|---|---|
| cargo fmt --all --check | 通过 |
| cargo clippy --workspace --locked --all-targets -- -D warnings | 通过 |
| cargo test --workspace --locked | **484 通过，0 失败，8 个显式忽略项** |
| 三节点回归（修复前 / 后） | 修复前预期失败：503 != 501；修复后通过 |

本次代码提交为 `2841e22`，之后的合并和提交只更新验收文档。日志：`count-mesh-final-fmt.log`、`count-mesh-final-clippy.log`、`count-mesh-final-tests.log`、`count-mesh-before.log`、`count-mesh-after.log`，均保存在上述远端源码目录。

8 个忽略项为已有显式测试入口：负载测量、延迟基准、公网 n0 relay、两项真实 EasyTier 测试、netns coordinator / worker fixture，以及 gio 桌面启动器。它们没有作为本次通过项计算，也未宣称已验收。

GitHub 跨平台 CI 随提交触发，状态以 PR 当前检查为准；本报告的通过结果是远端 Linux 容器测试。


## 仍需人工验收

- WS0 / WS6 审阅本请求，按依赖顺序集成 #23、#24、#22 和本补丁；不能用联合测试通过代替接口评审。
- 真实设备配对、重启、网络切换和真实厂商登录由协调者验收；不读取或自动导入现有真实凭据。
- Claude 真实订阅 / cch 验证按用户要求后置。
