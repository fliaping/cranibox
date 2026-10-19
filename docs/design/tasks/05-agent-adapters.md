# WS5 任务书：Agent 适配层 cranibox-agents + DSH 默认

> 估算：4 人周 · 建议执行者：Codex · 分支前缀：`ws5/`

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
1. 新建 `cranibox-agents`，用统一 `AgentAdapter` trait 管理 6 个 Agent：可逆写入模型配置、无头启动、会话发现、注册 Cranibox MCP。
2. **默认 Agent 改为 DeepSeek Harness（DSH）**；安装时**锁定已验证版本**（不用 `@latest`）。
3. 统一 Agent 目录：去掉 `inspect.rs` / `actions.rs` 两处写死的列表；修默认值不一致（代码默认 `codex`、README 写 `none` → 统一 `deepseek-harness`）。
4. **新接入 Pi**；OpenClaw 补进 MCP 列表。

## Adapter 契约（C4，详见 ARCHITECTURE §6.1）
实现 `AgentAdapter`：`id` / `display` / `detect` / `install_plan` / `wire` / `unwire` / `drift` / `headless` / `sessions` / `register_mcp` / `caps`。
注册表：`cranibox_agents::registry()`，顺序固定：DSH、Codex、Claude Code、Pi、Hermes、OpenClaw。

## 各 Agent 写哪些配置（ARCHITECTURE §6.2）
| Agent | 文件 | 写入的键 |
|---|---|---|
| DSH | `~/.dsh/profiles/*/cordis.patch.yml` + `~/.dsh/.env` + key store | `llm-pi-ai.config.providers.{cranibox, cranibox-responses, cranibox-anthropic}`，含 `{displayName, apiKeyEnv: CRANIBOX_GATEWAY_KEY, api, baseURL, models[]}`；另写 `agent-default-model` |
| Codex | `~/.codex/config.toml` + `cranibox-models.json` | `model_provider`、`model`、`model_catalog_json`、`[model_providers.cranibox]{name, base_url, wire_api="responses", experimental_bearer_token}`（`model_provider` 与该表必须同时写、同时删） |
| Claude Code | `~/.claude/settings.json` | `env.ANTHROPIC_BASE_URL`、`ANTHROPIC_AUTH_TOKEN`、`ANTHROPIC_DEFAULT_{OPUS,SONNET,HAIKU}_MODEL`、`ANTHROPIC_SMALL_FAST_MODEL`、`CLAUDE_CODE_SUBAGENT_MODEL`、`model`（不写 `ANTHROPIC_MODEL`） |
| Pi | `~/.pi/agent/models.json` + `settings.json` | `providers.cranibox{name, baseUrl, api, apiKey, compat.sendSessionAffinityHeaders, models[]}`；`defaultProvider`、`defaultModel` |
| Hermes | `~/.hermes/config.yaml` 及 `profiles/*` | `providers.cranibox{...}`；`model.provider`、`model.default`；保留用户自有字段 |
| OpenClaw | 优先 `openclaw config set`；退回 JSON5 | `models.mode=merge`、`models.providers.cranibox{...}`、`agents.defaults.model.primary`；清理 `~/.openclaw/agents/main/agent/models.json` 里我们写的那一项 |

统一参数：地址 `http://127.0.0.1:3424`，provider id `cranibox`，模型 id `<provider>/<model>`，本机 key `cbx-<agent>`。

## 可逆写入（借鉴 magpie `applied.go`）
1. 首次写入前逐字节备份为 `*.before-cranibox`。
2. `applied.json` 记录：写入的键、被覆盖的旧值、写后哈希。
3. `unwire`：文件哈希未变 → 逐字节还原；已变 → 只精确删除我们的键。
4. 启动 `drift`：只提示不强写。

## 范围
1. `crates/cranibox-agents`：6 个 adapter + `edit/{toml,yaml,json5}` 编辑器（YAML 保留注释）+ `applied.rs`。
2. `addons/deepseek-harness/install.sh`：把 `@latest` 改为锁定版本（版本号由你验证后定，写入常量；下载后 SHA-256 校验）；文档说明如何升级。
3. 改 `workspacectl/src/inspect.rs`、`workspace-web/src/actions.rs`：`AGENTS` / `MCP_AGENTS` 改为从 `registry()` 生成；默认 Agent = `deepseek-harness`。
4. 改 `workspace-runtime/src/peers/executions.rs` 与 `executions/mcp.rs`：去掉 Codex/ClaudeCode 硬编码 match，改为调用 `adapter.headless()`；MCP schema 列出全部 6 个。
5. 改 `peers/discovery.rs`：会话根目录改为 `adapter.sessions()`。
6. 给 6 个 Agent 注册 Cranibox MCP（computer / browser / devices / approvals）——`register_mcp`；审批经 DSH `dsh-user-approval` 或各自 hooks 接到审批卡（卡片渲染由 WS7，你提供事件）。
7. 契约测试：每个 adapter 的 wire/unwire/drift golden 文件快照。

## 不在范围内
llm 服务本身（WS2）；UI 模型下拉框（WS7，消费你的 `GET /agents`）；跨节点 Agent 执行的网络路径（已有 mesh，你只换启动方式）。

## 文件
`crates/cranibox-agents/**`、`addons/deepseek-harness/**`、`crates/workspacectl/src/inspect.rs`、`crates/workspace-web/src/actions.rs`、`crates/workspace-runtime/src/peers/{executions.rs,executions/mcp.rs,discovery.rs}`、相关 README 默认值表述。

## 契约
实现 C4；消费 C3 `LocalEndpoint` / `ModelPick` / `ModelEntry`。

## 步骤
1. trait + 编辑器 + `applied`；先做 DSH 与 Codex。
2. 改 install.sh 锁定版本；统一目录与默认值。
3. Claude Code、Pi、Hermes、OpenClaw。
4. 改 executions / discovery / MCP schema。
5. golden 测试 + 真实 CLI（mock 模型）冒烟。

## 验收标准
- [ ] 6 个 adapter 均有 golden：wire 前后快照；用户改过文件时 unwire 只删我们的键；未改过时逐字节还原。
- [ ] DSH 安装的是锁定版本，日志打印版本与 SHA。
- [ ] 默认 Agent（未设环境变量、首次启动）= `deepseek-harness`；inspect 与 actions 列表一致，含 Pi 与 OpenClaw。
- [ ] 真实 CLI + mock 模型：`headless` 一轮拿到回复；现有 hooks 相关测试不回归。
- [ ] OpenClaw：优先走 `openclaw config set`；缓存文件里我们的条目被清理。

## 怎么测
- `cargo test -p cranibox-agents`
- 真实 CLI：`ANTHROPIC_BASE_URL` / Codex `base_url` 指向 mock
- `python -m unittest` 里与 Agent 相关的用例
