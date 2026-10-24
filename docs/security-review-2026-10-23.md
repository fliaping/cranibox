# Cranibox 网络与身份认证审查

审查日期：2026-10-23。公开仓库审查版本：`d13825bc2dc9a7e1e9867cab9a35486d0f3c9ada`；本机安装目录的源码版本：`547165d50b468ed13a0313140331888398bfa1b7`。本机运行二进制与源码版本是否完全一致未另行验证。

方法：阅读部署配置、Web 会话、设备授权、iroh 配对、身份与名册代码；对本机服务进行少量无密码、只读 HTTP 验证。没有执行远程命令、修改授权、扫描公网、操作桌面或实施拒绝服务。未进行完整密码协议证明、依赖漏洞扫描及跨平台渗透测试。

## 判断

存在明确的高风险部署默认值与较大的设备信任范围。新版具备公钥身份、配对码确认、加密传输及目标端能力授权，不能说它“没有认证”；但目前更适合所有设备均由同一人管理、彼此高度信任的场景。公开管理入口必须另有身份认证。仅使用 HTTPS、Tailscale 或加密中继，不等于已经完成用户认证或最小权限隔离。

## 1. Workspace 管理页依赖外层认证：错误公开时为严重风险

Workspace 的 Web 控制中心没有独立账号系统。`guard` 检查 Host、Origin 和跨站请求；首次 `GET /` 自动生成会话 Cookie，并把 CSRF 值交给访问者。后续管理接口以这些会话数据鉴权。因此，没有外层认证时，网络可达的访问者可以自行取得所需会话。这是明确的信任边界，不能把 CSRF 或 Host 白名单当作用户身份认证。

本机验证：未提供密码或其他账号凭据，`GET http://127.0.0.1:9080/` 返回 200、发放 `aw_session` 和 CSRF；以所得会话调用只读 `POST /api/metrics` 返回 200。没有验证执行命令或写配置，但源码中的同一会话边界还保护终端、配对与设备管理。

本机 9080 实际绑定 `127.0.0.1`，不是已经证明公网可利用。危险条件是把该入口通过反向代理、隧道或其他转发开放，却只配置 TLS、不配置认证。`docs/web-control-center.md` 明确要求认证网关；`docs/iroh-network.md` 中简化的 nginx/Caddy/Cloudflare 示例未包含这一配置，容易被误用。

此问题特指 Workspace 控制中心。新版独立节点 UI 使用登录令牌；独立节点另外提供的 public relay listener 也与管理 UI 有区分，不能将所有 Cranibox 端口都视为匿名管理端口。

建议：管理页使用独立入口，接入可靠的认证网关并启用 MFA；确保不能绕过网关直达后端。relay、配对等机器入口按自身协议鉴权，不能因为它们需要访问就对整个管理站点取消认证。产品应补充管理入口内置认证或强制的部署检查，统一示例。

