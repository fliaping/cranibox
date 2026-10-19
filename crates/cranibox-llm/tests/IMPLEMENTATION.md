# WS2 / WS3 交付与验收记录

日期：2026-10-19。已跟进 main `6d4a8a4`（Phase 0 契约 PR #16），Windows CI 导入修复为 `8abcd1a`，P2 厂商与 OAuth 生命周期补充为 `0edb2e7`。本记录仅覆盖 WS2/WS3 自有文件；同一工作目录中正在进行的 WS5、WS6、WS7 改动未包含在本轮隔离验证中。WS4 首轮观测见 [Claude cch 报告](claude/CCH-FINDINGS.md)。

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

## 接入说明与契约请求

共享 C1–C8 类型保持不变。生产启动尚需 WS0/WS6 在 node/web 生命周期中持有 Handle，并提供持久化 `Arc<dyn AccountStore>`；不要用开发用 `MemoryAccountStore` 启动正式服务。调用者提供当前用户独占的 run 目录。

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
2. C7 缺少 refresh lease / generation 的调用方接口。当前只保证单进程刷新串行，写回前比较旧 secret。多节点必须先取得租约、以 generation 拒绝旧写入；同步 `AccountStore::put` 不能代替上游刷新前的租约。
3. Claude 节点选择需要名册持久化和旧节点清理。当前 `/claude/set-node` 只接受本机并保存在内存中；远端返回 `claude_node_offline`。没有 Claude 订阅 adapter，不代表完成 WS4 节点迁移。
4. 请在运行时启动处接入持久化 store / Router，UI 转发本机管理 socket，并按 THIRD-PARTY.md 更新 NOTICE。上述挂载点属于其他工作流，本次未修改。
5. C3 `SignIn` / `LlmMgmt` 尚无取消登录操作。当前只提供进程内 `OAuth::cancel`，没有另造管理路由；请协调者决定取消接口后再接入 UI。

## 已执行验证

全部运行在 `admin@192.168.217.6:22022` 的 `cranibox-validation` 容器、用户 abc、独立源码目录 `/config/validation-sources/llm-proxy-20261019`；未在日常容器或用户指定不用的本机运行测试。所有账号和上游均为 mock。

| 检查 | 结果 |
|---|---|
| `cargo fmt --all --check` | 通过 |
| `cargo clippy --workspace --locked --all-targets -- -D warnings` | 通过 |
| `cargo test --locked`（完整工作区） | 通过；已有显式忽略项保持忽略，延迟测试另行运行通过 |
| `cranibox-llm`（包含在最新完整工作区测试中） | 200 通过：169 单元、10 代理、5 协议回放、1 目录、15 订阅；延迟测试默认 ignored |
| 3×3 回放 | 请求、非流式响应、单字节 UTF-8 SSE、并行工具与 reasoning；另验证混合换行及截断错误 |
| 本机访问控制 | 非 loopback / 缺少 peer、全部转发头、外部 Host、浏览器头；socket 权限和 TCP 隔离 |
| 账号池 | 401/429/额度/连接失败首字节前切换；输出后不切换；同会话十次；刷新并发、断开和写失败恢复 |
| 订阅与国内预设 | 四家登录/adapter mock；全部 33 个预设端点和认证（含 MiMo `api-key`、阶跃 Messages Bearer）；智谱 200 信封/1113；Kimi schema/设备码；新增取消/拒绝/回调释放/写失败后凭据保留回归 |
| 显式延迟测试（120 样本/路径） | direct p50 446 µs；proxy p50 1305 µs；额外 859 µs，低于 2 ms。该值仅代表此主机和测试负载。 |
| Claude Code 2.1.303 本地 mock | API key 3 次 + Bearer 2 次；未见 cch；真实订阅行为未证实 |
| Linux / Windows / macOS CI | `0edb2e7` 的 GitHub Actions run `39247033227` 中，三平台均通过工作区 fmt / Clippy / test；Windows 管理客户端、拒绝重复服务及受保护 owner-only DACL 回归已通过。桌面打包等其他步骤状态以 PR 检查为准。 |

## 真实账号人工验收（尚未执行）

1. 由协调者用已授权账号分别登录 Codex、Kimi、Gemini、Copilot，确认登录状态、模型列表和一条流式工具请求；确认 Codex 刷新后仍可持续请求，撤销后显示重新登录。
2. 各用 DeepSeek、智谱、百炼、火山、MiniMax、阶跃、MiMo、千帆、混元/TokenHub 的真实 key 发一条最小请求；确认区域对应 host、套餐类型和扣费来源。测试连接会发送最小模型请求，可能产生费用。
3. Windows 补充实际不同用户、远程 SMB 连接拒绝验证；自动测试的同用户访问与 DACL 检查不能代替这两种真实身份测试。macOS 的 UDS、双栈和 OAuth mock 已由 CI 验证，真实浏览器登录仍需人工验收。
4. WS6 接入后验证持久化重启、多节点租约/轮换、Hub 离线回退、授权撤销和 mesh SSE；当前 mock Router 测试仅验证扩展点与避免路由循环。
5. 真实 Claude 订阅与默认官方 base URL 的可行性验证，按 cch 报告执行。不要将本地 mock 结果作为真实上游验收。

没有宣称真实厂商订阅已验收、Windows 不同用户/远程身份测试已完成或 WS4/WS6 已完成。
