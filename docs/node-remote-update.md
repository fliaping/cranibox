# 节点远程更新 / Remote node update

> 状态：设计稿（2026-10-24 修订）。平台实验见 [node-update-platform-trust.md](node-update-platform-trust.md)，
> 底座与上层的拆分评估见 [node-base-layer-split.md](node-base-layer-split.md)。

## 目标

用户的任意一台设备（macOS、Windows、Linux 桌面，无桌面服务器）都可以更新网络里的其他设备，也可以更新自己，
不需要登录目标机器手动安装。开发时：提交代码、CI 发布开发构建，在任一台设备上一条命令（或界面上点一下）
推给所有设备，然后在各平台直接验证。

## 授权：没有“管理员节点”

管理员就是用户本人，也就是 owner 身份。roster（owner 签名的设备名单）里的每台设备都是 owner 设备，
任何一台都可以更新其他节点或自己。判断条件只有一个：调用方在 roster 中（现有的 `is_owner_node`）。

- 访客配对的设备（只有消息权限）一律拒绝。
- 不在 grant 的权限位里新增 `update`：grant 可以单独授予 peer，容易误授权。
- 可选开关（默认关闭）：某台设备设为“远程更新需本机确认”。
- 每次更新的发起者、版本、结果写入活动记录。

更新是权限最高的操作（等于在对方机器上运行新代码），所以只走已有的 owner 授权通道，并复用 E2E 会话的授权复查。

## 分发：只用 npm

- 版本元数据和安装包都来自 npm registry。国内默认 npmmirror，海外 npmjs，可用 `CRANIBOX_NPM_REGISTRY` 覆盖，
  镜像缺版本（同步延迟）时自动换下一个。
- 目标节点用 HTTP 直接读 registry，**不需要 Node.js 或 npm**：读 packument，下载 `dist.tarball`，
  用 `dist.integrity`（sha512）校验，再自己解压。
- 通道：`latest`（正式）、`next`（预发布）、`dev`（CI 每次构建发布 `x.y.z-dev.N`）。
- 不防御 GitHub / npm 被劫持，完整性只靠 `dist.integrity`。GitCode 渠道不做。
- 推送（管理员设备把包通过 mesh 发给离线节点）暂不做，有需要再加。

## 架构：外壳 + 运行时，底座 + 上层

### 运行时目录

```
<data>/runtime/
  versions/<version>/       每个版本一个目录（解压后的 npm 平台包）
  current.json              指针：current / previous / state(pending|confirmed) / attempts / failed
  layout.lock               跨进程锁
```

- 更新 = 下载到 `<version>.staging`，校验，改名为 `<version>`，原子改写 `current.json`。
  永不覆盖正在运行的文件（Windows 禁止，且实验证明改名是可行的）。
- 新版本启动后必须在窗口期内“确认”（网络已重新连上），否则由本机的守护 / 启动器按指针自动回滚，
  回滚不依赖网络。崩溃循环同样触发回滚。已失败的版本不会被再次自动激活。
- 指针是 JSON，不用符号链接（Windows 需要权限）。

### 桌面应用

安装一次的外壳（Tauri：托盘、自启、窗口）只负责启动 `current` 指向的运行时，界面来自运行时提供的本地网页。
之后的更新不再经过安装器，也不再碰 `/Applications` 或 `Program Files`。外壳只在托盘 / 窗口代码变化时才需要重装。
无桌面服务器同样的目录布局，由 systemd / launchd / 任务计划启动一个很小的启动器。

### 底座与上层

- **底座**（极少变化，不随意重启）：对等网络连接、授权与身份、远程终端、远程桌面（VNC / Selkies / macOS 屏幕共享）、Computer Use 基础。
  它是兜底：需要输入密码或点击系统弹窗时，仍可通过终端和远程桌面完成。
- **上层**：任务、消息、项目同步、空间、模型代理、Agent、界面等。日常更新（包括开发构建）只重启上层，
  远程终端和桌面会话不断。
- 替换底座是单独的、更严格的流程：保留旧版本，新版本必须重新连上网络并确认，否则本机守护进程自动换回旧版。
- 先做“整体更新 + 自动回滚”，再按拆分评估的结果把底座独立出来。

### 底座与上层互相拉起、互相更新

两个进程各有自己的运行时目录和指针（`runtime/base/`、`runtime/upper/`，各自的 `current.json`），互为守护：

