# Cranibox 设计文档与分工

> 2026-10-19。基线：`fliaping/cranibox` `main@3dec00b`（等 #13/#14 合入后再开工 Phase 0）。
>
> **English:** Overall design, work plan, and per-workstream task briefs for the next Cranibox milestone: built-in iroh networking with an embedded relay, a personal-agent style UI for non-technical users, DeepSeek Harness as the default agent with adapters for other agent CLIs, and a shared model-subscription proxy that works across the user's own nodes.

| 文件 | 用途 |
|---|---|
| [ARCHITECTURE.md](./ARCHITECTURE.md) | 总体设计：目标/非目标、组件图、网络、安全、Agent、模型代理、UI/UX、存储、待决策 |
| [WORKPLAN.md](./WORKPLAN.md) | 分工：Phase 0 契约、8 条工作流、文件归属、分支/PR 约定、并行时间线 |
| [CONTRACTS.md](./CONTRACTS.md) | Phase 0 契约 C1–C8 在代码里的位置、实现方和协调者决定 |
| [OWNERS.md](./OWNERS.md) | 文件归属到文件级，以及 Phase 0 预建的挂载点 |
| [IMPLEMENTATION-STATUS.md](./IMPLEMENTATION-STATUS.md) | 2026-10-19 逐工作流实现、验证边界、缺口与下一步优先级 |
| [HANDOFF.md](./HANDOFF.md) | 当前集成状态及接手规则 |
| [tasks/](./tasks/) | 每条工作流一份任务书，可以直接交给 Codex、Claude Code 或其他编码 Agent |

## 任务书一览

| 文件 | 工作流 | 估算（人周） |
|---|---|---|
| [tasks/00-contracts-integration.md](./tasks/00-contracts-integration.md) | WS0 契约与集成（**协调者自留**） | Phase 0 ≈1，之后每周 ≈0.5 |
| [tasks/01-iroh-network.md](./tasks/01-iroh-network.md) | WS1 iroh + relay + 扫码配对 | 4.5 |
| [tasks/02-llm-core.md](./tasks/02-llm-core.md) | WS2 cranibox-llm 核心 | 6 |
| [tasks/03-subscriptions-cn.md](./tasks/03-subscriptions-cn.md) | WS3 订阅登录 + 国内厂商 | 3.5（+2.5） |
| [tasks/04-claude-node.md](./tasks/04-claude-node.md) | WS4 Claude 节点 | 3（+3 桥） |
| [tasks/05-agent-adapters.md](./tasks/05-agent-adapters.md) | WS5 Agent 适配 + DSH 默认 | 4 |
| [tasks/06-multi-node-vault.md](./tasks/06-multi-node-vault.md) | WS6 名册 / vault / 租约 / 兜底 | 7 |
| [tasks/07-ui-ux.md](./tasks/07-ui-ux.md) | WS7 个人 Agent 式 UI + App API | 5 |

## 实施状态

#13/#14 与 Phase 0（#16）已合并；WS1（#17）已合并。后续交付和未完成项以 [实现核对](./IMPLEMENTATION-STATUS.md) 和 [交接](./HANDOFF.md) 为准。上方设计与任务书保留目标规格，不能视作全部已验收。新工作从最新 main 切 `wsN/…` 分支。
