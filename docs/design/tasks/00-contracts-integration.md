# WS0 任务书：Phase 0 契约、集成与评审（协调者自留）

> 估算：Phase 0 约 1 人周，之后每周约 0.5 · 建议执行者：协调者 · 分支前缀：`ws0/`
> 这份任务书可以直接贴给编码 Agent。它在 `fliaping/cranibox` 仓库里工作。

## 背景（必读）
- 仓库：`github.com/fliaping/cranibox`（Rust workspace + React + Tauri）。Cranibox 是一个自托管、高可信的 Agent Brain：节点是「脑壳」，Agent（DeepSeek Harness/DSH，以及 Codex、Claude Code、Pi、Hermes、OpenClaw）是「大脑」。它提供设备互联（端到端加密）、人工审批、Computer Use 基础设施和统一的模型订阅代理。
- 用户面向**不懂编程的普通人**，体验对标 Meta Muse、Grok Bot、Manus Cue、OpenAI Dots 这类 Personal Agent。客户端**没有「连接设置」**，配对用扫码或链接，审批用卡片。终端不是重点。高级功能收在「高级」里。
- 只有**一台固定的常驻节点（Hub，Linux 上的 `cranibox serve`）**。机器之间的数据一律 E2E 加密。
- 总体设计在仓库的 [`docs/design/ARCHITECTURE.md`](../ARCHITECTURE.md)，分工和契约在 [`docs/design/WORKPLAN.md`](../WORKPLAN.md)。**先读这两份文档里与你相关的章节，再读本任务书。**
- 共享契约在 `crates/cranibox-contracts`（Phase 0 已合入）。**不要自行修改契约。** 需要改时，在 PR 里写 `CONTRACT-REQUEST: ...` 并通知协调者。

## PR 规则（必须遵守）
1. 从最新的 `main` 切分支，分支名用本任务书指定的前缀。一个任务可以拆成多个小 PR，每个 diff 尽量不超过 600 行；拷贝的第三方代码单独提一个 PR。
2. PR 标题写 `[WSx] <简述>`。描述里写：做了什么、是否涉及契约、怎么测的、风险，以及需要人工验收的步骤。
3. 合并前 CI 必须全绿：`cargo fmt --all --check`、`cargo clippy --workspace --locked --all-targets -- -D warnings`、`cargo test --locked`；改了 UI 还要跑 `node scripts/build-control-center-ui.cjs && git diff --exit-code` 和 Playwright。
4. **禁止 force-push、rebase 或 amend 已推送的提交。** 跟进 main 时用 `git merge origin/main`。`Cargo.lock` 冲突时，先合并再运行 `cargo update -w`，不要手改。
5. **只改「文件」一节里列出的路径。** 别的工作流拥有的文件（见 WORKPLAN §2）一律不动，需要时向协调者提需求。
6. 不要把真实凭据写进仓库、测试或日志。测试用 mock 上游和假 key。
7. 移植或拷贝第三方代码时，保留原版权头，或加上 `Ported from <project> <path>@<commit> (<license>)`，并在 PR 描述里列出，由协调者更新 `NOTICE`。
8. 每个 PR 都要协调者评审。不要自己合并到 main。

## 目标
1. 在所有工作流开工前合入 Phase 0（WORKPLAN §1），把共享契约 C1–C8、骨架 crate、热点文件改动和 mock 一次性定下来。
2. 之后负责每周集成、评审、`NOTICE`、文档和人工验收，并请用户拍板待决策项 D1–D8。

## 前置条件
- PR #13（`feat/verified-by-default`）和 #14（`feat/session-discovery`）已经合入 `main`。

## 范围
- 新建 `crates/cranibox-contracts`，按 WORKPLAN §1 的 C1–C8 写类型、trait、常量和 fixtures（`crates/cranibox-contracts/fixtures/*.json`）；mock 放在 `mock` feature 里（`MockRoster`、`MockLlmMgmt`、`MockAppApi`、`MemoryAccountStore`）。
- 新建三个空 crate `cranibox-net`、`cranibox-llm`、`cranibox-agents`（`lib.rs` + smoke test），加进根 `Cargo.toml` 的 `members`。
- `workspace-runtime/src/peers/config.rs`：`Permissions` 加 `llm`、`vault`；`Endpoint` 加 `iroh: Option<IrohAddr>`，都用 `serde(default)`，并补 v2 文件的 round-trip 测试。
- `workspace-node/src/peer/server.rs`：抽出 `pub fn mesh_router(app: App) -> Router`；挂上 `/nodes/{node}/llm` 和 `/nodes/{node}/vault` 两个占位子路由（新文件 `server/llm.rs`、`server/vault.rs`，先返回 501）；`auth` 中间件优先读请求扩展 `Authenticated`（来自 iroh 传输），没有时走现有的 bearer 或 Noise 逻辑。
- 在 `workspace-runtime` 里给现有 HTTP 和 Noise 路由实现 `PeerTransport`，只做薄封装，不改变行为。
- `ARCHITECTURE.md` 和 `WORKPLAN.md` 已在 `docs/design/`；新建 `docs/design/OWNERS.md`。
- CI 加契约快照测试：fixtures 能与 Rust 类型互相 round-trip。

## 不在范围内
- 任何业务逻辑：iroh、代理、vault、UI、adapter 都不写。

## 每周集成
- 维护分支 `integration/weekly`：每周从 main 合并一次，在 netns 环境里起 Hub + 2 个节点（脚本放在 `tests/integration/`），跑当周的端到端清单。

## 端到端验收清单（第 8 周）
1. 新的 Mac 客户端首次打开，扫描 Hub 的二维码，核对 SAS，完成配对。全程没有任何地址或 Token 输入框。
2. 添加一个 API key（DeepSeek）和一个 Codex 订阅，DSH 首页可以对话，模型下拉框能切换。
3. Codex、Claude Code、Pi、Hermes、OpenClaw 由 Cranibox 接管模型配置后都能对话；断开后原配置逐字节还原。
4. 停掉 Hub：状态胶囊变黄，Agent 不改配置照常对话（用的是已启用的本机账号）；Hub 恢复后 30 s 内回切。
5. 在 Claude 节点上登录 Claude，另一台节点的 Pi 用 `claude/*` 模型对话；Claude 节点离线时，返回 503 `claude_node_offline`，UI 上有说明。
6. 访客设备只拿到 messages 能力；访问 llm 和 vault 返回 403。
7. 只开放 443 的网络下，经内置 relay 互通；在 Cloudflare Tunnel 后面互通（人工验收）。
8. 跨设备执行命令、Computer Use 首次使用都会弹审批卡；拒绝后不会执行。
9. pcap：线上没有 key、token 或正文。

## 验收标准
- Phase 0 PR 合入，CI 全绿，现有测试不回归；每条工作流的负责人都确认过契约。

## 文件
`crates/cranibox-contracts/**`、`crates/cranibox-{net,llm,agents}/`（骨架）、根 `Cargo.toml`、`.github/workflows/rust-runtime.yml`、`crates/workspace-runtime/src/peers/config.rs`、`crates/workspace-node/src/peer/server.rs`、`crates/workspace-node/src/peer/server/{llm,vault}.rs`（占位）、`docs/design/**`。
