# 交接说明 / Handoff (2026-10-20)

## 当前状态

- 本轮 PR 已全部合并。代码基线为 main 的 `0d4bc49`（2026-10-20）；#22 经 #25 集成并保留原提交，GitHub 已将两者标记为 MERGED。
- #16 提供 Phase 0 共享契约 C1–C8；#17 提供默认 iroh、内置 relay、链接/二维码配对与双方独立 SAS 确认，最终头 `5a181c8` 的 7 项检查通过。交接及剩余事项见 [WS1-iroh.md](handoff/WS1-iroh.md)。
- #19 提供 Claude cch 首轮 mock 实验；#18 完成第三方转换器与许可证；#23 扩展默认 Unsupported 的 count_tokens/取消登录兼容接口；#20 提供 WS2/WS3 本机代理，最终头 `eedcc29` 的 15 项检查通过，合并提交 `31fadaf`。
- #24 接通私有管理登录取消及独立 Messages 计数，最终头 `964a0cf` 的 7 项检查通过，合并提交 `fd1ed77`。界面取消按钮及四家 OAuth 原生计数仍缺。
- #22 提供 WS5/WS6/WS7，最终头 `7ac8c99` 的 15 项检查通过。集成修复包括 Windows 节点栈溢出、持久化、配置快照、原生扩展冷启动，以及多节点/Web 端口和终端重连的测试同步。
- #25 补 mesh 计数及 501 透传，包含 #22/#24 的完整集成；修复三节点测试中 Hub 凭据更新与启用列表变更的同步顺序。最终头 `60d3443` 的 15 项检查通过，合并后完整文件树与该头一致。
- 各工作流实际完成内容、真实链路缺口、证据边界及后续优先级见 [IMPLEMENTATION-STATUS.md](IMPLEMENTATION-STATUS.md)。设计规格不代表全部交付。
- 原工作区仍有未提交改动，本次评审使用独立 worktree；接手前检查状态，勿覆盖。

## 合并顺序

1. #16、#17 已按顺序合入 main。
2. 本轮依次合并 #19 → #18 → #23 → #20 → #24 → #25；#22 原头在 #25 中保留并完成集成后随之合入。各 PR 均在最终头检查通过后合并。
3. 已有工作流先 merge main；新工作从 main 切 `wsN/...` 分支，按 `docs/design/tasks/NN-*.md` 执行。

## 待接手工作流
| WS | 任务书 | 状态 |
|---|---|---|
| WS1 iroh | tasks/01 | 核心已合并（#17），UI、名册接入等剩余清单见 WS1-iroh.md |
| WS2 订阅代理核心 | tasks/02 | #20 核心与 mock 回归、#24 私管取消和独立计数、#25 mesh 计数；界面取消与四家 OAuth 原生计数仍缺 |
| WS3 Codex/Kimi/国内厂商 | tasks/03 | #20 四家协议与 33 预设已交付；真实账户待验收 |
| WS4 Claude 节点 | tasks/04 | #19 首轮 mock 结论已合并；真实订阅与跨节点未实现；详细规格不进公开仓库 |
| WS5 Agent 适配（DSH 默认） | tasks/05 | #22 六 Agent 配置/会话/MCP；部分审批与 Pi 真工具回合待完成 |
| WS6 名册/同步/租约 | tasks/06 | #22 加密与租约核心；首次配对、OwnerKey 分发和真实 iroh 故障验收待接通 |
| WS7 UI/UX | tasks/07 | #22 个人界面及 App API；配对仍占位，远端 DSH 与部分审批链路缺失 |

## 规则
- 一个工作流一个分支，小 PR，CI 绿再合，禁止 force-push / 改历史，合 main 用 merge 不用 rebase。
- 契约（`cranibox-contracts`）变更需单独 PR，并在 PR 中说明影响的工作流。
- 本轮已修复 `local_catalog_and_shell_survive_gateway_replacement` 的关闭后身份锁释放问题，并强化为原节点对象仍存活时可重新打开；临时端口分配和 Windows 配置快照也已修复，验证边界见实现核对表。

## 待产品负责人决定
- D1 Claude 先做请求模拟，不行再接真实 Claude Code（建议）。
- D2 常驻节点离线时自动接管令牌刷新（建议）。
- D3 已有 CLI 登录默认在 Cranibox 重新登录（建议）。
- D5 主密钥可选恢复助记词（建议）。
- WS1 提出的 4 个问题与 6 个契约变更，见 #17 / WS1-iroh.md。
