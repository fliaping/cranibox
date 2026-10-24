# 管理员与访问策略 / Admin and access policy

> 中文在前，English summary below. 状态：方案已确定（2026-10-23），尚未实现。开发阶段的破坏性变更：不兼容 v1 名册，不提供迁移。

## 背景

现状（`cranibox-security-review-2026-10-23.md` 第 3 条）：

- `OwnerDevice` 配对时双方都签发凭据（`cranibox-net/src/pair.rs` 的 `issues`），并各自写入 `owner_permissions()` 全部能力、`Inbound::Accept`（`workspace-node/src/peer/transport_iroh.rs` 的 `ensure_peer`）。
- `Models::trust_members()` 为每个名册成员补齐全权限 grant 与 route，名册内两两全互信。
- owner 签名私钥封装分发给每个成员（`roster/transfer.rs`），任一成员都能签发名册。

结果是任一设备失陷，影响扩大到全部设备与名册签署权。

## 目标

1. 权限的来源是**人（管理员）**，与设备无关。设备只执行经管理员签名的指令，自身没有管理权。
2. 去中心化的用户认证：管理员身份是一对公私钥；被管理设备只需固定管理员公钥，即可独立验证指令，无需中心服务器。集群无中心、无主节点；写入权由管理员签名确定，不需要工作量证明或任何共识算法，节点之间只做签名状态的复制。
3. 管理员可在任意节点管理所有节点。
4. 配对与连接保持对等；**权限默认不对等**，可由管理员改成对等或反向。
5. 单管理员。普通用户无需理解密钥、证书、名册即可正确使用。

## 管理员身份

### 密钥层级

```text
恢复密钥（恢复码派生，离线保存）
  └─ AdminCert：授权一把管理员密钥
       └─ 管理员密钥（随机 ed25519，可用密码 / Touch ID / Windows Hello 解锁）
            └─ SessionCert：授权一把短期会话密钥（≤ 10 分钟）
                 └─ 名册（管理指令）
```

- **恢复密钥**：现有 recovery key，由系统生成的恢复码派生，不再支持自选恢复口令。设备固定它的公钥（`RosterStore::pin_identity`）。它只签 `AdminCert`，用于更换管理员密钥。
- **管理员密钥**：随机生成的 ed25519 私钥，代表管理员本人。从不以明文落盘，只在解锁瞬间出现在内存中，用来签一张会话证书后立即清零。
- **会话密钥**：每次解锁新生成，只保存在当前进程内存中，最长 10 分钟或显式退出后清零。所有管理指令都由会话密钥签名。

```text
AdminCert   { recovery_pub, admin_pub, serial, issued_at, sig }   // 恢复密钥签名，域 "cranibox-admin-cert-v1\0"
SessionCert { admin_pub, session_pub, node, method, issued_at,
              expires_at, sig }                                    // 管理员密钥签名，域 "cranibox-session-v1\0"
            // method: password | touch_id | windows_hello
            // expires_at - issued_at ≤ 600 秒
```

### 解锁方式

三种方式解锁的是**同一把**管理员私钥，只是加密方式不同：

| 方式 | 私钥的加密形式 | 存放位置 | 可用节点 |
| --- | --- | --- | --- |
| 管理员密码（基础方式） | Argon2id(密码, salt) 派生密钥加密 | 随名册分发到每台设备（`admin_vault.password`） | 任意节点：客户端、网页、CLI |
| Touch ID | Secure Enclave P-256 密钥协商（`biometryCurrentSet`）派生密钥加密 | 仅本机客户端配置目录 | Mac 客户端 |
| Windows Hello | Windows Hello 对固定挑战的签名派生密钥加密 | 仅本机客户端配置目录 | Windows 客户端 |

- 密码密文：`XChaCha20-Poly1305`，密钥为 `Argon2id(密码, salt)`，默认参数 64 MiB / 3 次 / 并行度 1（与现有 `passphrase_seed` 一致，参数随密文保存），AAD 为 `admin_pub`。密码至少 12 个字符。解锁后必须校验派生出的公钥等于 `admin_pub`，不等即报“密码数据已损坏”，不使用该密钥。
- 在客户端上启用 Touch ID / Windows Hello：输入一次密码解锁管理员私钥，再用本机硬件密钥加密一份存在本机。
- 修改密码：重新加密、递增 epoch 随名册分发；管理员私钥不变，设备无需任何操作。
- 生物识别必须绑定在硬件密钥的访问控制上，由硬件拒绝无人在场的使用。禁止“`LAContext` 弹窗通过后读取普通密钥”。
- 不使用 WebAuthn/passkey（需要真实域名和 Apple 开发者签名），不实现 USB 安全密钥。

