import { Copy, Fingerprint, KeyRound } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { Link } from "react-router-dom";
import type { Identity } from "../personal/api";
import { request } from "../workspace";
import { store, stored, useApp } from "./store";
import { Alert, Card, isThis, PageHead, Pill, Switch } from "./ui";
import { OtherNetworks, DirectRoutes } from "../pages/Nodes";
import { NetworkPaths } from "./NetworkPaths";

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
type IrohStatus = { lan: boolean; relays: string[]; n0_fallback: boolean; learned_relays: string[]; this_relay: { enabled: boolean; public_url: string | null } };
/** One labelled row of a settings group: what it is on the left, its state or control on the right. */
function Row({ title, detail, children }: { title: string; detail?: ReactNode; children?: ReactNode }) {
  return <div className="cx-setting-row"><div className="cx-grow"><strong>{title}</strong>{detail && <small className="cx-muted">{detail}</small>}</div>{children && <div className="cx-setting-control">{children}</div>}</div>;
}
/** A folded settings group that remembers whether it was open; its content mounts only when open. */
function Disclosure({ id, title, detail, children }: { id: string; title: string; detail: string; children: ReactNode }) {
  const key = "cranibox.network." + id;
  const [open, setOpen] = useState(() => stored(key, "") === "1");
  return <details className="cx-settings-group cx-disclosure" open={open} onToggle={event => { const next = (event.currentTarget as HTMLDetailsElement).open; setOpen(next); store(key, next ? "1" : ""); }}>
    <summary><h2>{title}</h2><small className="cx-muted">{detail}</small></summary>
    {open && children}
  </details>;
}
/** How this device takes part in the network: connections, relays, other networks.
 * Devices live under Devices; identity and recovery under Settings → Identity & security. */
export function NetworkPage() {
  const { t, devices } = useApp();
  const [status, setStatus] = useState<IrohStatus | null>(null), [error, setError] = useState(""), [busy, setBusy] = useState(false);
  const [editing, setEditing] = useState(false), [relays, setRelays] = useState(""), [notice, setNotice] = useState("");
  useEffect(() => { void request("/api/network", { operation: "status" }).then((value: { iroh?: IrohStatus }) => setStatus(value.iroh || null), problem => setError(problem instanceof Error ? problem.message : String(problem))); }, []);
  const peers = devices.filter(device => !isThis(device));
  const online = peers.filter(device => device.online);
  const direct = online.filter(device => device.path === "direct").length, relayed = online.filter(device => device.path === "relay").length;
  async function save(change: Partial<{ lan: boolean; n0_fallback: boolean; relays: string[] }>) {
    if (!status) return;
    setBusy(true); setError(""); setNotice("");
    try {
      const value = await request("/api/network", { operation: "iroh_settings", lan: status.lan, n0_fallback: status.n0_fallback, relays: status.relays, ...change }) as { iroh?: IrohStatus };
      if (value.iroh) setStatus(value.iroh);
      setNotice(t("已保存，重启 Cranibox 后生效。", "Saved. Takes effect after Cranibox restarts."));
      setEditing(false);
    } catch (problem) { setError(problem instanceof Error ? problem.message : String(problem)); } finally { setBusy(false); }
  }
  const inUse = status ? [...new Set([...(status.this_relay.public_url ? [status.this_relay.public_url] : []), ...status.relays, ...status.learned_relays])] : [];
  return <div className="cx-settings">
    <PageHead title={t("网络设置", "Network settings")} description={t("这台设备怎样连接你的其他设备。设备列表在「设备」里，身份与恢复口令在「本机设置 → 身份与安全」。", "How this device reaches your other devices. Your devices are under Devices; identity and recovery under Settings → Identity & security.")} />
    {error && <Alert>{error}</Alert>}
    {notice && <div className="cx-notice" role="status"><span>{notice}</span></div>}
    <section className="cx-settings-group" aria-label={t("连接", "Connection")}>
      <h2>{t("连接", "Connection")}</h2>
      <Row title={t("当前连接", "Right now")} detail={t("优先直连，打洞失败时经中继转发；两种都端到端加密。", "Direct when possible, through a relay otherwise; both end-to-end encrypted.")}>
        <span className="cx-row"><Pill tone="good">{t("直连 ", "Direct ") + direct}</Pill><Pill tone={relayed ? "warn" : ""}>{t("中继 ", "Relayed ") + relayed}</Pill><Pill>{t("离线 ", "Offline ") + (peers.length - online.length)}</Pill></span>
      </Row>
      {status && <Switch label={t("局域网发现", "Local network discovery")} detail={t("在同一局域网内直接找到已配对的设备，不经互联网。", "Find paired devices on the same local network without going through the internet.")} checked={status.lan} disabled={busy} onChange={lan => void save({ lan })} />}
    </section>
    <NetworkPaths />
    {status && <section className="cx-settings-group" aria-label={t("中继", "Relays")}>
      <h2>{t("中继", "Relays")}</h2>
      <Row title={t("使用的中继", "Relays in use")} detail={inUse.length ? <span className="cx-mono">{inUse.join(" · ")}</span> : t("没有自己的中继，使用公共中继。", "No relay of your own; public relays are used.")}>
        <button className="cx-btn small" onClick={() => { setRelays(status.relays.join("\n")); setEditing(!editing); }}>{editing ? t("取消", "Cancel") : t("编辑", "Edit")}</button>
      </Row>
      {editing && <div className="cx-setting-row cx-stack">
        <label className="cx-field">{t("自己的中继（每行一个 https 地址）", "Your relays (one https address per line)")}
          <textarea rows={3} spellCheck={false} placeholder="https://box.example.com" value={relays} disabled={busy} onChange={event => setRelays(event.target.value)} /></label>
        {status.learned_relays.length > 0 && <small className="cx-muted">{t("配对时学到的中继会自动使用：", "Relays learned while pairing are used automatically: ")}<span className="cx-mono">{status.learned_relays.join(", ")}</span></small>}
        <div className="cx-actions"><button className="cx-btn primary" disabled={busy} onClick={() => void save({ relays: relays.split("\n").map(value => value.trim()).filter(Boolean) })}>{t("保存", "Save")}</button></div>
      </div>}
      <Switch label={t("公共中继后备", "Public relay fallback")} detail={t("自己的中继不可用时，使用 n0 公共中继。", "Use the public n0 relays when your own are unavailable.")} checked={status.n0_fallback} disabled={busy} onChange={n0_fallback => void save({ n0_fallback })} />
      <Row title={t("这台设备作为中继", "This device as a relay")} detail={status.this_relay.enabled ? (status.this_relay.public_url || t("运行中（未设置公网地址）", "Running (no public address)"))
        : <details className="cx-inline-details"><summary>{t("未开启（查看自建命令）", "Off (view self-host command)")}</summary><small className="cx-muted">{t("在有公网 HTTPS 入口的机器上运行 ", "On a machine with a public HTTPS address, run ")}<code>cranibox serve --public-url https://…</code></small></details>}>
        <Pill tone={status.this_relay.enabled ? "good" : ""}>{status.this_relay.enabled ? t("开启", "On") : t("关闭", "Off")}</Pill>
      </Row>
    </section>}
    <Disclosure id="networks" title={t("其他网络（可选）", "Other networks (optional)")} detail={t("EasyTier、Tailscale", "EasyTier, Tailscale")}><OtherNetworks /></Disclosure>
    <Disclosure id="troubleshooting" title={t("排障", "Troubleshooting")} detail={t("直连路由、监听地址", "Direct routes, listener")}><DirectRoutes /></Disclosure>
  </div>;
}
