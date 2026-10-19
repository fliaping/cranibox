# 交接说明 / Handoff (2026-10-19)

## 当前状态
- main：PR #7–#17 已合入。#16 于 2026-10-19 合并，Phase 0 共享契约 C1–C8 已可用；说明见 `docs/design/CONTRACTS.md`、`docs/design/OWNERS.md`。
- PR #17 `ws1/iroh-network` 已于 2026-10-19 合并，merge commit `a23858f`：iroh 默认网络、内置 relay、链接/二维码配对。集成评审补上双方独立确认的授权修复和两处空间测试同步修正；最终提交 `5a181c8` 的 7 项跨平台 CI 及 Release 检查全部通过。交接及剩余事项见 `docs/design/handoff/WS1-iroh.md`。
- 本总交接文档在 #16 合并后才提交，已随 #17 补入 main。
- 已有草稿 PR #19（Claude cch 实测结论）、#18（协议转换器与许可证）、#20（WS2/WS3 本机订阅代理）。另一个 Agent 正在原工作区完成 WS5/WS6/WS7，接手前先协调，勿覆盖其未提交改动。

## 合并顺序
1. #16、#17 已按顺序合入 main；新工作可直接基于 main 开始。
2. 其他草稿当前是 #19 → #18 → #20 的堆叠关系；依次评审、改目标分支、验证后再合，禁止跳过草稿审核。
3. 已有工作流先 merge main；新工作从 main 切 `wsN/...` 分支，按 `docs/design/tasks/NN-*.md` 执行。

## 待接手工作流
| WS | 任务书 | 状态 |
|---|---|---|
| WS1 iroh | tasks/01 | 核心已合并（#17），UI、名册接入等剩余清单见 WS1-iroh.md |
| WS2 订阅代理核心 | tasks/02 | 草稿 #18/#20，已有 Agent 负责 |
| WS3 Codex/Kimi/国内厂商 | tasks/03 | 草稿 #20，与 WS2 一起推进 |
| WS4 Claude 节点 | tasks/04 | 草稿 #19 提交首轮实测结论；详细规格仍私下提供，勿写入公开仓库 |
| WS5 Agent 适配（DSH 默认） | tasks/05 | 原工作区 Agent 进行中，尚未交付 |
| WS6 名册/同步/租约 | tasks/06 | 原工作区 Agent 进行中，尚未交付 |
| WS7 UI/UX | tasks/07 | 原工作区 Agent 进行中，尚未交付 |

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
