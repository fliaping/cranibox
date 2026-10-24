# 底座与上层互相拉起、互相更新：原型验证（2026-10-24）

`duo.py` 是一个 Python 原型，只验证**进程语义**，不验证 Rust 实现：
两个进程（base、upper）互相检测心跳、互相拉起；任一方可以更新另一方（写入新版本目录、切换指针、杀掉并拉起、
等待确认，超时则回滚指针并拉起旧版）；崩溃循环在次数上限后回滚。版本指针的规则与 `crates/workspace-node/src/update/layout.rs` 一致。

用法：`python duo.py <目录，路径须含 duo-exp> harness <阶段>`，阶段：`init`、`start [wmi]`、`status`、`kill <角色>`、
`req <执行方角色> <版本> <行为JSON>`、`crashloop <角色>`、`killboth`、`jobinfo`（仅 Windows）、`events <行数>`、`cleanup`。
`start` 扮演“操作系统的开机项”，只启动 base，由 base 启动 upper。

## 结果

| 场景 | Linux | macOS 27 (arm64) | Windows 11 |
| --- | --- | --- | --- |
| base 启动 upper；发起调用结束后两者仍存活 | 通过 | 通过 | **仅在启动方式不是节点 exec 的后代时通过**（见下） |
| 杀 base，upper 在约 1.4–2.0 秒内拉起它，upper 不受影响 | 通过 | 通过 | 通过 |
| 杀 upper，base 拉起它 | 通过 | 通过 | 通过 |
| base 把 upper 更新到好版本 | 通过 | 通过 | 通过 |
| base 更新 upper 到“不确认”的版本，超时回滚，失败版本记入 `failed` | 通过 | 通过 | 通过 |
| base 更新 upper 到“启动即崩溃”的版本，回滚 | 通过 | （同上类场景，未单独跑） | 通过 |
| upper 把 base 更新到好版本 | 通过 | 通过 | 通过 |
| upper 更新 base 到“不确认”的版本，由 upper 回滚并拉起旧 base | 通过 | 通过 | 通过 |
| 崩溃循环：3 次重试（带退避）后回滚 | 通过 | 通过 | 通过 |
| 两方同时被杀：没有人能拉起（预期，需要操作系统层） | 符合预期 | 符合预期 | 符合预期 |

## Windows 的关键发现

用节点 `exec` 启动 base 时，两个进程在**那次调用结束的同一时刻被终止**，即使 `spawn` 用了
`DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB` 且创建调用成功。
`jobinfo` 显示该调用所在作业的限制标志为 `0x3000`（`KILL_ON_JOB_CLOSE | SILENT_BREAKAWAY`）。
没有查明“脱离标志创建成功仍被终止”的确切机制（可能是作业嵌套，也可能是节点结束命令时的其他清理）。

改用 WMI（`Win32_Process.Create`，父进程为系统服务）启动 base 后，进程在发起调用结束后仍然存活，之后的全部场景通过。

结论：**base 和 upper 必须由开机项 / 外壳 / 服务管理器启动，不能是节点 `exec` 的后代。**
base 与 upper 之间互相拉起使用 `CREATE_BREAKAWAY_FROM_JOB`，创建成功；回退到不带该标志的方式也未触发失败（本次未走到回退路径）。

## 没有验证的

- Rust 实现、真实的文件替换（可执行文件改名替换已在平台信任实验中验证过）。
- 开机项 / 登录项 / 计划任务 / launchd / systemd 本身的拉起（两方同时崩溃后由谁拉起底座）。
- Tauri 外壳启动底座时所处的作业环境；本实验只覆盖了节点 exec 与 WMI 两种启动方式。
- 在网络、终端、桌面连接存在的情况下，重启一方对另一方的影响。