### 选择顺序

在某节点发起管理操作时：

1. 本机是客户端且 Touch ID / Windows Hello 可用：直接使用。
2. 否则若有在线的、已启用 Touch ID / Windows Hello 的客户端：推送远程批准（见下节）。
3. 否则：在本节点输入管理员密码。

### 远程批准

1. 发起节点生成待批准请求：完整的新名册与相对当前名册的变更。
2. 请求经 mesh 推送给已启用 Touch ID / Windows Hello 的在线客户端。
3. 客户端**自行**根据新旧名册计算并显示摘要（不采用发起节点给的摘要文字），用户按 Touch ID / Windows Hello 批准。
4. 客户端签名并分发名册，发起节点看到结果。

请求 10 分钟过期；拒绝或过期即丢弃。用户也可以选择“改为输入密码”。

- 发起请求只要求名册成员资格，不要求任何访问权限；每个成员同时最多 1 个待批准请求，被拒绝后 1 分钟内不得再发，防止用弹窗轰炸诱导误批。
- 客户端批准时用本机 Touch ID / Windows Hello 解锁管理员私钥、建立自己的会话并签名。

**原型**：Secure Enclave 签名原型在 `/config/Workspace/cranibox-se-probe/`（`probe.swift`、`run.sh`、`verify.py`），可作为 Swift 胶水代码的起点。

### macOS Secure Enclave 验证记录（2026-10-23）

在目标 Mac（Mac16,7，macOS 26.6.2，Apple Swift 6.4）上，通过 `cranibox exec` 运行用 `swiftc` 编译的命令行原型，签名为 `adhoc,linker-signed`、无 TeamIdentifier，与当前 Cranibox 打包一致：

1. CryptoKit `SecureEnclave.P256.Signing.PrivateKey(accessControl: [.privateKeyUsage, .biometryCurrentSet])` 生成成功，`dataRepresentation`（569 字节）写入普通文件，**不使用钥匙串、无需开发者账号**。
2. 从文件重新加载后签名，Touch ID 弹窗出现在登录会话中；按指纹后签名成功，Python `cryptography` 独立验签通过。
3. 无人操作时一直阻塞至超时；按“取消”返回 `LocalAuthentication Code=-2 Canceled by user.`，未产生签名。

**仍需验证**：本方案用的是 `SecureEnclave.P256.KeyAgreement`（加密管理员私钥），机制与签名相同但未实测；Tauri `.app` 进程内调用；Windows Hello 签名派生密钥的确定性。实现方式：`build.rs` 编译一小段 Swift，导出“生成 / 加密 / 解密”函数供 Rust 调用。

## 管理指令

管理指令就是**签名后的完整新名册**，带递增的 epoch，发给被管理设备：

```text
Roster v2 {
  version: 2,
  admin_cert: AdminCert,
  admin_vault: { password: { salt, argon2_params, ciphertext } },
  epoch,                // 在同一 admin_cert.serial 内单调递增
  signed_at,            // 会话签名时刻，须落在 SessionCert 有效期内
  members: [RosterMember],   // 新增 controls_others: bool，默认 false
  access: [AccessEdge],      // 仅存放对默认权限的覆盖
  closed_sessions: [{ session_pub, last_epoch }],  // 见“会话关闭”
  session: SessionCert,
  sig,                  // 会话密钥签名，域 "cranibox-roster-v2\0"
}
AccessEdge { from: NodeId, to: NodeId, permissions: Permissions }
```

- 用完整状态而不是增量命令：离线设备漏收多条指令，上线后拿到最新一份即可对齐；防重放只需 epoch。
- 名册哈希 = SHA-256(签名消息)，签名消息沿用 `canonical::signing_message`（清空 `sig` 后的规范 JSON）。
- 排序：先比 `admin_cert.serial`，再比 `epoch`。更换管理员密钥（serial + 1）后 epoch 从 1 重新开始；这也是攻击者用偷来的会话签出极大 epoch 时的恢复手段。
- **有效权限** `effective(from, to)`：`access` 中有 `from → to` 的覆盖则用覆盖；否则 `from.controls_others` 为真时为“完全控制 + 消息与任务 + 共享模型和账号”，为假时只有“消息与任务”。界面上的开关就是 `controls_others`；新设备加入时无需改写其他条目，会自动被所有 `controls_others` 为真的成员控制。
- 对等即两台设备都可控制；反向调整即交换两台设备的 `controls_others`，递增 epoch。
- `owner_keys`、`owner_cert` 移除，owner key 不再存在。

