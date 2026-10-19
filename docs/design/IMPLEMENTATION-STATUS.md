# 设计实现核对（2026-10-19）

本表逐项对照 [ARCHITECTURE](ARCHITECTURE.md)、[WORKPLAN](WORKPLAN.md)、[CONTRACTS](CONTRACTS.md) 和 [tasks/00–07](tasks/)。核对基线是 main 已有的 WS0/WS1、PR #19/#18、PR #20 的本机代理，以及 PR #22 的 Agent/Vault/个人界面；#20/#22 的最终合并状态以 GitHub 为准。原工作区尚未提交的修改不算交付。

**结论：已有核心模块和较多自动回归，但尚未达到 tasks/00 的完整端到端验收标准。** 下文的“已实现”指代码存在；mock、预置互信和真实账户验收分别标明，不能互相替代。

## 工作流对照

| 工作流 | 已实现及证据 | 缺少或尚未验收 |
|---|---|---|
| WS0 契约/集成 | #16 提供 C1–C8、8 个 fixture round-trip 测试、crate 骨架、挂载点、归属及 CI；本次补第三方 NOTICE | 各模块已能编译不等于产品闭环；C3 count_tokens/取消登录、WS1 契约请求仍需协调 |
| WS1 iroh/relay/配对 | #17：默认 iroh、内置 relay、实时 grant/route allowlist、流式 PeerTransport、15 分钟一次性票据、双方独立确认 SAS；见 [交接](handoff/WS1-iroh.md) | Web/Tauri App 尚未调用真实配对；OwnerDevice 名册/OwnerKey 接入缺失；Windows 配对控制 CLI 尚为 Unix socket；Cloudflare 实机验收缺失；CLI/MCP 仍有 Noise 内层，路由迁移默认 opt-in |
| WS2 模型代理 | #18/#20：loopback 3424、安全检查、UDS/Windows pipe 管理面、Chat/Responses/Messages 3×3 转换、SSE/工具/reasoning、首字节前账号故障切换、亲和与冷却、curl 解析；见 [实现说明](../../crates/cranibox-llm/tests/IMPLEMENTATION.md) | count_tokens 仅部分本机原生 Messages/API key 路径可用，OAuth/远端未统一；取消登录尚未贯通 C3 管理 API 与 UI；跨节点 Claude 选择仅有本机占位 |
| WS3 订阅/国内厂商 | #20：Codex PKCE+手动 callback、Kimi device code、Gemini CodeAssist、Copilot 短 token、刷新轮换、33 个 API key 预设；mock 协议回归齐备，见 [端点与边界](../../crates/cranibox-llm/tests/providers/ENDPOINTS.md) | 四家真实订阅账户及真实 API key 尚未验收；Gemini 需部署者 OAuth client/CodeAssist 条件；Windows 不同用户与 SMB 管理面拒绝待实机确认 |
| WS4 Claude 节点 | #19 仅交付首轮 cch 可行性实验：Claude Code 2.1.303 对本地 mock 上游的 5 次请求未含 cch；见 [实验结论](../../crates/cranibox-llm/tests/claude/CCH-FINDINGS.md)。Vault 已有 ClaudeOAuth never_sync 约束 | 未实现真实 Claude 订阅 adapter/CLI 桥、迁移与旧节点凭据清理、跨节点 claude/* 对话及离线错误闭环；mock 结果无法推导官方真实上游行为。本工作流仍后置 |
| WS5 Agent 适配 | #22：DSH 默认及固定六种 Agent 注册表；原生配置写入、备份、漂移检测、恢复/解绑、session/MCP；DSH 固定版本与 SHA 校验；Pi models 和 stdio MCP 扩展；六 CLI 经本地 mock 可对话 | Codex/Hermes/OpenClaw 的 approval capability 为 false，未接入统一审批；OpenClaw 直接改 openclaw.json，偏离任务书优先 config set 的要求；Pi 真 CLI 完整 MCP 工具回合待验 |
| WS6 名册/Vault/租约 | #22：Owner 签名及 epoch 防重放、HPKE 封装 OwnerKey helper、SQLite 每记录 XChaCha+HPKE、默认仅本机启用、租约 fencing/接管回切、撤销后 DEK 轮换、远端路由与健康探测；预置互信的三进程 TCP/Noise 转发与 Hub 退出回退已有回归 | WS1 配对尚未分发 OwnerKey/加入名册；Models 启动仍自行 bootstrap Owner，传输仍是 DirectTransport；未完成真实配对后的 iroh 全链路、网络分区/延迟、Hub 恢复 30 秒与流中断边界验收 |
| WS7 个人界面 | #22：Web/Tauri 五栏、状态胶囊、引导、模型/设备/高级页、中英/主题、HTTP cookie+CSRF/SSE 与 Tauri IPC；桌面 tray/通知代码；主流程移除旧连接表单 | 配对 App API 仍为 503/404 占位；首页 DSH 仅本机 iframe、远端禁用，缺服务健康检查；专用配对/能力/Computer Use/模型写审批未接齐；三 OS 原生通知/tray、真实七条用户路径待验 |

