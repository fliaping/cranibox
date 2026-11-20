import { ArrowRight, ChevronRight, Network, Route, Workflow, X } from "lucide-react";
import { useEffect, useMemo, useState, type KeyboardEvent } from "react";
import { Link } from "react-router-dom";
import type { Device, Relation } from "../personal/api";
import { devicePath } from "./Home";
import { Direction, relationIcons, relationLabel, relationOf, relationSentence, relationTones } from "./relations";
import { store, stored, useApp } from "./store";
import { capabilityIcons, capabilityName, DeviceIcon, formatDeviceLabel, isThis, pathLabel, type Translate } from "./ui";

/** `GET /network/paths`: devices, measured paths (observer → target) and who may use whom. */
export type GraphData = {
  nodes: { id: string; name: string; relation?: Relation }[];
  paths: { from: string; to: string; kind: string; scope?: string | null; via?: string | null; rtt_ms: number | null; at: number }[];
  uses?: { from: string; to: string; kind: UseKind }[];
};
type UseKind = "control" | "guest" | "shared";
type Point = { x: number; y: number };
type GraphNode = Point & { id: string; name: string; relation: Relation; device?: Device; local: boolean; ring: boolean; sub: string };
type Edge = { from: string; to: string; kind: UseKind };
type Link2 = { a: string; b: string; kind: string; rtt: number | null; via: string | null };

const W = 960, H = 520, CENTER = { x: 480, y: 262 }, R = 30, BOX = { w: 184, h: 56 };
const LEFT = 128, RIGHT = 832, TOP = 92, BOTTOM = 432, PER_COLUMN = 6;
const FRESH = 15 * 60_000;

/** Polls `/network/paths` while the page is visible. */
export function useGraphData() {
  const { api } = useApp();
  const [data, setData] = useState<GraphData | null>(null);
  useEffect(() => {
    let active = true, pending = false;
    async function load() {
      if (pending || document.hidden) return;
      pending = true;
      try { const value = await api.call<GraphData>("GET", "/network/paths"); if (active) setData(value); } catch { /* The list below still works. */ } finally { pending = false; }
    }
    void load();
    const timer = setInterval(() => void load(), 15_000);
    return () => { active = false; clearInterval(timer); };
  }, [api]);
  return data;
}

function short(text: string, max: number) { return text.length > max ? text.slice(0, max - 1) + "…" : text; }
function spread(count: number, top = TOP, bottom = BOTTOM) {
  if (count <= 1) return [(top + bottom) / 2];
  const step = Math.min(84, (bottom - top) / (count - 1)), start = (top + bottom) / 2 - step * (count - 1) / 2;
  return Array.from({ length: count }, (_, i) => start + i * step);
}

/** Places devices like the design: your devices on an ellipse inside the identity zone,
 * the devices you control on the right, guests, controllers and shared devices on the left. */
