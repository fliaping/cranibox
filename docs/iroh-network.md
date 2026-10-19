# iroh 默认网络：内置 relay 与链接/二维码配对 / iroh default network

> WS1. Code: `crates/cranibox-net`, `crates/workspace-node/src/peer/transport_iroh.rs`,
> `crates/workspace-web/src/relay.rs`. Acceptance: `tests/integration/iroh_netns.sh`.

## 1. 是什么 / What it is

- 每台设备都用 iroh 1.3（QUIC）连接其他设备：先尝试打洞直连，失败时经 relay 中转。用户界面里不需要任何连接设置。
- 设备的 iroh 身份就是 `node-identity.json` 里的 ed25519 签名密钥（PR #7），所以 iroh `EndpointId` 就是配对时固定下来的 `signing_key`。拨号后会检查 TLS 认证得到的 `remote_id` 是否等于配对时的公钥，不一致就断开。
- Hub（`cranibox serve`）在控制中心同一个端口上内置 relay：`/relay`（WebSocket 升级）、`/ping`、`/generate_204`。只有已配对的 EndpointId（`peers.json` 里的 grant/route 固定公钥）和持有未过期配对链接的设备能使用它。
- relay 顺序：自有 relay（Hub 的 `--public-url`、配对时学到的 Hub relay、`networks.json` 里配置的）优先；自有 relay 连续 10 秒不可用时加入 n0 公共 relay 兜底，自有 relay 恢复 60 秒后再移除 n0。QAD 对自有 relay 关闭（经 TLS 反代或 Cloudflare Tunnel 时没有 UDP）。

Every device talks to other devices over iroh 1.3 (QUIC): hole punching first, relay when that fails. The node's ed25519 signing key is the iroh secret key, so the EndpointId is the key pinned at pairing; `remote_id` is checked on every dial. The hub embeds an allowlisted relay on the control-center port. Own relays are used first; the n0 public relays are added only while every own relay has been unreachable for 10 s, and removed after the own relay is reachable again for 60 s. QAD is off for own relays.

### ALPN

| ALPN | 用途 / use | 鉴权 / auth |
|---|---|---|
| `cranibox/peer-http/1` (C2) | 进程内调用方（`IrohTransport`，供 WS6/WS7）。服务端用 `Running::mesh_router()` 处理，注入 `Authenticated{transport: Iroh, e2e: true}` | iroh TLS 公钥 = grant 固定的 `signing_key`，且 `grant.usable()` |
| `cranibox/peer-fwd/1` | 本机回环转发（CLI、MCP、网关等进程外客户端）：`127.0.0.1:<port>` 原样接到对端 | 仍需路由自己的 bearer / Noise 会话（同机其他用户不能借用本节点身份） |
| `cranibox/pair/1` | 配对握手（持有一次性 secret 才能用） | 链接里的 16 字节 secret |

`peer-fwd` 与 `pair` 尚未写进契约，见 PR 里的 CONTRACT-REQUEST。

## 2. Hub 部署 / Deploying the hub

```bash
cranibox serve --public-url https://hub.example.com          # relay 默认开启（https public URL）
cranibox serve --public-url https://hub.example.com --no-relay  # 不跑 relay，设备用 n0
cranibox serve --relay                                         # 无 https public URL 也强制开启（仅调试）
```

- `cranibox serve` 只监听 loopback（默认 `127.0.0.1:9080` 或 `--port`）。TLS 由前面的反向代理或隧道终止，代理以纯 HTTP 转发到这个端口。
- **relay 必须位于 public URL 所在 origin 的根路径**：iroh 客户端总是请求 `<origin>/relay`。不要把 cranibox 挂在子路径下。
- 启动约 2 秒后做自检：从服务器自己访问 `<public-url>/relay`，期望 `101 Switching Protocols`。日志：
  - `iroh relay self-check: https://hub.example.com/relay OK (101)`
  - `iroh relay self-check FAILED: …`（附原因和修改建议，见第 5 节）

### nginx

```nginx
map $http_upgrade $connection_upgrade { default upgrade; '' close; }
server {
  listen 443 ssl;
  server_name hub.example.com;
  ssl_certificate     /etc/ssl/hub.pem;
  ssl_certificate_key /etc/ssl/hub.key;
  location / {
    proxy_pass http://127.0.0.1:9080;
    proxy_http_version 1.1;                       # 必须 / required
    proxy_set_header Upgrade $http_upgrade;       # 必须 / required
    proxy_set_header Connection $connection_upgrade;
    proxy_set_header Host $host;
    proxy_buffering off;
    proxy_read_timeout 60s;                       # relay 有心跳，60 s 足够（实测空闲 95 s 不断）
  }
}
```

Caddy 默认转发 Upgrade：`hub.example.com { reverse_proxy 127.0.0.1:9080 }`。

### Cloudflare Tunnel

```yaml
# ~/.cloudflared/config.yml
tunnel: <tunnel-id>
credentials-file: /root/.cloudflared/<tunnel-id>.json
ingress:
  - hostname: hub.example.com
    service: http://127.0.0.1:9080
  - service: http_status:404
```

