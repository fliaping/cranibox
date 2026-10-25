# 远程更新：真机验证清单

按顺序做。每一步写明“预期”；与预期不符就停下，记录现象（命令输出、截图），不要硬往下走。
测试设备：Windows 11（DESKTOP-HOME）、MacBook Pro（macOS 27 arm64）、Linux 服务器（无桌面）。

> 命令里的 `cranibox` 是桌面应用自带的命令行。Windows 在“设置 → 桌面应用 → 安装命令行工具”之后可用，
> macOS 同样在该处安装；Linux 的 deb / AppImage 已加入 PATH。找不到命令时，用安装目录里的完整路径。

## 0. 准备

1. 在 GitHub Actions 的这次运行里下载安装包（链接见回复）：
   - macOS：`cranibox-desktop-macos-latest` → `.dmg`
   - Windows：`cranibox-desktop-windows-latest` → `.exe`（NSIS 安装程序）
   - Linux 服务器：`cranibox-cli-linux` → 单个可执行文件 `cranibox-linux-x86_64`
2. 这个构建的版本号是 `1.1.0`（与你现在装的相同）。更新测试用的新版本来自 npm 的 `dev` 通道，
   版本形如 `1.1.1-dev.N`；**如果 npm 上还没有发布过这个包，第 3、4 节的“从 npm 更新”无法做**，
   用第 5 节的本地仓库测试代替（它不依赖 npm）。
