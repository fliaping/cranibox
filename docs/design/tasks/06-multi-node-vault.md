# WS6 任务书：多节点——名册、vault 同步、逐节点启用、租约、转发与 Hub 兜底

> 估算：7 人周 · 建议执行者：Codex 或 Claude Code（Rust 分布式 / 加密） · 分支前缀：`ws6/`

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
让订阅在主人的多台设备间安全同步与使用：
1. **名册（Roster）**：主人节点集合的唯一事实来源；决定谁能拿 `llm`/`vault`、谁是 Hub、谁是 Claude 节点。
2. **Vault**：账号密文加密同步（除 Claude）；每账号有**逐节点启用标志**（默认只在主节点启用）。
3. **选路**：本机已启用 → 直接执行；否则按 Hub → home → 其他已启用节点转发（`llm/1`）。
4. **租约**：多节点启用同一 OAuth 账号时，同一时间只有一个节点刷新；持有者离线后按 priority 自动接管（D2）。
5. **Hub 离线**：各节点用已同步且本机启用的账号起本机代理；Agent 始终只连 `127.0.0.1:3424`，感知不到切换。
6. **Claude**：`claude/*` 始终转发到 Claude 节点；离线 → 503 `claude_node_offline`。

## 设计依据（ARCHITECTURE §5.3 / §7.4）
### 数据模型（`<data>/llm/vault.db` SQLite）
```
Account   { id, provider, kind: ApiKey|OAuth|ClaudeOAuth, label, home_node,
            sync: Synced|NeverSync, version, writer, updated_at,
            sealed: XChaCha20-Poly1305(DEK, secret, aad=id‖version‖gen),
            wraps: {NodeId → HPKE(X25519, DEK)}, gen, sig }
Enablement{ account_id, enabled_on: Set<NodeId>, all_nodes: bool, version, writer, sig }
Lease     { account_id, holder, epoch, expires_at, sig }   // 仅 Synced OAuth
Roster    { owner_pub, epoch, members:[{node_id, signing_key, x25519_pub, role, priority, is_claude_node, name}], sig }
```
- ClaudeOAuth 固定 `NeverSync`，wraps 只有 home。
- `all_nodes=true`：之后新加入节点也自动启用。
- 本地静态保护：节点 X25519 私钥解 DEK；私钥优先钥匙串（Keychain / DPAPI / Secret Service），无头退回 0600；明文 `zeroize`。

### 同步（mesh `/vault/*`，C7）
Synced 账号密文 + Enablement + Lease 同步到**所有**名册节点（含未启用的），以便启用时已有最新 token。冲突：ApiKey/Enablement 取 `(version, updated_at, writer)` 较新；OAuth 取 `gen` 大者，相同取 `lease.epoch` 大者。只接受名册成员签名。

### 选路（`route/remote.rs`）
对模型 `p/m` 的每个候选账号：
1. 本机已启用 → 本机执行（Hub 在线也如此，减一跳）；
2. 否则：Hub（已启用且在线）→ home_node → 其他已启用在线节点（按 priority），经 `/llm/*` 转发；
3. 都不可用 → 试同提供商下一账号；全失败 → 503 + C8 code；
4. ClaudeOAuth 只有 Claude 节点一个候选。

失败判定：连接/握手失败、对端「代理不可用」、5 秒无响应头。连续 3 次失败标离线；连续 5 次成功且满 30 秒才恢复。流式中途不切换，只在首字节前重放。

### 租约（仅 `enabled_on` 之间）
- 单节点启用：无需租约。
- 多节点：默认持有者 = Hub（若在 enabled_on）否则 home；TTL 15 分钟，每 5 分钟续期。
- 非持有者直接用同步来的 access token；快过期或 401 时发 `refresh_request(id, gen)`；持有者对同一凭据只刷新一次，再广播（先 fsync 再广播）。
- 接管：租约过期且更高 priority 的 enabled 节点都联系不上 → 本节点以 `epoch+1` 自签（D2=自动）。
- 交还：原持有者稳定在线（5 次成功 + 30 s）后以 `epoch+1` 交还。
- 分区致同 gen 两份 token：保留 epoch 大的；厂商吊销整组 → `NeedsSignIn`。
- 停用持有者：先交还再停用；只剩一个启用节点时租约作废。

