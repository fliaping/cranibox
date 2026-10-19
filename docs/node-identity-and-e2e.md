# 节点身份与端到端加密 / Node identity and end-to-end encryption

> 中文在前，English summary below.

Cranibox 把节点看作 Agent 的“脑壳”：Codex、Claude Code 等 Agent 是大脑，节点负责身份、授权、人工审批和节点之间的安全传输。本文记录节点身份、凭据存储与签名记录（阶段 1）的设计、迁移和威胁模型；端到端加密通道与配对码确认在后续 PR 中补充到本文。

## 节点身份

- 每个节点拥有一个持久 Ed25519 签名密钥和一个独立的 X25519 Noise 静态密钥。私钥保存在 `peers.json` 同目录的 `node-identity.json`（其他文件名为 `<name>.identity`），Unix 下权限 `0600`；文件只创建一次，损坏时报错而不是重新生成。
- 公开身份 `PublicIdentity {version, node_id, signing_key, noise_key, signature}` 由签名密钥对 NodeID 与两把公钥的绑定签名，替换任一字段都会验证失败。
- 新节点的 NodeID 由公钥派生：`node-` + `SHA-256("cranibox-node-id-v1\0" ‖ 公钥)` 前 16 字节的十六进制。它与原随机 `node-<32 hex>` 同形，网关、Web、测试中已有的 NodeID 校验无需改动。
- 已有节点保留原 NodeID，首次运行时生成密钥并以自签名绑定（`key_derived_id: false`）。其他节点在首次接触时固定（pin）这把公钥。
- `cranibox-node peer identity` 显示公钥、指纹（如 `abcd-efgh-ijkl-mnop`）和是否为派生 ID，不显示任何私钥或凭据。

## 凭据摘要与迁移

- 授权记录只保存 `salt` 与 `credential = SHA-256(salt ‖ token)`，校验使用常量时间比较。新授权使用 16 字节随机盐。
- 读取旧版（`version: 1`）`peers.json` 时，明文 `token` 会立即在内存中替换为摘要，下一次写入（节点启动时即会写入）后磁盘上不再有明文。
- 兼容性：迁移得到的授权使用空盐，因此摘要恰好等于既有任务、执行与项目复制账本中已经保存的 `SHA-256(token)` 绑定，旧作业和回执继续可见。想得到加盐摘要，撤销后重新授权即可（会更换令牌）。
- 调用端保存的令牌文件不变；出站路由仍读取同一 `token_file`。

## 签名记录

- 写入配置时，目标节点用 Ed25519 私钥签名每条授权（覆盖调用方 NodeID、权限、摘要等全部字段），并把自身 `identity` 写入配置，`version` 升为 2。
- 版本 2 的配置中，未签名、签名无效或身份字段被删除的授权会使配置读取失败（已有数据保留，不自动修复），从而阻止只改 `peers.json` 就提升权限。
- 签名只能防御“能写 `peers.json` 但拿不到 `node-identity.json`”的篡改；拿到私钥文件的本机攻击者仍可以伪造记录。

## 兼容性与回滚

- 升级后首次启动会把配置写成版本 2。旧二进制无法读取版本 2（`deny_unknown_fields`），因此**不支持降级**；如需回退，请先备份 `peers.json`。
- NodeID 不变，已有路由、授权和账本继续生效。
- `/api/v1/mesh/identity` 的响应格式未改变，旧客户端不受影响。

## 威胁模型（阶段 1）

| 威胁 | 处理 |
| --- | --- |
| 磁盘上的 `peers.json` 泄露 | 只含加盐摘要，无法直接冒用；旧迁移授权为无盐 SHA-256，令牌本身是 256 位随机值，不可暴力破解 |
| 篡改 `peers.json` 提升权限 | Ed25519 签名校验失败，配置被拒绝 |
| 身份文件被替换 | 不会被静默覆盖；已固定该密钥的对端会拒绝新密钥 |
| 本机私钥泄露 | 不在防护范围内；需要撤销所有授权并重新配对 |
| 网络窃听/中继查看内容 | 由阶段 2 的端到端加密处理 |

## English summary

- Every node has a persistent Ed25519 signing key plus an X25519 Noise key (`node-identity.json`, 0600, never overwritten). New NodeIDs are `node-` + the first 128 bits of SHA-256 over the public key, the same shape as legacy IDs. Legacy random IDs are kept and bound to the key by a self-signature.
- `peers.json` stores only `SHA-256(salt ‖ token)`. Version 1 plaintext tokens are migrated on read with an empty salt, so the digest equals the `SHA-256(token)` binding existing ledgers already store. Re-grant to get a salted digest.
- Grants are signed by the target node and the config moves to version 2. Unsigned or tampered grants are rejected. Downgrading to an older binary is not supported, so back up `peers.json` first.
- Not covered: an attacker holding the local private key file, and transport confidentiality, which Phase 2 adds.
