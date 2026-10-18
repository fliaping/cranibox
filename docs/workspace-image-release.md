# Workspace 镜像发布与启动验收

发布入口为 `.github/workflows/build-webtop.yml`，GitHub Actions 中的名称是 **Build Multi-Arch Image**。普通 `main` 提交触发的 **Release** 只运行检查，不发布容器镜像。

## 手动验证与发布

先用 `workflow_dispatch` 选择目标分支、`desktop=xfce`、`version=latest`、`publish=false`。这是默认模式：构建并加载 Linux amd64 镜像，在工作流内运行首次启动、节点授权及重启验收，不登录镜像仓库，也不修改公开标签。`desktop=all` 同时检查 XFCE、LXQt 和 KDE。

需要发布时，明确设置 `publish=true`。此时每个桌面变体先构建 Linux amd64/arm64 多架构候选镜像，推送至带本次运行编号和重试编号的 `cranibox-check-*` 标签。工作流按返回的摘要拉取 amd64 候选并验收，通过后才将该摘要提升为 `ubuntu-{desktop}`，以及 `version` 非 `latest` 时的 `ubuntu-{desktop}-{version}`。版本号只接受 Docker 标签所需的受限字符，不直接插入 Shell。

推送 `v*` Git 标签会发布全部桌面变体。工作流串行处理发布，避免不同运行同时更新浮动标签；各变体独立验收、独立更新，并非整个桌面矩阵的原子发布。手动只选 XFCE 时，LXQt/KDE 标签保持原版本。

Docker Hub 作业成功后，阿里云作业从同一源码及构建缓存生成自己的多架构候选，按摘要拉取并独立验收，随后更新阿里云标签。它是重新构建，不能宣称两个仓库的镜像字节或摘要相同。任一仓库的验收失败，都不会在该作业内更新其公开标签；此前已成功发布的其他变体或仓库不会回滚。失败的候选标签可能保留，不能把候选标签当作正式验收版本。

运行产物 `image-startup-{desktop}`、`image-startup-aliyun-{desktop}` 包含脱敏 JSON；其中 `passed` 必须为 `true`，并核对 `revision` 与本次完整提交。仅有构建成功、Actions 的普通 Release 成功或协议标签存在，都不能替代启动验收。

## 启动验收范围

`scripts/verify-workspace-image.py` 使用当前 Docker 引擎中已经加载的镜像，不隐式拉取。先检查空间协议、完整来源提交和 Linux 平台，再分别创建空数据卷的普通 Workspace，以及预置独立身份和目标授权的空间节点。两种场景均执行首次启动和重启。

验收检查：

- Rust Web/网关监听器、嵌入静态资源、节点 API，以及内置程序和兼容命令链接。
- 默认 Web 服务已启用并运行，身份与运行凭据为私有文件。
- 桌面匿名访问被拒绝、正确登录可用，独立应用来源的根路径不暴露控制台。
- 空间预置身份保持、正确令牌可读取节点并执行命令、错误令牌被拒绝。
- 重启后身份、运行凭据与写入的工作目录文件保持。

每个测试容器只有独立 `/config` 命名卷，不发布宿主端口、不挂载宿主 Docker socket、不使用 privileged，并限制 CPU、内存和进程。结束时按本轮随机归属标签清理容器及卷；归属不符时拒绝删除，清理失败会使验收失败。日志与凭据保留在私有执行环境中，报告不输出密码、令牌、配置正文或子进程错误正文。

在授权的远端 Linux Docker 宿主机运行示例：

~~~sh
python3 scripts/verify-workspace-image.py \
  --sudo --image cranibox-local:test \
  --revision FULL_GIT_COMMIT \
  --report /private/image-startup.json
~~~

检查器本身的回归可在不接触 Docker 的环境运行：

~~~sh
python3 -m unittest discover -s tests -p test_workspace_image_check.py -v
~~~

多架构发布中的运行验收目前仅覆盖 **Linux amd64**；arm64 仅构建。它不代替浏览器桌面输入、编辑器 WebSocket、完整空间生命周期、跨物理设备或 Windows/macOS 原生客户端验收。公开默认镜像是否已经兼容，仍以[交接文档](implementation-handoff-2026-10-18.md)中的具体发布和真实拉取记录为准。