3. 配置目录（`runtime` 文件夹就在它旁边）：
   - Windows：`%APPDATA%\cranibox\`
   - macOS：`~/Library/Application Support/cranibox/`
   - Linux：`~/.config/cranibox/`
4. 开始前记录：`cranibox --version`，以及设备上现有应用的版本。**不要删除现有的配置目录**；
   整个测试只新增 `runtime` 文件夹，回退方法见第 8 节。

## 1. 安装新版，确认原有行为没变（未接管）

| 步骤 | 预期 |
| --- | --- |
| 安装新版并打开应用 | 窗口、托盘、设备列表和以前一样；**没有新增任何系统弹窗**（记录首次打开时的“仍然运行”提示，这是首次安装的已知一次性提示） |
| `cranibox update status` | `runtime.managed` 为 `false`，`can_update` 为 `false`，`reasons` 里写明“not started by the cranibox launcher” |
| 终端、桌面、消息等原有功能各用一次 | 与升级前一致 |

## 2. 接管（adopt）并以受管模式运行

> 桌面应用首次启动会自动接管，无需手动操作；下面“退出应用 → adopt”只在关闭了自动接管（`CRANIBOX_NO_MANAGED=1`）时需要。

| 步骤 | 预期 |
| --- | --- |
| 退出应用（托盘 → 退出） | 进程消失 |
| `cranibox update adopt` | 输出 `Adopted 1.1.0 in …\runtime`；`runtime` 文件夹里有 `versions\1.1.0` 和 `current.json` |
| 重新打开应用 | 窗口出现（可能比以前慢几秒）；界面和以前一样 |
| 任务管理器 / 活动监视器 | 能看到应用进程，以及它启动的 `cranibox` 子进程，其路径在 `runtime\versions\1.1.0\bin\` 下 |
| 约 20 秒后 `cranibox update status` | `runtime.managed: true`，`state: "confirmed"`，`version: "1.1.0"` |
| 设置页：开机自启开关、打开外部链接、打开终端窗口、隐藏、退出 | 全部可用（这些经由外壳的本机通道） |
| 托盘上的待审批数量 | 制造一个待审批项，数量变化；点开能处理 |
| 托盘 → “紧急停止”（如已启用 Computer Use） | 生效；如果无反应，记下来（这一项在受管模式下未在真机验证过） |
| 退出应用 | `cranibox` 子进程**随之退出**，不残留 |

**macOS 额外检查（关键，之前没有验证过）：**

1. 给 `Cranibox.app` 授予“屏幕录制”和“辅助功能”（如果还没有）。
2. 受管模式运行时，使用 Computer Use / 桌面共享功能一次。预期可用，**不弹出新的授权请求**。
3. 记录：功能是否正常、是否出现任何授权弹窗、系统设置里列出的是 `Cranibox` 还是 `cranibox` 可执行文件。

## 3. 本机更新（从 npm 的 dev 通道）

前提：npm 上已有 `@fliaping/cranibox` 及对应平台包的 `dev` 版本。

| 步骤 | 预期 |
| --- | --- |
| `cranibox update apply dev` | 打印状态 JSON，`phase: "activated"`，`to: "1.1.1-dev.N"`；随后提示“Activated…” |
| 等待几秒 | 应用窗口**自动重新连接**新版本的界面（可能闪一下），不需要手动重开 |
| 约 20 秒后 `cranibox update status` | `version: "1.1.1-dev.N"`，`state: "confirmed"`，`status.phase: "done"` |
| 全程是否出现任何系统弹窗 | **预期没有**（macOS：“无法验证开发者”；Windows：SmartScreen / “Windows 已保护你的电脑”；杀毒软件拦截）。出现就截图 |
| `cranibox update status` 再来一次 `apply dev` | 被拒绝，提示版本不比当前新 |
| `cranibox update rollback` | 切回 `1.1.0` 并重启；`status` 显示 `1.1.0` confirmed |
| `cranibox update apply 1.1.1-dev.N` | 被拒绝，提示该版本之前失败过，需要 `--force` |

Windows 额外检查：

- 在 PowerShell 运行 `reg query "HKLM\SYSTEM\CurrentControlSet\Control\CI\Policy" /v VerifiedAndReputablePolicyState`。
  值为 0 表示 Smart App Control 关闭；**为 1 或 2 时**，`cranibox update apply` 应当在下载前就报告
  “需要签名”，不应下载后失败。记录实际行为。
- 更新过程中 Defender 是否有任何提示或隔离记录。

## 4. 远程更新（核心功能）

前提：两台设备已配对为同一身份（同一 roster），并已核对配对码。用 `cranibox nodes` 取设备 ID。

| 步骤 | 预期 |
| --- | --- |
| 在 Mac 上：`cranibox update status --node <Windows 的 ID>` | 返回对方的版本、`managed`、`state` |
| 在 Mac 上：`cranibox update apply dev --node <Windows 的 ID>` | 依次打印 `resolving → downloading → verifying → activated`，随后连接中断，重新连上后打印 `done`；**Windows 全程无人操作** |
| 在 Windows 上确认 | `cranibox update status` 显示新版本 confirmed |
| 反向：在 Windows 上更新 Mac | 同样成功 |
| `cranibox update apply dev --all` | 更新所有其他在线设备；逐个报告 |
| 更新**自己**放最后：本机 `cranibox update apply dev` | 成功 |
| 用一台**只是访客配对**（不在 roster）的设备调用更新接口 | 被拒绝 |
| 目标节点未接管（第 1 节状态）时对它发起更新 | 返回 409，说明“cannot update itself”及原因 |

## 5. 本地仓库测试：更新与自动回滚（不依赖 npm）

这个测试自带一个假的 npm 仓库，在临时目录里验证“接管 → 启动 → 更新 → 起不来的版本自动回滚”，
不碰设备上的真实配置。仓库代码在 `tests/test_runtime_update.py`。

```
# macOS / Linux
CRANIBOX_BIN=/path/to/cranibox python3 -m unittest tests.test_runtime_update -v
# Windows (PowerShell)
$env:CRANIBOX_BIN="C:\path\to\cranibox.exe"; python -m unittest tests.test_runtime_update -v
```

预期：`OK`。Windows 上这个测试没有跑过：它用 `CTRL_BREAK_EVENT` 结束启动器，而启动器目前只处理 Ctrl-C，
如果收尾阶段报错但前面的断言都通过，记为“测试脚本需要适配 Windows”，不是更新功能的缺陷。

## 6. 平台信任：之前只验证过小程序，这里用真实构建

| 设备 | 检查 | 预期 |
| --- | --- | --- |
| macOS | `codesign --verify --strict runtime/versions/<版本>/bin/cranibox` | 通过（任意签名，含 ad-hoc） |
| macOS | `xattr runtime/versions/<新版本>/bin/cranibox` | 没有 `com.apple.quarantine` |
| macOS | 更新前后，Computer Use / 屏幕录制授权是否保留 | **记录**：ad-hoc 签名的更新版本可能让授权失效，需要重新授权 |
| Windows | `Get-Item runtime\versions\<新版本>\bin\cranibox.exe -Stream *` | 只有 `:$DATA`，没有 `Zone.Identifier` |
| 两者 | 更新后可执行文件能直接运行、无弹窗 | 是 |

## 7. 失败场景（可选，需要能发布到 npm 的人配合）

在一个**自己的测试包名**（设环境变量 `CRANIBOX_NPM_PACKAGE`）下发布一个起不来的版本，对一台已接管设备执行 apply：

- 预期：设备自动回到上一个版本（`status.phase` 变为 `rolled_back`，`state: confirmed`），期间不需要网络。
- 不要在正式的 `@fliaping/cranibox` 下发布起不来的版本。

## 8. 还原到未接管状态

1. 退出应用。
2. 删除配置目录里的 `runtime` 文件夹（只删它，不要动 `settings.json`、`peers.json`、身份文件）。
3. 重新打开应用：恢复为原来的单进程行为。

## 9. 需要带回来的信息

每台设备一份：

- 系统版本、`cranibox --version`、安装包文件名。
- 第 1–4 节每一步“符合预期 / 不符合”，不符合的附命令输出或截图。
- `runtime\current.json` 的内容（更新前后各一份）和 `runtime\update-state.json`。
- 任何系统弹窗（截图）、杀毒软件记录。
- macOS：第 2 节的授权观察；Windows：Smart App Control 的值。
