# Claude Code `cch` 观测（2026-10-19）

结论：本次官方 CLI → 本地网关的五次请求中，未观测到 `cch`。不能据此认定真实订阅上游不检查它，也不能得出它的算法或固定值。当前不实现猜测性的 `cch` 生成器。

环境：`admin@192.168.217.6:22022` 的 `cranibox-validation`，Linux x64、用户 abc。未使用本机运行环境或真实账号。官方 npm 包 `@anthropic-ai/claude-code-linux-x64@2.1.303`，压缩包 SHA-256：`2b073f9f6c68882f94a31dbd1b625fa5b8df078715e487aa68e3b0530d0b43f3`。

| 模式 | 次数 | 实际认证头 | billing 字符串中的 cch | JSON key / HTTP header 中的 cch |
|---|---:|---|---|---|
| 假 API key | 3 | x-api-key | 无 | 无 |
| 假 OAuth token | 2 | Authorization: Bearer | 无 | 无 |

全部请求为 `/v1/messages?beta=true`，模型 `claude-sonnet-4-5`，SSE；五次 CLI 都以 0 退出。`Reply OK only.` 对应 `cc_version=2.1.303.41b`；`Reply HELLO only.` 对应 `cc_version=2.1.303.eeb`。重复前者得到相同后缀；两种认证模式也相同。这是样本观察，不是后缀算法结论。

## 证据与复现

同目录两个 JSON 文件只保留认证头类型、billing 字符串、请求摘要和退出结果，不包含凭据或完整请求。`probe_cch.py` 启动随机 loopback 端口的 mock，使用独立配置、空项目、假凭据，关闭会话保存和非必要遥测。Python 3.12+：

```sh
mkdir -p /tmp/claude-cch
cd /tmp/claude-cch
npm pack @anthropic-ai/claude-code-linux-x64@2.1.303
python3 /path/to/probe_cch.py /tmp/claude-cch
python3 /path/to/probe_cch.py /tmp/claude-cch --oauth
```

## 仍需真实环境验证

本地 mock 不执行 Anthropic 的认证或请求校验；设置自定义 base URL、假 token，以及账号/服务端特性开关都可能影响 CLI 路径。尚未在已授权真实订阅、默认官方 base URL 上观察 `cch`，也未验证自写 HTTP 客户端会被订阅上游接受。WS4 的 HTTP 可行性暂为**未证实**。

下一步由已授权 Claude 节点的真实账号验证官方 CLI 与候选适配器；仅记录字段存在性、状态码和脱敏差异。若 HTTP 方式无法稳定使用，采用设计 D1 的官方 CLI 桥。不得把本报告的 mock 成功当作真实订阅验收。