- Zone 设置里保持 **WebSockets 开启**（默认开）。不要对 `/relay` 启用 Access 登录拦截（设备没有浏览器会话；relay 自己按 EndpointId 鉴权）。
- `cloudflared` 连接器需要能出站访问 Cloudflare 边缘的 7844 端口（UDP/QUIC 或 TCP/HTTP2，`--protocol http2`）。
- 人工验收步骤见 PR 描述。

### 端口 / Ports

| 位置 | 端口 |
|---|---|
| Hub 对外 | TCP 443（代理或隧道）；不需要开放 UDP |
| cranibox serve | loopback `--port`（默认 9080），`--gateway-port` 兼容监听 |
| 设备 | 只需出站 TCP 443；允许 UDP 时会自动打洞直连（`bind_port` 可固定 UDP 端口，默认随机） |

## 3. 配对 / Pairing

```bash
cranibox-node peer pair invite [--guest] [--qr invite.svg]   # 生成一次性链接 + 二维码（15 分钟，最多 16 张未用）
cranibox-node peer pair accept 'cranibox://pair/…'          # 另一台设备打开链接，显示配对码
cranibox-node peer pair flows                                # 邀请方看到同一个配对码
cranibox-node peer pair confirm <flow_id> [--reject]         # 两边都确认配对码一致
cranibox-node peer pair status                               # EndpointId、relay、每个对端的路径（direct/relay、RTT）
```

- 链接 `cranibox://pair/<base32>`，网页版 `https://<hub>/pair#<base32>`（Hub 才有），内容是 C2 `PairTicket`（邀请方 EndpointId、relay URL、16 字节一次性 secret、过期时间、用途）。
- 两边显示的配对码（SAS）由双方身份派生（复用 #7/#13 的 `pairing_code`），双方都在各自设备确认后才交换凭据、保存授权。只有一方确认时仍等待；任一方拒绝或超时都不写入。CLI 确认命令会等待另一端，请在两台设备分别执行。
- `OwnerDevice`（默认）：双方互相发 grant（全部能力，含 llm/vault），入站消息直接接受；`Guest`：只给对方 messages，入站消息待审。名册（roster）加入由 WS6 接手。
- 确认后：grant 固定对方身份并标记已核对；route 指向本机回环转发 `http://127.0.0.1:<port>`（带 lease 锁），旧路径保留为备用；Hub 的 relay 记入 `iroh-peers.json` 并立刻开始使用。
- 已有直连路由的对端（已固定公钥）：设置 `CRANIBOX_IROH_MIGRATE=1` 后，服务每 10 分钟探测一次，对端能用 iroh 时把 iroh 回环转发插到路由最前面。默认关闭：改路由会改变路由 binding，排队中的消息和任务会因此作废（重新配对则直接走 iroh）。

## 4. 高级覆盖（不在 UI 中）/ Advanced overrides

`networks.json`（与 Tailscale/EasyTier 同一文件）：

```json
{ "iroh": { "enabled": true, "relays": ["https://relay.example.com"], "n0_fallback": true, "bind_port": 0 } }
```

- `enabled: false` 关闭 iroh（其他传输不受影响）；`relays` 非空时不再自动采用配对学到的 relay；`n0_fallback: false` 时只用自有 relay。

环境变量（测试、私有 CA）：

| 变量 | 作用 |
|---|---|
| `CRANIBOX_IROH_RELAYS=https://a,https://b` | 覆盖 relay 列表 |
| `CRANIBOX_IROH_EXTRA_ROOT=/path/ca.der` | 额外信任一个 CA（DER），用于自签证书的 relay；自检也会使用 |
| `CRANIBOX_IROH_RELAY_ONLY=1` | 不开 UDP，只走 relay |
| `CRANIBOX_IROH_NO_LOOKUP=1` | 关闭 DNS/pkarr 地址发现（离线环境） |
| `CRANIBOX_IROH_MIGRATE=1` | 已固定公钥的直连路由自动改为 iroh 优先 |

## 5. 排障 / Troubleshooting

| 现象 | 原因与处理 |
|---|---|
| 自检 `expected HTTP 101, got 400` / 日志里 relay 反复重连 | 代理没有转发 `Upgrade`。nginx 加 `proxy_http_version 1.1` 和 `Upgrade`/`Connection` 头；Cloudflare 开启 WebSockets |
| 自检 `HTTP 404` | public URL 没指到 cranibox serve 的根路径，或 serve 用了 `--no-relay` |
| 自检 `HTTP 502/503/504/530` | 代理或隧道连不到 serve（cloudflared 未运行、端口错误、7844 出站被封） |
| 自检 `HTTP 200` | 代理在缓冲或改写 WebSocket 请求 |
| 自检 unreachable / 证书错误 | DNS、证书链；自签证书用 `CRANIBOX_IROH_EXTRA_ROOT` |
| `peer pair accept`：The other device did not answer | 邀请方离线，或双方都到不了链接里的 relay；检查 `peer pair status` 的 `relays` |
| `pair status` 里 relay 为 `false` | 本设备未配对到该 Hub（relay 拒绝未配对的 EndpointId），或网络不通 |
| 路径一直是 `relay` | 正常（例如只开 443）；允许 UDP 后会自动升级为 `direct` |