### 设备验证

1. 用固定的恢复公钥验证 `admin_cert`，`serial` 不得低于已见值。
2. 用 `admin_pub` 验证 `session`；`expires_at - issued_at ≤ 600`；`signed_at` 在 `[issued_at, expires_at]` 内。
3. 用 `session_pub` 验证名册；(`serial`, `epoch`) 严格大于本机当前名册；该会话不在本机已知的 `closed_sessions` 中，或 `epoch ≤ last_epoch`。
4. 时效：本机在线时，只接受 `now ≤ expires_at + 5 分钟` 的会话签出的名册。例外是**追赶**：本机名册落后、且该名册是从对端同步拉取的最新名册时，接受已过期会话签出的名册（离线设备上线后需要这样对齐）。
5. 对每个对端 P，取 `effective(P, 本机)`，重算本机签给 P 的 grant（grant 仍由本机身份签名，`peer/server.rs` 的能力检查不变）。权限为空时连接与凭据保留。
6. `trust_members()` 从“只补缺”改为“对齐”：已有 grant 也按策略重算。对名册成员，本机只能在策略基础上收紧，不能放宽。
7. 非成员（Guest、`cranibox peer grant` 直连授权）仍由本机决定：设备本来就能开放自己的资源，管理员策略管的是成员之间的访问。界面上单独列出这些本机授权。

过期的会话证书只影响新指令：设备已接受的名册在会话过期后仍然有效。

### 会话关闭

`signed_at` 由签名者填写，偷到会话密钥的人可以在过期后伪造一个落在有效期内的 `signed_at`。第 4 条只能挡住在线设备，挡不住追赶中的离线设备。因此：

- 每次新会话签出的第一份名册，在 `closed_sessions` 中列出此前已知会话及其最后的 epoch（保留最近 16 个）。设备据此拒绝这些会话此后签出的、epoch 更大的名册。
- 结论：会话密钥被偷后，攻击者能影响在线设备的 10 分钟窗口，以及**管理员下一次正常登录之前**一直离线的设备。管理员下一次登录后，旧会话全部失效。

`llm`、`vault` 现在只要求名册成员资格（`is_owner_node`），改为同时要求 `effective` 中的对应能力。实现时逐一核对 `cranibox-llm` 中按成员资格放行和广播的路径。

### 同步与冲突

名册很小（几 KB）且只有管理员一个写入者，因此每次同步完整状态，不使用哈希链。冲突处理与防冻结参考 Tailscale Tailnet Lock（`tka`）。

- **同步**：第一版复用现有 `Models::exchange_roster`（`/vault/roster` 的 GET/POST），节点之间建立连接时互报当前名册的 epoch 与哈希，发现对方更新即拉取并验证；本机更新则推送给对方。该接口只要求名册成员资格，不要求任何访问权限。只要存在一条未被拦截的路径，新名册最终送达所有在线节点，以此缓解冻结攻击（拦截新名册、让设备停留在旧策略）。名册不设过期时间，避免长期离线的设备失去全部权限。
- **冲突**：管理员可能同时在两个节点持有会话，签出 (serial, epoch) 相同但内容不同的两份名册。设备检测到后不自动二选一，停在当前名册并通知管理员“你在两处同时做了修改”；管理员认证后签发 epoch 更大的新名册覆盖。界面在发起修改前先同步一次，尽量避免冲突。
- **审计日志**：每台设备在本地记录最近 64 次名册变更的哈希、epoch、会话证书与变更摘要。每条记录都带管理员会话签名，失陷节点无法伪造；各设备各自保存，单个节点删除自己的记录不影响其他设备。

### 使用通知

每次安装新名册，所有成员都记录一条管理活动：解锁方式与所在节点（取自名册内的 `SessionCert`）、时间、变更摘要。只创建会话而未签名册的，由发起节点尽力广播；失陷节点可以不广播，但它签出的任何名册都带着会话信息被记录。客户端弹出通知，例如“管理员密码在 dev-workspace 上被用于：允许 笔记本 控制 服务器”。非本人操作时，用户可立即修改密码，必要时用恢复码更换管理员密钥。

## 用户界面

界面上不出现“名册、证书、签名、会话”等词。

### 每台设备一个开关

> ☐ 管理员允许这台设备控制我的其他设备