证据：[Web guard 与自动会话](https://github.com/fliaping/cranibox/blob/d13825bc2dc9a7e1e9867cab9a35486d0f3c9ada/crates/workspace-web/src/server.rs#L784)、[部署边界](https://github.com/fliaping/cranibox/blob/d13825bc2dc9a7e1e9867cab9a35486d0f3c9ada/docs/web-control-center.md)、[反代示例](https://github.com/fliaping/cranibox/blob/d13825bc2dc9a7e1e9867cab9a35486d0f3c9ada/docs/iroh-network.md)。

## 2. 默认 Compose 发布无认证桌面：高风险

`docker-compose.yml` 将 `CUSTOM_USER`、`PASSWORD` 注释，同时发布 `3001:3001`，未限制宿主机绑定地址。只要宿主网络、防火墙和上游网络允许访问，就可能开放无认证桌面。HTTPS 只加密链路，不阻止陌生人操作桌面。

推荐的 `docker-compose.remote.yml` 会强制要求密码，这是已有的防护；但它默认发布的 3001、8443 也未限制宿主绑定地址。不同启动方式的安全性并不相同。

本机补充观察：3001 绑定容器所有接口，匿名 HTTPS 首页返回 200；8443 首页返回 302。这些状态码不足以单独证明完整桌面或终端已可匿名控制，也不能推导宿主机或公网的可达性。

建议：默认强制设置随机密码；不准备直接对外服务时，将宿主端口明确绑定 `127.0.0.1`，经 SSH 隧道或受控入口访问。防火墙规则与 Docker 端口发布一并核实。

证据：[默认 Compose](https://github.com/fliaping/cranibox/blob/d13825bc2dc9a7e1e9867cab9a35486d0f3c9ada/docker-compose.yml#L44)、[remote Compose](https://github.com/fliaping/cranibox/blob/d13825bc2dc9a7e1e9867cab9a35486d0f3c9ada/docker-compose.remote.yml)。

## 3. 默认自有设备配对建立全面互信：高影响的设计风险

`OwnerDevice` 配对的 `owner_permissions()` 开启 exec、terminals、projects、desktop、computer、browser、messages、activity、spaces、llm、vault 全部能力。部分能力还要求本机提供者或共享开关可用；授予 capability 不等于自动开启全部底层服务。

此外，自有设备加入同一身份时会收到加密封装的 owner 签名私钥；本机解封后持久保存。名册成员之间会自动补齐授权与连接。因此，某台笔记本被恶意软件或其运行的 Agent 控制，影响可能扩大到其他自有设备及共享账号材料。共享 owner 私钥意味着每个持有者也拥有名册签署权，不能把其中某台当成低信任执行节点。

这属于授权设计与失陷后的影响范围，未证明陌生人可以绕过配对。Guest 配对只授予 messages，消息待审；但它并不是任意远程执行的沙箱。旧网关的设备组同样允许组内相互执行命令。

建议：只有同一安全等级的设备才以 OwnerDevice 配对；第三方设备和不可信 Agent 单独隔离。需要限制命令执行时使用单独的、最小能力授权，并在 OS 用户或容器边界限制执行权限。设备丢失后执行已有的恢复码 owner 轮换流程；仅移除成员不能令其忘记已经持有的 owner 私钥或已复制的数据。

证据：[全部能力与配对](https://github.com/fliaping/cranibox/blob/d13825bc2dc9a7e1e9867cab9a35486d0f3c9ada/crates/workspace-node/src/peer/transport_iroh.rs#L332)、[成员自动互信](https://github.com/fliaping/cranibox/blob/d13825bc2dc9a7e1e9867cab9a35486d0f3c9ada/crates/workspace-node/src/models.rs#L548)、[owner 私钥分发](https://github.com/fliaping/cranibox/blob/d13825bc2dc9a7e1e9867cab9a35486d0f3c9ada/crates/workspace-runtime/src/peers/roster/transfer.rs#L29)、[失陷后的 owner 轮换](https://github.com/fliaping/cranibox/blob/d13825bc2dc9a7e1e9867cab9a35486d0f3c9ada/crates/workspace-node/src/models.rs#L465)。

## 4. 独立节点登录链接使用持久可重用令牌：中风险，公开代理时影响升高

README 称 `ui-url` 输出“一次性登录令牌”，但 `access_token()` 会读取并长期复用 `ui.token`，重启后仍复用；`page_response()` 只比较令牌并签发 Cookie，没有消费、过期或轮换它。登录成功后重定向清除地址栏参数，并不使原链接失效。

令牌放在 URL query 中，若链接被分享、被浏览器扩展读取或进入反向代理访问日志，得到它的人只要能连接该 UI 就能再次登录。回环监听降低了远程攻击面；SSH 转发或公开反代会改变可达范围。这是静态代码确认，未尝试读取本机真实 `ui.token` 或重放真实用户链接。

建议：改成短有效期、单次消费的启动票据，交换会话后立即作废；不要让长期根访问凭据进入 URL。增加显式注销、令牌轮换及会话撤销能力，代理日志脱敏。

证据：[持久令牌与登录 URL](https://github.com/fliaping/cranibox/blob/d13825bc2dc9a7e1e9867cab9a35486d0f3c9ada/crates/workspace-node/src/web/standalone.rs#L101)、[兑换会话但不消费令牌](https://github.com/fliaping/cranibox/blob/d13825bc2dc9a7e1e9867cab9a35486d0f3c9ada/crates/workspace-node/src/web/standalone.rs#L593)、[README 用词](https://github.com/fliaping/cranibox/blob/d13825bc2dc9a7e1e9867cab9a35486d0f3c9ada/README.md#L38)。

## 5. 升级保留旧授权兼容模式：中至高风险，取决于凭据和链路

新签发 grant 默认 `require_verified: true`，未核对配对码时不可用；严格授权拒绝直接 bearer 请求，这是有效保护。但旧授权缺少该字段时反序列化为 false，显式兼容授权也如此。此时普通 bearer 验证通过就能使用权限，不要求证明固定公钥的持有权；“已经保存过对端公钥”不意味着所有兼容入口均强制公钥认证。

所以，单纯升级程序不等于把已有设备全部提升到严格认证。配合显式允许的私网明文 HTTP，或者可读取流量的旧中继，兼容 bearer 的泄露可能导致冒用。不能把新 iroh/Noise 的端到端保证泛化到旧网关与全部兼容路径。

建议：清点兼容授权，逐一撤销并重新签发严格授权，完成配对码核对；尽量关闭旧路径，避免长期依赖 `--allow-unverified` 和明文 HTTP。

证据：[旧字段默认值与新授权默认值](https://github.com/fliaping/cranibox/blob/d13825bc2dc9a7e1e9867cab9a35486d0f3c9ada/crates/workspace-runtime/src/peers/config.rs#L134)、[bearer 与严格认证分支](https://github.com/fliaping/cranibox/blob/d13825bc2dc9a7e1e9867cab9a35486d0f3c9ada/crates/workspace-node/src/peer/server.rs#L190)、[身份和 E2E 的兼容边界](https://github.com/fliaping/cranibox/blob/d13825bc2dc9a7e1e9867cab9a35486d0f3c9ada/docs/node-identity-and-e2e.md)。

## 其他需要注意的边界

- 公网 `/pair/request` 接受未认证申请，name、fingerprint 等申请信息由申请者填写；不能把名称当作可信身份。待审批队列最多 16 个，未见该创建入口的按来源限速，攻击者可能占满队列，影响正常配对；本次未发起洪泛验证。实际配对仍有公钥和审批检查，这不等于已发现无审批接管漏洞。[申请实现](https://github.com/fliaping/cranibox/blob/d13825bc2dc9a7e1e9867cab9a35486d0f3c9ada/crates/workspace-node/src/pair_url.rs#L64)
- iroh 默认允许 n0 公共 relay 兜底，并使用地址发现。自建 Hub 不等于连接永远不经过第三方。iroh 链路仍有加密，但 relay 可观察连接元数据、大小和时序。需要封闭运行时，配置自有 relay、关闭 `n0_fallback`，并按需关闭外部地址发现。[网络配置](https://github.com/fliaping/cranibox/blob/d13825bc2dc9a7e1e9867cab9a35486d0f3c9ada/crates/cranibox-net/src/node.rs#L86)
- 同一 OS 用户下的 Agent、本地程序和私钥文件没有强身份隔离。文件 `0600` 能限制其他用户，无法阻止同用户恶意程序；取得本机执行权通常也意味着取得其令牌和身份材料。不能依靠 MCP 不暴露审批工具来隔离仍可执行本地 CLI 的 Agent。

## 已有有效防护

新版 iroh 使用节点签名公钥作为 EndpointId，连接身份与配对身份绑定；配对邀请随机、限时、单次使用，普通链接配对需要双方确认后才交换凭据；新 grant 默认严格，执行端检查对应能力。独立节点 UI 默认监听回环，并需要登录令牌；Workspace 管理页也默认回环。现有 Noise 通道具备重放防护、密钥固定和加密流设计。

上述防护是源码确认的实现机制，不构成对全部协议、版本与部署方式的独立安全认证。本次没有发现证据足以断言“新版陌生设备可以直接绕过配对控制任意节点”。最优先处理的是管理入口认证、默认桌面暴露，以及自有设备的过大互信范围。
