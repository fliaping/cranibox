# 虚拟空间

“虚拟空间”是在管理设备的本地 Docker 引擎上运行的完整 Cranibox Linux 容器。每个空间拥有独立 NodeID、持久 `/config` 数据卷和登录凭据，作为普通授权节点接入项目与终端。Web 和 Tauri 使用相同的创建向导与 Rust 实现；它不是宿主机的系统虚拟桌面。

## 镜像拉取策略

向导的高级设置与 CLI `space create --pull-policy` 提供三种策略：`always`（默认，每次创建或归档重建前拉取）、`missing`（优先使用本地镜像，缺少时拉取）、`never`（只使用本地镜像）。策略随空间保存，归档恢复继续使用；启动已有容器不拉取镜像。旧目录没有此字段时继续按 `always` 处理。

离线部署或本地构建镜像可使用 `cranibox-node space create "Local lab" --image cranibox-local:test --pull-policy never`。镜像必须已经构建或加载到管理设备使用的 Docker 引擎，并具有兼容性标签。缺少或不兼容时在创建数据卷之前提示错误；准备完成后可重试同一空间。初始化与创建容器也禁止 Docker 隐式拉取镜像。此策略只控制空间管理器的镜像下载，不代表容器内首次启动的工具安装无需联网。

## 创建

1. 打开“虚拟空间 → 新建空间”，选择并检测容器运行环境。检测和安装发生在运行 Cranibox 的设备上。远程浏览器不能替自己的电脑安装依赖。
2. 环境缺失时，先检查向导展示的安装命令，再确认执行。每份方案只能确认一次，十分钟内有效；过期、操作改变或提交失败后重新核对。安装在后台继续，可查看结果并重新检测。需要管理员授权时使用操作系统的提示。
3. 填写名称，确认镜像、资源和数据存储后创建。界面显示下载、创建、启动和等待节点的状态；关闭页面不会取消操作。
4. 空间就绪后可打开浮动终端、浏览器桌面和编辑器，或进入统一项目页面管理该节点的工作副本与会话。这里不依赖 Workspace 网关；创建者通过目标本地授权访问空间。

默认镜像为 `xuping/agent-workspace:ubuntu-xfce`。镜像必须包含当前 Cranibox Web 内嵌节点及直连 API，并带有当前 Dockerfile 的 `io.cranibox.space-protocol=1` 标签；旧镜像会在创建数据卷前明确报错。可在高级设置指定包含本分支代码的新构建镜像。创建和归档重建遵循所选镜像拉取策略；恢复尚存在的容器不会自动升级镜像。

2026-10-19 04:41 UTC 的远端仓库元数据核对发现，默认公开标签的 Linux amd64/arm64 镜像均仍指向旧提交 `c56508ce5008180e067a9c489f4527aa30b80b31`，缺少该协议标签。当前默认公开镜像因此不能用于新建空间；此次只读取 index、manifest 与配置，未拉取镜像层或启动公开镜像。发布包含当前代码的新镜像后，仍需完成真实拉取、首次启动与重启检查。核对的摘要与范围见[交接文档](implementation-handoff-2026-10-18.md)，不要删除协议检查以兼容旧镜像。

镜像工作流提供默认不发布的验证模式，并在候选镜像首次启动、空间节点接入及重启验收通过后才更新公开标签，操作与范围见[镜像发布说明](workspace-image-release.md)。流程改进本身不表示默认公开标签已经更新。

## 平台引导

| 管理设备 | 提供的引导 | 需要用户完成的步骤 |
| --- | --- | --- |
| Windows | 安装 WSL2/Ubuntu，在 Ubuntu 中安装 Docker Engine；也支持已有的 Docker Linux 引擎 | Windows 管理员提示、必要时重启、首次启动 Ubuntu 创建用户 |
| macOS | Homebrew 安装 Colima 与 Docker CLI，启动 Colima；或安装并打开 OrbStack | 尚无 Homebrew 时先完成其安装；OrbStack 的首次设置和许可 |
| Linux | Ubuntu/Debian 使用 Docker 官方 apt 仓库安装 Engine、CLI 和 Compose 等组件 | 系统管理员提示，加入 docker 组后重新登录；其他发行版先手动安装 |
| Docker 中的 Web | 复用明确挂载的宿主 Docker socket，把空间接到管理容器已有的 bridge 网络 | 宿主机准备 Docker 与 socket 权限；不会在容器内安装宿主依赖 |

只支持本地 Unix socket 或 Windows named pipe 引擎，拒绝 SSH/TCP 远程 Docker context。空间容器内不会挂载宿主 Docker socket，不开启 privileged，也不直接挂载宿主项目目录。已安装 Docker 时不会自动卸载或替换已有安装。

### 安装确认与中断恢复

`space plan` 返回当前安装操作、命令、`plan_id` 和 `expires_in_seconds=600`。方案保存在管理端私有目录，便于分开的 CLI 调用完成核对与执行；同时只保留一份，新的方案替换旧方案。安装时重新核对操作与命令，并在启动系统命令前原子保存“方案已使用”和安装中状态。没有 ID、旧 ID、重复确认、过期或命令改变都不会启动安装；旧目录自动兼容，旧调用方缺少 ID 时需更新后重新准备。公开及共享空间目录不返回方案。

