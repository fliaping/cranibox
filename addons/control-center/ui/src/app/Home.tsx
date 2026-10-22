import { Bell, ChevronRight, Globe, Lock, Monitor, Network, Plus, SquareTerminal, Users } from "lucide-react";
import { Link, useNavigate } from "react-router-dom";
import type { Device } from "../personal/api";
import { useTerminalWindow } from "../terminal/TerminalWindowContext";
import { useApp } from "./store";
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
export function Home() {
  const { t, devices, approvals, setPairing, deviceName } = useApp();
  const mine = devices.filter(device => !device.guest).sort((a, b) => Number(isThis(b)) - Number(isThis(a)) || Number(b.online) - Number(a.online));
  const guests = devices.filter(device => device.guest);
  const online = mine.filter(device => device.online || isThis(device)).length;
  return <>
    <div className="cx-hero">
      <div>
        <h1>{t("你的每台设备，都是 Agent 的工作台", "Every device you own, ready for your agents")}</h1>
        <p>{t("终端、桌面、浏览器与 Computer Use，在每台设备上随时可用。设备之间端到端加密、对等直连，没有中心服务器。",
          "Terminal, desktop, browser and Computer Use on every device. Devices connect directly peer to peer, end-to-end encrypted, with no central server.")}</p>
        <div className="cx-hero-facts">
          <Pill tone="good" icon={Network}>{t(online + " / " + mine.length + " 台设备在线", online + " of " + mine.length + " devices online")}</Pill>
          <Pill icon={Lock}>{t("端到端加密", "End-to-end encrypted")}</Pill>
          <Pill icon={Users}>{t("所有设备对等", "All devices are equal peers")}</Pill>
        </div>
      </div>
      <div className="cx-actions"><button className="cx-btn primary" onClick={() => setPairing(true)}><Plus size={17} aria-hidden="true" />{t("添加设备", "Add device")}</button></div>
    </div>
    {approvals.length > 0 && <div className="cx-approval-strip" role="status">
      <Bell size={20} aria-hidden="true" />
      <div className="cx-grow"><strong>{t("有 " + approvals.length + " 件事等你确认", approvals.length + (approvals.length === 1 ? " request needs" : " requests need") + " your decision")}</strong>
        <ul>{approvals.slice(0, 2).map(card => <li key={card.id}>{approvalTitle(card, t)} · {card.from_node ? deviceName(card.from_node) : card.agent || deviceName(card.on_node)}</li>)}</ul></div>
      <Link className="cx-btn" to="/app/inbox">{t("去处理", "Review")}<ChevronRight size={16} aria-hidden="true" /></Link>
    </div>}
    <Section title={t("我的设备", "My devices")} description={t("点击设备查看终端、桌面、浏览器、项目与授权。", "Open a device for its terminal, desktop, browser, projects and access.")}>
      <div className="cx-device-grid">
        {mine.map(device => <DeviceCard key={device.node_id} device={device} />)}
        <button className="cx-add-device" onClick={() => setPairing(true)}><Plus size={26} aria-hidden="true" />{t("添加设备", "Add device")}<small className="cx-muted">{t("扫码或粘贴邀请链接", "Scan a QR code or paste a link")}</small></button>
      </div>
    </Section>
    {guests.length > 0 && <Section title={t("访客设备", "Guest devices")} description={t("访客只能使用你逐项允许的功能。", "Guests can use only what you allow, one capability at a time.")}>
      <div className="cx-device-grid">{guests.map(device => <DeviceCard key={device.node_id} device={device} />)}</div>
    </Section>}
    {!mine.length && <Empty icon={Network} title={t("还没有设备", "No devices yet")}>{t("本机节点正在启动，或尚未加入设备网络。", "This node is starting or has not joined a device network yet.")}</Empty>}
  </>;
}
