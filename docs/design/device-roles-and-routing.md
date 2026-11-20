# 设备角色与设备间转发 / Device roles and routing through devices

## 问题 / Problem

「我的设备 / 访客」只有一个方向，用户分不清谁能用谁：访客在邀请方看是「它用我」，在加入方看却是「我用它」。也没有办法加入一台**只执行任务、不能访问我的设备网络**的节点。另外，设备之间只能直连或经 relay；relay 很远时，即使另一台自己的设备能直连目标，也只能绕远路。

The old "my device / guest" choice had one word for two directions, and no way to add an untrusted worker. Devices also could not go through each other when the relay was far away.

## 角色 / Roles

规则只有一条：**箭头 A → B 表示 A 可以使用 B**，B 授权给 A，B 决定 A 能做什么。

| 配对用途 | 邀请方看到 | 加入方看到 | 谁发 grant | 方向 |
|---|---|---|---|---|
| 对等设备 `owner_device` | peer | peer | 双方（全部能力，加入名册） | 这台 ⇄ 新设备 |
| 受控设备 `worker` | controlled | controller | 仅加入方（messages、exec、terminals、projects） | 这台 → 新设备 |
| 访客 `guest` | guest | host（共享给我） | 仅邀请方（messages，其余逐项允许） | 新设备 → 这台 |

- 关系记在 `devices.json`（`Known.relation`）；旧配对从 grant/route 的方向推断。`peers.json` 和签名 grant 不变，降级安全。
- `Device.relation` 进入 App API，`guest` 字段保留给旧客户端。
- 模型订阅、vault、更新只给名册里的对等设备。
- 本人设备之间交换各自的非对等关系（`relations.json`），所以任一台都能画出整张图：「CI runner 由 Home server 控制」。

## 界面 / UI

- 添加设备：三张选择卡，每张带方向小图（这台 ⇄/→ 新设备）和一句话后果。加入方在核对配对码时看到反向说明；被控制时用警示色，按钮写「一致，允许控制」。
- 待办里的配对卡把用途翻译成方向，不显示原始值。
- 设备页：「设备网络」图——中间椭圆是同一身份的对等设备，右侧是受控设备，左侧是访客、控制本机的设备和共享给我的设备；曲线带箭头和沿方向流动的光点，悬停高亮相关边并淡化其他，点选打开玻璃质感详情面板。窄屏改为关系列表。下面按关系分组列出设备卡。
- 网络设置：「经设备转发」两个开关。

## 设备即路由器 / Devices as routers

见 [iroh-network.md §3.1](../iroh-network.md)。要点：一跳、只在本人设备之间、端到端加密、明显更快才用、直连恢复即切回、两端都可关闭。

## 不做 / Not done

- 多跳转发、按带宽选路：一跳已覆盖「relay 远、局域网另一台近」的主要场景，多跳会放大故障面。
- 经受控设备或访客转发：它们不在信任边界内。