关闭网页不会取消后台操作。节点退出或监控超时不主动终止已经启动的系统安装进程，重启后只提示监控中断，不自动重放。应先检查操作系统安装器或软件包管理器，再重新检测环境；系统关机、权限代理或服务管理器仍可能中断安装，不能据此保证事务完成。多步安装中断时，后续步骤可能尚未执行。

系统安装命令使用独立的空输入/输出流，避免节点关闭导致管道断开；当前只报告启动错误、退出码或监控超时，不保存软件包安装明细。管理员授权仍依赖系统界面；真实系统认证、取消和包安装尚待对应平台验收。安装结束状态为“需要重新检测”，不自动创建空间。Web/Tauri 共用一次性提交、响应丢失后的状态检查和键盘焦点恢复。

## 生命周期与数据

每个空间默认限制 2 CPU、4 GiB 内存、1 GiB shared memory，使用 `unless-stopped` 重启策略。管理目录最多保存 32 个空间，耗时变更串行执行，避免重复创建或互相覆盖。管理进程重启后，未结束的操作显示为中断；点击“启动 / 重试”会核对已有容器和数据后继续。

- **停止**结束该空间中的 Agent、Shell 和其他进程，保留容器、数据与身份。重新启动不会恢复原进程。
- **归档**停止并移除该容器，保留命名数据卷、身份与凭据；之后“恢复空间”重新创建容器并继续使用原数据。
- 系统不会自动删除空间数据卷，也不修改标签不属于本安装的同名资源。删除数据需用户另行处理。

目录保存在节点配置旁的 `spaces/` 下；`catalog.json` 和空间 Token 是私有文件。请连同管理端 `peers.json` 一起备份，保留创建者的身份。数据卷位于 Docker 引擎中，不在管理端目录里，需另外备份。界面目录不返回密码或 Token，只有主动点击“查看登录凭据”才读取空间密码。

空间中的目标授权允许创建者使用命令、终端和项目目录，不默认授予桌面转发权限。其他设备仍需另行取得空间授权。容器的发布端口仅绑定 Docker 引擎宿主机的 `127.0.0.1`；节点通信在本机回环或管理容器已有的 bridge 网络中进行。

## 图形桌面和编辑器

就绪卡片的“桌面”和“编辑器”会在浏览器新标签页打开 Selkies 与 code-server。Web 通过管理端的独立应用入口转发 HTTP 和 WebSocket，远程浏览器不需要访问引擎的回环地址，也不需要再次输入空间密码。Tauri 使用进程持有的临时回环服务，并打开系统浏览器。关闭标签页不会停止空间；停止、归档、身份变化、管理端退出或访问授权过期会终止转发，重新启动后需从卡片重新打开。

Web 部署须配置 `applicationsUrl` 为可被用户浏览器访问的独立 HTTP(S) 来源，不能与控制台、宿主桌面或宿主编辑器共用来源。来源由协议、主机和端口组成。例如控制台为 `https://workspace.example.com` 时，可配置 `https://apps.example.com`；同主机的独立 HTTPS 端口也可使用。浏览器编辑器建议使用 HTTPS。不要把应用入口配置到宿主 KDE 的端口。

~~~sh
workspacectl web configure --applications-url https://apps.example.com
~~~

镜像内置 Nginx 的应用入口为 HTTPS `3002`，Compose 默认发布 `3002:3002`。如果通过其他反向代理部署，将应用来源上的 `/cranibox-app-` 前缀保留完整路径转发到 Web 服务，并传递原始 `Host`、WebSocket Upgrade，关闭响应缓冲；其他路径不应转发控制台。现有旧镜像升级二进制时，还需单独补充该入口配置，参考 `nginx/cranibox-applications.conf`。非本机浏览器使用时，务必把默认 `https://localhost:3002` 改为实际入口。

后端只转发当前管理设备创建、标签匹配且运行中的容器，不接受任意目标地址。每次新请求重新核对容器身份和端口，容器替换后旧入口失效。打开链接只可兑换一次，30 秒内有效；兑换后使用限定路径的 HttpOnly、SameSite=Strict Cookie，HTTPS 下带 Secure。应用访问最长 12 小时；Web 访问还绑定原控制台会话。应用转发过程中，空间密码与上游应用 Cookie 保留在后端，不传入页面或浏览器存储。

“数据与访问方式”仍显示引擎宿主机上的回环地址及按需读取的登录凭据，供在引擎所在设备直接访问；首次访问可能需要接受空间的自签名证书。其他设备共享的空间目录仍为只读，不会自动获得管理端应用入口。

跨设备 VNC 访问见[跨设备桌面](remote-desktops.md)。统一项目视图会读取已接入节点的项目目录，但不自动复制项目文件或迁移 Agent 进程。

## 跨设备空间目录

