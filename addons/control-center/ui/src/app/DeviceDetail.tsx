import { Globe, Monitor, Plug, SquareTerminal, Trash2, Users } from "lucide-react";
import { useMemo, useState } from "react";
import { Link, Navigate, useLocation, useNavigate, useParams } from "react-router-dom";
import { DeviceAccess } from "../access/Access";
import { DesktopViewer } from "../desktop/DesktopViewer";
import { BrowserTargetPanel } from "../devices/BrowserTargetPanel";
import type { DirectClient } from "../devices/DirectDevicesView";
import { type Device } from "../personal/api";
import { FederatedProjectsView } from "../projects/FederatedProjectsView";
import type { ProjectClient } from "../projects/ProjectsView";
import { useTerminalWindow } from "../terminal/TerminalWindowContext";
import { request, useWorkspace } from "../workspace";
import { deviceCaps, devicePath, since } from "./Home";
import { relationLabel, relationOf, relationSentence, relationTones } from "./relations";
import { useApp } from "./store";
import { useTerminalNode } from "./targets";
import { UpdatePanel } from "./UpdatePanel";
import { Alert, capabilityIcons, capabilityName, Card, DeviceIcon, Empty, formatDeviceLabel, isThis, pathLabel, Pill, shortNodeId, Tabs } from "./ui";

/** The terminal opens in its own window from the header, so it needs no tab of its own. */
const tabs = ["overview", "desktop", "browser", "projects", "access"] as const;
type Tab = (typeof tabs)[number];

