import { ArrowLeft, Globe, Monitor, MousePointerClick, Plug, ShieldCheck, SquareTerminal } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Link, Navigate, useParams } from "react-router-dom";
import { webDesktopConnector } from "../desktop/WebRemoteDesktops";
import { VncViewer } from "../desktop/VncViewer";
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
import { Alert, capabilityIcons, capabilityName, Card, DeviceIcon, Empty, isThis, pathLabel, Pill, Section, Switch, Tabs } from "./ui";

const tabs = ["overview", "terminal", "desktop", "browser", "projects", "access"] as const;
type Tab = (typeof tabs)[number];

export function DeviceDetail() {
  const { t, devices, loading } = useApp();
  const params = useParams();
  const id = decodeURIComponent(params.id || ""), tab = (params.tab || "overview") as Tab;
  const device = devices.find(item => item.node_id === id);
  if (!tabs.includes(tab)) return <Navigate to={"/app/device/" + encodeURIComponent(id)} replace />;
  if (!device) return loading ? null : <Empty icon={Plug} title={t("找不到这台设备", "Device not found")} actions={<Link className="cx-btn" to="/app/home">{t("返回首页", "Back to home")}</Link>}>{t("它可能已被移除，或尚未与本机配对。", "It may have been removed, or it is not paired with this device yet.")}</Empty>;
  const local = isThis(device), reachable = local || device.online;
  const labels: Record<Tab, string> = { overview: t("概览", "Overview"), terminal: t("终端", "Terminal"), desktop: t("桌面", "Desktop"), browser: t("浏览器", "Browser"), projects: t("项目与会话", "Projects & sessions"), access: t("授权", "Access") };
  return <>
    <Link className="cx-btn ghost small" to="/app/home"><ArrowLeft size={15} aria-hidden="true" />{t("全部设备", "All devices")}</Link>
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
function Overview({ device }: { device: Device }) {
  const { t } = useApp();
  const local = isThis(device);
  return <>
    <Card>
      <dl className="cx-facts">
        <div><dt>{t("状态", "Status")}</dt><dd>{local || device.online ? t("在线", "Online") : t("离线", "Offline")}</dd></div>
        <div><dt>{t("连接路径", "Connection path")}</dt><dd>{local ? t("本机", "This device") : device.path === "direct" ? t("对等直连", "Direct peer connection") : device.path === "relay" ? t("经中继转发（仍端到端加密）", "Relayed (still end-to-end encrypted)") : t("未知", "Unknown")}</dd></div>
        <div><dt>{t("延迟", "Latency")}</dt><dd>{device.rtt_ms !== null ? device.rtt_ms + " ms" : "—"}</dd></div>
        <div><dt>{t("关系", "Relationship")}</dt><dd>{local ? t("你正在使用的设备", "The device you are using") : device.guest ? t("访客设备", "Guest device") : t("你的设备", "Your device")}</dd></div>
        <div><dt>{t("设备 ID", "Device ID")}</dt><dd className="cx-mono">{device.node_id}</dd></div>
      </dl>
    </Card>
    <Section title={t("能力", "Capabilities")} description={local ? t("在「本机 › 能力与权限」中安装、启动和共享。", "Install, start and share them under This device › Capabilities.") : t("可以在这台设备上使用的功能。", "What you can use on this device.")}>
      <div className="cx-grid">{deviceCaps.map(cap => {
        const Icon = capabilityIcons[cap], on = device.caps[cap] !== false;
        const tab = cap === "terminals" ? "terminal" : cap === "computer" ? "access" : cap;
        return <Card key={cap}><div className="cx-card-head"><span className={"cx-card-icon " + (on ? "" : "muted")}><Icon size={20} aria-hidden="true" /></span><div className="cx-grow"><h3>{capabilityName(cap, t)}</h3><small className="cx-muted">{on ? t("可用", "Available") : t("未授权", "Not allowed")}</small></div>
          {local && cap !== "terminals" ? <Link className="cx-btn small" to="/app/this">{t("管理", "Manage")}</Link> : <Link className="cx-btn small" to={devicePath(device, tab)}>{cap === "computer" ? t("授权", "Access") : t("打开", "Open")}</Link>}</div></Card>;
      })}</div>
    </Section>
  </>;
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
  const { t, locale } = useApp();
  const [open, setOpen] = useState(false);
  if (isThis(device)) return <Card className="cx-stack"><div className="cx-card-head"><span className="cx-card-icon"><Monitor size={20} aria-hidden="true" /></span><div className="cx-grow"><h3>{t("这就是你正在使用的设备", "This is the device you are using")}</h3>
    <p className="cx-muted">{t("要让其他设备查看这块屏幕，请在「能力与权限」中开启桌面共享。", "To let your other devices see this screen, turn on desktop sharing under Capabilities.")}</p></div></div>
    <div className="cx-actions"><Link className="cx-btn primary" to="/app/this">{t("管理桌面共享", "Manage desktop sharing")}</Link><Link className="cx-btn" to="/desktop">{t("所有远程桌面", "All remote desktops")}</Link></div></Card>;
  const allowed = device.caps.desktop !== false;
  return <div className="cx-stack">
    {!allowed && <Alert tone="warn">{t("这台设备没有共享桌面给你。", "This device does not share its desktop with you.")}</Alert>}
    {open ? <VncViewer target={{ node: device.node_id, local: false }} connect={webDesktopConnector} language={locale} />
      : <Card className="cx-stack"><div className="cx-card-head"><span className="cx-card-icon"><Monitor size={20} aria-hidden="true" /></span><div className="cx-grow"><h3>{t("远程桌面", "Remote desktop")}</h3>
        <p className="cx-muted">{t("画面经端到端加密的对等连接传输。需要对方设备已开启桌面共享。", "The screen streams over an end-to-end encrypted peer connection. The other device must have desktop sharing turned on.")}</p></div></div>
        <div className="cx-actions"><button className="cx-btn primary" disabled={!allowed || !device.online} onClick={() => setOpen(true)}>{t("连接桌面", "Connect to desktop")}</button><Link className="cx-btn" to="/desktop">{t("所有远程桌面", "All remote desktops")}</Link></div></Card>}
  </div>;
}
const directClient: DirectClient = command => request("/api/peers", command);
function BrowserTab({ device }: { device: Device }) {
  const { t, locale } = useApp();
  if (isThis(device)) return <Card className="cx-stack"><div className="cx-card-head"><span className="cx-card-icon"><Globe size={20} aria-hidden="true" /></span><div className="cx-grow"><h3>{t("本机浏览器", "This device's browser")}</h3>
    <p className="cx-muted">{t("Cranibox 可以启动一个独立配置的浏览器，供 Agent 操作，不影响你日常使用的浏览器。", "Cranibox can launch a browser with its own profile for agents to drive, separate from the browser you use every day.")}</p></div></div>
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
      {device.guest && <div className="cx-actions" style={{ marginTop: 16 }}><button className="cx-btn danger" disabled={busy} onClick={() => void run(() => api.call("PUT", "/devices/" + encodeURIComponent(device.node_id), { revoke: true }))}>{t("撤销访问", "Revoke access")}</button></div>}
    </Card>
    <Card label={t("我能对它做什么", "What I can do on it")}>
      <div className="cx-card-head"><span className="cx-card-icon muted"><MousePointerClick size={20} aria-hidden="true" /></span><div className="cx-grow"><h3>{t("我能对它做什么", "What I can do on it")}</h3>
        <p className="cx-muted">{t("由这台设备的主人决定，在那台设备上修改。", "Decided by that device's owner, and changed on that device.")}</p></div></div>
      <ul className="cx-toggle-list" style={{ listStyle: "none", margin: 0, padding: 0 }}>{deviceCaps.map(cap => <li key={cap} className="cx-toggle-row"><span>{capabilityName(cap, t)}</span><Pill tone={device.caps[cap] !== false ? "good" : ""}>{device.caps[cap] !== false ? t("允许", "Allowed") : t("未允许", "Not allowed")}</Pill></li>)}</ul>
    </Card>
  </div>;
}

