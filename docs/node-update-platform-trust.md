# 远程更新的平台信任实验 / Platform trust experiments for remote update

> 实验日期 2026-10-24。目的：确认节点自己下载并启动新版本时，Windows SmartScreen 与 macOS Gatekeeper 是否会弹窗拦截。
> 结论只覆盖下面列出的环境与做法；“未验证”的部分没有结论。

## 环境

| 节点 | 系统 | 用户 | 现有安装 |
| --- | --- | --- | --- |
| DESKTOP-HOME | Windows 11 专业工作站版 build 26300，x64 | 普通用户（非管理员） | `%LOCALAPPDATA%\Cranibox\`（按用户安装，不需要 UAC）；Smart App Control 状态 0（关闭） |
| xuping 的 MacBook Pro | macOS 27.0.1，arm64，Gatekeeper 开启 | 管理员组成员 | `/Applications/Cranibox.app`，自签名证书 `Cranibox Self-Signed Code Signing`，`spctl` 评估为 rejected，带 `com.apple.quarantine` |

Mac 上现有的应用本来就是手动放行后运行的，所以它是“被拦截后放行”的例子。

## 方法

通过 `workspacectl node exec` 在节点上运行，父进程就是 Cranibox 节点本身，与未来的更新进程一致。
测试程序是现场编译的无害小程序（Windows 用 .NET 编译为无签名 exe，Mac 用 `cc` 编译，链接器自动 ad-hoc 签名，
与 Rust 在 arm64 上的产物一致），在专用临时目录里运行并在结束时清除。

下载使用本机回环 HTTP：Windows 用 `curl.exe` 和 `Invoke-WebRequest`，Mac 用 `curl` 和 `tar -xzf`（模拟 npm 平台包的解压）。

## 结果

### Windows

| 项目 | 结果 |
| --- | --- |
| 程序内下载的 exe 是否带“来自网络”标记（`Zone.Identifier`） | 没有（`curl.exe`、`Invoke-WebRequest` 都只有 `:$DATA` 流） |
| 无签名、无信誉的 exe 运行 | 直接运行，退出码正确，15 秒内无任何阻塞 |
| 覆盖正在运行的 exe | 被拒绝（`IOException`） |
| 把运行中的 exe 改名、在原路径放入新文件、启动新文件 | 可行 |
| 旧进程退出后改名回滚 | 可行 |
| 普通用户、非提权 | 以上全部在此条件下完成 |

SmartScreen 判断依赖 `Zone.Identifier`。浏览器下载的文件会有，所以才出现“Windows 已保护你的电脑”。
我们自己写入的文件没有，实验里没有出现任何提示。

### macOS

| 项目 | 结果 |
| --- | --- |
| `curl` 下载的文件 | 没有 `com.apple.quarantine`，只有信息性的 `com.apple.provenance` |
| `tar -xzf` 解压得到的文件 | 同上 |
| 运行 ad-hoc 签名但未公证的下载文件 | 直接运行。`spctl --assess` 报 rejected，但这只是评估，不会拦截没有隔离标记的执行 |
| 去掉签名后运行 | 被内核杀掉（退出码 137）。arm64 要求至少有 ad-hoc 签名 |
| 去掉签名后用 `codesign -s -` 重签 | 恢复正常 |
| 改名运行中的二进制并在原路径放入新文件 | 旧进程不受影响，新文件运行 |
| 整个 `.app` 目录改名，再移入新 bundle | 旧进程不受影响，新 bundle 运行；回滚（改名换回）可行 |
| `open -n` 启动无隔离标记的 ad-hoc `.app` | 进程启动，没有拦截弹窗 |

## 结论

1. **由节点程序自己下载、解压、写入的文件，在两个平台上都不会触发“仍然运行”的拦截。**
   弹窗只针对带 `Zone.Identifier` / `com.apple.quarantine` 的文件，即浏览器或访达下载的文件。
2. 因此更新方案成立的前提是：**所有新代码都由节点程序用 HTTP 客户端下载并自己解压，不经过浏览器，不使用 `open` 打开 dmg，也不让用户双击安装包。**
3. macOS arm64 的二进制必须带签名，至少 ad-hoc。Rust 的 arm64 构建默认就有，解压和复制不会破坏它；
   但**任何会修改二进制内容的步骤（strip、patch）之后都必须重新签名**。更新程序应当在启动前运行 `codesign --verify`，失败则不切换版本。
4. 替换方式必须是“改名再放入新文件”，不能原地写入：Windows 禁止覆盖运行中的 exe；
   macOS 在原地写入签名过的二进制时有被内核判定签名失效的风险，本次短时间观察没有出现，但不能当作安全。
5. 两个平台的回滚都可以通过改名完成，不依赖安装器。
6. 首次安装外壳时仍会遇到一次“仍然运行”，这是不可避免的，之后的更新都不会再遇到。
   macOS 目前自签名且未公证，要消除首次提示需要 Apple Developer ID 与公证；Windows 需要 Authenticode 证书。这是产品分发的问题，不影响更新本身。

## 未验证，不能当作结论

- **Windows Smart App Control 开启的机器。** 本机是关闭状态（0）。SAC 开启时会按签名和云信誉拦截未签名程序，
  与 `Zone.Identifier` 无关，可能拦截我们下载的 exe。需要找一台开启 SAC 的 Windows 11 实验，或给二进制做 Authenticode 签名。
- **企业策略：** AppLocker、WDAC、杀毒软件的实时扫描和云信誉查询（Defender 对全新未知二进制的行为）。本次只用了很小的测试程序，
  没有用真实的 Cranibox 二进制，因此没有触发 Defender 的启发式检测。
- **macOS 的 App Management 权限（TCC）：** 一个进程替换 `/Applications` 下的另一个应用 bundle 可能弹出权限请求。本次没有碰真实的 `Cranibox.app`，
  这也是不修改已安装 bundle、把运行时放到用户目录的另一个理由。
- **带隔离标记的文件**，故意没有运行，避免在你的桌面上弹出系统对话框。
- **真实 Cranibox 二进制**（npm 平台包尚未发布）。需要在首个可用的包上重复以上实验。
- macOS 的 Intel 机型与旧系统版本。

## 对架构的影响

- 这些结果支持“外壳 + 用户目录里的运行时”：运行时目录由节点自己写入，不会被拦截；外壳只需要用户放行一次。
- 更新流程的验证步骤：下载 → 校验 npm `dist.integrity` → 解压 → macOS 上 `codesign --verify` → 改名切换 → 启动 → 确认，失败则改名回滚。
- CI 的平台自测要复现这里的步骤（用真实包），并增加 Windows Defender 开启的 runner 与 macOS runner。

## 补充实验：真实 v1.1.0 二进制与已有方案（同日）

| 项目 | 结果 |
| --- | --- |
| Windows，真实 `cranibox-windows-x86_64.exe`（47.9 MB） | 用 `curl.exe` 下载 7 秒；无 `Zone.Identifier`；无 Authenticode 签名；Defender 实时防护开启；运行 `--version` 输出 `cranibox 1.1.0`；Defender 检测数 0 |
| macOS，真实 `cranibox-macos-arm64`（34.8 MB） | `curl` 下载，sha256 与发行版 `SHA256SUMS` 一致；无隔离标记；`codesign --verify` 通过；运行 `--version` 正常 |
| macOS 真实二进制的签名 | `adhoc,linker-signed`，**指定要求（designated requirement）就是该文件的 cdhash**。任何绑定到这个二进制的 TCC 授权，在更新为新二进制后都会失效 |
| macOS 现有 `Cranibox.app` 的签名 | 指定要求为 `identifier "com.cranibox.node" and certificate root = H"703f…"`，更新后保持稳定 |
| `Cranibox.app` 的 `LSFileQuarantineEnabled` | 未设置。应用自己写入的文件不会被打隔离标记 |
| 子进程的 TCC 归属 | **未得出结论。** 从节点派生的子进程查询屏幕录制/辅助功能均为 0，但无法确认该应用本身是否被授予过这些权限 |

公网资料（均为搜索摘要，未逐篇核实原文）：

- Apple 开发者论坛：ad-hoc 签名没有稳定的指定要求，系统无法识别新版本与旧版本是同一代码，更新后屏幕录制等授权丢失；建议使用稳定的签名身份。
- Smart App Control 按签名与云信誉判断，不只看 Mark of the Web；已有案例：更新后下发的未签名可执行文件被 SAC 拦截。
- 同类方案：Velopack / Squirrel.Windows 使用每用户目录下的版本化文件夹加启动存根；`self-replace` / `self_update` 提供“运行中可执行文件改名替换”的原语；
  Tauri 官方 updater 需要更新签名密钥并替换整个应用包；Tailscale 的自动更新是客户端侧设置，管理控制台只对新设备生效，
  每个版本观察约一周后才放开自动更新。