export function DeviceDetail() {
  const { t, devices, loading, deviceName } = useApp();
  const params = useParams();
  const { openTerminal } = useTerminalWindow(), terminalNode = useTerminalNode();
  const id = decodeURIComponent(params.id || ""), tab = (params.tab || "overview") as Tab;
  const device = devices.find(item => item.node_id === id);
  if (!tabs.includes(tab)) return <Navigate to={"/app/device/" + encodeURIComponent(id)} replace />;
  if (!device) return loading ? null : <Empty icon={Plug} title={t("找不到这台设备", "Device not found")} actions={<Link className="cx-btn" to="/app/devices">{t("全部设备", "All devices")}</Link>}>{t("它可能已被移除，或尚未与本机配对。", "It may have been removed, or it is not paired with this device yet.")}</Empty>;
  const local = isThis(device), reachable = local || device.online;
  const labels: Record<Tab, string> = { overview: t("概览", "Overview"), desktop: t("桌面", "Desktop"), browser: t("浏览器", "Browser"), projects: t("项目与会话", "Projects & sessions"), access: t("权限", "Access") };
  const terminals = device.caps.terminals !== false && reachable;
  const seen = since(device.last_seen, t);
  return <>
    <div className="cx-device-head">
      <span className="cx-card-icon"><DeviceIcon kind={device.kind} size={24} /></span>
      <div className="cx-grow">
        <div className="cx-device-title">
          {(() => { const info = formatDeviceLabel(device.name, device.node_id, t); return <h1>{info.label}{info.rawHidden && <span className="cx-sr"> {info.rawHidden}</span>}</h1>; })()}
          {local ? <Pill tone="accent">{t("本机", "This device")}</Pill> : relationOf(device) !== "peer" && <Pill tone={relationTones[relationOf(device)]}>{relationLabel(relationOf(device), t)}</Pill>}
        </div>
        <div className="cx-device-meta"><span className={"cx-dot " + (reachable ? "online" : "")} aria-hidden="true" />{local ? t("在线", "Online") : reachable ? <>{t("在线", "Online")}<span aria-hidden="true">·</span>{pathLabel(device, t, deviceName)}</> : <>{t("离线", "Offline")}{seen && <><span aria-hidden="true">·</span>{t("最近在线 ", "last seen ") + seen}</>}</>}</div>
      </div>
      <div className="cx-actions">
        <button className="cx-btn" disabled={!terminals} onClick={() => openTerminal({ node: terminalNode(device) })}>{t("已有会话", "Existing sessions")}</button>
        <button className="cx-btn primary" disabled={!terminals} title={device.caps.terminals === false ? t("这台设备没有允许你打开终端。", "This device has not allowed you to open terminals.") : !reachable ? t("设备离线，恢复连接后即可打开终端。", "The device is offline. Open a terminal once it reconnects.") : undefined} onClick={() => openTerminal({ node: terminalNode(device), create: true })}><SquareTerminal size={16} aria-hidden="true" />{t("打开终端", "Open terminal")}</button>
      </div>
    </div>
    <Tabs label={t("设备分区", "Device sections")} items={tabs.map(item => ({ to: devicePath(device, item === "overview" ? "" : item), label: labels[item] }))} />
    {tab === "overview" && <Overview device={device} />}
    {tab === "desktop" && <DesktopTab device={device} />}
    {tab === "browser" && <BrowserTab device={device} />}
    {tab === "projects" && <ProjectsTab device={device} />}
    {tab === "access" && <AccessTab device={device} />}
  </>;
}
function Overview({ device }: { device: Device }) {
  const { t } = useApp();
  const navigate = useNavigate();
  const local = isThis(device), reachable = local || device.online;
  // The terminal is the header's primary action; these are what the device offers beyond it.
  const rows: { cap: (typeof deviceCaps)[number]; detail: string; action: string; to: string }[] = [
    { cap: "desktop", detail: local ? t("让其他设备查看和操作这块屏幕。", "Let your other devices see and control this screen.") : t("查看并操作它的屏幕。", "See and control its screen."), action: t("连接", "Connect"), to: devicePath(device, "desktop") },
    { cap: "browser", detail: local ? t("让 Agent 使用这台设备上的浏览器。", "Let agents use the browser on this device.") : t("让 Agent 使用它已登录的浏览器。", "Let agents use its signed-in browser."), action: t("打开", "Open"), to: devicePath(device, "browser") },
    { cap: "computer", detail: local ? t("让 Agent 看到屏幕并使用鼠标和键盘。", "Let agents see the screen and use the mouse and keyboard.") : t("Agent 可以看到它的屏幕并使用鼠标和键盘。", "Agents can see its screen and use the mouse and keyboard."), action: t("权限", "Access"), to: devicePath(device, "access") },
  ];
  return <div className="cx-stack loose">
    <div className="cx-grouped-list" aria-label={t("能力", "Capabilities")}>
      {rows.map(({ cap, detail, action, to }) => {
        const Icon = capabilityIcons[cap], on = device.caps[cap] !== false;
        return <section key={cap} className="cx-grouped-row" aria-label={capabilityName(cap, t)}>
          <span className={"cx-card-icon small " + (on ? "" : "muted")}><Icon size={17} aria-hidden="true" /></span>
          <div className="cx-grow"><strong>{capabilityName(cap, t)}</strong><small className="cx-muted">{on || local ? detail : t("这台设备没有允许你使用。", "This device has not allowed you to use it.")}</small></div>
          {local ? <Link className="cx-btn small" to="/app/this">{t("管理", "Manage")}</Link>
            : <button className="cx-btn small" disabled={!on || !reachable} onClick={() => navigate(to, cap === "desktop" ? { state: { connect: true } } : undefined)}>{action}</button>}
        </section>;
      })}
    </div>
    <div className="cx-grid two">
      <Card label={t("连接", "Connection")}>
        <h3>{t("连接", "Connection")}</h3>
        <dl className="cx-facts">
          <div><dt>{t("连接路径", "Path")}</dt><dd>{local ? t("本机", "This device") : !reachable ? t("离线", "Offline") : device.path === "direct" ? t("对等直连", "Direct peer connection") : device.path === "relay" ? t("经中继（端到端加密）", "Relayed (end-to-end encrypted)") : t("未知", "Unknown")}</dd></div>
          <div><dt>{t("延迟", "Latency")}</dt><dd>{local ? "—" : device.rtt_ms != null ? device.rtt_ms + " ms" : "—"}</dd></div>
          <div><dt>{t("最近在线", "Last seen")}</dt><dd>{reachable ? t("现在", "Now") : since(device.last_seen, t) || "—"}</dd></div>
        </dl>
      </Card>
      <Card label={t("系统", "System")}>
        <h3>{t("系统", "System")}</h3>
        <dl className="cx-facts">
          {device.os_version && <div><dt>{t("系统版本", "OS")}</dt><dd>{device.os_version}</dd></div>}
          {device.model && <div><dt>{t("机型", "Model")}</dt><dd>{device.model}</dd></div>}
          <div><dt>{t("关系", "Relationship")}</dt><dd>{local ? t("你正在使用的设备", "The device you are using") : relationOf(device) === "peer" ? t("你的设备", "Your device") : relationSentence(relationOf(device), device.name, t, relationOf(device) === "controlled" || relationOf(device) === "host" ? [t("这台设备", "This device")] : [])}</dd></div>
          <div><dt>{t("设备 ID", "Device ID")}</dt><dd className="cx-mono">{(() => { const s = shortNodeId(device.node_id); return <><span className="cx-id-pill">{s.display}</span>{s.full && <span className="cx-sr"> {s.full}</span>}</>; })()}</dd></div>
        </dl>
      </Card>
    </div>
    {!local && relationOf(device) === "peer" && device.same_identity === false && <OtherIdentity />}
    {!local && <UpdatePanel node={device.node_id} />}
    {!local && <RemoveDevice device={device} />}
  </div>;
}
/** A paired device that is not part of this identity: subscriptions do not sync. Pairing
 * as one of your own devices merges identities, so there is no separate step. */
