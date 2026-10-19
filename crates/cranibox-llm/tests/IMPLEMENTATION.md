# WS2 / WS3 交付与验收记录

日期：2026-10-19。已跟进 main `6d4a8a4`（Phase 0 契约 PR #16），Windows CI 导入修复为 `8abcd1a`，P2 厂商与 OAuth 生命周期补充为 `0edb2e7`。当前补充刷新协调器已保存凭据时避免重复写入的修复。本记录分别列出 WS2/WS3 隔离验证和包含其他工作流未提交代码的联合快照验证；联合通过不代表这些改动已随本 PR 提交或合入。WS4 首轮观测见 [Claude cch 报告](claude/CCH-FINDINGS.md)，真实 Claude 验证按用户要求后置。

## 已实现

- `spawn(Config) -> Handle`：默认监听 `127.0.0.1:3424` / `[::1]:3424`，逐请求核验连接来源和 Host；转发头、Tailscale 头、浏览器来源头统一拒绝。`cbx-*` 只标识 Agent，不是权限凭证。
- Chat / Responses / Messages 原生直通和以 Messages 为中间格式的转换，非流式与 SSE、并行工具调用、reasoning；首字节前暂存和重试，同一提供商账号池、会话亲和、限流与额度冷却。输出开始后不重放请求。
- C3 管理路由、类型化 socket 客户端与 SSE；Unix socket 0600，独立目录 0700，没有管理 TCP 监听。Windows 使用 owner-only named pipe 并拒绝远程客户端；同用户管理访问已在 Windows runner 通过，另增加拒绝重复服务和检查受保护 owner-only DACL 的回归。
- API key、自定义提供商、33 个内置预设、可校验的 TOML override、模型发现和静态模型表；curl 作为数据解析，不执行命令；最小连接探测。国内覆盖 DeepSeek、智谱/z.ai、百炼 Coding/Token、千问 Token、DashScope、火山、MiniMax、Moonshot、Kimi Code，新增阶跃、小米 MiMo、百度千帆、腾讯混元与 TokenHub。[P2 官方入口依据](providers/ENDPOINTS.md)。
- Codex：PKCE、loopback / 粘贴回调、Responses 订阅端点、刷新轮换和撤销判定；显式 CLI 登录导入要求确认原 CLI 可能失效。
- Kimi：设备码、pending/slow_down/过期/拒绝、国内外端点、五个 X-Msh 头、402、schema 适配。
- Gemini：Google PKCE、Code Assist 项目发现和协议适配。部署者需设置自己的桌面 OAuth client：`CRANIBOX_GOOGLE_CLIENT_ID` / `CRANIBOX_GOOGLE_CLIENT_SECRET`；账号需已开通 Code Assist 并能发现项目。没有自动导入 Gemini CLI 凭据，也没有新项目开通向导。
- Copilot：GitHub 设备码换短期 Copilot token、模型列表、严格的 API host 校验和 token 刷新。
- 登录生命周期：接收成功/拒绝回调、取消、过期或最后一个 OAuth 实例退出时释放回调监听；验证 state 后才处理拒绝，错误说明不反射到页面，回调响应禁止缓存和 Referer 传播。进程内 `OAuth::cancel` 可停止未交换的流程；已交换但尚未保存的凭据必须先重试保存，不能因取消而丢失。
- OAuth 刷新按本机账号串行，HTTP 断开不取消刷新；token 已轮换但存储暂时写失败时暂存在内存，下次写入时恢复。进程崩溃恢复和多节点刷新租约由 WS6 负责。
- 兼容 WS6 协调刷新：上游已持久化同一新 secret 时不再调用 `put`，避免重复版本和无谓的写失败；存储中已是更晚凭据时拒绝覆盖，保留协调器更新的账号元数据。仍保留普通 adapter 的写失败恢复路径。

## 接入说明与契约请求

共享 C1–C8 类型保持不变。生产启动必须在 node/web 生命周期中持有 Handle，并提供持久化 `Arc<dyn AccountStore>`；不要用开发用 `MemoryAccountStore` 启动正式服务。联合快照中的 `workspace-node/src/models.rs` 已接入 SQLite vault、Router 和协调刷新装饰器，App 层已接入管理 socket；这些属于其他工作流的未提交改动，交付时必须同时纳入。调用者提供当前用户独占的 run 目录。

```rust,ignore
let mut cfg = cranibox_llm::Config::new(node_id, private_run_dir, persistent_store);
cfg.presets = cranibox_llm::catalog::load_overrides(&presets_directory)?;
cfg.routing = Some(authenticated_mesh_router); // 可选，WS6 实现 Router
let llm = cranibox_llm::spawn(cfg).await?;
// 将 llm 保存在运行时状态，退出时 llm.shutdown().await。
```

WS6 可实现 `route::Router::{decide,forward}`，并在验证 mesh 身份与授权后调用 `Handle::dispatch_local`，避免再次远端路由。这个进程内入口不开放 TCP。`load_overrides` 在启动时加载，未实现文件热更新。

**CONTRACT-REQUEST（WS0/WS6）**：

