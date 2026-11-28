# Cranibox 安卓手机节点

Android 12+、ARM64。App 在前台服务中运行 Cranibox Rust 节点，复用设备界面；经无线调试或 USB 启动 shell 助手，为远端 Agent 提供独立虚拟屏上的 Computer Use。

构建需要 JDK 17、Android SDK 35 / build-tools 35.0.0、NDK r27 和 Rust Android 目标：

```sh
rustup target add aarch64-linux-android
./build-native.sh
./gradlew :core:test :app:lintDebug :app:assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

只运行不依赖 Android SDK 的协议测试：

```sh
./gradlew :core:test
```

首次安装后，打开 App 并允许通知，在底部「手机」页配对无线调试，或按页面给出的命令用 USB 启动助手。设备授权、消息和任务在「设备」页管理；「概览」页右下角按钮可启动或停止节点，通知栏可随时停止 Agent。

架构、安全边界、使用步骤与真机验收见 [安卓手机节点](../../docs/android-node.md)。ADB 协议代码改编自 Apache-2.0 项目 Mika，见 [NOTICE](NOTICE)。当前实现为第一期手机节点，手机本地模型执行 Agent 尚未开始。

在 Apple Silicon Mac 上实际启动模拟器并截图（需已安装 SDK、emulator 和 `system-images;android-35;google_apis;arm64-v8a`）：

```sh
python3 scripts/verify-emulator.py \
  --apk app/build/outputs/apk/debug/app-debug.apk \
  --output artifacts/emulator
```

脚本创建或复用专用的 `cranibox-verify-api35` AVD，安装 APK，保存浅色、深色和大字体的界面截图、UI 层级与 logcat，最后保留模拟器窗口。缺少 SDK 组件会直接报错，不会自动下载安装。截图需要人工查看；无线调试配对和虚拟屏控制需要在运行中的模拟器上另行验证，脚本不会将它们标记为通过。
