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

## 首次发布（引导）

首次四个包尚不存在时，可直接暂存创建，无需先运行 `npm publish`。现有 `NPM_TOKEN`
只有 stage 权限：它可以暂存，但不能直接发布；暂存本身不要求 2FA，审批才需要。
确认该 secret 已配置在仓库或 GitHub 的 `npm` environment，然后按以下顺序操作：

1. 开启发布并选择暂存模式：

   ```bash
   gh variable set CRANIBOX_PUBLISH_NPM --body true --repo fliaping/cranibox
   gh variable set CRANIBOX_NPM_PUBLISH_MODE --body stage --repo fliaping/cranibox
   ```

2. 将包含这些发布工具的修改推送到 main（`git push origin main`），等待 CI 完成。
   CI 使用 Node.js 24，stage 模式在 setup-node 后升级 npm 并输出版本
   （要求 npm >= 11.15.0、Node >= 22.14），从解包目录按三个平台包、入口包顺序暂存。
   **stage 模式下每次 main push 都暂存一个 dev 构建**，每个 run 只调用一次实际暂存流程；
   引导完成后及时切换 direct。Actions summary 给出结果和审批入口。

3. 本机使用满足上述版本要求的 npm，以有审批权限且启用 2FA 的账号登录，先查看再审批：

   ```bash
   python scripts/release/approve-staged.py
   python scripts/release/approve-staged.py --approve
   ```

   脚本仅处理这四个包的所有待审批版本，先列清单，再用 getpass 隐藏输入一次 OTP，
   按平台包、入口包顺序连续审批。OTP 有效期很短，请一次完成四个包；失败后重跑，
   使用新的 OTP 处理剩余项。也可运行 `npm stage list @fliaping/cranibox --json`
   （平台包分别查询）、`npm stage approve <stage-id> --otp <OTP>`，或在 npmjs.com
   对应包页面审批。操作有误时用 `python scripts/release/approve-staged.py --reject`
   批量拒绝（同样需要 2FA），或逐项 `npm stage reject <stage-id> --otp <OTP>`。

4. 在 npm Settings 为入口包及上表三个平台包**逐个**添加 Trusted Publisher：
   GitHub repository 为 `fliaping/cranibox`，workflow filename 为 `rust-runtime.yml`，
   environment 为 `npm`；tag 发布及手动补发还需配置 `distribute.yml` 的发布授权
   （复用工作流按调用方验证）。流水线已有 `id-token: write` 和 provenance。

5. 四个包的 OIDC 配置完成后，切回直接发布并删除临时 token：

   ```bash
   gh variable set CRANIBOX_NPM_PUBLISH_MODE --body direct --repo fliaping/cranibox
   gh secret delete NPM_TOKEN --env npm --repo fliaping/cranibox
   # 如果 token 配置在仓库级，则删除仓库级 secret；两处都有则都删除
   gh secret delete NPM_TOKEN --repo fliaping/cranibox
   ```

   此后 main 的 dev 发布由 GitHub OIDC 自动完成，不需要 token、2FA 或人工审批。
   `CRANIBOX_NPM_PUBLISH_MODE` 未设置时默认为 `direct`，只接受 `stage` / `direct`。

**暂存会占用该 SemVer 版本号，且暂存的 dist-tag 不可修改。** 标签选错需要拒绝后重新
暂存，并使用新版本号（dev 可通过下一次 main push 生成）；不能覆盖不同内容的同版本。
默认包名为 `@fliaping/cranibox`；自定义时设置仓库变量 `CRANIBOX_NPM_PACKAGE`，
本机审批脚本设置同名环境变量或传 `--package-name @scope/name`，并同步安装说明和设备包名。
npmmirror 不需要发布凭据。

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
stage 模式还查询 `npm stage list`，必要时查询 `npm stage view`：同版本、同标签且
完整性相同才跳过已有暂存；标签或内容不同、元数据无法核实时均报错。
`--mode stage --dry-run` 只演练目录暂存，不查询线上版本、不暂存，也不要求 OTP。

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