1. C3 `CanonicalRequest` / `Upstream` 缺少 count-tokens 操作。当前 `/v1/messages/count_tokens` 仅支持本机原生 Messages API key；OAuth 和远端计数返回 501。需要显式操作类型，不能把控制信息塞进用户 JSON 或 Agent 名称。
2. 原 refresh lease / generation 接入请求已通过 WS6 的 `vault::coordinated::Coordinated` 装饰器在联合快照中验证，无需为这条路径修改 C7。生产调用必须使用该装饰器；单独启用普通 adapter 仍只保证本进程串行，不能代替跨节点租约。
3. Claude 节点选择需要名册持久化和旧节点清理。当前 `/claude/set-node` 只接受本机并保存在内存中；远端返回 `claude_node_offline`。没有 Claude 订阅 adapter，不代表完成 WS4 节点迁移。
4. 运行时 store / Router 和 UI 转发已在联合快照中验证，请对应工作流提交并评审；NOTICE 仍需按 THIRD-PARTY.md 更新。上述挂载点不在本次自有提交中。
5. C3 `SignIn` / `LlmMgmt` 尚无取消登录操作。当前只提供进程内 `OAuth::cancel`，没有另造管理路由；请协调者决定取消接口后再接入 UI。

## 已执行验证

全部运行在 `admin@192.168.217.6:22022` 的 `cranibox-validation` 容器、用户 abc；自有隔离目录为 `/config/validation-sources/llm-proxy-20261019`，联合目录为 `/config/validation-sources/llm-integration-20261019`。未在日常容器或用户指定不用的本机运行测试。所有账号和厂商上游均为 mock；三节点联调用真实 TCP + Noise 传输。

| 检查 | 结果 |
|---|---|
| `cargo fmt --all --check` | 通过 |
| `cargo clippy --workspace --locked --all-targets -- -D warnings` | 通过 |
| `cargo test --locked`（联合快照完整工作区） | 460 通过、5 个已有显式忽略项；延迟测试此前另行运行通过 |
| `cranibox-llm`（WS2/WS3 自有代码隔离验证） | 202 通过：169 单元、12 代理、5 协议回放、1 目录、15 订阅；延迟测试默认 ignored |
| `cranibox-llm`（联合快照） | 210 通过，包含 WS6 的 6 项多节点测试及新增 2 项单元测试 |
| 3×3 回放 | 请求、非流式响应、单字节 UTF-8 SSE、并行工具与 reasoning；另验证混合换行及截断错误 |
| 本机访问控制 | 非 loopback / 缺少 peer、全部转发头、外部 Host、浏览器头；socket 权限和 TCP 隔离 |
| 账号池 | 401/429/额度/连接失败首字节前切换；输出后不切换；同会话十次；刷新并发、断开和写失败恢复 |
| 订阅与国内预设 | 四家登录/adapter mock；全部 33 个预设端点和认证（含 MiMo `api-key`、阶跃 Messages Bearer）；智谱 200 信封/1113；Kimi schema/设备码；新增取消/拒绝/回调释放/写失败后凭据保留回归 |
| 显式延迟测试（120 样本/路径） | direct p50 446 µs；proxy p50 1305 µs；额外 859 µs，低于 2 ms。该值仅代表此主机和测试负载。 |
| Claude Code 2.1.303 本地 mock | API key 3 次 + Bearer 2 次；未见 cch；真实订阅行为未证实 |
| Linux / Windows / macOS CI | `0edb2e7` 的 GitHub Actions run `39247033227` 中，三平台均通过工作区 fmt / Clippy / test；Windows 管理客户端、拒绝重复服务及受保护 owner-only DACL 回归已通过。桌面打包等其他步骤状态以 PR 检查为准。 |

联合快照以 `8076a2d4bda85c28ba179ee6fe2124e8db1108c5` 为底，覆盖 87 个当时的工作区文件，再覆盖本次 `engine.rs` / `proxy.rs` 修复。初始源码包 SHA-256 为 `0ed76a910b4ac37feb91bfdcde874c78d02a31cdfef62b2be276a33bc2512a70`；文件清单与哈希保存在验证主机的独立源码目录中。

新回归已先验证旧实现失败（预期 200、实际 503），再验证修复通过。联合测试还覆盖：加密同步和本机显式启用、SQLite 重启不丢凭据、并发刷新只调用一次厂商、租约过期接管与恢复交还、撤销后的密钥轮换，以及三节点通过 Noise 同步、转发、Hub 停止后继续使用本机账号。它们不能替代真实厂商验收。

## 真实账号人工验收（尚未执行）

1. 由协调者用已授权账号分别登录 Codex、Kimi、Gemini、Copilot，确认登录状态、模型列表和一条流式工具请求；确认 Codex 刷新后仍可持续请求，撤销后显示重新登录。
2. 各用 DeepSeek、智谱、百炼、火山、MiniMax、阶跃、MiMo、千帆、混元/TokenHub 的真实 key 发一条最小请求；确认区域对应 host、套餐类型和扣费来源。测试连接会发送最小模型请求，可能产生费用。
3. Windows 补充实际不同用户、远程 SMB 连接拒绝验证；自动测试的同用户访问与 DACL 检查不能代替这两种真实身份测试。macOS 的 UDS、双栈和 OAuth mock 已由 CI 验证，真实浏览器登录仍需人工验收。
4. 将已经联合验证的 WS6 / node / App 改动提交并完成评审，再在实际设备上验收配对、登录、重启和网络切换；自动化加密传输、租约、持久化、Hub 离线与撤销验证已通过。
5. 真实 Claude 订阅与默认官方 base URL 的验证后置，待恢复该任务时按 cch 报告执行。不要将本地 mock 结果作为真实上游验收。

没有宣称真实厂商订阅已验收、Windows 不同用户/远程身份测试已完成或 WS4/WS6 已完成。
