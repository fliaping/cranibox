# 客户端分发

Cranibox 使用托管平台分发：GitHub 构建并归档，GitCode 提供国内安装包入口，npm
提供 CLI 安装和升级。无需官网、对象存储或自建更新服务器。Homebrew 后续再接入。

此改动提供发布流水线和打包工具；必须完成下方账号配置并发布首个版本，才会有对应下载。
平台免费配额、附件大小和流量政策以账号实际显示为准，本项目不承诺无限免费托管。

## 下载与升级

- 国内 GUI / 独立 CLI：[GitCode 发行版](https://gitcode.com/fliaping0/agent-workspace/releases)。
- 海外及归档：[GitHub Releases](https://github.com/fliaping/cranibox/releases)。
- npm CLI：`npm install -g @fliaping/cranibox@latest`；国内可在这条命令后添加
  `--registry=https://registry.npmmirror.com`。示例包名在首次配置时确认。
- npm 安装需要 Node.js 22+；独立二进制和 GUI 不要求 Node.js。
- 每个 Release 的 `SHA256SUMS` 覆盖安装包、独立二进制及许可证附件。
  下载后用 `sha256sum --check SHA256SUMS`（Linux）、`shasum -a 256 <文件>`（macOS）
  或 PowerShell `Get-FileHash <文件> -Algorithm SHA256` 比对目标文件。
- npm 包由 npm 升级，不进行运行时自我替换；升级后需重启正在运行的节点。

这是安装包分发的实现。GUI 应用内自动更新尚未接入；目前通过发行版页面下载新版安装包更新。
后续接入 Tauri updater 时还需要更新签名密钥、平台更新产物和客户端界面，不能把 SHA256SUMS
当作客户端更新签名，也不能仅添加一个下载链接就声称支持应用内更新。

## 当前平台

| 平台 | GitHub 构建产物 | npm 平台包后缀 |
| --- | --- | --- |
| macOS Apple Silicon | `cranibox-macos-arm64` | `-darwin-arm64` |
| Linux x64 / glibc | `cranibox-linux-x86_64` | `-linux-x64-gnu` |
| Windows x64 | `cranibox-windows-x86_64.exe` | `-win32-x64` |

沿用现有原生构建矩阵；Linux glibc 兼容下限取决于 `.github/workflows/rust-runtime.yml`
的 Ubuntu runner。初版不宣称支持 Intel Mac、Linux arm64 或 musl/Alpine。
打包时校验实际 ELF / Mach-O / PE 头，runner 架构变化不会静默生成标错平台的 npm 包。

## 首次配置

### GitCode

默认沿用现有代码镜像 `fliaping0/agent-workspace`。若使用其他仓库，在 GitHub 仓库
Actions variables 中设置 `CRANIBOX_GITCODE_REPOSITORY=owner/repo`，并同步修改 README
下载链接。代码同步与附件发布使用相同的仓库变量。

在 GitHub 的 `gitcode` environment 配置：

| Secret | 用途 |
| --- | --- |
| `GITCODE_SSH_PRIVATE_KEY` | 沿用代码同步用的 SSH 私钥；有权向目标仓库推送 tag |
| `GITCODE_TOKEN` | GitCode 私人令牌；有权创建、编辑发行版并上传附件 |

流水线先推送**当前版本的 tag**，不强制覆盖远端 tag，再通过 GitCode API 创建发行版、
获取上传地址并用 PUT 上传文件。它不依赖另一个代码同步工作流何时完成。
原有代码同步工作流仍负责分支镜像；代码同步不代替附件发布。

附件上传后通过不携带发布令牌的下载 API 重新下载，核对 SHA-256。若平台要求登录、
返回 HTML、文件缺失或校验失败，任务失败，新的发行版不会被提升为 stable/latest。
请先上传实际安装包确认账号的附件额度，并在国内网络检查下载体验。

### npm

入口包为 `@fliaping/cranibox`，还需在 `fliaping` 个人作用域发布三个对应的平台包。
若以后改用其他作用域，在仓库 Actions variables 设置
`CRANIBOX_NPM_PACKAGE=@your-name/cranibox`。平台包名随之生成，如
`@your-name/cranibox-linux-x64-gnu`。若改名，请同步修改公开 README 的安装命令。

每个平台包包含原生二进制、项目许可证及第三方界面许可证；入口包提供 `cranibox`
命令。`optionalDependencies` 将平台包固定到**完全相同的版本**。没有 postinstall，
不从 GitHub 额外下载，不修改用户的全局 registry 配置。缺少平台包或版本不符时给出错误。

推荐使用 npm Trusted Publishing（OIDC）：

1. 首次创建这四个 npm 包：可以在本机通过 `npm login` 后发布生成的 `.tgz`，
   先三个平台包、后入口包；首次也可在 GitHub `npm` environment 临时配置具备对应
   包发布权限的 `NPM_TOKEN`，按 npm 的 2FA / token 政策完成首次发布。
2. 在每个包的 npm Settings / Trusted Publisher 添加 GitHub 仓库 `fliaping/cranibox`。
   自动发布的 workflow filename 填 `rust-runtime.yml`，environment 填 `npm`；
   若还要从 Actions 手动补发，额外添加 `distribute.yml`。可复用工作流按**调用方**文件验证。
3. 为 publisher 允许直接 `npm publish`。工作流使用 Node.js 24（npm 11.5.1+）和
   `id-token: write`，并生成 provenance。OIDC 配置完成后移除临时 `NPM_TOKEN`。

使用 `NPM_TOKEN` 时需要支持非交互发布的权限设置；流水线不会绕过 npm 的账号验证。
无需把 npm 发布凭据提供给 npmmirror；它是只读镜像。

最后，在仓库级 Actions variables 中按渠道启用自动分发：

| Variable | 值 |
| --- | --- |
| `CRANIBOX_PUBLISH_GITCODE` | `true` |
| `CRANIBOX_PUBLISH_NPM` | `true` |

未启用时仍按原流程发布 GitHub；不会因未配置 npm / GitCode 账号而自动上传。
仓库的 environment 可以配置自己的访问规则，脚本不额外要求人工审批。

## 发布一个版本

1. 更新 CLI Cargo 版本及 Cargo.lock、Tauri 配置、桌面前端 package.json / lockfile，
   以及 `npm/cli/package.json` 的版本。tag 必须对应同一个版本。
2. 将包含这些发布工具的变更合入主分支后，创建不可变的版本 tag，如 `v1.1.1`。
   预发布使用 `v1.2.0-beta.1`，不能使用带 build metadata 的 tag。
   GitHub Release 的 prerelease 状态必须与 tag 的预发布版本形式一致。
3. 现有 Rust / 桌面 CI 和分发测试通过后，发布 GitHub Release。
4. `.github/workflows/rust-runtime.yml` 直接调用 `distribute.yml`，下载该 Release 的
   安装包并核对 SHA256SUMS，再分发到已启用的平台。没有依赖 `release: published`
   事件：由 `GITHUB_TOKEN` 创建的 Release 不会再触发另一个工作流。
5. GitCode 完成全部附件校验后设置 latest；预发布保持 pre。npm 先平台包后入口包，
   正式版本使用 latest，预发布使用 next。GitCode 与 npm 任一渠道失败，不撤回另一渠道。
6. 国内升级通知前，在 npmmirror 检查入口包和全部平台包的目标版本都已同步；可在
   `https://npmmirror.com/sync/<包名>` 手动触发同步。镜像同步不是发布成功的即时保证。

发布脚本校验版本、文件清单、SHA-256 和二进制架构；不会代替完整的跨平台应用测试。
macOS Developer ID 签名 / Apple 公证沿用原构建配置，本改动不生成签名身份或绕过 Gatekeeper。

## 重试与本地验证

在 Actions 选择 **Distribute Cranibox**，输入已有 GitHub Release 的 tag，选择要补发的
渠道。默认 `dry_run=true`，只验证和打包，不推送 GitCode tag 或发布 npm。
确认打包结果后，关闭 dry run 才会实际发布。补发的 tag 必须包含发布工具；旧 tag 如需
使用新工具补发，应另行在本机检出相应源码并核对版本，不要移动已公开的 tag。

重试时，GitCode 只跳过公开下载字节完全一致的附件；同名文件不同则报错，不删除覆盖。
npm 只跳过 `dist.integrity` 与当前 tarball 一致的版本；不同则要求发布新版本。
不要用旧的正式版本执行发布来做归档补传，因为成功的稳定版发布会将该版本标记为 latest。

每次打包都生成 Actions artifact `cranibox-distribution-<tag>`（保留 14 天），其中包含
原始 Release 附件、npm `.tgz`、解包后的 npm 包及发布顺序 `publish-plan.json`，便于检查。

```bash
# 只读验证现有产物；assets/ 只放 Release 附件及 SHA256SUMS
python3 scripts/release/build-npm.py --tag v1.1.0 --assets assets --output dist/npm
python3 scripts/release/publish-npm.py --directory dist/npm --dry-run
python3 scripts/release/publish-gitcode.py --repository fliaping0/agent-workspace \
  --tag v1.1.0 --assets assets --notes-file notes.md --dry-run

# 离线发布逻辑与 npm 本地模拟仓库安装测试
python3 -m unittest discover -s tests -p test_distribution.py -v
```

GitCode API 参考：[创建 Release](https://docs.gitcode.com/docs/apis/post-api-v-5-repos-owner-repo-releases/)、
[附件上传地址](https://docs.gitcode.com/docs/apis/get-api-v-5-repos-owner-repo-releases-tag-upload-url/)、
[下载附件](https://docs.gitcode.com/docs/apis/get-api-v-5-repos-owner-repo-releases-attach-files-file-name-download/)。
npm 参考：[Trusted Publishing](https://docs.npmjs.com/trusted-publishers/)、
[npmmirror](https://npmmirror.com/)。
