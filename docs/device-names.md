# 设备名称 · Device names

每台设备默认显示系统主机名（macOS 上是“电脑名称”）。你可以给设备另起一个更容易辨认的名称。

On each device the default name is the system hostname (the Computer Name on macOS). You can give a device a name that is easier to recognise.

## 在界面里重命名 · In the app

在设备列表或设备详情页，点击名称旁的铅笔按钮，输入新名称后保存。名称最多 64 个字符，首尾空格会被去掉。留空并保存，会恢复为设备原来的名称。如果另一台设备已经使用这个名称，界面会提醒你，但仍允许保存；可以用设备 ID 区分它们。

On the device list or a device's page, click the pencil next to the name, type a new name, and save. Names can be at most 64 characters and leading or trailing spaces are ignored. Leave the name empty and save to use the device's own name again. If another device already has that name, you are warned but can still save; tell them apart by device ID.

## 在命令行里 · On the command line

```bash
cranibox nodes rename <node> "办公电脑"
cranibox nodes rename <node> "Work laptop"
cranibox nodes rename <node> ""            # 恢复原名 / use the original name
```

`<node>` 可以是节点 ID，也可以是当前不重名的设备名称。

`<node>` may be a node ID, or the device's current name if that name is unique.

## 名称如何保存与同步 · How names are stored and synced

名称是显示用的元数据，不是身份或凭证。它保存在本机的 `devices.json` 里（`alias` 与时间戳 `alias_at`），并在同一身份的设备之间同步：较新的更改生效。这不需要恢复码。

A name is display metadata, not an identity or a credential. It is stored on the device in `devices.json` (`alias` and a timestamp `alias_at`) and syncs between devices of the same identity; the latest change wins. This does not require a recovery code.

- **重命名本机**：这台设备会向其他设备（包括访客）介绍这个新名称。
- **Renaming this device**: the device advertises the new name to others, including guest devices.
- **重命名其他设备**：只是你给它起的名称，会同步到你的其他设备，不会改变那台设备对自己的称呼。访客看不到这个名称。
- **Renaming another device**: this is a name you choose. It appears on your other devices, and does not change what that device calls itself. Guest devices do not see it.

设备的加密身份和节点 ID 不会因为重命名而改变。

Renaming a device never changes its cryptographic identity or node ID.
