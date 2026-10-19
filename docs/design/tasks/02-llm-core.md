# WS2 任务书：模型代理核心 cranibox-llm（单节点）

> 估算：6 人周 · 建议执行者：Codex（Rust 服务端、流式 HTTP） · 分支前缀：`ws2/`

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
实现每个节点本机的订阅代理 `127.0.0.1:3424`，让 6 个 Agent 共用同一个模型代理。本工作流负责代理的「骨架和引擎」：入站服务、访问控制、直通与协议转换、账号池与首字节前重试、模型目录、管理 API、`Upstream` 注册表，以及 API key / 自定义提供商。

订阅登录（WS3）、Claude（WS4）、多节点（WS6）都接在你提供的接口上。

## 设计依据（ARCHITECTURE §7 要点）
- 端点：`/v1/chat/completions`、`/v1/responses`、`/v1/messages`（含 `count_tokens`）、`/v1/models`。模型 id：`<provider>/<model>`。
- **能直通就直通。** 需要转换时以 Anthropic Messages 为中间格式。代码来源：
  - Messages→Responses：拷 shunt `model/responses_request.rs`、`responses.rs`、`responses_schema.rs`
  - Messages→Gemini：拷 shunt `model/gemini_request.rs`、`gemini.rs`
  - Responses→Chat / Responses→Messages：拷 shunt `model/inbound_responses/*`
  - Chat→Messages：拷 link-assistant `openai.rs`、`openai_stream.rs`、`openai_tools.rs`
  - Messages→Chat：移植 magpie `gateway/anthropic.go`、`chat.go`、`chat_call_ids.go`、`chat_tool_shape.go`
  - Chat→Responses / Chat→Gemini：两段串联
- 来源版本：shunt `pleaseai/shunt@9d42ba2`（MIT OR Apache-2.0，可拷）；link-assistant `link-assistant/router@07901ad2`（Unlicense）；magpie `yetone/magpie@865b618`（MIT，只移植）；submux 只参考。
- **账号池**：只在同一提供商多账号/多 key 之间切换。401 / 429 / 额度用尽 / 连接失败，且还没写出任何字节时，换下一个，出错账号冷却（额度用完→等到重置；限流→retry-after；其他→1 分钟翻倍，最多 10 分钟）。同一会话尽量粘账号（`X-Session-Id` 或首条 user 消息哈希）。不做跨提供商回退。
- **访问控制（硬性）**：只绑 `127.0.0.1` / `[::1]`，逐连接校验；任一转发头（`Forwarded`、`X-Forwarded-*`、`X-Real-IP`、`CF-Connecting-IP`、`True-Client-IP`、`Tailscale-*`）→403；Host 非本机→403；`Origin` / `Sec-Fetch-*`→403。本机 key `cbx-<agent>` 只识别 Agent。
- 管理 API 只在 Unix socket / named pipe 上。

## 范围
1. `server/`：axum 入站、访问控制、SSE、首字节前缓冲（holdWriter 思路）、C8 的 503 错误体。
2. `proto/`：协议转换。拷贝放 `proto/vendor/{shunt,link_assistant}/`（保留原头）；移植放 `proto/port/magpie/`。
3. `upstream/mod.rs`：`Upstream` 注册表，给 WS3、WS4 各留注册位。`upstream/apikey.rs`：通用 API key 上游（Chat / Responses / Messages）。按 `presets.toml` 生成提供商。
4. `upstream/presets.toml`：海外 API key 预设（OpenAI、Anthropic、xAI、OpenRouter）+ **自定义提供商**。schema 按 C3。国内条目由 WS3 填，你负责 schema 与校验。
5. `catalog/`：汇总模型列表；`no_list` 用内置表。
6. `route/local.rs`：本机账号选择与切换；定义 `RouteDecision` 扩展点，WS6 在 `route/remote.rs` 实现转发。
7. `mgmt.rs`：C3 管理 API 全路由 + SSE 事件。
8. 账号存取一律经 C7 `AccountStore`；开发用 `MemoryAccountStore`。
9. 入口：`cranibox_llm::spawn(cfg) -> Handle`，在 workspace-node / workspace-web 各加一行启动（PR 注明）。

## 不在范围内
OAuth / 国内特殊处理（WS3）、Claude（WS4）、vault 持久化与远端转发（WS6）、写 Agent 配置（WS5）、UI（WS7）。

## 文件
`crates/cranibox-llm/src/{lib.rs,server/**,proto/**,catalog/**,route/local.rs,route/mod.rs,mgmt.rs,upstream/mod.rs,upstream/apikey.rs,upstream/presets.toml}`、`crates/cranibox-llm/tests/**`、`crates/cranibox-llm/Cargo.toml`。

## 契约
- 实现：C3（地址、mgmt、`ModelEntry`、`Upstream`/`SignIn` 调用方）、C8。
- 使用：C7 `AccountStore`、C1 `RosterView`（预留，WS6 接入）。

## 步骤
1. 骨架：server、访问控制、Chat/Messages 直通、`/v1/models`、mgmt `/status` `/accounts`。**第 1 周末合入（WS3/WS4 依赖）。**
2. 账号池、冷却、首字节前重试、会话亲和。
3. 协议转换，一次合入一个方向，每方向带回放 fixtures。
4. 预设表 + 自定义提供商：粘贴 key 后探测协议（依次试 Chat / Responses / Messages）；支持粘贴 curl（借鉴 magpie `curlOf`）。
5. mgmt 补齐；接入 node / web 启动。

## 验收标准
- [ ] 回放测试覆盖 3 入站 × 3 上游：非流式、流式、工具调用（含并行）、thinking / reasoning。
- [ ] 访问控制矩阵全 403：非 loopback、每种转发头、外部 Host、浏览器头。
- [ ] mock 401/429：未写出字节时自动换账号；流式中途出错不切换，直接返回错误。
- [ ] 同一会话 10 请求落同一账号。
- [ ] mgmt socket 0600，TCP 不可达。
- [ ] 本机直通额外延迟 p50 < 2 ms。

## 怎么测
- `cargo test -p cranibox-llm`
- `crates/cranibox-llm/tests/replay/*.json`
- `curl --unix-socket` 测 mgmt