| | 谁来做 | 怎么做 |
| --- | --- | --- |
| 日常更新上层 | 上层自己请求，底座执行 | 底座下载、校验、切换上层指针，重启上层；上层在窗口期内确认，否则底座回滚并重启旧版 |
| 更新底座 | 上层执行 | 底座本身要被替换，不能自己当守护。由上层下载并校验新底座，切换底座指针，拉起新底座；新底座在窗口期内确认（网络已连上），否则上层把指针切回并拉起旧底座 |
| 一方异常退出 | 另一方拉起它 | 带退避和次数上限（沿用 `attempts` / `failed`），超限则回滚到上一个已确认版本 |
| 两方都不在 | 操作系统层 | 开机 / 登录项只启动底座（Tauri 外壳、systemd、launchd、任务计划都指向底座）；底座再启动上层 |

不变量（实现时必须保证，并用测试覆盖）：

1. **同一时刻只更新一方。** 更新底座时上层必须在线且已确认；更新上层时底座必须在线且已确认。守护方不在线就拒绝更新。
2. **执行者在更新期间不换。** 上层更新底座的过程中，上层本身不能被更新或重启；底座更新上层同理。
3. **回滚不依赖网络。** 旧版本已在本机，指针切换和拉起是本地操作。
4. **确认要有实质内容。** 底座的确认条件是网络已重新连上并能响应 mesh 请求；上层的确认条件是能通过底座的接口完成一次握手。
5. **互相拉起不能互相拖垮。** 重启有退避；失败版本记入 `failed`，不再被自动激活。
6. **上层只能通过底座的接口更新底座，且要求 owner 授权。** 远程发起的“更新底座”由底座验证授权后转交给上层执行，不是上层自行决定。
7. **更新底座会让会话中断。** 终端、桌面、Noise 会话要重新建立。因此底座更新是单独的、需要明确发起的操作，默认不随上层更新一起做。

原型验证（Linux、macOS、Windows 真机，见 [validation/2026-10-24-mutual-supervision](validation/2026-10-24-mutual-supervision/README.md)）：
互相拉起约 1.4–2 秒；双向更新、超时回滚、崩溃循环回滚都通过；两方同时被杀时确实无人能拉起，必须靠操作系统层。
**Windows 的限制：** 经节点 `exec` 启动的进程会在该次调用结束时被一并终止，即使使用了脱离作业的标志。
所以底座必须由开机项 / 外壳 / 服务管理器启动，而不是由节点 `exec` 启动；用系统服务（WMI）启动则全部通过。

仍需验证：开机项 / 计划任务 / launchd / systemd 对底座的拉起，Tauri 外壳启动底座时的作业环境，以及重启一方时对另一方已有网络、终端、桌面连接的影响。

## 平台信任（实验结论）

由节点自己用 HTTP 客户端下载并解压的文件，在 Windows 和 macOS 上都不会触发 SmartScreen / Gatekeeper 的“仍然运行”
（无 `Zone.Identifier`，无 `com.apple.quarantine`），真实的 v1.1.0 二进制也已验证。约束：

- 所有新代码由节点程序自己下载，不经过浏览器，不 `open` dmg，不让用户双击安装包。
- 替换用“改名后放入新文件”，不原地覆盖。
- macOS 在切换前运行 `codesign --verify`；arm64 二进制必须带签名。
- macOS 的授权（TCC）绑定签名身份：ad-hoc 签名的指定要求是文件哈希，更新后授权会失效。
  外壳保持固定证书签名；运行时二进制也应使用同一固定证书签名。
- Windows：Smart App Control 开启的机器会按签名与信誉静默拦截未签名程序，没有按钮可点。
  更新前读取其状态，开启则报告“需要签名”，不半途失败。

未验证的项目见 [node-update-platform-trust.md](node-update-platform-trust.md)。

## 协议（mesh，仅 owner 设备）

```
GET  /api/v1/mesh/nodes/{node}/update             版本、build、平台、形态、可否自更新及原因、进行中的任务
POST /api/v1/mesh/nodes/{node}/update             {spec: 版本或通道, force?}   开始更新
GET  /api/v1/mesh/nodes/{node}/update/{id}        进度：resolving → downloading → verifying → staged → activating → restarting → done | failed | rolled_back
POST /api/v1/mesh/nodes/{node}/update/{id}/cancel
POST /api/v1/mesh/nodes/{node}/update/rollback
```

重启会断开连接，状态持久化在 `<data>/update/state.json`，节点回来后由发起方读取最终结果；
以节点用新版本重新响应 `GET …/update` 作为成功的依据。

## 降级与版本

版本按 SemVer 比较；`1.2.0-dev.9 < 1.2.0-dev.17 < 1.2.0`。默认拒绝降级，显式 `force` 才允许。
同一版本重复下发是幂等的。

## 管理入口

