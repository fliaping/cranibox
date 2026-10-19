# WS1 任务书：iroh 网络、内置 relay 与链接/二维码配对

> 估算：4.5 人周 · 建议执行者：Codex 或 Claude Code（熟悉 Rust、QUIC、axum） · 分支前缀：`ws1/`

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
1. 把 iroh 1.3 做成**默认内置网络**：设备之间先打洞直连，失败时经 relay 中转。复用现有的节点身份，复用现有的 mesh 路由。
2. 在 `cranibox serve` 里内置 relay（HTTPS/WSS，可以放在 Cloudflare Tunnel 后面）。
3. 用一次性 ticket（链接或二维码）完成配对，取代手填地址和 Token。
4. Tailscale、EasyTier、直连 HTTP 保留为可选传输（收进「高级」），行为不变。

## 已有结论（iroh spike 要点）
- iroh / iroh-relay `1.3.0`。`Endpoint::builder(presets::N0)`；`.relay_mode(RelayMode::Custom(..))`、`.secret_key(SecretKey::from_bytes(&seed))`、`.alpns(..)`。
- 同一 seed 的 EndpointId = ed25519 公钥。直接用 `node-identity.json` 的签名私钥作为 iroh SecretKey。
- QUIC bi-stream 上可跑 HTTP/1.1（axum + hyper），已验证 200。
- 内置 relay：axum 与 relay 同端口，按请求分发（`path == iroh_relay::http::RELAY_PATH`）。另提供 `/ping`、`/generate_204`。
- 只开 443：经 n0 relay 8 ms 建连；nginx TLS 反代后内置 relay 4 ms，空闲 95 s 仍存活。
- **反代不转发 Upgrade 时，客户端陷入「expected HTTP 101, got 400」重试。**
- 开发环境里测不了 cloudflared（出站 7844 不通），需人工验收。

## 范围
1. **`crates/cranibox-net`**：
   - `IrohNode::start(identity, cfg, roster) -> IrohNode`。relay 顺序：自有 relay → n0 兜底（`n0_fallback` 开关）。
   - `impl PeerTransport for IrohTransport`（C2）：按 NodeId 查 EndpointAddr，ALPN `cranibox/peer-http/1`，流式透传 SSE；连接复用 + 空闲超时。
   - `serve_mesh(endpoint, router)`：校验 TLS EndpointId == 固定 `signing_key`；注入 `Authenticated{transport: Iroh, e2e: true}`。
   - `relay::router(allow)`：基于 `iroh_relay::RelayService`，白名单（名册 + Grant/Route）热更新，取代 `AllowAll`。
   - `relay::self_check(public_url)`：探测 `/relay` 的 101；失败给出可读诊断。
   - `PathInfo`：直连 / 经中转 / RTT。
2. **workspace-node**：新增 `peer/transport_iroh.rs`，启动时起 IrohNode，调用 Phase 0 的 `mesh_router()`。**不改 `peer/server.rs`。**
3. **workspace-web / `cranibox serve`**：新增 `relay.rs`，挂 `/relay`、`/ping`、`/generate_204`；加 `--relay`/`--no-relay`、`--public-url`；启动自检。
4. **配对**：改造 `network/invitations.rs`，生成 C2 `PairTicket`，编码 `cranibox://pair/<base32>`，网页版 `https://<hub>/pair#...`，生成 QR SVG。实现 `pair_invite` / `pair_accept` / `pair_confirm`（签名对齐 C6，供 WS7 调用）。accept 后核对 SAS（复用 #7/#13 派生）；confirm：`OwnerDevice` 加入名册（WS6 接口，开发时 mock），`Guest` 生成 Grant（默认 messages）。一次性、15 分钟、最多 16 张。
5. **迁移**：对端 identity 声明 iroh 时，在 Route `alternatives` 最前插入 iroh 端点。
6. **文档**：`docs/iroh-network.md`（nginx / cloudflared 必须转发 Upgrade、端口、排障）。

## 不在范围内
UI（WS7）、名册签发（WS6）、llm/vault 业务逻辑（WS6）、改 Tailscale/EasyTier、在 iroh 上叠加 Noise。

## 文件
`crates/cranibox-net/**`、`crates/workspace-node/src/peer/transport_iroh.rs`、`crates/workspace-node/src/network/invitations.rs`、`crates/workspace-web/src/relay.rs`（及挂载处一行）、`docs/iroh-network.md`、`tests/integration/iroh_netns.sh`。

## 契约
- 提供：C2 `PeerTransport`（iroh）、`PairTicket`、`PathInfo`、pair_* 后端。
- 使用：C1 `RosterView`、C5 iroh 配置、Phase 0 `mesh_router()` / `Authenticated`。

## 步骤
1. `IrohNode` + `serve_mesh`；本机两 endpoint 互通，`/nodes` + exec 冒烟。
2. `PeerTransport`；现有 exec / 终端 WS / messages 测试在 iroh 上参数化跑通。
3. 内置 relay、白名单、自检。
4. ticket、二维码、pair 三步。
5. 迁移、文档、netns 脚本。

## 验收标准
- [ ] 只开 443 时经内置 relay 跑通 exec、终端往返、消息；同网 `PathInfo=Direct`。
- [ ] nginx TLS 反代：建连成功，空闲 95 s 不断；去掉 Upgrade 后自检明确报错。
- [ ] 非白名单 EndpointId 被拒；EndpointId ≠ 固定公钥时关连接。
- [ ] ticket 过期 / 复用 / secret 错误被拒；SAS 不一致时状态不变。
- [ ] pcap 无 exec 标记、token、消息正文。
- [ ] 现有 Tailscale / EasyTier / 直连测试通过。
- [ ] Cloudflare Tunnel 人工验收步骤写进 PR 描述。

## 怎么测
- `cargo test -p cranibox-net`
- `tests/integration/iroh_netns.sh`（需 root）
- `python -m unittest discover -s tests -p test_web_direct_peers.py`
