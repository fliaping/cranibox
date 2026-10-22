import { Copy, Fingerprint, KeyRound, Lock, Plus, Radio } from "lucide-react";
import { useEffect, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import type { Device, Identity } from "../personal/api";
import { Nodes } from "../pages/Nodes";
import { devicePath } from "./Home";
import { useApp } from "./store";
import { Card, isThis, Pill, Section, type Translate } from "./ui";

const WIDTH = 720, HEIGHT = 460, CX = WIDTH / 2, CY = HEIGHT / 2;
const short = (text: string, size = 16) => text.length > size ? text.slice(0, size - 1) + "…" : text;
function edgeLabel(device: Device, t: Translate) {
  if (!device.online) return "";
  const kind = device.path === "direct" ? t("直连", "direct") : device.path === "relay" ? t("中继", "relay") : "";
  return [kind, device.rtt_ms != null ? device.rtt_ms + " ms" : ""].filter(Boolean).join(" · ");
}
/** This device in the middle; every peer around it, joined by how it is reached right now. */
export function NetworkMap() {
  const { t, devices } = useApp();
  const navigate = useNavigate();
  const self = devices.find(isThis);
  const peers = devices.filter(device => !isThis(device)).sort((a, b) => Number(b.online) - Number(a.online) || a.name.localeCompare(b.name));
  const viaRelay = (device: Device) => device.online && device.path === "relay";
  const upper = peers.filter(viaRelay), lower = peers.filter(device => !viaRelay(device));
  const relayed = upper.length > 0;
  const radius = { x: Math.max(250, Math.min(320, 160 + peers.length * 20)), y: 165 };
  const relay = { x: CX, y: CY - radius.y * 0.42 };
  const spread = (items: Device[], from: number, to: number) => items.map((device, index) => {
    const degrees = items.length === 1 ? (from + to) / 2 : from + (to - from) * index / (items.length - 1);
    const angle = degrees * Math.PI / 180;
    return { device, x: CX + Math.cos(angle) * radius.x, y: CY + Math.sin(angle) * radius.y };
  });
  // Relayed peers sit behind the relay on the upper arc; the rest share the lower arc
  // (or the whole circle when nothing is relayed).
  const placed = relayed ? [...(upper.length === 1 ? spread(upper, 240, 240) : spread(upper, 205, 335)), ...spread(lower, 25, 155)]
    : lower.length === 1 ? spread(lower, 90, 90) : spread(lower, -90 + 180 / lower.length, 270 - 180 / lower.length);
  const open = (device: Device) => navigate(devicePath(device));
  return <Card className="cx-netmap" label={t("网络连接图", "Network map")}>
    <svg viewBox={`0 0 ${WIDTH} ${HEIGHT}`} role="img" aria-label={t("网络连接图：本机居中，连线表示与每台设备的连接方式", "Network map: this device in the middle, lines show how each device is reached")}>
      {relayed && <g className="cx-netmap-relay">
        <line x1={CX} y1={CY} x2={relay.x} y2={relay.y} />
        <rect x={relay.x - 34} y={relay.y - 15} width={68} height={30} rx={15} />
        <text x={relay.x} y={relay.y + 4} textAnchor="middle">{t("中继", "Relay")}</text>
      </g>}
      {placed.map(({ device, x, y }) => {
        const via = device.online && device.path === "relay";
        const kind = !device.online ? "offline" : via ? "relay" : "direct";
        const label = edgeLabel(device, t);
        const [fx, fy] = via ? [relay.x, relay.y] : [CX, CY];
        return <g key={device.node_id} className={"cx-netmap-edge " + kind}>
          <line x1={fx} y1={fy} x2={x} y2={y} />
          {label && <text x={(fx + x) / 2} y={(fy + y) / 2 - 6} textAnchor="middle">{label}</text>}
        </g>;
      })}
      {self && <g className="cx-netmap-node self" role="link" tabIndex={0} aria-label={self.name + " · " + t("本机", "this device")}
        onClick={() => open(self)} onKeyDown={event => { if (event.key === "Enter") open(self); }}>
        <circle cx={CX} cy={CY} r={30} />
        <text className="cx-netmap-initial" x={CX} y={CY + 6} textAnchor="middle">{self.name.slice(0, 1).toUpperCase()}</text>
        <text x={CX} y={CY + 50} textAnchor="middle">{short(self.name, 22)}</text>
      </g>}
      {placed.map(({ device, x, y }) => { const above = y < CY - 20; return <g key={device.node_id} className={"cx-netmap-node " + (device.online ? "online" : "offline") + (device.guest ? " guest" : "")}
        role="link" tabIndex={0} aria-label={device.name + " · " + (device.online ? edgeLabel(device, t) || t("在线", "online") : t("离线", "offline"))}
        onClick={() => open(device)} onKeyDown={event => { if (event.key === "Enter") open(device); }}>
        <circle cx={x} cy={y} r={22} />
        <text className="cx-netmap-initial" x={x} y={y + 5} textAnchor="middle">{device.name.slice(0, 1).toUpperCase()}</text>
        <text x={x} y={above ? y - 32 : y + 40} textAnchor="middle">{short(device.name)}</text>
      </g>; })}
      {!peers.length && <text className="cx-netmap-hint" x={CX} y={CY + 100} textAnchor="middle">{t("配对另一台设备后，它会出现在这里。", "Pair another device and it appears here.")}</text>}
    </svg>
    <div className="cx-netmap-legend" aria-hidden="true">
      <span><i className="direct" />{t("对等直连", "Direct")}</span><span><i className="relay" />{t("经中继（仍端到端加密）", "Relayed (still end-to-end encrypted)")}</span><span><i className="offline" />{t("离线", "Offline")}</span>
    </div>
    <ul className="cx-sr">{peers.map(device => <li key={device.node_id}>{device.name}: {device.online ? edgeLabel(device, t) : t("离线", "offline")}</li>)}</ul>
  </Card>;
}
export function useIdentity() {
  const { api } = useApp();
  const [identity, setIdentity] = useState<Identity | null>(null), [error, setError] = useState("");
  const load = () => api.call<Identity>("GET", "/identity").then(value => { setIdentity(value); setError(""); }, problem => setError(problem instanceof Error ? problem.message : String(problem)));
  useEffect(() => { void load(); }, [api]);
  return { identity, setIdentity, error, reload: load };
}
export function copyText(text: string) { return navigator.clipboard.writeText(text).then(() => true, () => false); }
/** The identity shared by all your devices; same-identity devices see each other without pairing. */
export function IdentityCard({ compact = false }: { compact?: boolean }) {
  const { t } = useApp();
  const { identity, error } = useIdentity();
  const [copied, setCopied] = useState(false);
  return <Card label={t("身份", "Identity")}>
    <div className="cx-card-head"><span className="cx-card-icon"><Fingerprint size={20} aria-hidden="true" /></span>
      <div className="cx-grow"><h3>{t("你的身份", "Your identity")}</h3>
        <p className="cx-muted cx-small">{t("同一身份下的设备互相可见，无需逐台配对。没有账号，也没有服务器保存它。", "Devices under one identity see each other without pairing each pair. No account, and no server keeps it.")}</p></div></div>
    {identity ? <>
      <div className="cx-between" style={{ marginTop: 12 }}><code className="cx-identity-code">{identity.code}</code>
        <button className="cx-btn small" onClick={() => void copyText(identity.code).then(setCopied)}><Copy size={14} aria-hidden="true" />{copied ? t("已复制", "Copied") : t("复制", "Copy")}</button></div>
      <div className="cx-actions" style={{ marginTop: 12 }}>
        <Pill>{t(identity.members + " 台设备", identity.members + (identity.members === 1 ? " device" : " devices"))}</Pill>
        <Pill tone={identity.recovery ? "good" : "warn"} icon={KeyRound}>{identity.recovery ? t("已设置恢复码", "Recovery code set") : t("尚未设置恢复码", "No recovery code yet")}</Pill>
        {!compact || !identity.recovery ? <Link className="cx-btn small" to="/app/identity">{t("身份与安全", "Identity & security")}</Link> : null}
      </div>
    </> : <p className="cx-muted cx-small" role={error ? undefined : "status"}>{error || t("正在读取身份…", "Loading identity…")}</p>}
  </Card>;
}
export function NetworkPage() {
  const { t, devices, setPairing } = useApp();
  const peers = devices.filter(device => !isThis(device));
  const online = peers.filter(device => device.online);
  return <>
    <div className="cx-page-head"><div><h1>{t("网络", "Network")}</h1><p>{t("设备之间优先对等直连，打洞失败时经中继转发；两种路径都端到端加密。", "Devices connect directly when they can and fall back to a relay. Both paths are end-to-end encrypted.")}</p></div>
      <div className="cx-actions"><button className="cx-btn primary" onClick={() => setPairing(true)}><Plus size={16} aria-hidden="true" />{t("添加设备", "Add device")}</button></div></div>
    <div className="cx-actions" style={{ marginBottom: 16 }}>
      <Pill tone="good" icon={Lock}>{t("端到端加密", "End-to-end encrypted")}</Pill>
      <Pill tone="good">{t("直连 ", "Direct ") + online.filter(device => device.path === "direct").length}</Pill>
      <Pill tone={online.some(device => device.path === "relay") ? "warn" : ""}>{t("中继 ", "Relayed ") + online.filter(device => device.path === "relay").length}</Pill>
      <Pill>{t("离线 ", "Offline ") + (peers.length - online.length)}</Pill>
    </div>
    <NetworkMap />
    <div className="cx-grid two" style={{ marginTop: 16 }}>
      <IdentityCard compact />
      <Card label={t("添加设备", "Add a device")}>
        <div className="cx-card-head"><span className="cx-card-icon"><Radio size={20} aria-hidden="true" /></span>
          <div className="cx-grow"><h3>{t("添加设备", "Add a device")}</h3>
            <p className="cx-muted cx-small">{t("扫描二维码、粘贴邀请链接，或输入运行 Cranibox 的服务器地址（如 https://box.example.com）——服务器主人批准后自动配对，并优先使用它的中继。", "Scan a QR code, paste an invitation link, or enter the address of a server running Cranibox (e.g. https://box.example.com). Once its owner approves, the devices pair and prefer its relay.")}</p></div></div>
        <div className="cx-actions" style={{ marginTop: 12 }}><button className="cx-btn" onClick={() => setPairing(true)}>{t("开始配对", "Start pairing")}</button></div>
      </Card>
    </div>
    <Section title={t("高级", "Advanced")} description={t("连接路径、监听地址，以及 EasyTier / Tailscale 网络。", "Connection paths, listener, and EasyTier / Tailscale networks.")}>
      <Nodes embedded />
    </Section>
  </>;
}