- 开关即 `controls_others`。打开：它对其他所有成员有完全控制、消息与任务、共享模型和账号。关闭：只有消息与任务。
- “A 控制 B、B 不能控制 A”即 A 开、B 关；反过来互换两个开关。
- 针对某一对设备、逐项能力的覆盖放在“高级”中，默认折叠。

| 能力分组 | 包含 |
| --- | --- |
| 完全控制 | `exec`、`terminals`、`projects`、`desktop`、`computer`、`browser`、`spaces`、`activity` |
| 消息与任务 | `messages` |
| 共享模型和账号 | `llm`、`vault`；仅当发起方 `controls_others` 为真时默认开启 |

### 流程

- **首次设置**（任意节点）：设置管理员密码 → 显示恢复码 → 输入恢复码末 4 位确认已保存。在客户端上完成时，顺带启用 Touch ID / Windows Hello。本设备默认“可以控制”。
- **添加设备**：新设备显示二维码；在已有设备上点“添加设备”、核对 6 位配对码，回答“这台新设备可以控制你的其他设备吗？”（默认：否，附一句原因），然后按选择顺序完成管理员认证。
- **调整权限**：设备页切换开关 → 显示“将会发生的变化” → 管理员认证。
- **被拒绝时**：显示可操作提示，例如“笔记本无权控制台式机。到 设置 › 设备 打开它的控制权限”，而不是 401。
- **恢复码状态**：设备页常驻提示“恢复码已保存”。本期不提供重新生成恢复码：恢复公钥是所有设备固定的信任根，更换它需要旧恢复码签名，否则偷到管理员会话的人就能连根替换。

## 配对

- 传输层不变：SAS 配对码确认、双方签发凭据、固定公钥、建立路由。
- `ensure_peer` 对 `OwnerDevice` 不再写入 `owner_permissions()`，改为空权限 grant；权限只来自名册对齐。
- 新设备在 SAS 确认后的配对通道中收到恢复公钥并固定（首次信任由配对码确认保证）。
- 加入身份需要管理员签名新名册；签名前新设备只有 Guest 能力。
- 新设备 `controls_others` 默认为假，配对确认界面可改。由此：所有可控制的成员可以控制它，它对其他成员只有消息与任务（目标端 `Inbound::Hold`）。
- 加入方只得到签名名册（含密码加密的管理员私钥密文），不再收到任何明文私钥。

## 移除与失陷

- 移除设备：管理员删除该成员及涉及它的覆盖。其他设备对齐后撤销它的 grant 与路由；它已获得的数据无法收回。
- 丢失客户端：其本机的 Touch ID / Windows Hello 密文只能在该设备硬件上解开，无需处理；移除该设备即可。
- 会话密钥泄露：最长 10 分钟后失效。期间攻击者可签发任意名册，包括修改密码；收到通知后用密码或恢复码纠正。
- 管理员私钥泄露（在失陷节点上输入密码）：用恢复码签发 `serial + 1` 的 `AdminCert`，更换管理员密钥，同时设置新密码。各客户端上旧的 Touch ID / Windows Hello 密文随之失效，需在客户端上重新启用。
- 未加入任何身份的设备：只有本机与非成员授权，行为与今天的单节点一致；首次设置或加入身份后才有名册。
- 恢复码泄露：等同身份泄露，只能重建身份并逐台重新加入。

## 威胁模型

| 场景 | 结果 |
| --- | --- |
| 普通设备 B 失陷 | 只能以 `effective(B, *)` 的能力访问其他设备；无法签发指令、无法回滚 epoch |
| 在失陷节点上输入管理员密码 | 管理员私钥在解锁瞬间可被截获。缓解：优先远程批准，私钥用后立即清零，使用通知，恢复码更换密钥 |
| 只偷到会话密钥 | 在线设备上最长 10 分钟；对一直离线的设备持续到管理员下次登录（会话关闭）。可签出极大 epoch，由恢复码更换管理员密钥解决。使用通知可发现 |
| 远程批准弹窗轰炸 | 每成员 1 个待批准请求、拒绝后冷却 1 分钟 |
| 失陷节点发起恶意变更请求 | 远程批准时客户端独立计算并显示摘要，用户可拒绝 |
| 离线暴力破解密码 | 任何成员都持有密码密文，可离线猜测。**本期接受此风险**，使用高强度 Argon2id 参数与最低密码强度 |
| 失陷设备 B 返回恶意数据给 A | 不在本设计范围；A 上的 Agent 必须把命令输出、消息、任务结果、同步文件当作不可信输入 |
| 离线设备 | 同步新名册前按旧策略执行 |
| 拦截新名册（冻结攻击） | 连接时互报 epoch 与哈希，任一未被拦截的路径即可送达 |
| 同时在两处修改（冲突） | 设备停在当前名册并通知管理员，由管理员签发更新的名册 |
| 同 OS 用户下的恶意程序 | 可使用本机 grant 与凭据；会话期间可读取进程内存中的会话密钥 |