function layout(devices: Device[], data: GraphData | null, t: Translate) {
  const known = new Map(devices.map(device => [device.node_id, device]));
  const local = devices.find(isThis)?.node_id || "";
  const relations = new Map<string, { name: string; relation: Relation }>();
  for (const node of data?.nodes || []) relations.set(node.id, { name: node.name, relation: node.relation || "peer" });
  for (const device of devices) relations.set(device.node_id, { name: device.name, relation: isThis(device) ? "peer" : relationOf(device) });
  // Edges: what the server reported, or (older nodes) this device's own relations.
  const edges: Edge[] = [];
  const add = (edge: Edge) => { if (edge.from !== edge.to && !edges.some(e => e.from === edge.from && e.to === edge.to && e.kind === edge.kind)) edges.push(edge); };
  for (const edge of data?.uses || []) if (relations.has(edge.from) && relations.has(edge.to)) add(edge);
  if (local) for (const device of devices) {
    const relation = relationOf(device), id = device.node_id;
    if (relation === "controlled") add({ from: local, to: id, kind: "control" });
    if (relation === "controller") add({ from: id, to: local, kind: "control" });
    if (relation === "guest") add({ from: id, to: local, kind: "guest" });
    if (relation === "host") add({ from: local, to: id, kind: "shared" });
  }
  const label = (id: string) => { const name = relations.get(id)?.name || id; return name === id ? id.slice(0, 13) : formatDeviceLabel(name, id, t).label; };
  const ids = [...relations.keys()];
  const order = (a: string, b: string) => Number(b === local) - Number(a === local) || Number(known.get(b)?.online ?? 0) - Number(known.get(a)?.online ?? 0) || label(a).localeCompare(label(b));
  const peers = ids.filter(id => relations.get(id)!.relation === "peer").sort(order).slice(0, 8);
  const of = (relation: Relation) => ids.filter(id => relations.get(id)!.relation === relation).sort(order);
  const right = of("controlled"), left = [...of("guest"), ...of("controller"), ...of("host")];
  const hidden = Math.max(0, right.length - PER_COLUMN) + Math.max(0, left.length - PER_COLUMN);
  const nodes = new Map<string, GraphNode>();
  const rx = peers.length <= 1 ? 0 : Math.min(158, 64 + peers.length * 24), ry = rx * 0.68;
  const named = (ids: string[], join: string) => ids.map(label).join(join);
  const by = (id: string, kind: UseKind, outgoing: boolean) => edges.filter(e => e.kind === kind && (outgoing ? e.from === id : e.to === id)).map(e => outgoing ? e.to : e.from);
  const sub = (id: string, relation: Relation) => {
    const device = known.get(id), sep = t("、", ", ");
    if (relation === "peer") return id === local ? t("本机", "This device") : device && !device.online ? t("离线", "Offline") : device?.badges.includes("hub") ? t("常驻节点", "Always on") : "";
    if (relation === "controlled") { const who = by(id, "control", false); return who.length ? t("由 " + named(who, sep) + " 控制", "Controlled by " + named(who, sep)) : relationLabel(relation, t); }
    if (relation === "controller") { const whom = by(id, "control", true); return whom.length ? t("控制 " + named(whom, sep), "Controls " + named(whom, sep)) : relationLabel(relation, t); }
    if (relation === "guest") { const whom = by(id, "guest", true); return whom.length ? t("可使用 " + named(whom, sep), "Uses " + named(whom, sep)) : relationLabel(relation, t); }
    const who = by(id, "shared", false); return who.length ? t("共享给 " + named(who, sep), "Shared with " + named(who, sep)) : relationLabel(relation, t);
  };
  const place = (id: string, point: Point) => {
    const relation = relations.get(id)!.relation, device = known.get(id);
    nodes.set(id, { id, ...point, name: label(id), relation, device, local: id === local, ring: relation === "peer", sub: sub(id, relation) });
  };
  peers.forEach((id, i) => {
    const angle = (-150 + i * 360 / peers.length) * Math.PI / 180;
    place(id, { x: CENTER.x + rx * Math.cos(angle), y: CENTER.y + ry * Math.sin(angle) });
  });
  const ys = spread(Math.min(right.length, PER_COLUMN));
  right.slice(0, PER_COLUMN).forEach((id, i) => place(id, { x: RIGHT, y: ys[i] }));
  const ls = spread(Math.min(left.length, PER_COLUMN));
  left.slice(0, PER_COLUMN).forEach((id, i) => place(id, { x: LEFT, y: ls[i] }));
  // Measured paths between your devices, one line per pair; this device's own view wins.
  const links: Link2[] = [];
  const now = Date.now();
  for (const path of [...(data?.paths || [])].sort((a, b) => Number(b.from === local) - Number(a.from === local))) {
    if (!nodes.get(path.from)?.ring || !nodes.get(path.to)?.ring || now - path.at > FRESH) continue;
    if (links.some(link => (link.a === path.from && link.b === path.to) || (link.a === path.to && link.b === path.from))) continue;
    links.push({ a: path.from, b: path.to, kind: path.kind, rtt: path.rtt_ms, via: path.via?.startsWith("device:") ? path.via.slice(7) : null });
  }
  return { nodes, edges: edges.filter(e => nodes.has(e.from) && nodes.has(e.to)), links, zone: { rx: rx + 78, ry: ry + 66 }, hidden, local, label };
}

