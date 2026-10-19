# 文件归属（OWNERS）

> Phase 0（WS0）定稿。和 [WORKPLAN §2](./WORKPLAN.md#2-文件归属避免冲突) 一致，这里按目录列到文件级，并写明 Phase 0 已经预建的挂载点。
> 规则：**只改自己拥有的路径**。要改别人的文件或契约，在 PR 里写 `CONTRACT-REQUEST: ...` 并 @协调者（WS0）。

## 共享与契约（WS0）
| 路径 | 拥有者 | 说明 |
|---|---|---|
| 根 `Cargo.toml`（`members` 已一次性加入 4 个新 crate） | WS0 | 其他人不需要再改成员列表 |
| `.github/workflows/*` | WS0 | 契约快照测试：`cargo test -p cranibox-contracts --test fixtures` |
| `crates/cranibox-contracts/**`（含 `fixtures/*.json`） | WS0 | C1–C8，见 [CONTRACTS.md](./CONTRACTS.md) |
| `docs/design/**`、`NOTICE` | WS0 | |
| `crates/workspace-runtime/src/peers/config.rs` | WS0 | 改字段必须走契约 PR |
| `crates/workspace-runtime/src/peers/transport.rs` | WS0 | 现有 HTTP/Noise 的 `PeerTransport` 薄封装 |
| `crates/workspace-node/src/peer/server.rs`（`mesh_router()`、`auth`） | WS0 | WS1 只调用 `Running::mesh_router()` |
| `crates/workspace-web/src/server.rs` 里 `router()` 的两行挂载 | WS0 | 已挂好 `relay::router()` 和 `app_api::router()`，之后不用再动 |

## WS1 iroh 网络
| 路径 | 说明 |
|---|---|
| `crates/cranibox-net/**` | `IrohTransport`、`serve_mesh`、`relay::router(allowlist)`、ticket 编解码 |
| `crates/workspace-node/src/peer/transport_iroh.rs`（新） | 注入 `Authenticated` 后调用 `mesh_router()` |
| `crates/workspace-node/src/network/invitations.rs` | 单次使用语义沿用 |
| `crates/workspace-web/src/relay.rs` | Phase 0 已建空文件并挂载在 browser guard 之外 |
| `crates/workspace-node/src/network/config.rs` 中新增的 `iroh` 段 | 文件 `networks.json`，键 `iroh`（C5） |
| `crates/cranibox/src/**` 中 `serve` 的 `--relay/--no-relay/--public-url` | 只加这几个参数 |

## WS2 模型代理核心
`crates/cranibox-llm/src/{server.rs, proto.rs, catalog.rs, mgmt.rs, route/mod.rs, route/local.rs, upstream/mod.rs, upstream/apikey.rs, upstream/presets.toml}`、`crates/cranibox-llm/Cargo.toml`、`crates/cranibox-llm/tests/**`。
- `upstream/mod.rs` 的 `registry()` 里已经为 WS3、WS4 各留好一个标记块，它们只能在自己的块里加行。

## WS3 订阅登录与国内厂商
`crates/cranibox-llm/src/{auth/**, upstream/codex/**, upstream/kimi/**, upstream/gemini/**, upstream/copilot/**, upstream/cn/**}`；`upstream/mod.rs` 中 `// --- WS3` 块；`presets.toml` 的国内条目由 WS3 提 PR、WS2 评审。

## WS4 Claude 节点
`crates/cranibox-llm/src/upstream/claude/**`；`upstream/mod.rs` 中 `// --- WS4` 块。兼容性细节不进公开仓库，由协调者单独提供。

## WS5 Agent 适配
`crates/cranibox-agents/**`、`addons/deepseek-harness/**`、`crates/workspacectl/src/inspect.rs`、`crates/workspace-web/src/actions.rs`、`crates/workspace-runtime/src/peers/{executions.rs, executions/**, discovery.rs}`。

## WS6 多节点
`crates/cranibox-llm/src/{vault/**, route/remote.rs, route/health.rs}`、`crates/workspace-runtime/src/peers/roster.rs`（新，需要在 `peers/mod.rs` 加一行 `pub mod roster;`，已获准）、`crates/workspace-node/src/peer/server/{llm.rs, vault.rs}`（Phase 0 已建占位）。
- 给 grant 发放 `llm`/`vault` 的 CLI 或配对逻辑也由 WS6（配合 WS1 的 OwnerDevice 配对）负责；`peer/mod.rs` 里目前固定写 `false`。

## WS7 UI/UX
`addons/control-center/ui/**`、`addons/control-center/{web.js, web.css, index.*.html}`（生成物）、`apps/node-desktop/src/**`、`design-system/**`、`crates/workspace-web/src/app_api.rs`（Phase 0 已建空文件并挂在 guard 之内）、`apps/node-desktop/src-tauri/src/app_api.rs`（新，含在 Tauri 里注册 `app_*` 命令的那几行）。
- 前端 mock 直接读 `crates/cranibox-contracts/fixtures/*.json`，不要复制一份再改。

## 所有人
`Cargo.lock`：冲突时先 `git merge origin/main`，再 `cargo update -w`，禁止手改。
