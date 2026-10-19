# Phase 0 共享契约（CONTRACTS）

> 代码在 `crates/cranibox-contracts`（只有类型、trait、常量、fixtures 和 `mock` feature，没有业务逻辑）。
> 本文是 [WORKPLAN §1](./WORKPLAN.md#1-phase-0先定共享契约ws0约-1-周所有人开工前合入-main) 的落地版：列出每条契约在代码里的位置、实现方，以及协调者在任务书没写清的地方做的决定。
> 改契约只能由 WS0 提 PR（标签 `contract`），并同步更新 fixtures；契约快照测试（`cargo test -p cranibox-contracts --test fixtures`）会拦住没更新 fixture 的改动。

## 契约一览

| 契约 | 模块 | 实现方 | 使用方 | Fixtures |
|---|---|---|---|---|
| C1 名册与身份 | `roster`：`Roster`、`RosterMember`、`Role`、`RosterView`、`ROSTER_SIG_DOMAIN`、`NODE_ID_DOMAIN` | WS6 | WS1、WS2、WS7 | `roster.json` |
| C2 传输与 mesh 路由 | `net`：`ALPN_PEER_HTTP`、`PeerTransport`、`Authenticated`、`TransportKind`、`PathKind`/`PathInfo`、`BoxBody`、`TransportError`、`routes::*`、`IrohAddr`、`PairTicket`、`PairPurpose` | WS1（iroh）、WS0（`workspace-runtime::peers::transport::DirectTransport`） | WS6、WS7 | `pair_ticket.json` |
| C3 模型代理 | `llm`：`LLM_ADDR`、`LLM_MGMT_SOCKET`、`Wire`、`ModelEntry`、`Availability`、`AccountSummary`、`AccountKind`、`AccountState`、`Upstream`、`UpstreamCx`、`CanonicalRequest`、`UpstreamResponse`、`UpstreamError`、`SignIn`、`SecretBox`、`LlmHealth`、`Preset`；`llm::mgmt`：路由常量、请求/响应类型、`MgmtEvent`、`LlmMgmt` | WS2（核心、apikey、mgmt）、WS3（订阅、国内）、WS4（Claude） | WS5、WS6、WS7 | `llm_*.json`、`presets.example.toml` |
| C4 Agent 适配 | `agents`：`AgentAdapter`、`AgentId`（固定顺序）、`DEFAULT_AGENT`、`LocalEndpoint`、`ModelPick`、`Applied`、`Drift`、`HeadlessTask`、`Launch`、`SessionRef`、`AgentCaps` 等；注册表 `cranibox_agents::registry()` | WS5 | WS7 | `agents_applied.json` |
| C5 配置与文件位置 | `config`：文件名常量、`IrohConfig`、`serve` 参数名、`RELAY_PATH` | WS1（`iroh` 段、`serve` 参数）、WS2/WS5/WS6（各自文件） | 全部 | `iroh_config.json` |
| C6 App API | `app`：`routes::*`、`ipc::*`、`StatusPill`、`Device`、配对请求/响应、`ApprovalCard`、`AgentInfo`、`AppEvent`、`AppError`、`AppApi` | WS7 | UI | `app_*.json` |
| C7 Vault | `vault`：`Account`、`Enablement`、`Lease`/`LeaseMsg`、`SignedRecord`、`PullReq`/`PullResp`、`Push`、`RefreshRequest`/`RefreshResponse`、`VAULT_SIG_DOMAIN`、`HPKE_SUITE`、`AccountStore` | WS6 | WS2（只经 `AccountStore`） | `vault_*.json` |
| C8 错误码 | `error`：`ErrorCode`（6 个）、`UnavailableBody` | WS2、WS6 | UI、Agent | `error_unavailable.json` |

Mock（`features = ["mock"]`）：`MockRoster`、`MockLlmMgmt`、`MockAppApi`、`MemoryAccountStore`，全部从同一批 fixtures 加载。

## 已接入现有代码的挂载点

| 位置 | 内容 |
|---|---|
| `workspace-runtime/src/peers/config.rs` | `Permissions.llm`、`Permissions.vault`；`Endpoint.iroh: Option<IrohAddr>` |
| `workspace-runtime/src/peers/transport.rs` | `DirectTransport: PeerTransport`（现有 HTTP + Noise，行为不变） |
| `workspace-node/src/peer/server.rs` | `mesh_router(app, network_discovery)`、`Running::mesh_router()`；`auth` 先认 `Authenticated` 扩展 |
| `workspace-node/src/peer/server/{llm,vault}.rs` | 挂在 `/api/v1/mesh/nodes/{node}/{llm,vault}`，有对应能力返回 501，没有返回 403 |
| `cranibox-llm/src/upstream/mod.rs` | `registry()`，为 WS2/WS3/WS4 各留一个标记块；各模块的空文件已按归属建好 |
| `cranibox-agents/src/lib.rs` | `registry()`（目前为空）、`adapter(id)` |
| `workspace-web/src/{relay,app_api}.rs` | 空路由，已挂进 `router()`：relay 在 browser guard 之外，App API 在 guard 之内 |

## 协调者决定（任务书没写清时取最简单的方案）

1. **`Permissions.llm`/`vault` 用 `skip_serializing_if = "is_false"`**，不只是 `serde(default)`：grant 的签名覆盖整个序列化结果，这样已签名的旧 grant 字节不变、签名继续有效。
2. **`mesh_router(app, network_discovery: bool)`** 多一个参数：现有 provider 网络监听需要公开 `/identity`，iroh 不需要。WS1 用 `Running::mesh_router()`（固定为 `false`）。
3. **`Authenticated` 的鉴权**：要求 `endpoint_id` 存在、等于 grant 里固定的 `signing_key`、并且 `grant.usable()`（严格模式下要已核对配对码）；任何一项不满足返回 401。名册检查留给 WS6。
4. **HTTP 和 Noise 合成一个 `DirectTransport`**（`kind() = Direct`）：现有 `Route::api` 已经在固定公钥后自动走 Noise。这个封装只支持 JSON 的 GET/POST、1 MiB 以内、缓冲响应；非 iroh 传输上的 SSE 是 WS1 的 secure_ws 适配。
5. **`PairTicket` 的二进制编码定为 postcard**，再做 RFC 4648 base32（无填充）。
6. **JSON 约定**：字段 `snake_case`；带数据的枚举用内部标签 `"kind"`（`AppEvent` 用 `event` + `data`，`MgmtEvent` 用 `event`）；可选字段输出 `null`；公钥和签名是小写 hex，密文是标准 base64。`AccountKind` 的值是 `api_key`/`oauth`/`claude_oauth`。
7. **`Availability::Unavailable` 带 C8 `code`**，不带自由文本，方便前端 i18n。
8. **`SignInStart` 的每个变体都带 `flow_id`**，`complete` 要用。`SignInInput = {callback_url?, code?}`。
9. **`UpstreamCx = {account, secret, preset, node}`**；HTTP 客户端由 WS2 在实现里自己持有，契约 crate 不依赖 reqwest。
10. **`AgentAdapter` 的 `wire`/`unwire`/`drift`/`register_mcp` 多一个 `env: &Env` 参数**：注册表里的 adapter 是 `&'static`，不能自己持有路径；测试也可以指向临时目录。
11. **审批动作只有三个：`allow_once`/`allow_always`/`deny`**。配对卡用 `allow_once` 表示「一致」、`deny` 表示「不一致」。
12. **`AccountStore` 是同步 trait**：`list/get/get_secret/put/remove`，`put(.., None)` 保留原密钥。
13. **`IrohConfig` 写进 `networks.json` 的 `iroh` 键**（`network/config.rs` 读写的文件），缺省 `enabled=true`、`n0_fallback=true`（D4 建议）。Phase 0 只定 schema，读写由 WS1 加。
14. **契约 crate 多依赖一个 `tokio`（只开 `sync`）**，因为 `RosterView::subscribe` 返回 `watch::Receiver`。没有引入 `schemars`：fixtures 加快照测试就是 schema。
15. **`relay.rs` 和 `app_api.rs` 的挂载由 WS0 预先加好**（任务书原本让 WS1、WS7 各自改 `workspace-web/src/server.rs`），避免两条工作流改同一处。