- CLI：`cranibox update status|check|apply --node <id>|--all --to <版本或通道>|rollback --node <id>`。
- 控制中心节点页：版本列、可更新标记、多选更新、实时进度、失败原因、回滚。
- 更新前显示提醒：该节点是否有独立的远程桌面兜底（例如目前 Windows 节点没有）。
- 批量更新时先更新一台、确认后再更新其余；更新自己放在最后。

## 开发 / 测试闭环

1. CI 构建各平台，向 npm 发布 `x.y.z-dev.N`，`dev` 通道指向最新。
2. 开发者在任一设备执行 `cranibox update apply --all --to dev`。
3. 各节点下载、校验、切换、重启、确认；失败则自动回滚。
4. 开发者在各平台验证功能。
5. CI 在 Linux / macOS / Windows runner 上起两个节点，A 把 B 更新到新版本，断言版本、重启后可达，
   再发布一个故意启动失败的构建，断言自动回滚。

## 已知问题与前置条件

- **运行时产物已补齐：** CI 在 macOS / Windows / Linux 均先构建并保存无 GUI 的独立 CLI，
  再打包桌面应用；npm 平台包使用该 CLI 作为节点运行时。完成 npm 发布配置后，main 的
  开发构建进入 `dev` 通道，设备可执行 `cranibox update apply dev`。
- **引导：** 已安装的旧版本没有更新能力，需要手动安装一次包含本功能的版本。
- Windows 节点目前没有远程桌面兜底（`desktop: false`）。
- 未验证：Smart App Control 开启的 Windows、macOS 子进程的 TCC 归属、macOS App Management 权限。

## 实现状态（2026-10-24）

已实现并在 Linux 上验证：

- **运行时与回滚：** `cranibox update adopt|launch|status|apply|rollback`。启动器按指针启动节点，崩溃退避重启，
  新版本未确认或崩溃循环则本地回滚，不依赖网络；处理 SIGTERM / Ctrl-C。节点稳定运行 20 秒后才确认，
  激活后以退出码 75 请求重启。真实二进制端到端测试：`tests/test_runtime_update.py`
  （接管、启动、经 npm `dev` 标签更新并重启、起不来的版本自动回滚、失败版本不再重试）。
- **远程更新：** mesh 接口 `GET/POST /api/v1/nodes/{node}/update`、`GET …/update/{id}`、`POST …/update/rollback`，
  仅限已核对配对码的 roster 成员且必须走端到端加密通道。两节点集成测试
  `peer/server/multi_node_update.rs`：owner 设备经真实 Noise 通道更新并回滚另一节点，roster 之外的对端被拒绝。
- **分发：** npm 是唯一渠道。CI 在三个平台构建独立的 `cranibox`，主干推送发布 `x.y.z-dev.N` 到 `dev` 标签，
  打 tag 发布正式版（`docs/distribution.md`）。
- **互相拉起协议：** `update/duo.rs` 实现底座与上层互相监督、互相更新、超时回滚、崩溃循环回滚及全部不变量，
  含按角色独立的版本指针；有模拟进程的测试和一个 Linux 真实分离进程测试。
- **桌面外壳的受管模式：** 设备已 `adopt` 时，桌面应用只做外壳（托盘、窗口、自启），内嵌启动器运行受管版本，
  窗口加载其本地界面，原生操作经带令牌的回环通道回调外壳；未 `adopt` 的设备行为与以前完全相同。

尚未实现 / 尚未验证：

- **底座 / 上层的进程拆分。** `duo.rs` 是协议部分，尚未接到真实节点上；节点仍是单进程，
  所以整体更新会重启整个节点，终端和桌面会话会中断。拆分的范围与未知项见 [node-base-layer-split.md](node-base-layer-split.md)。
- **真机验证：** macOS、Windows 上的端到端，桌面外壳受管模式的实际 GUI 行为，macOS 上子进程与 TCC 授权的归属，
  Smart App Control 开启的 Windows，首次 npm 发布和 `dev` 通道的真实 Actions 运行。
- 桌面外壳的托盘“紧急停止”在受管模式下走既有的 `/api/peers` 接口，未在真机验证。

## 分期

1. **运行时机制：** npm 客户端与解压、版本目录与回滚、版本比较（已分给 Codex 实现）；
   mesh 接口、授权、状态机、启动器 / 守护、CLI、CI 自测。
2. **底座拆分：** 按评估结果，让日常更新只重启上层，VNC / 终端会话在更新中保持。
3. **桌面外壳：** 外壳启动 `current` 运行时，界面来自运行时。
4. **体验：** 控制中心界面、批量与分批更新、dev 通道发布流程。