## 实现顺序

1. `cranibox-contracts`：Roster v2、`AdminCert`、`SessionCert`、`AccessEdge`、`controls_others`、`closed_sessions`、`admin_vault`、签名域、`effective()`；`RosterView` 增加按 (from, to) 查询有效权限。
2. 管理员会话：密码解锁、会话密钥、内存清零；CLI `cranibox admin init|login|logout|status`。
3. 策略执行：`trust_members()` 对齐、本机只能收紧、`llm`/`vault` 按 `effective` 授权、管理活动记录。
4. 配对：`ensure_peer` 改为空权限 grant，加入身份走管理员签名，固定恢复公钥，不再分发任何私钥；删除 owner key 相关代码（`OwnerKey`、`SealedOwnerKey`、`rotate_owner` 等）。
5. 远程批准与使用通知。
6. macOS Touch ID（先在 Mac 上验证 Secure Enclave 密钥协商，再在 Tauri `.app` 中复测）。
7. Windows Hello（先做可行性验证）。
8. UI：开关、分组、向导、变更摘要、恢复码状态。

每一步附测试；多节点用例放在 `workspace-node/src/peer/server/multi_node.rs`。第 1–4 步完成后，系统即可只用管理员密码完整工作；第 5–8 步是体验增强。

### 必须覆盖的测试

- 验证链：错误的恢复公钥、低 serial 的 AdminCert、超过 600 秒的会话、`signed_at` 越界、错误签名域，均拒绝。
- 排序：相同或更低的 (serial, epoch) 拒绝；serial + 1 且 epoch 重置为 1 接受。
- 冲突：同一 (serial, epoch) 不同哈希，设备保持当前名册并产生冲突事件。
- 会话关闭：被关闭会话签出的更大 epoch 拒绝；在线设备拒绝过期会话的新名册；追赶时接受。
- 权限：`controls_others` 切换后，两端 grant 在一次同步内按 `effective` 对齐；覆盖优先于默认；新成员自动被可控制成员控制。
- 配对：`OwnerDevice` 配对后、名册到达前，双方 grant 权限为空。
- 密码：错误密码、篡改密文、篡改 AAD 均失败；解锁后公钥不符时拒绝使用。
- 私钥清零：解锁路径使用 `Zeroizing`，会话结束后不再持有私钥。

## 不在本期范围

审查报告第 1、2、4、5 条（管理页认证、默认桌面不设密码、`ui.token` 重复使用、旧 grant 兼容模式）；iroh-gossip；USB 安全密钥；WebAuthn/passkey；多管理员；离线暴力破解的门限 OPRF 防护；重新生成恢复码。

## English summary

Authority comes from a single human admin, independent of devices, via decentralized public-key authentication: every device pins the recovery public key and verifies admin instructions on its own. The chain is recovery key (generated recovery code, offline) → `AdminCert` → a random ed25519 admin key → a short-lived `SessionCert` (≤ 10 minutes) → the signed roster. The admin key never rests in plaintext: it is encrypted under an Argon2id password key (shipped in the roster so any node can unlock it), and optionally re-encrypted locally under Touch ID (Secure Enclave key agreement) or Windows Hello. On unlock it only signs a session certificate and is zeroized. Preference: local Touch ID / Windows Hello, then remote approval on an online client (which computes the change summary itself), then the password. The cluster has no center and needs no proof of work or consensus: write authority is fixed by the admin signature, and nodes only replicate signed state. Instructions are complete signed rosters with a strictly increasing epoch, replicated as full state (no hash chain) and exchanged whenever nodes connect (against freeze attacks, following Tailscale Tailnet Lock); two rosters with the same (serial, epoch) are a conflict, devices stay on their current roster and the admin signs a newer one. Each new session lists previously known sessions as closed, bounding a stolen session key to the 10-minute window for online devices and to the admin's next login for offline ones. Each member has a `controls_others` switch; effective permissions come from it unless an `AccessEdge` override exists. Pairing stays symmetric but grants start empty; new devices default to not controlling others. Every session and roster change is recorded and notified to clients. Offline password guessing is an accepted risk for now. No migration: v1 rosters are dropped.
