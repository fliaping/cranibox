# 客户端分发

节点运行时只通过 npm 分发和更新；GitHub Actions 负责构建，GitHub Release 保留安装包、
独立 CLI 和许可证归档。首次发布前需要完成下方 npm 配置。

## 安装与设备更新

通过 npm 安装 CLI 需要 Node.js 22+：

```bash
npm install -g @fliaping/cranibox@latest
# 国内可使用镜像，二进制已包含在平台包中
npm install -g @fliaping/cranibox@latest --registry=https://registry.npmmirror.com
cranibox --version
```

已经运行受管运行时的设备可以直接更新，不需要 Node.js 或 npm：

```bash
cranibox update apply latest   # 正式版
cranibox update apply next     # 预发布
cranibox update apply dev      # main 的开发构建
# 同一 owner 的远程设备
cranibox update apply dev --node <节点ID>
```

节点读取 npm registry，下载平台包并核对 `dist.integrity`（SHA-512），再切换运行时；
未确认或崩溃时由启动器回滚。国内默认 npmmirror，镜像同步可能滞后，缺版本时回退其他
registry；可用 `CRANIBOX_NPM_REGISTRY` 覆盖。旧版本需要先手动安装具备更新能力的版本，
受管目录与启动方式见[节点远程更新](node-remote-update.md)。桌面外壳更新仍需重新安装。

## 平台包

入口包为 `@fliaping/cranibox`，三个平台包均包含**无 GUI 的独立 CLI / 节点运行时**：

| 平台 | 构建文件名 | npm 平台包 |
| --- | --- | --- |
| macOS Apple Silicon | `cranibox-macos-arm64` | `@fliaping/cranibox-darwin-arm64` |
| Linux x64 / glibc | `cranibox-linux-x86_64` | `@fliaping/cranibox-linux-x64-gnu` |
| Windows x64 | `cranibox-windows-x86_64.exe` | `@fliaping/cranibox-win32-x64` |

平台包中的命令位于 `bin/cranibox`（Windows 为 `bin/cranibox.exe`），同时附带 LICENSE 和
第三方许可证。入口包的 `optionalDependencies` 精确固定到相同版本；没有 postinstall，
安装时不从 GitHub 下载。CLI 在桌面应用打包前复制保存，避免被应用的同名二进制覆盖。

Linux glibc 下限取决于 CI 的 Ubuntu runner；暂不支持 Intel Mac、Linux arm64 或 musl。
打包时校验 ELF / Mach-O / PE 架构，并核对 `SHA256SUMS` 的文件清单与 SHA-256。
Release 中的校验清单覆盖全部安装包、CLI 和许可证附件；下载完整附件后可运行
`sha256sum --check SHA256SUMS`。现有桌面应用签名流程保持不变。

## 首次配置

1. 在 npm 创建入口包和三个平台包。首次发布可本机登录后按平台包、入口包顺序发布，
   或在 GitHub 的 `npm` environment 临时配置具有对应发布权限的 `NPM_TOKEN`。
2. 沿用 npm Trusted Publishing（OIDC）：为四个包配置 GitHub 仓库 `fliaping/cranibox`、
   workflow `rust-runtime.yml`、environment `npm`。手动补发的调用方是 `distribute.yml`，
   也需具备相应发布授权。复用工作流按调用方验证。流水线使用 Node.js 24、
   `id-token: write` 和 provenance；OIDC 配置完成后移除临时 `NPM_TOKEN`。
3. 设置仓库 Actions variable `CRANIBOX_PUBLISH_NPM=true`，启用正式版和 dev 自动发布。
   默认包名为 `@fliaping/cranibox`；如需更改，设置 `CRANIBOX_NPM_PACKAGE`，并同步修改
   安装说明和设备使用的包名。npmmirror 不需要发布凭据。

## 正式版、预发布与 dev

- **正式版 / 预发布：** 更新 CLI Cargo、Tauri、桌面前端与 npm 入口包的版本及相应
  lockfile，再创建不可变的 `vX.Y.Z` 或 `vX.Y.Z-beta.N` tag。CI 通过后创建 GitHub Release，
  调用 `distribute.yml` 下载归档、校验 SHA256SUMS 并打包。版本必须一致，禁止 build metadata。
  npm 按三个平台包、入口包顺序发布；正式版使用 `latest`，预发布使用 `next`。
- **开发构建：** 每次 main push（包括仅修改文档）在 desktop、native、Wayland 和分发测试
  全部通过后，由 `publish-dev` 发布 `<crates/cranibox 的 Cargo version>-dev.<github.run_number>`。
  `--version` 覆盖四个 npm 包的版本及精确依赖，不修改源码版本，也不检查 tag / Cargo
  版本一致性；仍严格校验 SemVer，禁止 build metadata。二进制 `--version` 仍为 Cargo
  版本。开发包显式使用 `--dist-tag dev`，不会推进 `latest` 或 `next`。

未启用发布变量时 CI 仍构建和测试。国内通知升级前应确认镜像已同步目标版本及全部平台包。

## 重试与验证

Actions 的 **Distribute Cranibox** 可输入已有 Release tag 补发；默认 `dry_run=true`，
只校验和打包，确认后关闭 dry run 发布。tag 必须包含发布工具。dev 重跑沿用原 run number，
版本不变。两条路径共用 `publish-npm.py`：已发布版本只有 `dist.integrity` 与当前 tarball
完全一致才跳过；内容不同则报错，必须发布新版本。跳过已有版本不会重设 dist-tag。

打包结果保存在 Actions artifact `cranibox-distribution-<tag>` 或
`cranibox-distribution-dev-<run_number>`，保留 14 天，包含校验后的产物、npm 包、
tarball 与 `publish-plan.json`。

```bash
# assets/ 包含三个 CLI、许可证及 SHA256SUMS
python3 scripts/release/build-npm.py --tag v1.1.0 --assets assets --output dist/npm
python3 scripts/release/publish-npm.py --directory dist/npm --dry-run

# 开发构建覆盖版本；使用另一空目录
python3 scripts/release/build-npm.py --version 1.1.0-dev.42 --assets assets --output dist/npm-dev
python3 scripts/release/publish-npm.py --directory dist/npm-dev --dist-tag dev --dry-run

python3 -m unittest discover -s tests -p test_distribution.py -v
```
