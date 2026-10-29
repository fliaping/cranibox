# 远程控制的安全：操作审计与分级批准 / Remote control security

> 状态：第一部分已实现（2026-10-29，北京时间）。建立在[设备访问控制](../device-access-control.md)第一阶段之上，
> 管理员签名策略见 [`admin-access-policy.md`](../admin-access-policy.md)。

**English summary.** Every remote operation that reaches a device is now described by that device
(kind, risk tier, redacted summary), written to its own hash-chained audit log with the outcome,
and shown in the approval prompt. Commands that destroy data, escalate privileges, persist access or
fetch-and-run code, and uploads into key or login files, always need a one-time approval on the
target, even when Exec is Allow. Approvals can be revoked one by one. Reads are coalesced and peers are
rate-limited so the log cannot be flooded. Redaction and the dangerous-command list are a speed bump,
not a boundary: Exec in Allow mode still means "this device may run anything I can run".

## 1. 问题

第一阶段的访问控制解决了「谁可以用哪项能力」，但有三处不够：

1. **看不到对方做了什么。** 审计只记录能力级事件（会话开始、结束、批准），没有「执行了哪条命令、
   写了哪个文件、停了哪个空间、结果如何」。
2. **批准时不知道在批准什么。** 弹窗只写「MacBook 请求使用 执行命令」。用户无法区分 `ls` 和 `rm -rf ~`。
3. **允许就是全部允许。** 执行命令设为「允许」后，任何命令都直接运行，包括删除整个主目录、`sudo`、
   往 `~/.ssh/authorized_keys` 写公钥。

## 2. 威胁模型

| 威胁 | 例子 | 本次之前 | 现在 |
|---|---|---|---|
| 已配对设备失陷 | 笔记本被盗或中了木马，用它控制家里的电脑 | 只要能力是「允许」，静默执行任何操作，只留下能力级记录 | 每个操作在目标机留痕；破坏性命令和关键文件写入必须当场批准 |
| 误操作 | 智能体在错误的设备上执行 `rm -rf build` 的变体 | 直接执行 | 递归删除需要目标机确认 |
| 批准被滥用 | 批准了 8 小时终端，期间对方做了别的事 | 只能全部断开 | 能逐条撤销；危险操作不被限时批准覆盖 |
| 社会工程 | 对方在请求里塞「请点允许」之类文字 | 请求方文字不进入弹窗（已有） | 弹窗内容由目标机根据请求体生成，类型名来自固定表，请求方只能影响已脱敏的摘要 |
| 日志被冲掉 | 失陷设备每秒发大量请求，把旧记录挤出 1 万条上限 | 不会，因为没记操作 | 只读操作 10 分钟合并一条，拒绝 1 分钟合并一条，每台设备每分钟最多 240 个非只读操作（超出返回 429） |
| 敏感信息进日志 | 命令行里带令牌、密码 | — | 摘要脱敏，见 §4.3 |
| 本机管理员改日志 | 有本机 root 权限的攻击者 | 链式哈希可发现中间篡改，但能整体重写 | 不变；见 §6 |

不在本文范围：管理 UI 本身的认证（见安全审查）、拥有者设备之间默认全信任（见 `admin-access-policy.md`）、
传输层加密（已由 iroh + Noise 端到端提供）。

## 3. 参考：OpenClaw 的远程节点

