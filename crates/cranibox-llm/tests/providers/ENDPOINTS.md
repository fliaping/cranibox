# P2 国内厂商入口核对

核对日期：2026-10-19。以下为官方文档地址和预设选择依据，不代表已用真实付费账号验收。所有自动测试都使用本机 mock 和假 key。

| 预设 | 官方参考 | 对接方式 |
|---|---|---|
| `stepfun` / `stepfun-global` | https://github.com/stepfun-ai/Step-3.5-Flash/blob/main/README.md | 国内/国际各自的 Chat `/v1`、Messages 根路径；官方配置使用 Bearer 认证 |
| `mimo` | https://platform.xiaomimimo.com/#/docs/getting-started/quickstart | Chat `/v1`、Messages `/anthropic`，两种协议都使用 `api-key` |
| `mimo-token` | https://platform.xiaomimimo.com/#/docs/getting-started/beginner-guide | 独立 Token Plan host；当前文档使用 `token-plan-api.mimo.mi.com` |
| `qianfan` | https://cloud.baidu.com/doc/qianfan-docs/s/4mh4su2pb | OpenAI 兼容 `/v2` + Bearer API key，避免沿用旧版 access_token 认证 |
| `hunyuan` | https://cloud.tencent.com/document/product/1729/111007 、https://cloud.tencent.com/document/product/1729/128203 | 旧 OpenAI 兼容 `/v1`、Messages `/anthropic`；文档提示 2026-04-17 起新模型转到 TokenHub |
| `tencent-tokenhub` | https://cloud.tencent.com/document/product/1823/126337 、https://cloud.tencent.com/document/product/1823/126343 | 独立开通的 key；Chat `/v1`、Messages 根路径（没有 `/anthropic` 前缀） |

同一厂商的套餐、按量及不同平台分别保存为独立预设，不根据 key 前缀猜 host，不在失败时跨平台重试。MiMo 的认证头与通用 OpenAI / Anthropic 头不同，由 `mimo_api_key` quirk 同时覆盖模型发现和请求转发。

静态模型表仅提供初始候选，不保证账号有权限或当前地区仍提供该型号；真实账号验收需核对厂商控制台。TokenHub Responses 当前存在模型限制，未在本预设中宣告支持；可经代理转换 Chat / Messages。mock 只验证客户端地址拼接和认证，不能证明真实账号权限或付费请求成功。
