# 安卓手机节点

2026-11-02：新增安卓 App（`apps/node-android`），让安卓手机成为 Cranibox 网络中的一个节点。手机运行与桌面版相同的 Rust 节点，并在一块单独的后台虚拟屏上提供 Computer Use，其他设备上的 Agent 可以通过现有授权、租约、紧急停止和 MCP 操作手机上的 App。手机控制的做法参考 [Mika](https://github.com/wangzan101/Mika)（Apache-2.0），ADB 配对与连接代码改编自 Mika，见 `apps/node-android/NOTICE`。

2026-11-27：节点 App 改为原生 Jetpack Compose + Material 3 四 Tab 界面（对话、动态、设备、本机），并内置 PicoClaw Agent Runtime（`libpicoclaw.so`），可在手机上直接对话、经 Cranibox MCP 操作已配对节点。终端/桌面会话仍可在无底栏的专用页中打开节点界面。

| 期 | 内容 | 状态 |
| --- | --- | --- |
| 一 | 手机作为节点：配对、消息与协作任务、命令执行、Computer Use（虚拟屏、启动 App、点击、滑动、输入、按键、截图） | 原生 Compose 控制器与受控界面已落地；Android 15 ARM64 模拟器此前已通过入网与无线配对验收；物理手机待验收 |
| 二 | 手机自带对话 Agent：内置 PicoClaw Runtime，连接本机模型网关或自定义 OpenAI 兼容接口，经 `cranibox mcp` / `app-mcp` 调用设备、任务与 Computer Use，遵守既有审批与权限 | 已接入本机 Runtime 与「对话」Tab；真机与模型网关联调待验收 |

## 结构

~~~
┌──────────────────────── Cranibox 安卓 App（uid = App）────────────────────────┐
│ MainActivity：WebView 打开本机节点界面（ui-url 登录链接）+ 手机控制设置页        │
│ NodeService（前台服务）                                                          │
│   ├─ 子进程 libcranibox.so serve --listen 127.0.0.1:7377   ← Rust 节点，入网     │
│   │      peer computer --address 127.0.0.1:<桥端口> --token-file <私有令牌文件>  │
│   └─ PhoneBridge：127.0.0.1:<桥端口> POST /v1/action（Bearer 令牌）              │
│          └─ AndroidPhone → AgentDisplay（ImageReader 截图）                      │
│                         → ICraniboxShell Binder ───────────┐                     │
│ AdbClient：本机无线调试配对（SPAKE2）与 TLS 连接 ──exec:──┐  │                     │
└──────────────────────────────────────────────────────────┼──┼─────────────────────┘
                                                           ▼  │
                           adbd ── app_process（shell，uid 2000）HelperMain
                                    ShellService：建可信虚拟屏、按屏启动 App、
                                    注入触摸/按键、以 shell 身份执行有限命令
                                    经 ContentProvider.call 把 Binder 交给 App
~~~

- **Rust 节点**：`cargo build -p cranibox --no-default-features --target aarch64-linux-android`，作为 `lib/arm64-v8a/libcranibox.so` 打进 APK（`useLegacyPackaging`，安装时解出，可执行；安卓 10 起 App 不能执行自己写入的文件，只有原生库目录可以）。Android 不走桌面版复制 runtime 再执行的自动启动路径，首次私密配置发布使用 SELinux 允许的原子重命名。NodeService 设好 `HOME=filesDir`、`TMPDIR=cacheDir`、`SHELL=/system/bin/sh`、`PATH=/system/bin:/system/xbin` 后以子进程启动它，配置在 `filesDir/.config/cranibox/settings.json`。节点身份、配对、授权、消息、任务都与桌面相同，节点管理 API 由原生界面调用（本机 `127.0.0.1:7377` 的 App API）。终端与远程桌面在无底栏的专用 WebView 中打开对应哈希路由，避免与底部导航重叠。
- **手机控制**：与 Mika 相同，不需要 root，也不需要电脑。用户在「开发者选项 › 无线调试」里点「使用配对码配对设备」，App 用 mDNS 找到配对端口（`_adb-tls-pairing._tcp`），用户把六位配对码填进 App 的通知输入框（可留在设置页分屏）。配对后 App 用自己的 RSA 钥匙经 TLS 连上本机 adbd（`_adb-tls-connect._tcp`），用 `exec:` 以 shell 身份启动 `app_process` 助手。助手拿到 App 的 ContentProvider，把自己的 Binder 交过去。mDNS 不可用时，连接会尝试上次端口和本机端口扫描；设置页也支持手填配对端口、连接端口。调试钥匙经 Android Keystore 的 AES-GCM 密钥加密存放，不随系统备份或换机迁移。
- **虚拟屏**：助手用 `DisplayManager.createVirtualDisplay` 建一块与主屏相同比例、最长边不超过 1920 像素的虚拟屏，画面输出到 App 进程里的 ImageReader。安卓 14 起尝试带 `OWN_FOCUS | STEAL_TOP_FOCUS_DISABLED` 的可信屏，避免抢主屏焦点；shell 没有 `ADD_TRUSTED_DISPLAY` 时去掉可信相关标志再建。状态会报告是否获得可信屏。Agent 的输入只发到这块屏；不同系统版本和厂商对副屏焦点、应用启动的支持需真机验证。
- **实时预览**：底部「屏幕」页和概览的「查看 Agent 屏幕」可查看完整画面。`AndroidPhone.frames` 发布独立的 Bitmap 快照，最高 10 帧/秒、短边不超过 540 像素；原始尺寸截图仍供 Computer Use 使用。`AgentScreenView` 按比例完整显示，隐藏时取消订阅、清除视图引用，断开时显示连接提示。预览只读，点击画面不会向虚拟屏注入输入。
- **跨应用悬浮窗**：在「屏幕」页点击「开启跨应用悬浮窗」，按提示启用系统无障碍服务中的「Cranibox · Agent 屏幕悬浮窗」。`AgentOverlayService` 使用 `TYPE_ACCESSIBILITY_OVERLAY`，不需要普通「显示在其他应用上层」权限；服务不请求读取窗口内容或执行手势能力。窗口支持拖动标题、展开/收起、关闭、停止 Agent，并避开系统栏。完整「屏幕」页在前台时自动隐藏悬浮窗，切到其他应用后恢复；断开手机控制或停用无障碍服务会移除窗口。关闭窗口不停止 Agent，停止 Agent 后仍可查看最后画面，恢复须在本机操作。
- **悬浮窗缩放**：在展开的悬浮窗内双指捏合，或拖动右下角 `↘` 手柄，可连续调整窗口大小；点按手柄恢复默认 248dp 宽度。尺寸以 dp 保存，收起再展开、服务重新连接后恢复。最小宽度通常为 200dp，最大值根据虚拟屏比例和主屏可用空间计算，内容保持等比完整显示；横屏空间不足时保留可用按钮和黑边。双指操作会取消已经按下的子控件，并消费整段多指手势，不向 Agent 虚拟屏注入触摸。模拟器实际手势、尺寸边界及服务重连验证见 `dist/android/agent-resize/REPORT.zh-CN.md`。
- **USB 备用启动**：设置页可生成本 App 外部文件目录中的 `usb-start.sh`，显示可复制的 `adb shell sh ...` 命令。助手用完一次性口令后，脚本换为不含口令的状态提示。手机重启后需重新启动助手；App 进程被系统回收时，仍存活的助手用轮换后的恢复口令重新交接。
- **Shell 系统调用**：助手从 `com.android.shell` 的 LoadedApk 创建独立 Context，确保调用包名与 uid 2000 匹配。Provider 交接使用系统 `content` 命令相同的 external-provider 接口，并释放引用；应用启动使用没有 ApplicationThread 的 shell 路径。虚拟屏任务切换使用 `moveTaskToFront` 且不带 `MOVE_TASK_WITH_HOME`，避免 Recents 启动路径将主屏退回桌面。签名、uid/pid、APK 路径、一次性口令检查保持生效。
- **Computer Use 桥**：App 在回环地址实现与 `agent-desktop-bridge` 相同的 `POST /v1/action` 协议，节点按原有流程转发。安卓上所有 App 都能连 127.0.0.1，所以桥要求 `Authorization: Bearer <令牌>`；令牌放在 App 私有目录，节点配置里只记文件路径（`token_file`），状态视图不暴露令牌。

## 安全边界

- 助手以 shell 身份运行，权限远高于普通 App，所以：
  - 只把 Binder 交给签名证书指纹与启动参数一致的本 App；
  - 交接要过一次性口令，口令经标准输入传递，不进命令行，因为同为 shell 的进程能读 `/proc/<pid>/cmdline`；
  - Provider 只接受 uid 2000 的调用（`android:permission="android.permission.INTERACT_ACROSS_USERS_FULL"`，普通 App 拿不到），并核对调用方自报的 uid/pid 与 APK 路径。
- 桥只监听 127.0.0.1，并且每个请求都要带 Bearer 令牌，用常量时间比较。请求体不超过 2 MiB，响应不超过 8 MiB。拒绝未知动作。
- 远端调用方仍需目标所有者单独授予 computer 权限；input 动作需要租约；紧急停止持久保存，只能在手机上恢复。手机通知栏常驻「停止 Agent」按钮，作用与紧急停止相同。
- 输入请求串行执行；紧急停止独立于输入队列，并通知 shell 助手中断长文本与手势，补发按键释放或触摸取消。等待中的请求拿到执行权后会重新检查暂停状态。暂停文件损坏时默认暂停，只有手机上明确恢复才能重新输入。
- 输入只发到 App 自己建的虚拟屏，不发到主屏。在虚拟屏上启动的 App 若正在主屏上被用户使用，助手不启动它（返回 `in_use_on_main_screen`），以免把用户正在看的界面挪走。
- 按键白名单：Back、Enter、Tab、Escape、Backspace/Delete、方向键、PageUp/PageDown、Home/End（光标）、空格，以及 ctrl+a/c/v/x/z。不提供系统 Home、最近任务、电源和音量键，这些会影响主屏。
- 非 ASCII 文本经剪贴板粘贴：App 写入剪贴板，再在虚拟屏注入粘贴键。这会覆盖用户剪贴板，在状态里说明。二期改为助手以 shell 身份写入、用完还原。
- 命令执行（exec、终端）以 App 自己的 uid 运行，不经助手，也拿不到 shell 权限。桥不提供任意 shell 命令。
- 支付、密码等安全页面（FLAG_SECURE）在虚拟屏上截图为黑屏，Agent 应请用户在手机上处理。

## 桥协议（安卓实现）

与 [Computer Use](computer-use.md) 相同，`POST /v1/action`，`X-Agent-Session` 为调用方会话，另需 `Authorization: Bearer`。坐标以虚拟屏像素为准，也接受 `coordinate_space: normalized`。

| 动作 | 安卓上的做法 |
| --- | --- |
| status | `{"service":"cranibox-android-bridge","state":"ready｜paused｜backend-unavailable","paused","backend_available","resolution":{width,height},"display_id","helper":{connected,trusted_display},"capabilities":[…]}`；助手未连接时 `backend_available=false` |
| emergency_stop / resume | 持久保存暂停状态（App 私有文件），暂停期间所有改变状态的动作返回 423 |
| screenshot | 虚拟屏最新一帧，PNG base64：`{"data","width","height"}` |
| zoom | 裁剪 region 后 PNG |
| cursor_position | 最后一次触点（触摸屏没有光标） |
| left_click / double_click | 触摸点击（两次间隔 80 ms） |
| right_click | 长按 600 ms |
| middle_click | 不支持，返回 400 |
| mouse_move | 只记录位置，不注入 |
| left_click_drag | 一笔滑动，300 ms |
| scroll | 从 coordinate（缺省为屏幕中心）按方向滑动；距离 = 屏幕短边 × min(amount,10) / 12，down 表示内容向上走（手指上滑） |
| type | ASCII 用 KeyCharacterMap 按键序列；含非 ASCII 时经剪贴板粘贴 |
| key | 白名单内的键或组合，名称大小写不敏感（`Return`/`Enter`、`BackSpace`、`ctrl+a` 等） |
| window_list | 虚拟屏上的任务：`{"windows":[{"id":taskId,"title":包名,"package"}]}` |
| window_focus | 把该任务移到最前 |
| app_list（新增） | 可启动的 App：`{"apps":[{"package","label"}]}` |
| open_app（新增） | `{"package"}`，在虚拟屏启动；结果 `started｜in_use_on_main_screen｜not_found｜error` |

`app_list` 与 `open_app` 已加入节点的 `Action` 类型和 device_computer MCP 说明。桌面桥收到后照常报 unsupported。

## 构建

需要 JDK 17、Android SDK（platform 35、build-tools）、NDK r27、rustup 的 `aarch64-linux-android` 目标。macOS（Apple Silicon）上：

~~~sh
cd apps/node-android
rustup target add aarch64-linux-android
./build-native.sh            # cargo build → app/src/main/jniLibs/arm64-v8a/libcranibox.so
./gradlew :core:test :app:lintDebug :app:assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
~~~

App 支持 Android 12（API 31）及以上的 ARM64 手机，当前版本为 `0.1.0`。APK 包含提取到原生库目录运行的节点、许可证与 Mika NOTICE。缺少原生节点时构建会失败，提示先运行 `build-native.sh`。GitHub Actions 的 `Android phone node` 工作流会构建 ARM64 节点、运行协议测试与 lint，上传 Debug APK 和检查报告；当前工作区未触发远端工作流。

`core` 模块是纯 JVM 代码（ADB 协议、桥协议、HTTP 服务、按键与手势换算），在任何平台都能跑 `./gradlew :core:test`。无完整 SDK 时可用 `ANDROID_JAR=/path/to/android.jar ./gradlew :typecheck:compileKotlin` 检查 App/助手 Kotlin。Google 不提供 linux-arm64 的 aapt2 和 NDK 编译器；arm64 Linux 可用宿主 clang 加 NDK sysroot 构建 Rust（见 `build-native.sh` 注释），完整 APK 打包还需通过用户态模拟等方式运行 x86_64 aapt2。本次在隔离的临时 SDK 中通过 QEMU 用户态模拟完成了打包，没有将模拟器依赖写入项目。

## 首次使用与验收

1. 安装 APK，打开 Cranibox 并允许通知。节点会以前台服务启动，底部「设备」页提供设备配对、访问授权、消息与任务界面。
2. 在底部「手机」页开启无线调试并配对，或用页面提供的 USB 命令启动助手。配对端口与连接端口通常不同，请按对应字段填写。
3. 在手机上授权另一设备的 computer 权限；远端先读取状态，再领取租约，执行 `app_list`、`open_app`、截图和输入。
4. 对照主屏验证：尝试打开当前正在主屏使用的 App，应返回 `in_use_on_main_screen`；点击、拖动与键盘操作应仅影响 Agent 虚拟屏。在 Android 12/13 和不支持可信副屏的系统上重点检查焦点行为。
5. 点击通知或悬浮窗的「停止 Agent」，确认后续输入被拒绝，重启 App 后仍暂停；在本机「概览」或「屏幕」页恢复。点击「概览」页节点卡片中的「停止」应关闭 Rust 子进程、回环桥和 shell 助手。
6. 验证中文粘贴、系统回收 App 后的助手重连，以及无线调试关闭/重开后的重新连接。厂商后台限制可能要求用户允许 Cranibox 持续后台运行。
7. 远端打开应用后，在「屏幕」页确认画面随操作更新；启用悬浮服务后切到桌面，检查预览、拖动、收起、展开、关闭/重开、横竖屏边界和停止按钮。测试无障碍悬浮窗时优先用 `screencap` 与 `dumpsys window` 取证：普通 `uiautomator dump` 会暂时抑制无障碍服务，造成窗口消失或重建，不能据此判断产品故障。

自动检查覆盖了配对密码学、ADB 流、助手口令、HTTP 认证、坐标/手势/按键、紧急停止并发与 Rust Bearer 令牌轮换。构建与静态检查通过不代表完成上述真机验证；当前环境未连接安卓手机。

2026-11-02 本地验证：106 项 JVM 测试与 6 项 Rust Computer Use 测试通过；Android lint 为 0 错误、4 项提示（AGP 版本更新与未提供 ChromeOS x86_64 ABI）。Debug APK 的 v2 签名、16 KiB 对齐、压缩完整性、许可证资源和原生节点内容均已检查。交付文件位于 `dist/android/cranibox-android-0.1.0-arm64-debug.apk`（22.6 MiB），同目录附 `SHA256SUMS`；该目录为本地构建产物，不进入 Git。

2026-11-04 MacMini 模拟器验证：在专用 `cranibox-verify-api35`（Android 15 / API 35 / Google APIs ARM64）上实际运行 APK。已验证节点启动、中文设备列表、通知输入配对码的无线 ADB 配对、TLS 重连、shell 助手交接及可信虚拟屏。通过带令牌的回环桥验证应用列表、在副屏启动 Google Clock、任务列表/切换、PNG 截图/裁剪、点击、滚动、按键、英文输入和中文剪贴板输入；确认主屏仍显示 Cranibox，并拒绝将主屏正在使用的 Cranibox 移到副屏。停止 Agent 后输入返回 HTTP 423，在 App 内恢复后回到 ready。108 项 JVM 测试通过，lint 为 0 错误、7 项提示，最终 APK 的 v2 签名通过。

可重复的模拟器检查：

~~~sh
python3 scripts/verify-emulator.py --sdk "$ANDROID_HOME" \
  --apk app/build/outputs/apk/debug/app-debug.apk --output artifacts/emulator-ui
# 在 App 内完成无线配对并连接手机控制后：
python3 scripts/verify-phone-control.py --adb "$ANDROID_HOME/platform-tools/adb" \
  --output artifacts/emulator-controls
~~~

功能脚本只接受专用 AVD，使用 debug App 的 `run-as` 在内存读取桥令牌，不将令牌写入日志或验收文件。截图需要人工检查输入后的实际文字。物理手机、厂商后台限制、Android 12–14 以及手机自带模型 Agent 不属于此次已通过的范围。完整本地报告与截图见 `dist/android/MACMINI-VALIDATION.zh-CN.md`。

随后完成跨节点 Mobile Use 验收：模拟器 `node-f9eda1ea3ccf3cb87718188017f2eea5` 以 owner_device 加入已有身份，自动获得工作区、MacMini、MacBook 的相互连接；在手机端为工作区开启 `computer=allow`。工作区通过节点 CLI 实际完成状态、应用列表、租约申请/续期/释放、副屏应用启动、任务切换、截图、点击、滚动、中英文输入。无租约、停止后的旧租约和释放后的租约均被拒绝；远端停止后无法申请新租约。目标绑定 MCP 的 `device_computer` 可发现，并返回实际手机 PNG 图片内容块。

这次验证修复了 Android 本机恢复仅清除桥暂停、未清除节点暂停的问题。App 的停止/恢复现在调用本机 `peer computer-owner`，同步节点暂停状态与控制租约；停止时先立即暂停桥输入，恢复失败时保持暂停。新版已在同一模拟器更新安装，验证手机停止撤销远端租约、手机恢复后重新取得租约，完成后释放测试租约并保持设备在线。报告与跨节点截图位于 `dist/android/mobile-use-network/REPORT.zh-CN.md`。

2026-11-05 补充：实时「屏幕」页和无障碍悬浮窗已部署到上述 MacMini 模拟器。实际验证跨节点打开短信草稿、应用内/跨应用画面、拖动、收起/展开、关闭/重开、横屏边界，以及浮窗停止撤销租约和本机恢复。最终保留草稿未发送，设备在线且无控制租约。证据见 `dist/android/agent-preview/REPORT.zh-CN.md`，交付包校验见 `dist/android/build-agent-preview-verified.json`。当前预览只读，未实现画面内直接接管。

## 对话 Agent 与 PicoClaw Runtime

本机在 `NodeService` 中与 `libcranibox.so serve` 一起启动 `libpicoclaw.so gateway --host 127.0.0.1 --allow-empty`。

- **构建**：`apps/node-android/build-native.sh` 在打好 `libcranibox.so` 之后，从 Git 固定检出 [PicoClaw v0.3.1](https://github.com/sipeed/picoclaw/releases/tag/v0.3.1)（commit `2cf030d2fd3b871d7ec17e3be34c24688aac76da`），以 `CGO_ENABLED=0 GOOS=android GOARCH=arm64 go build -tags stdjson` 交叉编译，产出 `app/src/main/jniLibs/arm64-v8a/libpicoclaw.so`。GitHub Actions 的 Android 工作流同样执行这一步。PicoClaw 为 MIT 许可，全文见 `apps/node-android/THIRD_PARTY_LICENSES`。
- **配置**：每次启动在 `filesDir/picoclaw/`（`PICOCLAW_HOME`）写入 schema v2 的 `config.json` 与 `.security.yml`。默认 `model_list` 指向本机 Cranibox 模型网关 `http://127.0.0.1:3424/v1`（`cranibox-llm` 的 `LLM_ADDR`）。若用户在「对话」设置中填写了兼容 OpenAI 的地址、密钥和模型 ID，则将其作为默认模型，以便在尚未订阅网关时也能对话。网关监听 `127.0.0.1:18790`。本地 `exec` 工具关闭，设备操作走 Cranibox MCP。
- **MCP**：为 PicoClaw 注册三个 stdio 服务器，命令均为本机 `libcranibox.so`（绝对路径），环境变量与节点进程一致（`HOME`、`XDG_CONFIG_HOME`）：
  - `cranibox`：`libcranibox.so --config <settings.json> mcp`（官方舰队 MCP：设备、命令、文件、任务等，受既有授权与审批约束）
  - `cranibox-devices`：`app-mcp --data <filesDir/.config/cranibox> --service devices`（只读设备/审批列表，不能代人批准）
  - `cranibox-computer`：`app-mcp --service computer`（Computer Use，同样走既有租约与紧急停止）
- **对话协议**：原生 Chat 经 OkHttp 连接 `ws://127.0.0.1:18790/pico/ws?session_id=…`，使用 `Authorization: Bearer <token>`。客户端发送 `{type:"message.send",id,session_id,payload:{content}}`；服务端事件为 `message.create` / `message.update` / `message.delete` / `typing.*` / `error` / `pong`（对照 v0.3.1 的 `pkg/channels/pico/protocol.go`）。v0.3.1 的 Pico 通道没有中止帧，「停止」只结束本机的进行中状态显示。
- **安全**：令牌自动生成并写在 `filesDir/picoclaw/.security.yml`（仅应用私有目录）。网关只绑定 127.0.0.1。不启用 URL query token。

## 界面

四 Tab 底部导航（对话为默认首页）：

- **对话**：原生 Compose 流式聊天、Markdown、工具调用状态、待审批横幅、会话历史、模型设置。
- **动态**：待你审批（本机 inbox、本地/远程访问请求）与进行中任务；新待办走系统通知。
- **设备**：原生设备卡片、重命名、能力开关（关/询问/允许）、扫码或粘贴 `cranibox://` 配对；终端/桌面进入无底栏专用页。
- **本机**：节点启停、紧急停止/恢复、虚拟屏预览。预览页可打开「接管」，把触摸映射到 Agent 虚拟屏（不操作手机主屏，且受紧急停止约束）。无线调试配对为三步引导，端口可自动发现或手动填写。

