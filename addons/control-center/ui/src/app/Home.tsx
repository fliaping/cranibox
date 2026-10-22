import { Activity, Bell, ChevronRight, Gauge, Globe, Lock, Monitor, Network, Plus, Sparkles, SquareTerminal, X } from "lucide-react";
import { useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import type { Device } from "../personal/api";
import { useTerminalWindow } from "../terminal/TerminalWindowContext";
import { store, useApp } from "./store";
import { approvalTitle, capabilityIcons, capabilityName, Card, DeviceIcon, Empty, isThis, pathLabel, Pill, Section } from "./ui";
import { useTerminalNode } from "./targets";

export const deviceCaps = ["terminals", "desktop", "computer", "browser"] as const;
export function devicePath(device: Device, tab = "") {
  return "/app/device/" + encodeURIComponent(device.node_id) + (tab ? "/" + tab : "");
}
export function DeviceCard({ device }: { device: Device }) {
  const { t } = useApp();
  const navigate = useNavigate(), { openTerminal } = useTerminalWindow(), terminalNode = useTerminalNode();
  const local = isThis(device), reachable = local || device.online;
  return <Card className={"cx-device pa-card " + (reachable ? "" : "offline")} label={device.name}>
    <div className="cx-card-head">
      <span className="cx-card-icon"><DeviceIcon kind={device.kind} /></span>
      <div className="cx-grow">
        <Link className="cx-device-name" to={devicePath(device)}><h3>{device.name}</h3></Link>
        <div className="cx-device-meta"><span className={"cx-dot " + (reachable ? "online" : "")} aria-hidden="true" />{reachable ? <>{t("在线", "Online")}<span aria-hidden="true">·</span>{pathLabel(device, t)}</> : t("离线", "Offline")}</div>
      </div>
      {local ? <Pill tone="accent">{t("本机", "This device")}</Pill> : device.guest ? <Pill tone="info">{t("访客", "Guest")}</Pill> : null}
    </div>
    <div className="cx-cap-badges" aria-label={t("能力", "Capabilities")}>
      {deviceCaps.map(cap => { const Icon = capabilityIcons[cap]; const on = device.caps[cap] !== false; return <span key={cap} className={"cx-cap " + (on ? "on" : "off")} title={on ? undefined : t("未授权", "Not allowed")}><Icon aria-hidden="true" />{capabilityName(cap, t)}</span>; })}
      {device.badges.includes("claude") && <span className="cx-cap on">Claude</span>}
    </div>
    <div className="cx-quick">
      <button className="cx-btn small" disabled={!reachable || device.caps.terminals === false} onClick={() => openTerminal({ node: terminalNode(device), create: true })}><SquareTerminal size={15} aria-hidden="true" />{t("终端", "Terminal")}</button>
      <button className="cx-btn small" disabled={!reachable || device.caps.desktop === false} onClick={() => navigate(devicePath(device, "desktop"))}><Monitor size={15} aria-hidden="true" />{t("桌面", "Desktop")}</button>
      <button className="cx-btn small" disabled={!reachable || device.caps.browser === false} onClick={() => navigate(devicePath(device, "browser"))}><Globe size={15} aria-hidden="true" />{t("浏览器", "Browser")}</button>
    </div>
  </Card>;
}
/** Every paired device as a card: status, path, capabilities and quick actions. */
export function Devices() {
  const { t, devices, setPairing } = useApp();
  const mine = devices.filter(device => !device.guest).sort((a, b) => Number(isThis(b)) - Number(isThis(a)) || Number(b.online) - Number(a.online));
  const guests = devices.filter(device => device.guest);
  return <>
    <Section title={t("我的设备", "My devices")} description={t("点击设备查看终端、桌面、浏览器、项目与授权。", "Open a device for its terminal, desktop, browser, projects and access.")}
      actions={<button className="cx-btn primary" onClick={() => setPairing(true)}><Plus size={16} aria-hidden="true" />{t("添加设备", "Add device")}</button>}>
      <div className="cx-device-grid">
        {mine.map(device => <DeviceCard key={device.node_id} device={device} />)}
        <button className="cx-add-device" onClick={() => setPairing(true)}><Plus size={26} aria-hidden="true" />{t("添加设备", "Add device")}<small className="cx-muted">{t("扫码、粘贴邀请链接或服务器地址", "Scan a QR code, or paste a link or server address")}</small></button>
      </div>
    </Section>
    {guests.length > 0 && <Section title={t("访客设备", "Guest devices")} description={t("访客只能使用你逐项允许的功能。", "Guests can use only what you allow, one capability at a time.")}>
      <div className="cx-device-grid">{guests.map(device => <DeviceCard key={device.node_id} device={device} />)}</div>
    </Section>}
    {!mine.length && <Empty icon={Network} title={t("还没有设备", "No devices yet")}>{t("本机节点正在启动，或尚未加入设备网络。", "This node is starting or has not joined a device network yet.")}</Empty>}
  </>;
}
/** Network health at a glance, plus the things people do most. */
export function Home() {
  const { t, devices, approvals, status, setPairing, deviceName } = useApp();
  const navigate = useNavigate(), { openTerminal } = useTerminalWindow(), terminalNode = useTerminalNode();
  const mine = devices.filter(device => !device.guest);
  const peers = devices.filter(device => !isThis(device));
  const online = peers.filter(device => device.online);
  const direct = online.filter(device => device.path === "direct").length;
  const latencies = online.map(device => device.rtt_ms).filter((value): value is number => typeof value === "number");
  const latency = latencies.length ? Math.round(latencies.reduce((sum, value) => sum + value, 0) / latencies.length) : null;
  const reachable = devices.filter(device => isThis(device) || device.online).sort((a, b) => Number(isThis(b)) - Number(isThis(a)));
  const [target, setTarget] = useState("");
  const selected = reachable.find(device => device.node_id === target) || reachable[0];
  return <>
    <Welcome />
    <div className="cx-page-head"><div><h1>{t("首页", "Home")}</h1><p>{t("你的设备网络概况和常用操作。", "Your device network at a glance, and the things you do most.")}</p></div>
      <div className="cx-actions"><button className="cx-btn primary" onClick={() => setPairing(true)}><Plus size={17} aria-hidden="true" />{t("添加设备", "Add device")}</button></div></div>
    <section className="cx-stats" aria-label={t("网络概况", "Network summary")}>
      <Link className="cx-stat" to="/app/devices"><Network aria-hidden="true" /><strong>{mine.filter(device => isThis(device) || device.online).length}<small> / {mine.length}</small></strong><span>{t("设备在线", "devices online")}</span></Link>
      <Link className="cx-stat" to="/nodes"><Activity aria-hidden="true" /><strong>{online.length ? direct + " / " + online.length : "—"}</strong><span>{t("对等直连", "direct connections")}</span></Link>
      <Link className="cx-stat" to="/nodes"><Gauge aria-hidden="true" /><strong>{latency === null ? "—" : latency + " ms"}</strong><span>{t("平均延迟", "average latency")}</span></Link>
      <Link className="cx-stat" to="/app/inbox"><Bell aria-hidden="true" /><strong>{approvals.length}</strong><span>{t("待你确认", "waiting for you")}</span></Link>
      <Link className="cx-stat" to="/app/models"><Sparkles aria-hidden="true" /><strong>{status?.models_available ?? "—"}</strong><span>{t("可用模型", "models available")}</span></Link>
    </section>
    {approvals.length > 0 && <div className="cx-approval-strip" role="status">
      <Bell size={20} aria-hidden="true" />
      <div className="cx-grow"><strong>{t("有 " + approvals.length + " 件事等你确认", approvals.length + (approvals.length === 1 ? " request needs" : " requests need") + " your decision")}</strong>
        <ul>{approvals.slice(0, 2).map(card => <li key={card.id}>{approvalTitle(card, t)} · {card.from_node ? deviceName(card.from_node) : card.agent || deviceName(card.on_node)}</li>)}</ul></div>
      <Link className="cx-btn" to="/app/inbox">{t("去处理", "Review")}<ChevronRight size={16} aria-hidden="true" /></Link>
    </div>}
    <div className="cx-grid two">
      <Card label={t("常用操作", "Quick actions")}>
        <h3>{t("常用操作", "Quick actions")}</h3>
        <label className="cx-field" style={{ marginTop: 12 }}>{t("设备", "Device")}
          <select className="cx-select" value={selected?.node_id || ""} onChange={event => setTarget(event.target.value)} disabled={!reachable.length}>
            {reachable.map(device => <option key={device.node_id} value={device.node_id}>{device.name}{isThis(device) ? " · " + t("本机", "this device") : ""}</option>)}
          </select></label>
        <div className="cx-quick-grid">
          <button className="cx-btn" disabled={!selected || selected.caps.terminals === false} onClick={() => selected && openTerminal({ node: terminalNode(selected), create: true })}><SquareTerminal size={16} aria-hidden="true" />{t("打开终端", "Open terminal")}</button>
          <button className="cx-btn" disabled={!selected || isThis(selected) || selected.caps.desktop === false} onClick={() => selected && navigate(devicePath(selected, "desktop"))}><Monitor size={16} aria-hidden="true" />{t("远程桌面", "Remote desktop")}</button>
          <button className="cx-btn" disabled={!selected || selected.caps.browser === false} onClick={() => selected && navigate(devicePath(selected, "browser"))}><Globe size={16} aria-hidden="true" />{t("浏览器", "Browser")}</button>
          <button className="cx-btn" disabled={!selected} onClick={() => selected && navigate(devicePath(selected))}><ChevronRight size={16} aria-hidden="true" />{t("设备详情", "Device details")}</button>
        </div>
      </Card>
      <Card label={t("设备", "Devices")}>
        <div className="cx-between"><h3>{t("设备", "Devices")}</h3><Link className="cx-btn small ghost" to="/app/devices">{t("全部", "All")}<ChevronRight size={14} aria-hidden="true" /></Link></div>
        <div className="cx-list" style={{ marginTop: 12 }}>{devices.slice().sort((a, b) => Number(isThis(b)) - Number(isThis(a)) || Number(b.online) - Number(a.online)).slice(0, 6).map(device => <Link key={device.node_id} className="cx-list-row" to={devicePath(device)}>
          <span className="cx-card-icon"><DeviceIcon kind={device.kind} size={18} /></span>
          <div className="cx-grow"><strong>{device.name}</strong><small>{pathLabel(device, t)}</small></div>
          <span className={"cx-dot " + (isThis(device) || device.online ? "online" : "")} aria-hidden="true" />
        </Link>)}</div>
      </Card>
    </div>
  </>;
}
/** First-run guidance that never blocks the page (a dialog would trap focus and input). */
function Welcome() {
  const { t, setPairing } = useApp();
  const [open, setOpen] = useState(() => { try { return localStorage.getItem("cranibox.onboarded") !== "1"; } catch { return false; } });
  if (!open) return null;
  const finish = () => { store("cranibox.onboarded", "1"); setOpen(false); };
  return <Card className="cx-welcome" label={t("欢迎使用 Cranibox", "Welcome to Cranibox")}>
    <div className="cx-card-head"><span className="cx-card-icon"><Lock size={20} aria-hidden="true" /></span>
      <div className="cx-grow"><h3>{t("欢迎使用 Cranibox", "Welcome to Cranibox")}</h3>
        <p className="cx-muted">{t("把你的每台设备连成一个 Agent 工作台：终端、桌面、浏览器与 Computer Use 都能跨设备使用。设备之间端到端加密、对等直连；重要操作始终先问你。",
          "Turn every device you own into one workspace for your agents: terminal, desktop, browser and Computer Use across devices. Devices connect peer to peer with end-to-end encryption, and important actions always ask you first.")}</p></div>
      <button className="cx-icon-btn" aria-label={t("关闭欢迎", "Dismiss welcome")} onClick={finish}><X size={16} aria-hidden="true" /></button></div>
    <div className="cx-actions" style={{ marginTop: 12 }}>
      <button className="cx-btn primary" onClick={() => { finish(); setPairing(true); }}><Plus size={15} aria-hidden="true" />{t("连接另一台设备", "Connect another device")}</button>
      <Link className="cx-btn" to="/app/models" onClick={finish}><Sparkles size={15} aria-hidden="true" />{t("添加模型订阅", "Add a model subscription")}</Link>
    </div>
  </Card>;
}
