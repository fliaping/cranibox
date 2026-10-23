# 浏览器关闭后的终端会话管理评估

核验日期：2026-10-13。此文保留方案评估背景；当前代码已通过外部 herdr 实现持久会话，
见 [herdr 对接说明](herdr-integration.md)。

## 结论

已采用 herdr 作为独立后台会话引擎，保留 Cranibox 的 React + xterm.js 页面、
认证和远程节点 WebSocket 通道。需要新增适配层与会话管理页面，不必重写终端复用器。
若优先使用完整的现成 Web 界面，Zellij 是更直接的候选：官方已有 Web 服务与会话入口。

评估时从 GitHub 官方发布 API 核验的正式版本是 herdr v0.9.3（2026-09-29）和
Zellij v0.45.1（2026-08-28）。开发分支、在线文档和已安装二进制可能有差异；集成应
锁定版本，以二进制附带的 schema 和实际 CLI 参数为准。

## 候选项目

| 项目 | 会话管理 | Web 接入 | 适合本项目的方式 |
| --- | --- | --- | --- |
| herdr | 后台运行、命名会话、工作区、标签、分屏、Agent 状态和控制 | 正式版提供 CLI 和本地 Socket API；不是 React 组件 | xterm.js 显示 herdr 客户端；管理操作通过 CLI/API 适配 |
| Zellij | 后台会话、会话切换、分屏、布局恢复 | 官方内置 Web 服务、登录 token、会话 URL 和代理子路径配置 | 代理现有 Web 界面，或使用终端客户端作为本项目的会话后端 |
| tmux | 后台会话、窗口/分屏、列举/重命名/结束、重新接回 | 不自带 Web 界面，可接现有终端 | Linux/macOS 上简单稳定；Windows 原生统一接入需其他方案或 WSL |

herdr 使用 Rust，Apache-2.0；官方发布提供 Linux、macOS 和 Windows 二进制，但 v0.9.3 的 terminal attach 在 Windows 返回不支持，不能用于当前浏览器持久终端。
Zellij v0.45.1 的官方发布资产也包含 Windows MSVC 包，但其 FAQ 仍描述 Windows
通过 WSL 使用。发布资产与文档存在差异，不能据此直接承诺 Windows 功能与 Linux
完全一致。本次只做了 herdr Linux 实测，没有运行 Windows/macOS 或 Zellij 的原生测试。

## herdr 正式版核验

下载正式发布的 Linux x86_64 二进制到临时目录，使用独立的配置、状态、缓存和
运行目录启动名为 cranibox-evaluation 的后台会话。测试结果：

- 在 pane 中运行每秒写入一个标记的长命令。
- 终止第一个终端客户端后，标记数从 3 增加到 5。
- 断开前后 Shell PID 均为 1492641，说明持续运行的是同一个 Shell。
- 重新启动客户端，能够看到持续产生的 PERSIST_TICK 输出。
- session list 能列出运行中的命名会话；测试结束已停止测试服务。

这份评估验证 herdr 客户端断开和接回；当前集成另有真实 WebSocket 与浏览器回归。
结果记录在 [验证记录](validation/2026-10-13-herdr-session-engine.json)。

正式版 CLI 支持 session list/attach/stop/delete；命名会话的显示名称可由 Cranibox
维护，不能假定存在 session rename 命令。导出的 v0.9.3 JSON schema 没有提供
attach_terminal、resize_terminal 或 subscribe_terminal 这样的网页原始 PTY 接口。
管理 API 能读 pane、发送输入与查询状态，但不能把读取文本快照等同于可靠的实时
ANSI 终端流。初期应通过 herdr 终端客户端桥接，不直接复制它的内部二进制协议。

## Cranibox 的调整方向

1. 会话由执行机器上的后台引擎拥有。网页 WebSocket 只负责接入和脱离；网页关闭、
   切换页面、网络断开、心跳超时均不得结束后台会话。
2. 当前工作区启动本机引擎；远程机器由该机的 cranibox 节点管理本机引擎，通过
   既有向外连接的 WebSocket 汇总会话，不要求浏览器直接访问每个远程节点。
3. 会话页面提供创建、显示名称、节点/目录、运行状态、最后接入时间、重新接回、
   脱离和明确的结束操作。原“断开”改成“脱离”，“结束会话”才销毁后台工作。
4. 继续使用现有登录、CSRF 与一次性接入票据。引擎的本地 Socket 不直接暴露给浏览器；
   列举与接入都需要校验会话归属，不能只凭可猜测的会话名称操作。
5. 调整远程协议：关闭浏览器 attachment 与关闭执行会话是两个不同的操作。
   Node 与 Gateway 临时断连时，后台引擎仍保留会话，重连后重新列举与接入。
6. 后台引擎要独立于临时 PTY 的进程清理范围，尤其不能被 Windows 的临时终端
   Job Object 或 Unix PTY 会话清理一起结束。退出引擎才处理它的 Shell 子进程。

复用 herdr 或 Zellij 都会增加一个外部二进制及后台服务进程；herdr 的命名会话有
各自的服务端点。运行成本和打包方式应在实际接入后测量，不能用“Rust”直接推断
资源一定低于当前实现。直接将完整程序合并成 Cranibox Rust 模块需要维护源码和
内部接口，不建议作为第一步。

浏览器关闭后保持运行与机器重启后恢复是不同保证。herdr 引擎持续运行时保留
真实进程；引擎或系统重启后恢复布局/部分 Agent 对话，不代表任意旧 Shell 进程
继续运行。第一阶段保证网页关闭与连接中断后的会话存活。

## 一手资料

- [herdr 官方仓库与许可证](https://github.com/herdrdev/herdr)
- [herdr v0.9.3 正式发布](https://github.com/herdrdev/herdr/releases/tag/v0.9.3)
- [herdr Socket API](https://herdr.dev/docs/socket-api/)
- [Zellij Web 客户端与子路径配置](https://zellij.dev/documentation/web-client.html)
- [Zellij v0.45.1 正式发布](https://github.com/zellij-org/zellij/releases/tag/v0.45.1)
- [Zellij 平台 FAQ](https://zellij.dev/faq/)
- [tmux 官方仓库](https://github.com/tmux/tmux)
