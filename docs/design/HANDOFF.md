# 交接说明 / Handoff (2026-10-19)

## 当前状态
- main：PR #7–#15 已合入（安全阶段 1–4、Apache-2.0、设计文档）。
- PR #16 `ws0/contracts`（未合并）：Phase 0 共享契约 C1–C8，`crates/cranibox-contracts` + 空 crate `cranibox-net` / `cranibox-llm` / `cranibox-agents`。CI 通过。说明见 `docs/design/CONTRACTS.md`、`docs/design/OWNERS.md`。
- PR #17 `ws1/iroh-network`（基于 #16，未合并）：iroh 默认网络、内置 relay、链接/二维码配对，核心已可用。交接见该分支 `docs/design/handoff/WS1-iroh.md`。

## 合并顺序
1. 合 #16（契约）→ 2. #17 改 base 为 main，CI 绿后合。
3. 之后各工作流从 main 切 `wsN/...` 分支，按 `docs/design/tasks/NN-*.md` 执行。

## 待接手工作流
| WS | 任务书 | 状态 |
|---|---|---|
| WS1 iroh | tasks/01 | 进行中（#17），剩余清单见 WS1-iroh.md |
| WS2 订阅代理核心 | tasks/02 | 未开始 |
| WS3 Codex/Kimi/国内厂商 | tasks/03 | 未开始（依赖 WS2 骨架） |
| WS4 Claude 节点 | tasks/04 | 未开始；详细规格私下另行提供，勿写入公开仓库 |
| WS5 Agent 适配（DSH 默认） | tasks/05 | 未开始 |
| WS6 名册/同步/租约 | tasks/06 | 未开始 |
| WS7 UI/UX | tasks/07 | 未开始（可先用 C6 mock） |

## 规则
- 一个工作流一个分支，小 PR，CI 绿再合，禁止 force-push / 改历史，合 main 用 merge 不用 rebase。
- 契约（`cranibox-contracts`）变更需单独 PR，并在 PR 中说明影响的工作流。
- 已知不稳定测试：`local_catalog_and_shell_survive_gateway_replacement`。

## 待产品负责人决定
- D1 Claude 先做请求模拟，不行再接真实 Claude Code（建议）。
- D2 常驻节点离线时自动接管令牌刷新（建议）。
- D3 已有 CLI 登录默认在 Cranibox 重新登录（建议）。
- D5 主密钥可选恢复助记词（建议）。
- WS1 提出的 4 个问题与 6 个契约变更，见 #17 / WS1-iroh.md。