/** The point where a line from `node` towards `toward` leaves the node's shape. */
function anchor(node: GraphNode, toward: Point): Point {
  if (node.ring) {
    const dx = toward.x - node.x, dy = toward.y - node.y, d = Math.hypot(dx, dy) || 1;
    return { x: node.x + dx / d * (R + 7), y: node.y + dy / d * (R + 7) };
  }
  return { x: node.x + (toward.x > node.x ? BOX.w / 2 + 4 : -BOX.w / 2 - 4), y: node.y };
}
function bezier(a: Point, c1: Point, c2: Point, b: Point, t: number): Point {
  const u = 1 - t;
  return { x: u * u * u * a.x + 3 * u * u * t * c1.x + 3 * u * t * t * c2.x + t * t * t * b.x, y: u * u * u * a.y + 3 * u * u * t * c1.y + 3 * u * t * t * c2.y + t * t * t * b.y };
}
/** Whether `p` is inside a node's shape or its name, with some breathing room. */
function hits(node: GraphNode, p: Point) {
  return node.ring ? Math.abs(p.x - node.x) < 64 && p.y - node.y > -R - 14 && p.y - node.y < R + 44 : Math.abs(p.x - node.x) < BOX.w / 2 + 8 && Math.abs(p.y - node.y) < BOX.h / 2 + 8;
}
/** An S-curve from `from` to `to` that bends around every other device in the way. */
function curve(from: GraphNode, to: GraphNode, others: GraphNode[]) {
  const target = to.ring ? to : { x: to.x + (from.x > to.x ? BOX.w / 2 : -BOX.w / 2), y: to.y };
  const origin = from.ring ? from : { x: from.x + (to.x > from.x ? BOX.w / 2 : -BOX.w / 2), y: from.y };
  const a = anchor(from, target), b = anchor(to, origin);
  const bend = (b.x - a.x) * 0.5;
  const obstacles = others.filter(node => node.id !== from.id && node.id !== to.id);
  let best = { lift: 0, blocked: Infinity };
  for (const lift of [0, -70, 70, -110, 110, -150, 150]) {
    const c1 = { x: a.x + bend, y: a.y + lift }, c2 = { x: b.x - bend, y: b.y + lift };
    let blocked = 0;
    for (let i = 1; i < 32; i++) { const p = bezier(a, c1, c2, b, i / 32); blocked += obstacles.filter(node => hits(node, p)).length; }
    if (blocked < best.blocked) best = { lift, blocked };
    if (!blocked) break;
  }
  const c1 = { x: a.x + bend, y: a.y + best.lift }, c2 = { x: b.x - bend, y: b.y + best.lift };
  const f = (n: number) => n.toFixed(1);
  return { d: `M${f(a.x)},${f(a.y)} C${f(c1.x)},${f(c1.y)} ${f(c2.x)},${f(c2.y)} ${f(b.x)},${f(b.y)}`, mid: bezier(a, c1, c2, b, 0.5) };
}

const useLabels = (t: Translate): Record<UseKind, string> => ({ control: t("控制", "Controls"), guest: t("访客使用", "Guest use"), shared: t("使用", "Uses") });