Web 和 Tauri 的“虚拟空间 → 其他设备”汇总当前接入端已保存的直连设备。空间创建者在“入站授权”中为查看方单独勾选“查看此设备管理的虚拟空间目录”；旧授权默认不具备此能力。只查看目录时无需授予命令、终端、项目、消息或桌面权限。

共享内容限定为空间 UUID、名称、节点 ID、运行环境和管理目录中记录的状态，最多 32 个空间。每组目录标明归属节点与检查时间；该状态不表示已探测容器。读取不会触发 Docker 操作或修复中断任务，不返回镜像、端口、数据卷、错误详情、密码及 Token。

页面打开时立即读取，并每 15 秒检查一次；切回页面及手动刷新也会更新。撤权、凭据变更、连接移除或目录不可用后清除显示，迟到响应不能恢复旧连接的目录。断线会明确提示读取失败，不代表空间被删除。目前不保存远端空间目录的离线快照，也没有目录增量同步或远程容器管理。

共享成功的目录直接展示；未共享和不可用设备集中在默认收起的“设备目录状态”中，显示分类数量，展开后可查看设备、检查时间、连接错误和授权说明。正在检查的设备只显示汇总进度，避免多设备重复提示占满页面。手动刷新保留展开选择，权限撤销仍会立即清除旧目录。空间节点与普通节点同样参与目录读取，也可展示其管理的下层空间。

目录记录不会自动创建空间节点的连接。要访问其中的终端、项目或桌面，仍需空间节点本身授权并配置可达的连接。停止、归档、恢复以及登录凭据只在管理设备原有的本机空间界面提供。

## CLI 与验证

~~~sh
cranibox-node space preflight
# 先阅读返回的安装命令，再手动使用本次 plan_id 确认；仅适用于对应平台
cranibox-node space plan linux-docker
cranibox-node space install linux-docker --plan-id REVIEWED_PLAN_UUID
cranibox-node space create "Agent lab" --provider docker
cranibox-node space list
cranibox-node space stop SPACE_UUID
cranibox-node space resume SPACE_UUID
cranibox-node space remove SPACE_UUID

# 在管理设备为查看方授予只读空间目录
cranibox-node peer grant --node VIEWER_NODE --output-token /private/viewer.token --spaces
# 查看方保存目标连接后读取目录
cranibox-node peer call --node MANAGER_NODE spaces
~~~

Web 使用有登录、Origin 和 CSRF 校验的 `POST /api/spaces`；Tauri 使用 `spaces` IPC，二者共用 Rust Store。CLI 等待操作结束，Web/Tauri 在后台运行并轮询。远端目录使用经目标 Token 和 NodeID 校验的只读 GET /api/v1/mesh/nodes/NODE/spaces；浏览器经 POST /api/peers 的 space_catalog 操作访问，目标凭据保留在后端。

`tests/test_virtual_spaces.py` 使用受限模拟 Docker 和真实 Rust 节点验证创建、身份验证、命令执行、失败重试、CLI 生命周期、停止与归档恢复、数据保留、镜像策略、资源归属、安装确认及认证边界，不会连接宿主 Docker。Rust 空间单元测试验证方案绑定/过期/消耗与重启恢复，并用独立 shell 夹具验证安装监控超时或取消后事务继续，不执行系统包管理器。tests/test_space_catalog.py 使用真实 Rust 节点验证只读目录、独立权限、撤销、错误数据、目标归属及请求期间连接/同一文件凭据变更；不连接 Docker。浏览器测试验证向导、安装命令确认、连续点击、响应丢失、共享目录、撤权及迟到响应、移动端布局和模拟 Tauri IPC 下的目录与终端入口。

2026-10-18 在远端 Linux 的真实 Docker 引擎完成了本地镜像创建、授权命令、停止、归档重建，以及身份和文件保留验证；实际浏览器也通过创建向导、空间浮动终端、关闭浏览器后附着原 Shell、最小化恢复、桌面与编辑器的 HTTPS 入口及 WebSocket，以及停止和归档恢复流程。375px 深浅主题布局已检查，详见[远端验证记录](remote-workspace-validation-2026-10-18.md)。使用与开发容器相同 KDE 基础镜像的验证叠加镜像，并非完整 Dockerfile 的干净构建；未验证仓库镜像拉取，也未执行 Windows/macOS/Linux 的系统依赖安装。Tauri 应用入口的 IPC 已测试，实际系统浏览器启动及 Windows/macOS 原生流程仍需对应环境验收。

2026-10-19 另从产品提交 `2299b4559846eb1019a2b9e5faa6ba3b17f7e0bd` 的归档完成 XFCE linux/amd64 的完整 Dockerfile 构建，本地镜像 `cranibox-image-validation:2299b45-xfce` 通过启动检查器 **44/44**，包括空配置和预置空间身份首次启动、授权、命令执行与重启后身份/文件保留，所属容器和卷均已清理。这不替代上述完整空间 UI 流程，也不表示当前 arm64 或公开仓库镜像已通过；具体来源、镜像摘要和证据见[交接文档](implementation-handoff-2026-10-18.md)。
