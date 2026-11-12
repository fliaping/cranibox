import { Bell, ChevronRight, Globe, Lock, Monitor, Plus, SlidersHorizontal, SquareTerminal } from "lucide-react";
import { Link, useNavigate } from "react-router-dom";
import type { Device, Relation } from "../personal/api";
import { DeviceGraph } from "./DeviceGraph";
import { relationLabel, relationOf, relationTones } from "./relations";
import { useTerminalWindow } from "../terminal/TerminalWindowContext";
import { useApp } from "./store";
import { approvalTitle, DeviceIcon, formatDeviceLabel, isThis, pathLabel, Pill, type Translate } from "./ui";
import { useTerminalNode } from "./targets";

export const deviceCaps = ["terminals", "desktop", "computer", "browser"] as const;
export function devicePath(device: Device, tab = "") {
  return "/app/device/" + encodeURIComponent(device.node_id) + (tab ? "/" + tab : "");
}
export function since(seconds: number | null | undefined, t: Translate) {
  if (!seconds) return "";
  const age = Math.max(0, Math.round(Date.now() / 1000 - seconds));
  if (age < 60) return t("刚刚", "just now");
  if (age < 3600) return t(Math.round(age / 60) + " 分钟前", Math.round(age / 60) + " min ago");
  if (age < 86400) return t(Math.round(age / 3600) + " 小时前", Math.round(age / 3600) + " h ago");
  return new Date(seconds * 1000).toLocaleDateString();
}
/** One device per row: what it is, how it is reached, and the three things people open on it. */
function DeviceRow({ device }: { device: Device }) {
  const { t, deviceName } = useApp();
  const navigate = useNavigate(), { openTerminal } = useTerminalWindow(), terminalNode = useTerminalNode();
  const local = isThis(device), reachable = local || device.online, relation = relationOf(device);
  const info = formatDeviceLabel(device.name, device.node_id, t);
  // Only what this device lets you open: a row of greyed-out buttons says nothing useful.
  const terminals = device.caps.terminals !== false, desktop = device.caps.desktop !== false, browser = device.caps.browser !== false;
  const seen = since(device.last_seen, t);
  return <section className={"cx-device " + (reachable ? "" : "offline")} aria-label={device.name}>
    <span className="cx-card-icon"><DeviceIcon kind={device.kind} size={20} /></span>
    <div className="cx-grow">
      <div className="cx-device-title">
        <Link className="cx-device-name" to={devicePath(device)}><h3>{info.label}{info.rawHidden && <span className="cx-sr"> {info.rawHidden}</span>}</h3></Link>
        {info.shortId && <span className="cx-id-pill">{info.shortId}</span>}
        {local ? <Pill tone="accent">{t("本机", "This device")}</Pill> : relation !== "peer" ? <Pill tone={relationTones[relation]}>{relationLabel(relation, t)}</Pill> : null}
      </div>
      <div className="cx-device-meta"><span className={"cx-dot " + (reachable ? "online" : "")} aria-hidden="true" />
        {local ? t("在线", "Online") : reachable ? <>{t("在线", "Online")}<span aria-hidden="true">·</span>{pathLabel(device, t, deviceName)}</> : <>{t("离线", "Offline")}{seen && <><span aria-hidden="true">·</span>{t("最近在线 ", "last seen ") + seen}</>}</>}
      </div>
    </div>
    {reachable && <div className="cx-quick">
      {terminals && <button className="cx-btn small" onClick={() => openTerminal({ node: terminalNode(device), create: true })}><SquareTerminal size={15} aria-hidden="true" />{t("终端", "Terminal")}</button>}
      {local ? <Link className="cx-btn small" to="/app/this"><SlidersHorizontal size={15} aria-hidden="true" />{t("共享设置", "Sharing")}</Link> : <>
        {desktop && <button className="cx-btn small" onClick={() => navigate(devicePath(device, "desktop"), { state: { connect: true } })}><Monitor size={15} aria-hidden="true" />{t("桌面", "Desktop")}</button>}
        {browser && <button className="cx-btn small" onClick={() => navigate(devicePath(device, "browser"))}><Globe size={15} aria-hidden="true" />{t("浏览器", "Browser")}</button>}
        {!terminals && !desktop && !browser && <Link className="cx-device-none" to={devicePath(device, "access")}>{t("它还没有向你开放功能", "Nothing shared with you yet")}</Link>}
      </>}
    </div>}
    <Link className="cx-icon-btn cx-device-open" to={devicePath(device)} aria-label={t("查看 " + info.label, "Open " + info.label)}><ChevronRight size={18} aria-hidden="true" /></Link>
  </section>;
}
/** The home of the app: every device, how it is connected, and one click into each. */
export function Devices() {
  const { t, devices, approvals, setPairing, deviceName } = useApp();
  const order = (a: Device, b: Device) => Number(isThis(b)) - Number(isThis(a)) || Number(b.online) - Number(a.online) || a.name.localeCompare(b.name);
  const of = (relation: Relation) => devices.filter(device => (isThis(device) ? "peer" : relationOf(device)) === relation).sort(order);
  // Who may use whom decides the group: a device you control is not one that controls you.
  const groups = ([
    ["peer", t("我的设备", "My devices"), t("同一身份，可以互相使用", "one identity, each can use the other")],
    ["controlled", t("受控设备", "Controlled devices"), t("你可以在上面执行任务，它们不能访问你的设备", "you can run tasks on them; they cannot reach yours")],
    ["host", t("共享给我的设备", "Shared with me"), t("别人允许你使用，它们不能访问你的设备", "others let you use them; they cannot reach yours")],
    ["guest", t("访客设备", "Guest devices"), t("只能使用你逐项允许的功能", "can use only what you allow")],
    ["controller", t("控制这台设备的设备", "Devices that control this one"), t("可以在这台设备上执行任务，这台设备不能访问它们", "they can run tasks here; this device cannot reach them")],
  ] as [Relation, string, string][]).map(([relation, title, note]) => ({ relation, title, note, list: of(relation) })).filter(group => group.list.length);
  const others = devices.filter(device => !isThis(device)), online = others.filter(device => device.online);
  const relayed = online.filter(device => device.path === "relay").length, offline = others.length - online.length;
  const first = approvals[0];
  return <>
    <div className="cx-page-head"><div><h1>{t("设备", "Devices")}</h1>
      <p className="cx-summary">
        <span><Lock size={13} aria-hidden="true" />{t("端到端加密", "End-to-end encrypted")}</span>
        {others.length > 0 && <span>{t(online.length + " / " + others.length + " 台其他设备在线", online.length + " of " + others.length + " other devices online")}</span>}
        {online.length - relayed > 0 && <span>{t(online.length - relayed + " 台直连", online.length - relayed + " direct")}</span>}
        {relayed > 0 && <span className="warn">{t(relayed + " 台经中继", relayed + " relayed")}</span>}
        {offline > 0 && <span>{t(offline + " 台离线", offline + " offline")}</span>}
      </p></div>
      <div className="cx-actions"><button className="cx-btn primary" onClick={() => setPairing(true)}><Plus size={16} aria-hidden="true" />{t("添加设备", "Add device")}</button></div></div>
    {first && <Link className="cx-approval-strip" to="/app/inbox">
      <Bell size={18} aria-hidden="true" />
      <span className="cx-grow"><strong>{t(approvals.length + " 件事等你确认", approvals.length + (approvals.length === 1 ? " request needs" : " requests need") + " your decision")}</strong>
        <span className="cx-muted">{approvalTitle(first, t)} · {first.from_node ? deviceName(first.from_node) : first.agent || deviceName(first.on_node)}{approvals.length > 1 && t(" 等", " and more")}</span></span>
      <span className="cx-strip-go">{t("去处理", "Review")}<ChevronRight size={16} aria-hidden="true" /></span>
    </Link>}
    {groups.map(group => <div key={group.relation}>
      {groups.length > 1 && <div className="cx-list-title"><h2>{group.title}</h2><small>{group.note}</small></div>}
      <div className="cx-device-list">{group.list.map(device => <DeviceRow key={device.node_id} device={device} />)}</div>
    </div>)}
    {!others.length && <button className="cx-add-device" onClick={() => setPairing(true)}><Plus size={22} aria-hidden="true" /><span>{t("添加第二台设备", "Add a second device")}</span>
      <small className="cx-muted">{t("在另一台电脑、服务器或手机上安装 Cranibox，然后扫码或粘贴配对链接。", "Install Cranibox on another computer, server or phone, then scan the code or paste the pairing link.")}</small></button>}
    {/* The list is for doing; the graph below explains who may use whom. */}
    <DeviceGraph />
  </>;
}