function OtherIdentity() {
  const { t } = useApp();
  return <Card label={t("不在你的身份中", "Not in your identity")} className="cx-detail-join">
    <div className="cx-card-head"><span className="cx-card-icon"><Users size={20} aria-hidden="true" /></span><div className="cx-grow"><h3>{t("不在你的身份中", "Not in your identity")}</h3>
      <p className="cx-muted cx-small">{t("这台设备已配对，但属于另一个身份，所以模型订阅不会同步。以「我的设备」方式配对时会自动合并：在两台设备上移除对方，然后在已有订阅的设备上生成配对二维码，用这台设备扫码或打开链接。这台设备原来的身份里不能还有其他设备。", "This device is paired but belongs to another identity, so model subscriptions do not sync. Pairing as one of your own devices merges identities: remove each device on the other, then create a pairing code on the device that has your subscriptions and open it on this one. This device's current identity must not include other devices.")}</p></div></div>
  </Card>;
}
/** Remove a device from this network. A lost or stolen device of your identity is
 * removed with the recovery code instead, which also replaces the identity key. */
function RemoveDevice({ device }: { device: Device }) {
  const { t, api, run, busy, setNotice } = useApp();
  const navigate = useNavigate();
  const [confirming, setConfirming] = useState(false);
  const remove = () => void run(async () => {
    await api.call("PUT", "/devices/" + encodeURIComponent(device.node_id), { revoke: true });
    setNotice(t("已移除 " + device.name, "Removed " + device.name));
    navigate("/app/devices");
  });
  return <Card label={t("移除设备", "Remove device")} className="cx-detail-remove">
    <div className="cx-card-head"><span className="cx-card-icon muted"><Trash2 size={20} aria-hidden="true" /></span><div className="cx-grow"><h3>{t("移除设备", "Remove device")}</h3>
      <p className="cx-muted cx-small">{relationOf(device) === "guest" ? t("撤销这台访客设备的全部访问，并从列表中删除。之后需要重新配对。", "Revoke every permission of this guest device and remove it from the list. It must pair again to come back.")
        : relationOf(device) !== "peer" ? t("撤销两台设备之间的全部访问，并从列表中删除。之后需要重新配对。", "Revoke all access between the two devices and remove it from the list. It must pair again to come back.")
        : t("把它移出你的身份：其他设备不再信任它，它也会停止同步。适合不再使用、重装过或重复出现的设备。", "Take it out of your identity: your other devices stop trusting it and it stops syncing. Use this for a device you no longer use, reinstalled, or that shows up twice.")}</p></div></div>
    {!confirming ? <div className="cx-actions"><button className="cx-btn danger" disabled={busy} onClick={() => setConfirming(true)}>{t("移除…", "Remove…")}</button></div>
      : <div className="cx-stack" style={{ marginTop: 8 }}>
        {relationOf(device) === "peer" && <Alert tone="warn">{t("设备丢失或被盗时，请改到「身份与安全」用恢复码移除。普通移除不会更换身份密钥，被盗设备仍持有旧密钥，可能重新加入。", "If the device was lost or stolen, remove it with your recovery code under Identity & security instead. A plain removal keeps the identity key, which a stolen device still holds and could use to rejoin.")}</Alert>}
        <div className="cx-actions">
          <button className="cx-btn" onClick={() => setConfirming(false)}>{t("取消", "Cancel")}</button>
          {relationOf(device) === "peer" && <Link className="cx-btn" to="/app/identity">{t("用恢复码移除", "Remove with recovery code")}</Link>}
          <button className="cx-btn danger" disabled={busy} onClick={remove}>{t("确认移除 ", "Remove ") + device.name}</button>
        </div>
      </div>}
  </Card>;
}
function DesktopTab({ device }: { device: Device }) {
  const { t, locale, openExternal } = useApp();
  // Arriving from a "Desktop" button already says what the person wants: connect right away.
  const wanted = (useLocation().state as { connect?: boolean } | null)?.connect === true;
  const [open, setOpen] = useState(wanted && !isThis(device) && device.online && device.caps.desktop !== false);
  if (isThis(device)) return <Card className="cx-stack"><div className="cx-card-head"><span className="cx-card-icon"><Monitor size={20} aria-hidden="true" /></span><div className="cx-grow"><h3>{t("这就是你正在使用的设备", "This is the device you are using")}</h3>
    <p className="cx-muted">{t("要让其他设备查看这块屏幕，请在「能力与权限」中开启桌面共享。", "To let your other devices see this screen, turn on desktop sharing under Capabilities.")}</p></div></div>
    <div className="cx-actions"><Link className="cx-btn primary" to="/app/this">{t("管理桌面共享", "Manage desktop sharing")}</Link></div></Card>;
  const allowed = device.caps.desktop !== false;
  return <div className="cx-stack">
    {!allowed && <Alert tone="warn">{t("这台设备还没开启桌面共享。在那台设备上打开「设置 → 能力与权限 → 远程桌面」并允许其他设备使用（macOS 还需在系统设置中打开「屏幕共享」）。", "This device does not share its desktop yet. On that device, open Settings → Capabilities → Remote desktop and allow your other devices (on macOS, also turn on Screen Sharing in System Settings).")}</Alert>}
    {open ? <DesktopViewer target={{ node: device.node_id, local: false }} language={locale} openExternal={openExternal} />
      : <Card className="cx-stack"><div className="cx-card-head"><span className="cx-card-icon"><Monitor size={20} aria-hidden="true" /></span><div className="cx-grow"><h3>{t("远程桌面", "Remote desktop")}</h3>
        <p className="cx-muted">{t("画面经端到端加密的对等连接传输。需要对方设备已开启桌面共享。", "The screen streams over an end-to-end encrypted peer connection. The other device must have desktop sharing turned on.")}</p></div></div>
        <div className="cx-actions"><button className="cx-btn primary" disabled={!allowed || !device.online} onClick={() => setOpen(true)}>{t("连接桌面", "Connect to desktop")}</button></div></Card>}
  </div>;
}
const directClient: DirectClient = command => request("/api/peers", command);
function BrowserTab({ device }: { device: Device }) {
  const { t, locale } = useApp();
  if (isThis(device)) return <Card className="cx-stack"><div className="cx-card-head"><span className="cx-card-icon"><Globe size={20} aria-hidden="true" /></span><div className="cx-grow"><h3>{t("本机浏览器", "This device's browser")}</h3>
    <p className="cx-muted">{t("Agent 可以使用你正在用的 Chrome（保留登录状态，每次连接由 Chrome 请你允许），或一个独立配置的托管浏览器。", "Agents can use the Chrome you already use (signed in; Chrome asks you to allow each connection), or a separate managed browser.")}</p></div></div>
    <div className="cx-actions"><Link className="cx-btn primary" to="/app/this">{t("管理浏览器共享", "Manage browser sharing")}</Link></div></Card>;
  if (device.caps.browser === false) return <Alert tone="warn">{t("这台设备没有共享浏览器给你。", "This device does not share its browser with you.")}</Alert>;
  return <Card><BrowserTargetPanel client={directClient} node={device.node_id} language={locale} /></Card>;
}
function ProjectsTab({ device }: { device: Device }) {
  const { locale } = useApp();
  const { snapshot } = useWorkspace(), { openTerminal } = useTerminalWindow();
  const client = useMemo<ProjectClient>(() => ({
    projects: command => request("/api/projects", command),
    terminals: (node, command) => request("/api/terminal/sessions", { ...command, node }),
    open: (node, session) => openTerminal({ node: node || "", session: session.id }),
  }), [openTerminal]);
  const local = isThis(device);
  return <FederatedProjectsView client={client} direct={directClient} language={locale} gatewayEnabled={!!snapshot}
    devices={local && snapshot?.local_node ? [{ id: snapshot.local_node.id, name: device.name, online: true, terminal_sessions: true, can_copy: true }] : []} />;
}
function AccessTab({ device }: { device: Device }) { return <DeviceAccess device={device} />; }
