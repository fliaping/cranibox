# WS3 任务书：订阅登录（Codex / Kimi / Gemini / Copilot）与国内厂商

> 估算：MVP 3.5 人周，第二阶段另加 2.5 · 建议执行者：Claude Code 或 Codex · 分支前缀：`ws3/`

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
在 WS2 的 `Upstream` / `SignIn` 接口上接入订阅登录和国内厂商：用户点「添加订阅」或「粘贴套餐 key」即可使用。

## 前置条件
WS2 骨架已合入，`upstream/mod.rs` 注册表可用。未合入前可先用 `MemoryAccountStore` 写单测。

## 范围（MVP）
1. **ChatGPT / Codex 订阅**：OAuth（loopback 回调；无浏览器时粘贴回调 URL）；refresh_token 轮换；上游 Codex Responses（`chatgpt.com/backend-api/codex`）。拷 shunt：`auth/codex/{auth,login,store}.rs`、`auth/callback.rs`、`adapters/responses/{http,request,body,sse_parse,error,inbound,context}.rs`。账号池参考 `pool.rs` 自写，接到 WS2 池上。
2. **Kimi Code**：控制台 key + OAuth 设备码（`auth.kimi.com/api/oauth/{device_authorization,token}`），每请求带 5 个 `X-Msh-*` 头。国际 `https://api.kimi.ai/coding/v1` 与 `/coding`；国内 `https://api.kimi.com/...`。拷 shunt `auth/kimi/*` 与 `forward_kimi_oauth`（402 轮换）；移植 magpie `gateway/kimi_schema.go`。另接 Moonshot 开放平台（按量 key）。
3. **国内预设与特殊处理**（`presets.toml` 国内条目 + `upstream/cn/`）：
   - **DeepSeek**：API key。`https://api.deepseek.com/v1` 与 `/anthropic`。移植 `gateway/deepseek_schema.go`。
   - **智谱 GLM Coding Plan**：bigmodel `https://open.bigmodel.cn/api/coding/paas/v4`（Responses `/api/v1`）、Anthropic `/api/anthropic`；z.ai 同结构。移植 `zhipu_coding.go`（1113→coding 地址并记住）、`zhipu_envelope.go`（HTTP 200 里的错误转标准错误）。
   - **阿里百炼 Coding Plan**（新增）：北京 `https://coding.dashscope.aliyuncs.com/v1` 与 `/apps/anthropic`；国际 `coding-intl...`；不支持 Responses；key 按 host 区分不按前缀；内置模型列表。另：百炼/千问 Token Plan、DashScope 按量。
   - **火山方舟**：Coding `https://ark.cn-beijing.volces.com/api/coding/v3`（含 Responses）、Anthropic `/api/coding`；Agent Plan `/api/plan/v3`；**路径不再拼 `/v1`**；`/models` 不可用，用内置列表（`ark-code-latest`、`doubao-seed-2.1-pro` 等）。
   - **MiniMax**：国际 `https://api.minimax.io/v1` 与 `/anthropic`；国内 `api.minimaxi.com`；备用 `api.minimax.cn/anthropic`。套餐 key `sk-cp-`，按量 `sk-api-`；把按量 key 粘进套餐时给友好提示。
   - 同时有 Anthropic / OpenAI 地址的厂商**默认走 Anthropic 兼容**。每个预设要有中文 `label`、`key_hint`、区域选项，以及「测试连接」最小请求。
4. **吊销判定**：`token_revoked` / `invalid_grant` / `refresh_token_reused` → `NeedsSignIn`，只记 header 哈希。移植 magpie `signin_revoked.go`。
5. **导入现有 CLI 登录**（D3，默认不导入）：实现从 `~/.codex/auth.json` 导入，但 UI 默认引导重新登录；导入时警告原 CLI 可能失效。

## 第二阶段（+2.5）
- Gemini Code Assist：Google OAuth。拷 shunt `auth/google`、`adapters/gemini`、`model/gemini*`。
- GitHub Copilot：设备码 → Copilot 短期 token。移植 magpie `provider/copilot_{signin,accounts,entitlement,auto}.go`、`ghtoken/`。
- 国内 P2：阶跃、小米 MiMo、百度千帆、腾讯混元（magpie `presets.go:188-288`）。

## 不在范围内
Claude（WS4）；代理引擎与协议转换（WS2）；多节点令牌同步与租约（WS6，你只实现单节点 `refresh()`）；插件订阅、签到、额度看板。

## 文件
`crates/cranibox-llm/src/auth/**`、`crates/cranibox-llm/src/upstream/{codex,kimi,gemini,copilot}/**`、`crates/cranibox-llm/src/upstream/cn/**`、`presets.toml` 国内条目、`upstream/mod.rs` 注册行、`crates/cranibox-llm/tests/providers/**`。

## 契约
实现 C3 `Upstream` / `SignIn`，以及 `SignInStart` 三种形态（Browser / DeviceCode / PasteCallback）。

## 验收标准
- [ ] 每厂商 mock 上游契约测试：地址拼接、请求头、特殊处理（智谱信封/1113、火山不拼 `/v1`、Kimi `X-Msh-*`）、错误映射到 `UpstreamError`。
- [ ] Codex OAuth：本地假授权服务器跑通登录、刷新、轮换、吊销 + 「粘贴回调」路径。
- [ ] Kimi 设备码：假服务器跑通轮询、超时、拒绝。
- [ ] 人工验收清单写进 PR（真实 Codex / Kimi / 智谱 / 百炼 / 火山 / MiniMax / DeepSeek，各发一条；协调者执行）。

## 怎么测
`cargo test -p cranibox-llm providers::`；人工验收见 PR 描述。
