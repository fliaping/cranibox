import { Languages, Lock, Moon, Plus, QrCode, Radio, Sun, SunMoon } from "lucide-react";
import { Link } from "react-router-dom";
import { DesktopAppSettings } from "./DesktopAppSettings";
import { devicePath } from "./Home";
import { inShell } from "./shell";
import { useApp, type Theme } from "./store";
import { Card, DeviceIcon, isThis, pathLabel, Pill, Section } from "./ui";

/** Shown wherever devices are managed: pairing is symmetric between peers. */
export function PairingCard() {
  const { t, devices, setPairing } = useApp();
  return <Section title={t("我的设备", "My devices")} description={t("所有设备彼此对等。任意一台都可以发出邀请或接受邀请。", "All devices are equal peers. Any of them can send or accept an invitation.")}
    actions={<button className="cx-btn primary" onClick={() => setPairing(true)}><Plus size={16} aria-hidden="true" />{t("添加设备", "Add device")}</button>}>
    {devices.length ? <div className="cx-list">{devices.map(device => <Link key={device.node_id} className="cx-list-row" to={devicePath(device)}>
      <span className="cx-card-icon"><DeviceIcon kind={device.kind} size={18} /></span>
      <div className="cx-grow"><strong>{device.name}</strong><small>{pathLabel(device, t)}</small></div>
      {isThis(device) ? <Pill tone="accent">{t("本机", "This device")}</Pill> : device.guest ? <Pill tone="info">{t("访客", "Guest")}</Pill> : <Pill tone={device.online ? "good" : ""}>{device.online ? t("在线", "Online") : t("离线", "Offline")}</Pill>}
    </Link>)}</div> : <Card><p className="cx-muted"><QrCode size={16} aria-hidden="true" /> {t("还没有配对的设备。", "No paired devices yet.")}</p></Card>}
  </Section>;
}
/** iroh is the built-in transport: summarize how peers are currently reached. */
export function PeerTransport() {
  const { t, devices } = useApp();
  const peers = devices.filter(device => !isThis(device) && device.online);
  const direct = peers.filter(device => device.path === "direct").length, relay = peers.filter(device => device.path === "relay").length;
  return <Card className="cx-stack" label="iroh">
    <div className="cx-card-head"><span className="cx-card-icon"><Radio size={20} aria-hidden="true" /></span>
      <div className="cx-grow"><h3>{t("iroh 对等连接", "iroh peer connections")}</h3>
        <p className="cx-muted cx-small">{t("内置传输：设备之间优先直连，打洞失败时经中继转发；两种路径都是端到端加密。", "Built-in transport: devices connect directly when possible and fall back to a relay. Both paths are end-to-end encrypted.")}</p></div>
      <Pill tone="good" icon={Lock}>{t("端到端加密", "End-to-end encrypted")}</Pill></div>
    <div className="cx-actions"><Pill tone="good">{t("直连 ", "Direct ") + direct}</Pill><Pill tone={relay ? "warn" : ""}>{t("中继 ", "Relayed ") + relay}</Pill><Pill>{t("离线 ", "Offline ") + devices.filter(device => !isThis(device) && !device.online).length}</Pill></div>
  </Card>;
}
export function Settings() {
  const { t, theme, setTheme, locale, setLocale } = useApp();
  const themes: [Theme, string, typeof Sun][] = [["system", t("跟随系统", "System"), SunMoon], ["light", t("浅色", "Light"), Sun], ["dark", t("深色", "Dark"), Moon]];
  return <div className="cx-stack loose">
    {inShell() && <DesktopAppSettings section="app" />}
    <Card label={t("外观", "Appearance")}><div className="cx-between"><div><h3>{t("外观", "Appearance")}</h3><p className="cx-muted cx-small">{t("浅色、深色，或跟随系统。", "Light, dark, or follow your system.")}</p></div>
      <div className="cx-segment" role="group" aria-label={t("外观", "Appearance")}>{themes.map(([value, label, Icon]) => <button key={value} aria-pressed={theme === value} onClick={() => setTheme(value)}><Icon size={15} aria-hidden="true" /> {label}</button>)}</div></div></Card>
    <Card label={t("语言", "Language")}><div className="cx-between"><div><h3><Languages size={16} aria-hidden="true" /> {t("语言", "Language")}</h3><p className="cx-muted cx-small">{t("所有页面都会切换。", "Applies to every page.")}</p></div>
      <label><span className="cx-sr">{t("语言", "Language")}</span><select className="cx-select" aria-label={t("语言", "Language")} value={locale.startsWith("zh") ? "zh-cn" : "en"} onChange={event => setLocale(event.target.value)}><option value="zh-cn">简体中文</option><option value="en">English</option></select></label></div></Card>
    <Card label={t("设置向导", "Setup guide")}><div className="cx-between"><div><h3>{t("设置向导", "Setup guide")}</h3><p className="cx-muted cx-small">{t("重新检查这台设备的权限、身份与恢复码。", "Check this device's permissions, identity and recovery code again.")}</p></div>
      <Link className="cx-btn" to="/app/setup">{t("打开向导", "Open setup guide")}</Link></div></Card>
    <Card label={t("隐私", "Privacy")}><h3>{t("隐私", "Privacy")}</h3><p className="cx-muted cx-small">{t("设备之间的流量端到端加密，经对等连接直接传输。没有中心服务器保存你的数据；需要你决定的操作都会出现在「待办」。", "Traffic between your devices is end-to-end encrypted and travels over direct peer connections. No central server keeps your data; anything that needs your decision appears in your Inbox.")}</p></Card>
  </div>;
}
