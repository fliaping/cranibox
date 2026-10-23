import { Activity, ArrowLeft, Globe, Monitor, MousePointerClick, Plug, ShieldCheck, SquareTerminal, Trash2, Users } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Link, Navigate, useNavigate, useParams } from "react-router-dom";
import { DesktopViewer } from "../desktop/DesktopViewer";
import { BrowserTargetPanel } from "../devices/BrowserTargetPanel";
import type { DirectClient } from "../devices/DirectDevicesView";
import { grantKeys, notAvailable, type Device, type Grant, type GrantKey } from "../personal/api";
import { FederatedProjectsView } from "../projects/FederatedProjectsView";
import type { ProjectClient } from "../projects/ProjectsView";
import { useTerminalWindow } from "../terminal/TerminalWindowContext";
import { request, useWorkspace } from "../workspace";
import { deviceCaps, devicePath } from "./Home";
import { useApp } from "./store";
import { useTerminalNode } from "./targets";
import { Alert, capabilityIcons, capabilityName, Card, DeviceIcon, Empty, isThis, pathLabel, Pill, Switch, Tabs, type Translate } from "./ui";

const tabs = ["overview", "terminal", "desktop", "browser", "projects", "access"] as const;
type Tab = (typeof tabs)[number];

export function DeviceDetail() {
  const { t, devices, loading } = useApp();
  const params = useParams();
  const id = decodeURIComponent(params.id || ""), tab = (params.tab || "overview") as Tab;
  const device = devices.find(item => item.node_id === id);
  if (!tabs.includes(tab)) return <Navigate to={"/app/device/" + encodeURIComponent(id)} replace />;
  if (!device) return loading ? null : <Empty icon={Plug} title={t("找不到这台设备", "Device not found")} actions={<Link className="cx-btn" to="/app/devices">{t("全部设备", "All devices")}</Link>}>{t("它可能已被移除，或尚未与本机配对。", "It may have been removed, or it is not paired with this device yet.")}</Empty>;
  const local = isThis(device), reachable = local || device.online;
  const labels: Record<Tab, string> = { overview: t("概览", "Overview"), terminal: t("终端", "Terminal"), desktop: t("桌面", "Desktop"), browser: t("浏览器", "Browser"), projects: t("项目与会话", "Projects & sessions"), access: t("授权", "Access") };
  return <>
    <Link className="cx-btn ghost small" to="/app/devices"><ArrowLeft size={15} aria-hidden="true" />{t("全部设备", "All devices")}</Link>
    <div className="cx-device-head">
      <span className="cx-card-icon"><DeviceIcon kind={device.kind} size={26} /></span>
      <div className="cx-grow">
        <h1>{device.name}</h1>
        <div className="cx-device-meta"><span className={"cx-dot " + (reachable ? "online" : "")} aria-hidden="true" />{reachable ? <>{t("在线", "Online")}<span aria-hidden="true">·</span>{pathLabel(device, t)}</> : t("离线", "Offline")}</div>
      </div>
      <div className="cx-actions">{local && <Pill tone="accent">{t("本机", "This device")}</Pill>}{device.guest && <Pill tone="info">{t("访客", "Guest")}</Pill>}</div>
    </div>
    <Tabs label={t("设备分区", "Device sections")} items={tabs.map(item => ({ to: devicePath(device, item === "overview" ? "" : item), label: labels[item] }))} />
    {tab === "overview" && <Overview device={device} />}
    {tab === "terminal" && <TerminalTab device={device} />}
    {tab === "desktop" && <DesktopTab device={device} />}
    {tab === "browser" && <BrowserTab device={device} />}
    {tab === "projects" && <ProjectsTab device={device} />}
    {tab === "access" && <AccessTab device={device} />}
  </>;
}
function since(seconds: number | null | undefined, t: Translate) {
  if (!seconds) return "—";
  const age = Math.max(0, Math.round(Date.now() / 1000 - seconds));
  if (age < 60) return t("刚刚", "just now");
  if (age < 3600) return t(Math.round(age / 60) + " 分钟前", Math.round(age / 60) + " min ago");
  if (age < 86400) return t(Math.round(age / 3600) + " 小时前", Math.round(age / 3600) + " h ago");
  return new Date(seconds * 1000).toLocaleString();
}
function Overview({ device }: { device: Device }) {
  const { t } = useApp();
  const navigate = useNavigate(), { openTerminal } = useTerminalWindow(), terminalNode = useTerminalNode();
  const local = isThis(device), reachable = local || device.online;
  const open: Record<string, () => void> = {
    terminals: () => openTerminal({ node: terminalNode(device), create: true }),
    desktop: () => navigate(devicePath(device, "desktop")),
    browser: () => navigate(devicePath(device, "browser")),
    computer: () => navigate(devicePath(device, "access")),
  };
  return <div className="cx-detail-grid">
    <Card label={t("连接", "Connection")} className="cx-detail-status">
      <div className="cx-card-head"><span className={"cx-card-icon " + (reachable ? "" : "muted")}><Activity size={20} aria-hidden="true" /></span><div className="cx-grow"><h3>{t("连接", "Connection")}</h3></div>
        <Pill tone={reachable ? "good" : ""}>{reachable ? t("在线", "Online") : t("离线", "Offline")}</Pill></div>
      <dl className="cx-facts">
        <div><dt>{t("连接路径", "Path")}</dt><dd>{local ? t("本机", "This device") : device.path === "direct" ? t("对等直连", "Direct peer connection") : device.path === "relay" ? t("经中继（端到端加密）", "Relayed (end-to-end encrypted)") : t("未知", "Unknown")}</dd></div>
        <div><dt>{t("延迟", "Latency")}</dt><dd>{local ? "—" : device.rtt_ms != null ? device.rtt_ms + " ms" : "—"}</dd></div>
        <div><dt>{t("最近在线", "Last seen")}</dt><dd>{reachable ? t("现在", "Now") : since(device.last_seen, t)}</dd></div>
      </dl>
    </Card>
    <Card label={t("系统", "System")}>
      <div className="cx-card-head"><span className="cx-card-icon"><DeviceIcon kind={device.kind} size={20} /></span><div className="cx-grow"><h3>{t("系统", "System")}</h3></div></div>
      <dl className="cx-facts">
        <div><dt>{t("系统版本", "OS")}</dt><dd>{device.os_version || "—"}</dd></div>
        <div><dt>{t("机型", "Model")}</dt><dd>{device.model || "—"}</dd></div>
        <div><dt>{t("关系", "Relationship")}</dt><dd>{local ? t("你正在使用的设备", "The device you are using") : device.guest ? t("访客设备", "Guest device") : t("你的设备", "Your device")}</dd></div>
        <div><dt>{t("设备 ID", "Device ID")}</dt><dd className="cx-mono">{device.node_id}</dd></div>
      </dl>
    </Card>
    {deviceCaps.map(cap => {
      const Icon = capabilityIcons[cap], on = device.caps[cap] !== false;
      return <Card key={cap} className="cx-cap-tile" label={capabilityName(cap, t)}>
        <div className="cx-card-head"><span className={"cx-card-icon " + (on ? "" : "muted")}><Icon size={20} aria-hidden="true" /></span><div className="cx-grow"><h3>{capabilityName(cap, t)}</h3><small className="cx-muted">{on ? t("可用", "Available") : t("未授权", "Not allowed")}</small></div></div>
        <div className="cx-actions">
          {local && cap !== "terminals" ? <Link className="cx-btn small" to="/app/this">{t("管理", "Manage")}</Link>
            : <button className="cx-btn small primary" disabled={!on || !reachable} onClick={open[cap]}>{cap === "computer" ? t("授权", "Access") : cap === "terminals" ? t("打开终端", "Open terminal") : t("打开", "Open")}</button>}
        </div>
      </Card>;
    })}
    {!local && !device.guest && device.same_identity === false && <JoinIdentity device={device} />}
    {!local && <RemoveDevice device={device} />}
  </div>;
}
/** A paired device that is not part of this identity: subscriptions do not sync until it joins. */
function JoinIdentity({ device }: { device: Device }) {
  const { t, api, busy, run } = useApp();
  const [sent, setSent] = useState(false);
  return <Card label={t("加入同一身份", "Join this identity")} className="cx-detail-join">
    <div className="cx-card-head"><span className="cx-card-icon"><Users size={20} aria-hidden="true" /></span><div className="cx-grow"><h3>{t("不在你的身份中", "Not in your identity")}</h3>
      <p className="cx-muted cx-small">{t("这台设备已配对，但属于另一个身份，所以模型订阅不会同步，也看不到你的其他设备。邀请它加入后，它会改用你的身份（它原来的身份里不能还有其他设备）。", "This device is paired but belongs to another identity, so model subscriptions do not sync and it cannot see your other devices. Once it joins, it uses your identity (its current identity must not include other devices).")}</p></div></div>
    {sent ? <p className="cx-notice" role="status">{t("邀请已发送。请在那台设备的「待办」里确认，10 分钟内有效。", "Invitation sent. Accept it in that device's Inbox within 10 minutes.")}</p>
      : <div className="cx-actions"><button className="cx-btn primary" disabled={busy || !device.online} onClick={() => void run(async () => { await api.call("POST", "/devices/" + encodeURIComponent(device.node_id) + "/join"); setSent(true); })}>{t("邀请加入我的身份", "Invite to my identity")}</button></div>}
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
      <p className="cx-muted cx-small">{device.guest ? t("撤销这台访客设备的全部访问，并从列表中删除。之后需要重新配对。", "Revoke every permission of this guest device and remove it from the list. It must pair again to come back.")
        : t("把它移出你的身份：其他设备不再信任它，它也会停止同步。适合不再使用、重装过或重复出现的设备。", "Take it out of your identity: your other devices stop trusting it and it stops syncing. Use this for a device you no longer use, reinstalled, or that shows up twice.")}</p></div></div>
    {!confirming ? <div className="cx-actions"><button className="cx-btn danger" disabled={busy} onClick={() => setConfirming(true)}>{t("移除…", "Remove…")}</button></div>
      : <div className="cx-stack" style={{ marginTop: 8 }}>
        {!device.guest && <Alert tone="warn">{t("设备丢失或被盗时，请改到「身份与安全」用恢复码移除。普通移除不会更换身份密钥，被盗设备仍持有旧密钥，可能重新加入。", "If the device was lost or stolen, remove it with your recovery code under Identity & security instead. A plain removal keeps the identity key, which a stolen device still holds and could use to rejoin.")}</Alert>}
        <div className="cx-actions">
          <button className="cx-btn" onClick={() => setConfirming(false)}>{t("取消", "Cancel")}</button>
          {!device.guest && <Link className="cx-btn" to="/app/identity">{t("用恢复码移除", "Remove with recovery code")}</Link>}
          <button className="cx-btn danger" disabled={busy} onClick={remove}>{t("确认移除 ", "Remove ") + device.name}</button>
        </div>
      </div>}
  </Card>;
}
function TerminalTab({ device }: { device: Device }) {
  const { t } = useApp();
  const { openTerminal } = useTerminalWindow(), terminalNode = useTerminalNode();
  const allowed = device.caps.terminals !== false, reachable = isThis(device) || device.online;
  return <Card className="cx-stack">
    <div className="cx-card-head"><span className="cx-card-icon"><SquareTerminal size={20} aria-hidden="true" /></span><div className="cx-grow"><h3>{t("持久终端", "Persistent terminal")}</h3>
      <p className="cx-muted">{t("会话在设备上持续运行；关闭窗口只会断开连接，可以随时接回。", "Sessions keep running on the device. Closing the window only detaches, so you can reattach any time.")}</p></div></div>
    {!allowed && <Alert tone="warn">{t("这台设备没有允许你打开终端。", "This device has not allowed you to open terminals.")}</Alert>}
    {!reachable && <Alert tone="warn">{t("设备离线，恢复连接后即可打开终端。", "The device is offline. Open a terminal once it reconnects.")}</Alert>}
    <div className="cx-actions">
      <button className="cx-btn primary" disabled={!allowed || !reachable} onClick={() => openTerminal({ node: terminalNode(device), create: true })}><SquareTerminal size={16} aria-hidden="true" />{t("打开新终端", "Open new terminal")}</button>
      <button className="cx-btn" disabled={!allowed || !reachable} onClick={() => openTerminal({ node: terminalNode(device) })}>{t("查看已有会话", "Show existing sessions")}</button>
    </div>
  </Card>;
}
function DesktopTab({ device }: { device: Device }) {
  const { t, locale, openExternal } = useApp();
  const [open, setOpen] = useState(false);
  if (isThis(device)) return <Card className="cx-stack"><div className="cx-card-head"><span className="cx-card-icon"><Monitor size={20} aria-hidden="true" /></span><div className="cx-grow"><h3>{t("这就是你正在使用的设备", "This is the device you are using")}</h3>
    <p className="cx-muted">{t("要让其他设备查看这块屏幕，请在「能力与权限」中开启桌面共享。", "To let your other devices see this screen, turn on desktop sharing under Capabilities.")}</p></div></div>
    <div className="cx-actions"><Link className="cx-btn primary" to="/app/this">{t("管理桌面共享", "Manage desktop sharing")}</Link></div></Card>;
  const allowed = device.caps.desktop !== false;
  return <div className="cx-stack">
    {!allowed && <Alert tone="warn">{t("这台设备还没开启桌面共享。在那台设备上打开「本机设置 → 能力与权限 → 远程桌面」并允许其他设备使用（macOS 还需在系统设置中打开「屏幕共享」）。", "This device does not share its desktop yet. On that device, open Settings → Capabilities → Remote desktop and allow your other devices (on macOS, also turn on Screen Sharing in System Settings).")}</Alert>}
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
function AccessTab({ device }: { device: Device }) {
  const { t, api, run, busy } = useApp();
  const [grant, setGrant] = useState<Grant | null>(null), [state, setState] = useState<"loading" | "ready" | "unsupported" | "error">("loading"), [error, setError] = useState("");
  const path = "/devices/" + encodeURIComponent(device.node_id) + "/grant";
  const load = useCallback(async () => {
    try { setGrant(await api.call<Grant>("GET", path)); setState("ready"); }
    catch (problem) { if (notAvailable(problem)) setState("unsupported"); else { setError(problem instanceof Error ? problem.message : String(problem)); setState("error"); } }
  }, [api, path]);
  useEffect(() => { void load(); }, [load]);
  if (isThis(device)) return <Card><p className="cx-muted">{t("这是你正在使用的设备。在其他设备的「授权」中选择它们能在这里做什么。", "This is the device you are using. Open another device's Access tab to choose what it can do here.")}</p></Card>;
  const visible = grantKeys.filter(key => !device.guest || !["llm", "vault"].includes(key));
  // Older nodes only expose guest capabilities on the device itself.
  const legacy = state === "unsupported" && device.guest;
  const permissions: Partial<Record<GrantKey, boolean>> = grant?.permissions || (legacy ? device.caps : {});
  const editable = state === "ready" || legacy;
  async function change(key: GrantKey, value: boolean) {
    const next = { ...permissions, [key]: value };
    await run(async () => {
      if (state === "ready") setGrant(await api.call<Grant>("PUT", path, { permissions: next }).then(value => value || { permissions: next }));
      else await api.call("PUT", "/devices/" + encodeURIComponent(device.node_id), { caps: next });
    });
  }
  return <div className="cx-grid two">
    <Card label={t("它能对本机做什么", "What it can do on this device")}>
      <div className="cx-card-head"><span className="cx-card-icon"><ShieldCheck size={20} aria-hidden="true" /></span><div className="cx-grow"><h3>{t("它能对本机做什么", "What it can do on this device")}</h3>
        <p className="cx-muted">{device.guest ? t("访客只能使用你逐项打开的功能，无法访问模型订阅和凭据。", "Guests can use only what you turn on here, never model subscriptions or credentials.") : t("你的设备默认全部允许，可以按需收紧。", "Your own devices are allowed everything by default. Tighten as needed.")}</p></div></div>
      {state === "loading" && <p className="cx-muted" role="status">{t("正在读取授权…", "Loading access…")}</p>}
      {state === "error" && <Alert action={<button className="cx-btn small" onClick={() => void load()}>{t("重试", "Retry")}</button>}>{error}</Alert>}
      {state === "unsupported" && !legacy && <Alert tone="info">{t("这台设备上的节点版本还不支持逐项授权。更新 Cranibox 后即可在这里调整。", "This node does not support per-device access yet. Update Cranibox to adjust access here.")}</Alert>}
      {editable && <div className="cx-toggle-list">{visible.map(key => <Switch key={key} label={capabilityName(key, t)} checked={!!permissions[key]} disabled={busy} onChange={value => void change(key, value)} />)}</div>}
    </Card>
    <Card label={t("我能对它做什么", "What I can do on it")}>
      <div className="cx-card-head"><span className="cx-card-icon muted"><MousePointerClick size={20} aria-hidden="true" /></span><div className="cx-grow"><h3>{t("我能对它做什么", "What I can do on it")}</h3>
        <p className="cx-muted">{t("由这台设备的主人决定，在那台设备上修改。", "Decided by that device's owner, and changed on that device.")}</p></div></div>
      <ul className="cx-toggle-list" style={{ listStyle: "none", margin: 0, padding: 0 }}>{deviceCaps.map(cap => <li key={cap} className="cx-toggle-row"><span>{capabilityName(cap, t)}</span><Pill tone={device.caps[cap] !== false ? "good" : ""}>{device.caps[cap] !== false ? t("允许", "Allowed") : t("未允许", "Not allowed")}</Pill></li>)}</ul>
    </Card>
  </div>;
}

