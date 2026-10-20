# WS2 / WS3 交付与验收记录

日期：2026-10-19。已跟进 main `d2e5ec2` 和代理 PR #20 分支 `eedcc29`。本次补全私有管理登录取消、独立 token 计数与 OAuth 刷新；共享接口单独提交为 [CONTRACT-REQUEST #23](https://github.com/fliaping/cranibox/pull/23)，尚待协调者评审。WS6 / node / App 已提交为 PR #22。本记录区分代理核心和多节点集成。Claude 真实验证按用户要求后置，首轮观测见 [cch 报告](claude/CCH-FINDINGS.md)。

## 已实现

- 数据面默认监听 `127.0.0.1:3424` / `[::1]:3424`，逐请求检查来源与 Host，拒绝转发头、Tailscale 头和浏览器来源头。`cbx-*` 只标识 Agent，不是权限凭证。
- Chat / Responses / Messages 原生直通及协议转换、非流式 / SSE、并行工具与 reasoning；首字节前重试、同提供商账号池、会话亲和、限流与额度冷却。输出开始后不重放请求。
- 管理面：类型化客户端和 SSE；Unix socket 0600、目录 0700；Windows owner-only named pipe，拒绝远程访问，包含重复服务和受保护 DACL 回归；没有管理 TCP 监听。
- API key、自定义提供商、33 个预设、TOML override、模型发现与静态目录；curl 仅解析数据，不执行命令。国内包含 DeepSeek、智谱/z.ai、百炼 Coding/Token、千问 Token、DashScope、火山、MiniMax、Moonshot、Kimi Code、阶跃、MiMo、千帆、混元和 TokenHub。[官方入口依据](providers/ENDPOINTS.md)。
- Codex：PKCE、loopback / 粘贴回调、Responses 订阅端点、刷新轮换和撤销判定。显式 CLI 登录导入须确认原 CLI 可能失效。
- Kimi：设备码、pending / slow_down / 过期 / 拒绝、国内外端点、五个 X-Msh 头、402 和 schema 适配。
- Gemini：Google PKCE、Code Assist 项目发现和协议适配。部署者需配置自己的 `CRANIBOX_GOOGLE_CLIENT_ID` / `CRANIBOX_GOOGLE_CLIENT_SECRET`；账号需已开通 Code Assist 且可发现项目。没有自动导入 Gemini CLI 凭据或新项目开通向导。
- Copilot：GitHub 设备码换短期 Copilot token、模型列表、严格 API host 校验和 token 刷新。
- 登录生命周期：完成、拒绝、取消、过期和退出时释放回调监听；先检查 state，不反射错误详情，禁止缓存和 Referer 传播。新增 `signin_cancel(flow)`：重复取消成功、未知 flow 返回 404；完成后保留账号；已交换但未保存的凭据必须先重试保存，不能因取消丢失。数据 TCP 端口不提供该管理接口。
- OAuth 刷新按本机账号串行，HTTP 断开不取消刷新；轮换后暂时写失败会保留内存并重试。协调器已持久化同一 secret 时不重复写入；遇到更晚凭据拒绝覆盖，保留账号元数据。跨节点租约和崩溃恢复由 WS6 负责。
- 独立 token 计数：强制 Messages、非流式并移除 JSON `stream`，不会调用生成接口代替计数。OAuth adapter 计数可复用计数前 / 401 后刷新流程。

## 接入与契约请求

C3 增加兼容默认实现的 `Upstream::count_tokens`、`SignIn::cancel`、`LlmMgmt::signin_cancel`；序列化结构和 fixtures 不变，旧实现默认 Unsupported。私有管理路径为 `POST /signin/{flow}/cancel`，无请求体，成功 204。尚未增加 UI 取消按钮。

生产启动必须持有 `spawn(Config) -> Handle` 并提供持久化 `Arc<dyn AccountStore>`、用户独占的运行目录；退出调用 `Handle::shutdown`。不要用开发用 MemoryAccountStore 启动正式服务。PR #22 的 node 已接入 SQLite vault / Router / 协调刷新，App 使用私有管理 socket，由对应工作流评审。

WS6 实现 `Router::{decide,forward,forward_count_tokens}`，身份与授权检查后调用 `Handle::dispatch_local` / `dispatch_count_local`，避免再次远端路由。这些进程内入口不开放 TCP。TOML override 只在启动加载，尚无热更新。

