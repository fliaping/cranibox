# 交接说明 / Handoff (2026-10-19)

## 当前状态
- main：PR #7–#17 已合入。#16 于 2026-10-19 合并，Phase 0 共享契约 C1–C8 已可用；说明见 `docs/design/CONTRACTS.md`、`docs/design/OWNERS.md`。
- PR #17 `ws1/iroh-network` 已于 2026-10-19 合并，merge commit `a23858f`：iroh 默认网络、内置 relay、链接/二维码配对。集成评审补上双方独立确认的授权修复和两处空间测试同步修正；最终提交 `5a181c8` 的 7 项跨平台 CI 及 Release 检查全部通过。交接及剩余事项见 `docs/design/handoff/WS1-iroh.md`。
- 本总交接文档在 #16 合并后才提交，已随 #17 补入 main。
- #19（Claude cch 首轮 mock 实验）及 #18（第三方转换器与许可证）已合并。#20（WS2/WS3 本机代理）和 #22（WS5/WS6/WS7）正在本轮集成，最终状态以对应 PR 为准。#22 已合入 main 的 WS1 代码，补 Windows 持久化和多节点启动修复。
- 各工作流实际完成内容、真实链路缺口、证据边界及后续优先级见 [IMPLEMENTATION-STATUS.md](IMPLEMENTATION-STATUS.md)。设计规格不代表全部交付。
- 原工作区仍有未提交改动，本次评审使用独立 worktree；接手前检查状态，勿覆盖。

## 合并顺序
1. #16、#17 已按顺序合入 main；新工作可直接基于 main 开始。
2. 本轮按 #19 → #18 → #20 → #22 集成；每个 PR 改为 main 后处理冲突并核验最新头 CI，禁止跳过草稿审核。
3. 已有工作流先 merge main；新工作从 main 切 `wsN/...` 分支，按 `docs/design/tasks/NN-*.md` 执行。

## 待接手工作流
| WS | 任务书 | 状态 |
|---|---|---|
| WS1 iroh | tasks/01 | 核心已合并（#17），UI、名册接入等剩余清单见 WS1-iroh.md |
| WS2 订阅代理核心 | tasks/02 | #18 已合并，#20 核心与 mock 回归已交付；计数/取消管理接口待贯通 |
| WS3 Codex/Kimi/国内厂商 | tasks/03 | #20 四家协议与 33 预设已交付；真实账户待验收 |
| WS4 Claude 节点 | tasks/04 | #19 首轮 mock 结论已合并；真实订阅与跨节点未实现；详细规格不进公开仓库 |
| WS5 Agent 适配（DSH 默认） | tasks/05 | #22 六 Agent 配置/会话/MCP；部分审批与 Pi 真工具回合待完成 |
| WS6 名册/同步/租约 | tasks/06 | #22 加密与租约核心；首次配对、OwnerKey 分发和真实 iroh 故障验收待接通 |
| WS7 UI/UX | tasks/07 | #22 个人界面及 App API；配对仍占位，远端 DSH 与部分审批链路缺失 |

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