OpenClaw（[`openclaw/openclaw`](https://github.com/openclaw/openclaw)，读的是 `4e472c7e`）的网关可以
调用远程节点上的命令。和我们相关的做法：

- **配对和命令面分开。** 设备配对只建立身份；节点声明的命令要另行批准，未批准前一个命令也没有
  （`docs/nodes/command-policy.md`、`pairing-and-status.md`）。
- **按平台的默认白名单，危险命令要显式写进配置。** `gateway.nodes.commands.allow` 才能打开危险命令，
  `deny` 永远优先。
- **执行批准在执行的机器上判定**（`docs/tools/exec-approvals.md`）：模式有 deny / allowlist / ask / auto /
  full；`ask` 可以是 off / on-miss / always；没人回答时 `askFallback` 默认 deny；调用方和执行方取更严的
  那个。「允许一次」只覆盖这条命令，「总是允许」把它加入白名单；批准绑定完整 argv、cwd、环境变量和
  可执行文件的真实路径或哈希。`strictInlineEval` 让 `python -c`、`node -e` 这类内联代码总要批准。
  待批请求 30 分钟过期，先回答者生效。
- **能批准的人要单独授权。** 解析批准需要 `operator.approvals` 作用域，文档把它视为与远程执行同级的权限。
- **审计只记元数据。** 不记提示词、参数、输出、路径；标识符用安装本地的 HMAC 化名；文档明确说审计
  记录**不是**授权凭据（`docs/gateway/audit.md`）。
- **令牌有期限。** 引导令牌 10 分钟过期，设备令牌可轮换，`nodes remove` 撤销角色。

### 3.1 我们取了什么、没取什么

| 方面 | OpenClaw | Cranibox 的决定 | 理由 |
|---|---|---|---|
| 判定位置 | 执行机 | 目标机（已是如此） | 一致 |
| 默认 | 未批准即无命令 | 新设备默认关闭/询问（第一阶段） | 一致 |
| 危险命令 | 白名单 + 显式开启 | 「允许」之上再加一层：危险模式**每次**确认，且只能「允许这一次」 | 我们面向普通用户，不让他们维护命令白名单；但必须挡住最常见的破坏和持久化手段 |
| 批准粒度 | 绑定 argv/cwd/env/可执行文件 | 「允许本次」绑定整个请求体的 SHA-256（已有），危险操作强制此粒度 | 已经比 argv 更严：任何字段变化都要重新批准 |
| 批准作用域 | 单独的 approvals 作用域 | 只有本机界面或本机 CLI 能批准，网络接口不能（已有，测试覆盖） | 一致 |
| 审计内容 | 只有元数据，HMAC 化名 | **记录脱敏后的操作摘要**（命令、路径、空间名），只留在目标机本地 | 用户要能看懂「对方在我电脑上做了什么」，只记元数据对普通用户没有意义；日志不出本机、权限 0600 |
| 审计用途 | 不是授权凭据 | 同样：审计只用于事后查看，授权只看策略文件 | 一致 |
| 过期 | 待批 30 分钟 | 待批 120 秒（已有），被拒后冷却 60 秒 | 普通用户的批准发生在眼前，短期限更安全 |

## 4. 设计

### 4.1 操作描述

目标机在 `access::enforce` 中，读完（有上限的）请求体后调用 `access::operations::describe`，得到：

```json
{"kind":"exec.run","risk":"danger","summary":"rm -rf /home/u/build  (cwd /home/u)","confirm":true}
```

- `kind`：固定的机器名，如 `exec.run`、`files.write`、`spaces.stop`、`terminal.create`。界面用它查中文名，
  请求方无法自定义标题。来自请求体的子类型只保留 `[A-Za-z0-9_-]`，最长 32 字符。
- `risk`：三档。
  - `read` 只读：列表、状态、检查更新、读消息。
  - `change` 修改，可恢复：发消息、停止或启动虚拟空间、重命名或结束终端、使用模型。
  - `danger` 高风险：运行命令、读写文件、新建或连接终端、看与控制屏幕、控制浏览器、导入导出项目、
    运行智能体、安装或回滚更新、使用账号凭据。
- `summary`：脱敏后的一行说明，最长 240 个字符。
- `confirm`：见 §4.4。

上传、下载按 4 MiB 分块，只有第一块（offset 0）记一条，后续块不重复记录。

### 4.2 审计格式

沿用 `access-audit.jsonl`（每行一条 JSON，`prev` 是上一行原文的 SHA-256，超过 1 万条时写入检查点并重新
链接，已有）。新增事件 `operation`，以及三个可选字段：

```json
{"index":812,"at":1793271234,"event":"operation","from":"node-29…","name":"MacBook",
 "capability":"exec","session_key":null,"basis":"allow","result":"ok","prev":"5c1e…",
 "operation":{"kind":"exec.run","risk":"danger","summary":"git status"},
 "status":200,"transport":"e2e"}
```

- `result`：`ok`（< 400）、`refused`（403）、`rate_limited`（429）、`failed`（其他）。
- `basis`：`allow` 表示按「允许」放行，否则是批准请求的 ID，界面显示「经你批准」。
- `transport`：`e2e` 是验证过的端到端通道，`bearer` 是旧版令牌请求，便于发现还在用旧路径的设备。
- `request_created` 事件也带上 `operation`，所以「谁请求了什么、你怎么回答、最后执行结果」能连起来看。
- 旧日志行没有这些字段，哈希按原文计算，升级后校验仍然通过。

写入时机：响应生成之后，在阻塞线程里追加，与策略文件共用同一把文件锁。

**合并与限速**（防止刷日志）：

- 只读操作：同一设备、同一 `kind`，10 分钟记一条。
- 被拒绝或限速：同一设备、同一 `kind`、同一结果，1 分钟记一条。
- 每台设备每分钟最多 240 个非只读操作，超过返回 429 并记一条 `rate_limited`。按这个上限，把 1 万条
  记录挤满也要 40 分钟以上，且期间每一条都是可见的异常。

### 4.3 脱敏

在写入和展示之前完成，规则是手写的（不引入正则库）：

- 命令：程序名和参数逐个处理。
  - `--token x`、`--password x` 这类参数名之后的值 → `***`。
  - `NAME=value`、`name:value`，名字含 pass/pwd/token/secret/key/auth/credential/cookie/session/signature/private → `NAME=***`。
  - `Authorization: Bearer x`、`Basic x` → 保留方案名，值换成 `***`。
  - URL 中的 `user:password@` → `***@`。
  - 常见密钥前缀（`sk-`、`ghp_`、`github_pat_`、`glpat-`、`xoxb-`、`AKIA`、`AIza`、JWT 的 `eyJ`…）→ `***`。
  - 32 个字符以上、同时含字母和数字、没有路径分隔符的长串（十六进制密钥、base64）→ `***`。
  - `bash -c "…"` 里的脚本按空白切分后用同样的规则处理。
- 环境变量：**只记名字**，不记值。
- 文件：只记路径，不记内容。
- 消息、任务、模型请求、账号凭据：不记内容。
- 去掉控制字符，超长截断加「…」。

这是尽力而为。脱敏漏掉的东西仍然只在目标机本地，权限 0600；但不应把日志当作可以分享的材料。

### 4.4 危险操作每次确认

`confirm: true` 的操作：

- 即使能力是「允许」，也会创建批准请求；
- 30 分钟、8 小时的限时批准**不覆盖**它；
- 用户在界面或 CLI 选了更长的时长，也会被收窄成「仅本次」；
- 批准绑定这个请求的摘要哈希，用完即失效，第二次执行要再次批准。

目前判定为 `confirm` 的：

- 命令（看完整命令行，包括 `sh -c` 里的脚本）：
  - 递归删除：`rm -r/-rf`、`rmdir /s`、`rd /s`、`del /s`、`Remove-Item -Recurse`；
  - 磁盘：`mkfs*`、`fdisk`、`parted`、`wipefs`、`shred`、`diskpart`、`diskutil`、`format`、写设备的 `dd of=`；
  - 关机：`shutdown`、`reboot`、`poweroff`、`halt`、`Stop-Computer`、`Restart-Computer`；
  - 提权或账号：`sudo`、`su`、`doas`、`runas`、`pkexec`、`visudo`、`passwd`、`useradd`、`usermod`；
  - 持久化：`crontab`、`launchctl`、`schtasks`、`systemctl enable/disable/mask/stop`、`reg add/delete`，以及命令中
    出现 `authorized_keys`、`.bashrc`、`.zshrc`、`.profile`、`LaunchAgents`、`/etc/sudoers`、`/etc/systemd`、
    自启动目录等；
  - 下载并运行：`curl`/`wget`/`iwr` 加管道到 `sh`/`bash`/`python`/`node`/`pwsh` 等，以及 `Invoke-Expression`/`iex`；
  - 其他：递归 `chmod`/`chown`、`docker rm/rmi/prune`、`killall`、`pkill`。
- 上传文件到：`~/.ssh/`、shell 启动文件、`~/Library/LaunchAgents`、`/Library/LaunchDaemons`、
  `~/.config/autostart`、`~/.config/systemd`、`/etc/`、Windows「启动」文件夹、`~/.gnupg/`、`~/.cranibox/`、
  `/usr/bin`、`/usr/local/bin`。

**这不是安全边界。** 能运行命令的人可以把命令包装成看不出来的样子（写脚本再执行、base64 解码等）。
它挡住的是误操作和照本宣科的攻击；真正的边界仍然是「执行命令」这项能力本身是关闭、询问还是允许。
界面里「允许」的警告文字保持不变。

### 4.5 界面

- **批准弹窗**说清楚哪台设备要做什么：「MacBook 想在本设备上运行命令」，下面是等宽字体的摘要和风险标签
  （只读 / 修改 / 高风险）。
  - 危险操作只有「拒绝」和「允许这一次」，并写明「这个操作可能删除数据、获得管理员权限或长期留下访问入口，
    所以每次都要你确认……不认识就拒绝。」
  - 普通操作说明限时批准的含义：「『允许本次』只放行这一个操作；选择 30 分钟或 8 小时后，对方在这段时间内
    可以继续使用『执行命令』而不再询问。」
  - 「拒绝」仍是默认焦点，Esc 不能关闭。
- **当前授权**（设备详情的「访问」页）：列出该设备的每条有效批准（能力、仅本次或有效期、是否正在使用），
  每条都可以「撤销」；同时列出「始终允许」的能力并说明如何收回。
- **操作记录**（原「访问记录」，设备详情页和「设备 → 操作记录」）：远程操作显示设备、操作名、摘要、
  结果（已完成 / 被阻止 / 过于频繁 / 失败）、风险、是否经你批准；可按设备、能力、类型（远程操作 / 批准与设置）筛选；
  顶部显示链是否完整。
- **无界面的设备**：`cranibox access list` 在待批请求下打印 `wants to: exec.run [danger, asks every time]: rm -rf …`，
  `cranibox access log` 在每条操作下打印摘要和状态码。

## 5. 实现位置

| 部分 | 文件 |
|---|---|
| 契约：`Risk`、`Operation`、`EventKind::Operation`、可选字段 | `crates/cranibox-contracts/src/access.rs` |
| 描述、分级、脱敏、危险判定 | `crates/workspace-node/src/access/operations.rs` |
| `authorize_for` / `start_for`（`once`）、`record`、批准收窄 | `crates/workspace-node/src/access/mod.rs` |
| 中间件：描述、限速、记录结果、空间名 | `crates/workspace-node/src/peer/server/access.rs` |
| CLI 输出 | `crates/workspace-node/src/access/cli.rs` |
| 界面 | `addons/control-center/ui/src/access/{Access.tsx,types.ts,access.css}` |
| 测试 | `access/operations.rs`（脱敏、分级、危险判定），`peer/server/multi_node.rs` 的 `operations_are_audited_…`（端到端：审计、脱敏、一次性批准、撤销、拒绝冷却），`ui/tests/access.spec.ts`（弹窗、操作记录、逐条撤销） |

## 6. 仍然存在的风险与后续建议

需要决定的事项（未实现）：

1. **「允许」没有期限。** 建议给 terminals/exec/desktop/computer/browser 的「允许」加可选期限（例如 7 天后
   自动退回「询问」），或在设置页提示「已允许 N 天」。
2. **旧版令牌（bearer）请求。** 未验证身份的旧授权仍可调用；日志里的 `transport: bearer` 可以帮助发现，
   建议在所有设备升级后停用。
3. **日志只在本机。** 拥有本机管理员权限的攻击者可以重写整条链。可选的后续：把每条记录的哈希定期发给
   拥有者的另一台设备（第二阶段复制），或发给调用方留底（调用方自己的「我做过什么」记录）。本次没有做
   调用方记录，因为调用方的记录可被调用方伪造，对目标机的用户没有证明力。
4. **文件读取。** `files.read` 属于执行命令能力，能读出任何文件（包括密钥）。它记为高风险并留痕，但不会每次询问。
   如果希望读 `~/.ssh`、浏览器配置等也每次确认，可以把 `sensitive_path` 应用到读取。
5. **危险命令清单的误报。** 例如 `rm a.txt && grep -r foo` 因为同时出现 `rm` 和 `-r` 会要求确认。
   宁可多问一次；如果智能体自动化因此卡住，用户可以在目标机上批准，或改用不触发规则的写法。
6. **限速阈值。** 240 次/分钟是保守估计，大量自动化（例如智能体循环执行短命令）可能触及。
7. **命令行调用方不等待批准。** `cranibox exec` 收到 `approval_required` 后立即退出；目标机批准后需要
   原样重新执行同一条命令（参数、目录、超时都相同，否则请求内容不同，会再次询问）。界面会等待
   并自动重试。可以考虑让命令行也等待批准。

## 7. 验证

```bash
cargo test --locked -p cranibox-node access
npm run test:access --prefix addons/control-center/ui
```

两台无界面节点上的手工验证：调用方 `cranibox exec --node <目标> -- …`，目标机上用
`cranibox access list` 看待批请求和操作说明，`cranibox access approve|deny|revoke` 回答，`cranibox access log`（每次都会校验整条链）
检查记录与脱敏；目标机界面「设备 → 操作记录」应显示同样的内容。