原生 Messages API key 已接入 `POST /v1/messages/count_tokens`。**当前 Codex、Kimi、Gemini、Copilot adapter 没有原生计数实现，仍明确返回 501。** 新接口只允许 adapter 按实际能力实现计数；默认 Router 也返回 Unsupported，不能改发生成请求。

CONTRACT-REQUEST（WS0/WS6）：#23 单独评审共享接口。跨节点计数另行提交：RemoteRouter 使用既有 LLM_COUNT_TOKENS 路径，Coordinated 代理计数方法，mesh 入口保留 Unsupported 的 501。不能用进程内 Router 测试替代真实 mesh 联调。刷新租约沿用现有 C7；生产需要 Coordinated，普通 adapter 仅保证本进程串行。Claude 节点持久化由 WS6 / App 交付，没有 Claude adapter 不代表 WS4 完成。

## 已执行验证

全部在 `admin@192.168.217.6:22022` 的 `cranibox-validation` 容器、用户 abc 运行。本次核心源码目录 `/config/validation-sources/llm-completion-20261019`，独立构建目录 `/config/validation-build/ws2-completion`。没有在本机运行构建、测试或服务；账号和厂商上游均为 mock。

| 检查 | 结果 |
|---|---|
| cargo fmt --all --check | 通过 |
| cargo clippy --workspace --locked --all-targets -- -D warnings | 通过 |
| cargo test --workspace --locked（核心与当前 main） | **455 通过，0 失败，6 个显式忽略项** |
| cranibox-llm | **209 通过**：169 单元、12 代理、5 协议回放、1 目录、18 订阅、4 计数；延迟默认 ignored |
| cranibox-contracts | **11 通过**：单元、原有 fixtures、2 项接口兼容测试 |
| 新增管理回归 | 浏览器/设备码取消、幂等、未知/非法 flow、回调端口释放、停止轮询、完成后保留账号、待保存凭据取消被拒绝且重试不重复交换、数据 TCP 隔离 |
| 新增计数回归 | OAuth 401 后仍走计数；旧 adapter / Router 501 且从不生成；本地分发绕过再次路由 |
| 原有核心回归 | 3×3 协议回放、单字节 UTF-8 SSE、工具 / reasoning、来源与 OS socket 控制、账号池、首字节前重试、亲和、并发刷新与写失败恢复 |
| 订阅与国内厂商 | 四家登录 / adapter mock 和全部 33 个预设端点、认证、套餐差异 |
| 先前延迟测试（120 样本/路径） | direct p50 446 µs；proxy p50 1305 µs；额外 859 µs，仅代表此前主机和负载，本轮未重跑 |
| Claude Code 2.1.303 本地 mock | API key 3 次、Bearer 2 次；未见 cch，真实订阅行为未证实 |
| 跨平台 CI | 本轮 Actions 尚在运行，以对应 PR 当前检查为准，不用旧提交结果代替本次验证 |

日志位于远端源码目录：`completion-fmt.log`、`completion-clippy-private.log`、`completion-tests-private.log`。共享缓存曾出现不属于当前源码的旧 contract 元数据，切换独立构建目录后全量通过；表中只采用独立目录结果。

先前联合快照的加密同步、SQLite 重启、刷新租约与撤销作为历史证据保留。本次 mesh 计数在 `/config/validation-sources/count-mesh-20261019` 单独验证，结果随其集成 PR 记录；上述 455 项不代表新多节点补丁已验收。

## 尚需人工验收

1. 使用已授权真实账号登录 Codex、Kimi、Gemini、Copilot，确认状态、模型列表、流式工具请求、刷新和撤销。Gemini 需部署者配置 OAuth client / Code Assist 项目。
2. 国内厂商真实 key 最小请求，核对区域 host、套餐和扣费来源；连接测试会发送模型请求，可能产生费用。
3. Windows 不同用户和远程 SMB 拒绝验证；同用户与 DACL 自动测试不能代替真实身份测试。macOS 真实浏览器登录仍需人工验收。
4. 完成 #22 与计数集成请求评审，再在实际设备验收配对、登录、重启和网络切换；mock 上游和加密传输测试不能替代真实设备/账号验收。
5. Claude 真实订阅与官方 base URL 验证按用户要求后置，恢复时依据 cch 报告执行。