## 范围
1. `workspace-runtime/src/peers/roster.rs`：签发 / 校验 / 持久化 / `impl RosterView`；Owner 密钥生成与 HPKE 分发到主人设备；成员增删（删时触发 DEK 轮换回调）。
2. `cranibox-llm/src/vault/**`：SQLite、AEAD、HPKE（`DHKEM(X25519, HKDF-SHA256) + HKDF-SHA256 + ChaCha20Poly1305`）、钥匙串适配、`impl AccountStore`（替换 WS2 的 Memory）。
3. mesh 子路由：填实 Phase 0 占位 `server/llm.rs`、`server/vault.rs`（C2 / C7）；鉴权：`Permissions.llm|vault && roster.is_owner_node`。
4. `route/remote.rs` + `route/health.rs`：选路、Hub 健康状态机（Remote → Suspect → Local → Probing → Remote）、离线判定与防抖。
5. 租约状态机与故障注入测试。
6. 移除节点：名册 epoch+1 + 轮换该节点见过的所有 DEK；UI 提示去厂商轮换 API key（文案 key 交给 WS7）。
7. **先跑在现有 Noise / 直连传输上**（Phase 0 的 `PeerTransport` 封装）；iroh 就绪后自动受益，不绑死 WS1。

## 不在范围内
iroh 本身（WS1）；协议转换与本机池（WS2）；OAuth 登录实现（WS3/WS4，你调用 `Upstream::refresh`）；UI（WS7）。

## 文件
`crates/cranibox-llm/src/{vault/**,route/remote.rs,route/health.rs}`、`crates/workspace-runtime/src/peers/roster.rs`、`crates/workspace-node/src/peer/server/{llm.rs,vault.rs}`、`crates/cranibox-llm/tests/multi_node/**`、`tests/integration/vault_netns.sh`。

## 契约
实现 C1 `RosterView`、C7 vault 消息、C2 mesh 子路由业务、C8 错误码；消费 C3 `Upstream::refresh`、C2 `PeerTransport`。

## 步骤
1. 名册 + Owner 密钥；单测签名与 epoch。
2. vault 本地（SQLite + AEAD + 钥匙串）+ `AccountStore`；让 WS2 切到真实存储。
3. `vault/1` 同步（两节点）；Enablement。
4. `llm/1` 转发 + 选路；Claude 强制转发。
5. 健康状态机与 Hub 离线兜底。
6. 租约 + 故障注入（分区、持有者宕机、交还）。
7. 移除节点与 DEK 轮换。

## 验收标准
- [ ] 三节点 netns：添加账号于 A → B/C 密文可见；默认仅 A 启用；B 手动启用后可本机执行。
- [ ] Hub 下线：状态切 Local，Agent 不改配置继续对话（已启用账号）；Hub 恢复后 30 s 内回切；进行中的流式会话不中途切换。
- [ ] 两节点启用同一 OAuth：只有持有者刷新；持有者宕机后另一节点在 TTL 后接管；分区愈合后保留 epoch 较大 token。
- [ ] Claude 请求始终到 Claude 节点；该节点离线 → 503 `claude_node_offline`。
- [ ] 访客节点（非名册）访问 `/llm/*` `/vault/*` → 403。
- [ ] 移除节点后其 wraps 全部轮换；旧 DEK 无法解新密文。
- [ ] pcap：无 key、token、账号密文明文。

## 怎么测
- `cargo test -p cranibox-llm multi_node::`
- `tests/integration/vault_netns.sh`（杀进程、断网、延迟）
- 与 WS1 联调：同一套场景跑在 iroh 上
