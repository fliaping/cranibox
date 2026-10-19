# 协议转换代码来源

本次引入的代码与项目自身实现分开提交。保留各项目许可证；根目录 NOTICE 由 WS0 协调者更新。本文不授予第三方订阅或商标使用权限。

| 来源 | 固定提交 | 许可证 | 本仓库位置与改动 |
|---|---|---|---|
| pleaseai/shunt | `9d42ba251dd1f0651e9d150767e3ff98a1ae042b` | MIT OR Apache-2.0 | `src/proto/vendor/shunt/model/**`：选取 Responses、Messages、Gemini 请求和事件转换器及原测试；调整 import、去掉上游服务依赖。`compat.rs` 为本项目适配类型。许可证在同级目录。 |
| link-assistant/router | `07901ad24576e7f045270fe62ac56f8a72446c53` | Unlicense | `src/proto/chat.rs` 参考 `src/openai.rs`、`openai_tools.rs` 的 Chat→Messages 映射重新实现；许可证在 `src/proto/vendor/link_assistant/LICENSE`。 |
| yetone/magpie | `865b61808690bc9fd716257949414ec97618a5a5` | MIT | `src/proto/port/magpie/mod.rs` 从 `internal/gateway/{anthropic,chat,chat_call_ids,chat_tool_shape}.go` 移植 Messages→Chat 映射；许可证在同目录。`upstream/cn/mod.rs` 移植 schema 规则；OAuth/Copilot、智谱信封和地址修正规则按来源注释重新实现。 |

`auth/signin.rs` 的 OAuth 字段、`upstream/{codex,kimi,gemini}` 的协议字段也参考上述 shunt 固定版本；没有引入其路由、账号库或运行时。

所有回放 fixture 均为合成数据，不含用户对话或真实凭据。第三方转换器保留上游测试；本项目额外测试 3×3 协议、工具、reasoning、分片与截断行为。
