# WS4 任务书：Claude 节点

> 估算：3 人周（本机 Claude Code 桥另加 3 人周，见 D1） · 建议执行者：Claude Code（真实账号验收由协调者执行） · 分支前缀：`ws4/`
>
> **这是公开简版。** 详细规格由协调者单独提供给本工作流的负责人，不放进仓库。

## 背景（必读）
- 仓库：`github.com/fliaping/cranibox`（Rust workspace + React + Tauri）。Cranibox 是一个自托管、高可信的 Agent Brain：节点是「脑壳」，Agent（DeepSeek Harness/DSH，以及 Codex、Claude Code、Pi、Hermes、OpenClaw）是「大脑」。它提供设备互联（端到端加密）、人工审批、Computer Use 基础设施和统一的模型订阅代理。
- 总体设计在仓库的 [`docs/design/ARCHITECTURE.md`](../ARCHITECTURE.md)（§7.3），分工和契约在 [`docs/design/WORKPLAN.md`](../WORKPLAN.md)。**先读这两份文档里与你相关的章节，再读本任务书。**
- 共享契约在 `crates/cranibox-contracts`（Phase 0 已合入）。**不要自行修改契约。** 需要改时，在 PR 里写 `CONTRACT-REQUEST: ...` 并通知协调者。
- PR 规则与其他任务书相同（见 [WORKPLAN §3](../WORKPLAN.md)）：从 `main` 切 `ws4/…` 分支、小步 PR、CI 全绿、禁止 force-push/rebase/amend、不提交真实凭据、不自己合并到 main。

## 目标
Claude 订阅只在一台指定的 **Claude 节点**上运行，**凭据永不同步**；其他节点把 `claude/*` 请求转发给它（转发由 WS6 实现）；兼容性细节单独维护，不在公开文档中描述。

## 范围（公开部分）
1. **登录与本机刷新**：只在 Claude 节点上完成；凭据类型 `ClaudeOAuth`，带 `never_sync`，只写本机 vault，不为其他节点生成密钥包裹。
2. **上游适配**：实现 C3 `Upstream`（provider=`claude`）和 `SignIn`，输入为 Messages 形态（协议转换由 WS2 提供）。
3. **额度状态**：把账号状态（可用 / 需重新登录 / 额度用完）上报给管理 API，供 UI 显示。
4. **设为 Claude 节点**：管理接口 `POST /claude/set-node`：在新节点重新登录，旧节点删除凭据，不迁移 token。
5. **可行性结论（第 1–2 周）**：交付结论文档，协调者据此决定 D1。
6. **可选：本机 Claude Code 桥**（D1=B，或 HTTP 方式不可行时）：在 Claude 节点上调用本机官方 `claude` CLI 处理请求。

## 不在范围内
远端转发与健康探测（WS6）；协议转换主干（WS2）；UI（WS7，你只提供账号状态和「设为 Claude 节点」管理接口）。

## 文件
`crates/cranibox-llm/src/upstream/claude/**`、`upstream/mod.rs` 注册表中的一行、`crates/cranibox-llm/tests/claude/**`。

## 验收标准
- [ ] 可行性结论已交付。
- [ ] `never_sync`：vault 同步日志里看不到 Claude 凭据；其他节点的本地 vault 里没有该账号。
- [ ] 其他节点经转发可以使用 `claude/*` 模型；Claude 节点离线时返回 503 `claude_node_offline`。
- [ ] 「设为 Claude 节点」后，旧节点上不再有凭据。
- [ ] 人工验收：真实账号经 Codex、Pi、DSH 各跑一轮（协调者执行）。

## 怎么测
- `cargo test -p cranibox-llm claude::`（用 mock 上游和假凭据）
