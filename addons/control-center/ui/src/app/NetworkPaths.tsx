import { useEffect, useState } from "react";
import { useApp } from "./store";
import { Alert } from "./ui";

type Scope = "lan" | "internet" | "relay" | "overlay";
type Measurement = { from: string; to: string; kind: string; scope?: Scope | null; via?: string | null; rtt_ms: number | null; at: number };
type Paths = { nodes: { id: string; name: string }[]; paths: Measurement[] };
const glyphs = { lan: "⌂", internet: "↗", relay: "⇄", overlay: "◇", unknown: "?" };
const band = (ms: number | null | undefined) => ms == null ? "unknown" : ms < 30 ? "fast" : ms < 100 ? "good" : ms < 250 ? "slow" : "high";

export function NetworkPaths() {
  const { api, devices, t } = useApp();
  const [data, setData] = useState<Paths | null>(null), [error, setError] = useState(""), [now, setNow] = useState(Date.now);
  useEffect(() => {
    let active = true, pending = false;
    async function refresh() {
      setNow(Date.now());
      if (pending || document.hidden) return;
      pending = true;
      try {
        const value = await api.call<Paths>("GET", "/network/paths");
        if (active) { setData(value); setNow(Date.now()); setError(""); }
      } catch (problem) {
        if (active) setError(problem instanceof Error ? problem.message : String(problem));
      } finally { pending = false; }
    }
    void refresh();
    const timer = setInterval(() => void refresh(), 10_000);
    return () => { active = false; clearInterval(timer); };
  }, [api]);
  const nodes = [...new Set([...(data?.nodes.map(node => node.id) || []), ...devices.map(device => device.node_id)])].slice(0, 64).map(id => {
    const name = devices.find(device => device.node_id === id)?.name || data?.nodes.find(node => node.id === id)?.name;
    return { id, name: name && name !== id ? name : id.slice(0, 13) };
  });
  const paths = new Map(data?.paths.map(path => [path.from + ":" + path.to, path]));
  const labels = { lan: t("局域网直连", "LAN direct"), internet: t("互联网直连（打洞）", "Internet direct (hole-punched)"), relay: t("中转", "Relay"), overlay: t("叠加网络（Tailscale / EasyTier）", "Overlay (Tailscale / EasyTier)"), unknown: t("路径未知", "Unknown path") };
  function describe(from: typeof nodes[number], to: typeof nodes[number]) {
    const path = paths.get(from.id + ":" + to.id);
    const scope = path?.scope || (path?.kind === "relay" ? "relay" : "unknown");
    const ageMs = path ? Math.max(0, now - path.at) : 0;
    const minutes = Math.floor(ageMs / 60_000), hours = Math.floor(minutes / 60), days = Math.floor(hours / 24);
    const age = !path ? t("尚未测量", "Not measured") : days ? t(days + " 天前", days + " d ago") : hours ? t(hours + " 小时前", hours + " h ago") : minutes ? t(minutes + " 分钟前", minutes + " min ago") : t("刚刚", "Just now");
    const latency = path?.rtt_ms == null ? t("延迟未知", "Unknown latency") : path.rtt_ms + " ms";
    const route = labels[scope] + (path?.via ? " " + path.via : "");
    const stale = !!path && ageMs > 600_000;
    const details = [route, latency, age, ...(stale ? [t("已过期", "Stale")] : [])].join(" · ");
    const title = from.name + " → " + to.name + " · " + details;
    return { title, details, stale, content: <span tabIndex={0} title={title} aria-label={title} className={"cx-path-value cx-latency-" + band(path?.rtt_ms) + (stale ? " cx-path-stale" : "")}><span aria-hidden="true">{glyphs[scope]}</span><span aria-hidden="true">{path?.rtt_ms == null ? "—" : path.rtt_ms + " ms"}</span></span> };
  }
  const title = t("设备间连接", "Connections between devices");
  return <section className="cx-settings-group cx-network-paths" aria-label={title}>
    <h2>{title}</h2>
    <p className="cx-muted cx-small">{t("行是发起设备，列是目标设备。每台设备独立测量；超过 10 分钟的记录会淡化显示。", "Rows are from devices; columns are destinations. Each device measures independently. Measurements older than 10 minutes are faded.")}</p>
    {error && <Alert>{error}</Alert>}
    {!data && !error && <p role="status">{t("正在读取连接…", "Loading connections…")}</p>}
    {data && nodes.length < 2 && <p>{t("添加另一台设备后可查看设备间连接。", "Add another device to see connections between devices.")}</p>}
    {nodes.length > 1 && <>
      <div className="cx-path-matrix"><table>
        <caption className="cx-sr">{title}</caption>
        <thead><tr><th scope="col">{t("从 ↓ 到 →", "From ↓ To →")}</th>{nodes.map(node => <th scope="col" key={node.id}>{node.name}</th>)}</tr></thead>
        <tbody>{nodes.map(from => <tr key={from.id}><th scope="row">{from.name}</th>{nodes.map(to => {
          if (from.id === to.id) return <td key={to.id} />;
          const cell = describe(from, to);
          return <td key={to.id} title={cell.title}>{cell.content}</td>;
        })}</tr>)}</tbody>
      </table></div>
      <div className="cx-path-list">{nodes.map(from => <section key={from.id} aria-label={t("从 ", "From ") + from.name}>
        <h3>{t("从 ", "From ") + from.name}</h3><ul>{nodes.filter(to => to.id !== from.id).map(to => {
          const cell = describe(from, to);
          return <li key={to.id}><div className="cx-between"><strong>→ {to.name}</strong>{cell.content}</div><small className="cx-muted">{cell.details}</small></li>;
        })}</ul>
      </section>)}</div>
    </>}
    <div className="cx-path-legend cx-small" aria-label={t("连接图例", "Connection legend")}>
      <div>{Object.entries(labels).map(([scope, label]) => <span key={scope}>{glyphs[scope as keyof typeof glyphs]} {label}</span>)}</div>
      <div>{[["fast", "<30 ms"], ["good", "30–99 ms"], ["slow", "100–249 ms"], ["high", "≥250 ms"], ["unknown", t("延迟未知", "Unknown latency")]].map(([key, label]) => <span key={key} className={"cx-path-value cx-latency-" + key}>{label}</span>)}</div>
    </div>
  </section>;
}
