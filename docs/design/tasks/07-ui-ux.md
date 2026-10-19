# WS7 任务书：UI/UX——个人 Agent 式界面 + App API

> 估算：5 人周 · 建议执行者：Claude Code（前端 + 少量 Rust） · 分支前缀：`ws7/`

## 背景（必读）
- 仓库：`github.com/fliaping/cranibox`（Rust workspace + React + Tauri）。Cranibox 是一个自托管、高可信的 Agent Brain：节点是「脑壳」，Agent（DeepSeek Harness/DSH，以及 Codex、Claude Code、Pi、Hermes、OpenClaw）是「大脑」。它提供设备互联（端到端加密）、人工审批、Computer Use 基础设施和统一的模型订阅代理。
- 用户面向**不懂编程的普通人**，体验对标 Meta Muse、Grok Bot、Manus Cue、OpenAI Dots 这类 Personal Agent。客户端**没有「连接设置」**，配对用扫码或链接，审批用卡片。终端不是重点。高级功能收在「高级」里。
- 只有**一台固定的常驻节点（Hub，Linux 上的 `cranibox serve`）**。机器之间的数据一律 E2E 加密。
- 总体设计在仓库的 [`docs/design/ARCHITECTURE.md`](../ARCHITECTURE.md)，分工和契约在 [`docs/design/WORKPLAN.md`](../WORKPLAN.md)。**先读这两份文档里与你相关的章节，再读本任务书。**
- 共享契约在 `crates/cranibox-contracts`（Phase 0 已合入）。**不要自行修改契约。** 需要改时，在 PR 里写 `CONTRACT-REQUEST: ...` 并通知协调者。

## PR 规则（必须遵守）
1. 从最新的 `main` 切分支，分支名用本任务书指定的前缀。一个任务可以拆成多个小 PR，每个 diff 尽量不超过 600 行；拷贝的第三方代码单独提一个 PR。
2. PR 标题写 `[WSx] <简述>`。描述里写：做了什么、是否涉及契约、怎么测的、风险，以及需要人工验收的步骤。
3. 合并前 CI 必须全绿：`cargo fmt --all --check`、`cargo clippy --workspace --locked --all-targets -- -D warnings`、`cargo test --locked`；改了 UI 还要跑 `node scripts/build-control-center-ui.cjs && git diff --exit-code` 和 Playwright。
4. **禁止 force-push、rebase 或 amend 已推送的提交。** 跟进 main 时用 `git merge origin/main`。`Cargo.lock` 冲突时，先合并再运行 `cargo update -w`，不要手改。
5. **只改「文件」一节里列出的路径。** 别的工作流拥有的文件（见 WORKPLAN §2）一律不动，需要时向协调者提需求。
6. 不要把真实凭据写进仓库、测试或日志。测试用 mock 上游和假 key。
7. 移植或拷贝第三方代码时，保留原版权头，或加上 `Ported from <project> <path>@<commit> (<license>)`，并在 PR 描述里列出，由协调者更新 `NOTICE`。
8. 每个 PR 都要协调者评审。不要自己合并到 main。

## 目标
把控制中心和 Tauri 客户端改造成**面向普通人的 Personal Agent 界面**：
- 第一屏就是能用的对话（嵌入 DSH Web UI）；
- **删掉所有「连接设置」**（服务端地址、Token、直连路径等移出主流程）；
- 配对 = 扫码 / 链接；需要人决定的事 = 审批卡片；
- 高级功能收进「高级」。

完整信息架构与线框见 `ARCHITECTURE.md` §8。本任务书只列实现要点与验收。

## 范围

### A. App API 聚合层（Rust）
新增 `crates/workspace-web/src/app_api.rs` 与 `apps/node-desktop/src-tauri/src/app_api.rs`，实现 C6：
- `GET /api/v1/app/status` → 状态胶囊
- `GET /api/v1/app/devices`
- `POST /api/v1/app/pair/{invite,accept}`、`POST /api/v1/app/pair/{flow}/confirm`
- `GET/POST /api/v1/app/approvals`
- `/api/v1/app/models/*` 代理 C3 mgmt
- `GET/PUT /api/v1/app/agents`
- `GET /api/v1/app/events`（SSE）

Tauri 侧同名 IPC `app_*`，共用 JSON。前期用 `cranibox-contracts` 的 `MockAppApi`；各后端就绪后逐个切换，feature flag `CRANIBOX_APP_MOCK=1` 可回退。

数据来源：held messages / inbound（#9/#12）、配对（WS1 `pair_*`）、llm mgmt（WS2）、名册（WS6）、agents（WS5）。某源未就绪时返回空列表或占位，不阻塞 UI。

### B. 信息架构重排（React）
主导航五栏：**首页（对话）**、**收件箱**、**设备**、**模型**、**设置**；「高级」为设置下二级。