## 会阻断完整体验的连接点

1. **配对 UI → WS1 → 名册/Vault**：`workspace-node/src/app.rs` 的 `pair_invite`/`pair_accept` 返回 `pairing_unavailable`，`pair_confirm` 无真实流程。需要接入 `transport_iroh` 的已有 Handle，维持双方本地 SAS 确认，再完成 OwnerKey 安全分发、名册加入及 relay 授权同步。不能仅展示二维码就标完成。
2. **模型服务 → 真实配对关系**：`workspace-node/src/models.rs` 为新节点自建 Owner，并使用 `DirectTransport`；HPKE helper 和预置互信测试尚未构成首次配对产品流程。需验证断网、撤销、重入及不可重放的刷新租约。
3. **首页 DSH 与统一审批**：补远端 Hub 的 DSH 会话入口、运行健康判定；把能力/Computer Use/模型配置写入等专用事件及决策接入。现有 held message/Agent tool 审批和卡片展示不能代表所有来源已接通。

## 验证证据及限制

- WS1 最终头 `5a181c8` 的跨平台 7 项 CI 与 Release 通过，详见 [WS1 交接](handoff/WS1-iroh.md)。原 netns/TCP 443/95 秒空闲/抓包验收先于双方独立确认修复；修复后未能重跑 netns，不能把旧结果当成新头实测。
- WS2/WS3 的本机模拟上游回归及性能样本见 [IMPLEMENTATION](../../crates/cranibox-llm/tests/IMPLEMENTATION.md)；历史 120 样本附加延迟 p50 859 μs。该数字不是本次完整集成后的性能复测，也不是公网厂商时延。
- WS5–7 的原交付含六 CLI 假 key/mock 上游、Web/桌面浏览器测试与中英截图。配对截图/SAS 来自 mock，远端互信由测试预置，均不能视为真实设备首次配对验收。
- `tests/integration/vault_netns.sh` 需要网络 namespace 能力；跳过（77）不算通过。Cloudflare、真实订阅、Windows 不同用户/SMB、原生系统通知和 Pi 工具回合继续列为待人工验收。
- 本次集成解决 Cargo.lock/Cargo.toml 冲突，并修复 Windows 对只读句柄重复 fsync 的拒绝访问，以及 gateway 节点提前占用固定模型端口导致多节点启动失败。`write_private` 自带 sync，保留持久化语义；模型服务由 App/mesh 路由按需启动。最终 CI 请按 PR #20/#22 的最新提交核验，旧头失败不构成修复后的结果。

## 下一步优先级

| 优先级 | 工作 | 完成判据 |
|---|---|---|
| P0 | 真实首次配对、OwnerKey/名册/relay 接通 | 新设备扫码/链接、双方 SAS、重启后信任保留；Guest 与陌生节点拒绝；无地址/token 表单 |
| P0 | 远端 DSH 与审批来源补齐 | 浏览器可在 Hub 对话；各真实审批事件可批准/拒绝且决策到达执行端 |
| P1 | Vault/刷新/路由故障验收 | 真实 iroh 多节点、网络分区/延迟、Hub 下线本机兜底、恢复 30 秒回切、撤销后不可解密、流中断不重放 |
| P1 | 真实账户与系统验收 | 四家订阅/API key、Pi MCP 工具回合、Windows pipe 用户隔离、三 OS tray/通知、Cloudflare 按清单通过 |
| P2 | 契约与适配收尾 | count_tokens/取消登录贯通；OpenClaw config set 或经明确评审调整规格；WS1 待决策与 OwnerKey 恢复方案确定 |
| 后置 | WS4 Claude 节点 | 明确真实上游可行路线，再做登录/迁移/清理/跨节点对话/离线错误验收 |

不使用一个完成百分比混合代码量、自动回归与真实环境验收；后续更新本文件时应附对应提交与可复现证据。