export function DeviceGraph() {
  const { t, devices, deviceName } = useApp();
  const data = useGraphData();
  const graph = useMemo(() => layout(devices, data, t), [devices, data, t]);
  const [hover, setHover] = useState(""), [selected, setSelected] = useState("");
  const [showUses, setShowUses] = useState(() => stored("cranibox.graph.uses", "1") === "1");
  const [showPaths, setShowPaths] = useState(() => stored("cranibox.graph.paths", "1") === "1");
  const labels = useLabels(t);
  const focus = hover || selected;
  const near = useMemo(() => {
    const set = new Set<string>();
    if (!focus) return set;
    set.add(focus);
    for (const edge of graph.edges) if (edge.from === focus || edge.to === focus) { set.add(edge.from); set.add(edge.to); }
    for (const link of graph.links) if (link.a === focus || link.b === focus) { set.add(link.a); set.add(link.b); }
    return set;
  }, [focus, graph]);
  const nodes = [...graph.nodes.values()];
  if (nodes.length < 2) return null;
  const key = (event: KeyboardEvent, id: string) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); setSelected(selected === id ? "" : id); } if (event.key === "Escape") setSelected(""); };
  const counts = (["peer", "controlled", "guest", "host", "controller"] as Relation[]).map(relation => [relation, nodes.filter(node => node.relation === relation).length] as const).filter(([, n]) => n);
  const pick = graph.nodes.get(selected);
  const top = Math.max(0, Math.min(CENTER.y - graph.zone.ry, ...nodes.map(node => node.y - (node.ring ? R + 20 : BOX.h / 2))) - 24);
  const bottom = Math.min(H, Math.max(CENTER.y + graph.zone.ry, ...nodes.map(node => node.y + (node.ring ? R + 44 : BOX.h / 2))) + 24);
  const title = t("设备网络", "Device network");
  return <section className="cx-graph-card" aria-label={title}>
    <header className="cx-graph-head">
      <div><h2><Workflow aria-hidden="true" />{title}</h2><p>{t("箭头 A → B 表示 A 可以使用 B：B 授权给 A，B 决定 A 能做什么。", "An arrow A → B means A can use B: B granted it, and B decides what A may do.")}</p></div>
      <div className="cx-graph-toggles" role="group" aria-label={t("显示", "Show")}>
        <button type="button" aria-pressed={showUses} onClick={() => { setShowUses(!showUses); store("cranibox.graph.uses", showUses ? "0" : "1"); }}><ArrowRight aria-hidden="true" />{t("使用关系", "Who uses whom")}</button>
        <button type="button" aria-pressed={showPaths} onClick={() => { setShowPaths(!showPaths); store("cranibox.graph.paths", showPaths ? "0" : "1"); }}><Route aria-hidden="true" />{t("连接路径", "Connections")}</button>
      </div>
    </header>
    <div className="cx-graph-body">
      <div className="cx-graph-stage">
        <svg className={"cx-graph" + (focus ? " has-focus" : "")} viewBox={`0 ${top} ${W} ${bottom - top}`} role="group" aria-label={title} onClick={event => { if (event.target === event.currentTarget) setSelected(""); }}>
          <defs>
            <radialGradient id="cxg-zone" cx="50%" cy="45%" r="60%"><stop offset="0%" className="cxg-zone-in" /><stop offset="100%" className="cxg-zone-out" /></radialGradient>
            <linearGradient id="cxg-ring" x1="0" y1="0" x2="1" y2="1"><stop offset="0%" className="cxg-ring-a" /><stop offset="100%" className="cxg-ring-b" /></linearGradient>
            <radialGradient id="cxg-node" cx="50%" cy="35%" r="70%"><stop offset="0%" className="cxg-node-in" /><stop offset="100%" className="cxg-node-out" /></radialGradient>
            <filter id="cxg-soft" x="-30%" y="-30%" width="160%" height="160%"><feGaussianBlur stdDeviation="6" /></filter>
            <pattern id="cxg-dots" width="22" height="22" patternUnits="userSpaceOnUse"><circle cx="1" cy="1" r="1" className="cxg-dot" /></pattern>
            {(["control", "guest", "shared"] as UseKind[]).map(kind => <marker key={kind} id={"cxg-arrow-" + kind} viewBox="0 0 10 10" refX="7" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M1,1.5 L8,5 L1,8.5 Q2.6,5 1,1.5 Z" className={"cxg-arrow " + kind} /></marker>)}
          </defs>
          <rect y={top} width={W} height={bottom - top} fill="url(#cxg-dots)" className="cxg-backdrop" />
          <g className="cxg-zone">
            <ellipse cx={CENTER.x} cy={CENTER.y} rx={graph.zone.rx} ry={graph.zone.ry} fill="url(#cxg-zone)" />
            <ellipse cx={CENTER.x} cy={CENTER.y} rx={graph.zone.rx} ry={graph.zone.ry} className="cxg-zone-line" />
            <text x={CENTER.x} y={CENTER.y - graph.zone.ry + 24} textAnchor="middle" className="cxg-zone-label">{t("我的身份 · 对等设备", "My identity · peer devices")}</text>
          </g>
          {showPaths && graph.links.map(link => {
            const a = graph.nodes.get(link.a)!, b = graph.nodes.get(link.b)!;
            const pa = anchor(a, b), pb = anchor(b, a), mid = { x: (pa.x + pb.x) / 2, y: (pa.y + pb.y) / 2 };
            const through = link.via ? deviceName(link.via) : "";
            const text = (link.via ? t("经 " + short(through, 10), "via " + short(through, 10)) : link.kind === "direct" ? t("直连", "Direct") : t("中继", "Relay")) + (link.rtt != null ? " " + link.rtt + " ms" : "");
            const lit = near.has(link.a) && near.has(link.b) && (focus === link.a || focus === link.b);
            const width = text.length * 6.4 + 16;
            return <g key={link.a + link.b} className={"cxg-link " + (link.via ? "hop" : link.kind) + (lit ? " lit" : "")}>
              <line x1={pa.x} y1={pa.y} x2={pb.x} y2={pb.y} />
              <g className="cxg-chip" transform={`translate(${mid.x},${mid.y})`}><rect x={-width / 2} y={-10} width={width} height={20} rx={10} /><text textAnchor="middle" y={4}>{text}</text></g>
            </g>;
          })}
          {showUses && graph.edges.map(edge => {
            const from = graph.nodes.get(edge.from)!, to = graph.nodes.get(edge.to)!;
            const { d, mid } = curve(from, to, nodes);
            const lit = focus === edge.from || focus === edge.to;
            return <g key={edge.from + edge.to + edge.kind} className={"cxg-edge " + edge.kind + (lit ? " lit" : "")}>
              <path d={d} className="cxg-edge-glow" />
              <path d={d} className="cxg-edge-line" markerEnd={`url(#cxg-arrow-${edge.kind})`} />
              <path d={d} className="cxg-edge-flow" />
              <text x={mid.x} y={mid.y - 9} textAnchor="middle" className="cxg-edge-label">{labels[edge.kind]}</text>
            </g>;
          })}
          {nodes.map(node => {
            const Icon = relationIcons[node.relation];
            const online = node.local || node.device?.online !== false;
            const cls = "cxg-node " + relationTones[node.relation] + (node.ring ? " ring" : " box") + (node.local ? " local" : "") + (online ? "" : " offline") + (near.has(node.id) ? " lit" : "") + (selected === node.id ? " selected" : "");
            const label = node.name + " · " + (node.local ? t("本机", "This device") : relationLabel(node.relation, t)) + (node.sub && !node.local ? " · " + node.sub : "");
            const events = { onMouseEnter: () => setHover(node.id), onMouseLeave: () => setHover(""), onFocus: () => setHover(node.id), onBlur: () => setHover(""), onClick: () => setSelected(selected === node.id ? "" : node.id), onKeyDown: (event: KeyboardEvent) => key(event, node.id) };
            if (node.ring) return <g key={node.id} className={cls} transform={`translate(${node.x},${node.y})`} tabIndex={0} role="button" aria-pressed={selected === node.id} aria-label={label} {...events}>
              {node.local && <circle r={R + 12} className="cxg-pulse" />}
              <circle r={R + 6} className="cxg-halo" filter="url(#cxg-soft)" />
              <circle r={R} className="cxg-disc" />
              <circle r={R} className="cxg-ring" />
              <g transform="translate(-11,-11)" className="cxg-icon"><DeviceIcon kind={node.device?.kind || "laptop"} size={22} /></g>
              <text y={R + 20} textAnchor="middle" className="cxg-name">{short(node.name, 18)}</text>
              {node.sub && <text y={R + 35} textAnchor="middle" className="cxg-sub">{node.sub}</text>}
            </g>;
            return <g key={node.id} className={cls} transform={`translate(${node.x - BOX.w / 2},${node.y - BOX.h / 2})`} tabIndex={0} role="button" aria-pressed={selected === node.id} aria-label={label} {...events}>
              <rect width={BOX.w} height={BOX.h} rx={16} className="cxg-card-shadow" filter="url(#cxg-soft)" />
              <rect width={BOX.w} height={BOX.h} rx={16} className="cxg-card" />
              <circle cx={28} cy={BOX.h / 2} r={16} className="cxg-badge" />
              <g transform={`translate(19,${BOX.h / 2 - 9})`} className="cxg-badge-icon"><Icon size={18} /></g>
              <text x={52} y={24} className="cxg-name">{short(node.name, 15)}</text>
              <text x={52} y={41} className="cxg-sub">{short(node.sub, 20)}</text>
              {!online && <circle cx={BOX.w - 14} cy={14} r={4} className="cxg-off" />}
            </g>;
          })}
        </svg>
        {graph.hidden > 0 && <p className="cx-graph-more">{t("另有 " + graph.hidden + " 台设备在下方列表中", graph.hidden + " more in the list below")}</p>}
      </div>
      <aside className="cx-graph-panel" aria-live="polite">
        {pick ? <NodeDetail node={pick} graph={graph} onClose={() => setSelected("")} /> : <>
          <h3>{t("一眼看清谁能用谁", "Who can use whom, at a glance")}</h3>
          <p className="cx-muted">{t("悬停或点选一台设备，查看它和谁有关、谁能使用它。", "Hover or select a device to see what it connects to and who can use it.")}</p>
          <ul className="cx-graph-counts">{counts.map(([relation, n]) => { const Icon = relationIcons[relation]; return <li key={relation} className={relationTones[relation]}><span className="cx-graph-swatch"><Icon aria-hidden="true" /></span><span className="cx-grow">{relationLabel(relation, t)}</span><strong>{n}</strong></li>; })}</ul>
        </>}
      </aside>
    </div>
    <footer className="cx-graph-legend" aria-label={t("图例", "Legend")}>
      <span className="peer">{t("对等设备（同一身份）", "Peers (one identity)")}</span>
      <span className="control">{t("控制：我的设备 → 受控设备", "Control: my device → controlled")}</span>
      <span className="guest">{t("访客：别人的设备 → 我的设备", "Guest: their device → mine")}</span>
      <span className="shared">{t("共享给我：我的设备 → 别人的设备", "Shared: my device → theirs")}</span>
      <span className="direct">{t("直连", "Direct")}</span><span className="relay">{t("中继", "Relay")}</span><span className="hop">{t("经另一台设备转发", "Through another device")}</span>
    </footer>
    <ul className="cx-graph-list" aria-label={t("使用关系列表", "Who uses whom")}>
      {graph.edges.length ? graph.edges.map(edge => <li key={edge.from + edge.to + edge.kind} className={edge.kind}>
        <Direction from={graph.label(edge.from)} to={graph.label(edge.to)} both={false} /><small>{labels[edge.kind]}</small>
      </li>) : <li className="peer"><Network aria-hidden="true" />{t("你的设备彼此对等，都可以互相使用。", "Your devices are peers: each can use the others.")}</li>}
    </ul>
  </section>;
}

function NodeDetail({ node, graph, onClose }: { node: GraphNode; graph: ReturnType<typeof layout>; onClose: () => void }) {
  const { t, deviceName } = useApp();
  const Icon = relationIcons[node.relation];
  const sep = t("、", ", ");
  const names = (ids: string[]) => ids.map(graph.label);
  const incoming = (kind: UseKind) => names(graph.edges.filter(e => e.kind === kind && e.to === node.id).map(e => e.from));
  const outgoing = (kind: UseKind) => names(graph.edges.filter(e => e.kind === kind && e.from === node.id).map(e => e.to));
  const others = node.relation === "controlled" ? incoming("control") : node.relation === "controller" ? outgoing("control") : node.relation === "guest" ? outgoing("guest") : node.relation === "host" ? incoming("shared") : [];
  const sentence = node.local ? t("这是你正在使用的设备。", "This is the device you are using.") : relationSentence(node.relation, node.name, t, others);
  const uses = graph.edges.filter(e => e.from === node.id).map(e => graph.label(e.to));
  const usedBy = graph.edges.filter(e => e.to === node.id).map(e => graph.label(e.from));
  const device = node.device;
  return <div className={"cx-graph-detail " + relationTones[node.relation]}>
    <div className="cx-graph-detail-head">
      <span className="cx-graph-swatch big"><Icon aria-hidden="true" /></span>
      <div className="cx-grow"><h3>{node.name}</h3><span className={"cx-pill " + relationTones[node.relation]}>{node.local ? t("本机", "This device") : relationLabel(node.relation, t)}</span></div>
      <button type="button" className="cx-icon-btn" aria-label={t("关闭", "Close")} onClick={onClose}><X size={16} aria-hidden="true" /></button>
    </div>
    <p className="cx-graph-sentence">{sentence}</p>
    <dl className="cx-facts">
      {node.relation !== "peer" && <div><dt>{t("它能使用", "It can use")}</dt><dd>{uses.length ? uses.join(sep) : t("无", "Nothing")}</dd></div>}
      {node.relation !== "peer" && <div><dt>{t("可以使用它的", "Can use it")}</dt><dd>{usedBy.length ? usedBy.join(sep) : t("无", "Nobody")}</dd></div>}
      {device && !node.local && <div><dt>{t("连接", "Connection")}</dt><dd>{pathLabel(device, t, deviceName)}</dd></div>}
      {device && node.relation !== "peer" && <div><dt>{t("模型订阅", "Model subscriptions")}</dt><dd>{t("不共享", "Not shared")}</dd></div>}
    </dl>
    {device && !node.local && <div className="cx-cap-badges">{(["terminals", "desktop", "browser"] as const).map(cap => { const CapIcon = capabilityIcons[cap], on = device.caps[cap] !== false; return <span key={cap} className={"cx-cap " + (on ? "on" : "off")}><CapIcon aria-hidden="true" />{capabilityName(cap, t)}</span>; })}</div>}
    {device ? <Link className="cx-btn small" to={devicePath(device)}>{t("打开设备", "Open device")}<ChevronRight size={14} aria-hidden="true" /></Link>
      : <p className="cx-muted cx-small">{t("这台设备由你的另一台设备管理，可在那台设备上调整。", "Another of your devices manages this one; change it there.")}</p>}
  </div>;
}