| 现有 | 新位置 |
|---|---|
| Overview | 拆入首页 + 状态胶囊 |
| Nodes、devices/* | 「设备」；DirectAccess / DirectPaths / DirectListener 等 →「高级 › 网络」 |
| NetworkPage、Tailscale*、NetworkProviders、NetworkInvitations | 「高级 › 网络」；邀请并入「添加设备」 |
| Messages、HeldMessages | 「收件箱」 |
| Agents | 「设置 › Agent」+「模型」 |
| TerminalPage、Services、Resources、Performance、Diagnostics、History、Projects、Spaces、Desktop、Browser | 「高级」 |
| Onboarding | 重写为新引导（§8.3.1） |

### C. 关键新界面（必须做）
1. **引导**：欢迎 → 扫 Hub 二维码 → SAS 核对 → 可选添加订阅 → 进入首页。
2. **添加设备**：二维码 + 链接 +「我的 / 访客」选择；访客勾选能力。
3. **首页**：iframe / webview 嵌入 DSH（`/proxy/3080/`）；顶栏含设备切换、状态胶囊、审批铃铛；右侧「任务」抽屉。DSH 未就绪时显示安装进度卡。
4. **状态胶囊**：五种状态文案见 ARCHITECTURE §8.3.5；点击展开小面板。
5. **收件箱 / 审批卡**：统一 `ApprovalCard` 组件（配对 / 消息 / 能力 / Agent 工具 / Computer Use / 模型写操作）；桌面通知与托盘可批准。
6. **模型页**：订阅卡片（「本机启用」「在所有节点启用」、逐节点展开）；Claude 卡无同步开关，仅「设为 Claude 节点」；「各 Agent 用哪个模型」下拉（DSH 第一并标推荐；「自动」为首项）。
7. **添加订阅**底部表单：账号登录 / 国内套餐 / 粘贴 Key / 自定义；OAuth 打开浏览器；设备码显示代码；Claude 多一步确认节点。
8. **设备页**：我的设备（在线、直连/中转、RTT、徽标）与访客设备（能力开关、撤销）。
9. **高级**：进入提示「面向熟悉网络和命令行的用户」；收纳原网络/终端/诊断等。

### D. Tauri 客户端
- 与控制中心共用 React，适配器 IPC vs HTTP。
- **删除** `apps/node-desktop/src/main.tsx` 里「服务端地址 / 节点 Token」表单与 `connection` 区块；旧配置只读保留在「高级 › 网络 › 旧版 Workspace 中继」。
- 托盘：状态行、「N 条待审批」、打开、暂停共享屏幕、退出（保留 #14 的 Linux 菜单项做法）。
- 首次运行未配对 → 直接进引导。

### E. design-system
新增组件：`Card`、`Pill`、`Sheet`（底部表单）、`ApprovalCard`、`QrPanel`、`StatusPill`。沿用现有配色变量；暖白 / 深色两套。中英双语。

### F. 生成物
改 UI 后必须跑 `node scripts/build-control-center-ui.cjs`，提交更新后的 `addons/control-center/{web.js,web.css,index.en.html,index.zh-cn.html}`。**禁止手改这些生成文件。**

## 不在范围内
iroh / vault / llm 业务逻辑；改 adapter 写配置逻辑（WS5）；新做对话页（用 DSH Web UI）。若用户提供 Claude-Design / UI/UX Pro Max 技能来源，协调者安装后再用；否则按 ARCHITECTURE §8 + 现有 design-system。

## 文件
`addons/control-center/ui/**`、`addons/control-center/{web.js,web.css,index.*.html}`（生成）、`apps/node-desktop/src/**`、`apps/node-desktop/src-tauri/src/app_api.rs`、`crates/workspace-web/src/app_api.rs`（及路由挂载一行）、`design-system/**`、相关 Playwright 测试。

## 契约
实现 C6 App API；消费 C1 / C2 PathInfo / C3 mgmt / C4 agents（经 mock 或真实）。

## 步骤
1. App API 骨架 + Mock；状态胶囊与空壳五栏导航。
2. 引导 + 配对（接 WS1 mock → 真实）。
3. 首页嵌入 DSH；收件箱审批卡（接已有 held messages）。
4. 模型页 + 添加订阅表单（接 WS2 mgmt mock → 真实）。
5. 设备页；删 Tauri 连接表单；「高级」收纳。
6. Playwright 7 条用户路径；截图交付用户评审。

## 验收标准
- [ ] Playwright：引导、配对、添加订阅、选模型、审批、设备、高级，7 条路径通过（先 mock 后真实）。
- [ ] 主流程任何页面**不出现**「服务端地址」「Token」「relay」「Tailscale」「EasyTier」「IP:port」字样（「高级」除外）。
- [ ] Tauri 首次运行无连接表单，直接引导。
- [ ] 现有 130 个 UI 用例：仍适用的继续通过；迁到「高级」的更新选择器后通过。
- [ ] 截图（1280×800，中英各一套）交付：引导、首页、添加设备二维码、SAS 核对、模型页、添加订阅、审批卡、设备、状态胶囊五种状态、高级入口。
- [ ] `build-control-center-ui.cjs` 后 `git diff --exit-code` 通过。

## 怎么测
- `npm test --prefix addons/control-center/ui`
- `npm test --prefix apps/node-desktop`（含 Playwright）
- `CRANIBOX_APP_MOCK=1` 本地看全流程；联调时关 mock
