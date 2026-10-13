# cua-driver 与 agent-desktop-bridge

检查日期：2026-10-12。这里的 cua-driver 指 Cua 官方项目。当前容器没有找到
本地 cua-driver 可执行文件；结论来自官方资料与现有 bridge 源码，尚未做实机替换实验。

## 结论

当前 KDE/Wayland + Selkies 桌面继续保留 agent-desktop-bridge。
cua-driver 可作为后续 Windows/macOS/X11 节点的候选驱动，但需要在各节点测试。
应分开考虑底层桌面控制和 Workspace 的统一节点接口：驱动验证通过后，可以将
bridge 缩成驱动适配层，而不必保留第二套截图和输入实现。

## 能力比较

| 职责 | 现有 bridge | cua-driver 评估 |
| --- | --- | --- |
| 桌面截图与输入 | 调用 Selkies Pixelflux computer-use，使用浏览器所见桌面 | 有原生截图与输入；必须验证实际会话、显示器和坐标一致 |
| 原生窗口识别 | XWayland/xdotool 有限制；Wayland 焦点使用 wlrctl 或坐标后备 | 窗口与无障碍工具更完整；KDE/KWin 仍是实验级支持 |
| MCP | 本项目提供 STDIO MCP 适配器 | 官方已有 MCP，可减少自建工具重复 |
| 会话与权限 | Workspace 会话 TTL、状态和持久化 emergency pause | 官方有能力清单、会话期限、策略和 revoke；不应再造同类驱动权限系统 |
| 紧急停止 | Workspace 持久化暂停，恢复入口统一 | revoke --all 暂停当前运行时，重启后需由 Workspace 重新施加持久化暂停策略 |
| 浏览器控制 | 与 Workspace 托管浏览器流程集成 | 存在专门浏览器工具，但 KDE Wayland 通用路线有只读限制 |
| 节点接入 | 当前只服务本机；远端应走 Workspace 节点协议 | 驱动/MCP 接入不自动完成 Workspace 节点注册、认证、重连和命令协议 |

## 当前阻碍

官方平台表将 KDE/KWin 标为 Experimental：KWin helper 为只读，目标寻址的
原始输入会被拒绝。Wayland 默认通过实际 X11 窗口使用 XWayland 路线，原生
Wayland 必须显式启用。不能把某个 XWayland 应用测试通过，等同于整个 KDE 桌面可控。

现有 bridge 还维护 Selkies 后端状态、截图尺寸和坐标转换，以及 Workspace
对暂停/恢复的统一入口。这些职责不会因驱动已有 click/screenshot 工具而自动消失。
同时，cua-driver 已有权限和 revoke 功能，不能以“没有紧急停止”作为保留 bridge 的理由。

## 可替换的验收条件

1. 对当前 Selkies 的同一个 KDE 会话完成截图、点击、拖动、键盘、Unicode 输入、
   滚动和原生 Wayland 窗口切换，覆盖动态分辨率。
2. 多个 Agent 共享驱动时，正确执行会话范围、到期和撤销，停止后确实没有残留输入。
3. Workspace 紧急停止立即撤销驱动权限；驱动重启、节点重连和 Workspace 重启后
   仍保持暂停，直到用户通过统一入口恢复。
4. 托管浏览器的授权和已有用户浏览器绑定不受影响；跨平台安装和更新可维护。

达到上述条件后，可让 Agent 直接使用 cua-driver MCP，或由 Rust 节点调用其 SDK。
Workspace 只保留节点路由、健康检查和暂停状态管理。当前无需安装替换或重写 bridge。

## 官方资料

- [平台支持与已知边界](https://cua.ai/docs/cua-driver/concepts/platform-support)
- [权限、能力清单与撤销](https://cua.ai/docs/cua-driver/guides/permissions)
- [Cua Driver 概览](https://cua.ai/docs/cua-driver)

本地实现依据：scripts/agent-desktop-bridge.py、scripts/agent-desktop-mcp.py
和 crates/workspacectl/src/desktop.rs。
