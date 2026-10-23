import {
  Bell,
  Check,
  Globe,
  Laptop,
  LoaderCircle,
  Monitor,
  MousePointerClick,
  Server,
  Smartphone,
  SquareTerminal,
  X,
  type LucideIcon,
} from "lucide-react";
import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { NavLink } from "react-router-dom";
import type { Approval, Device, Invite, Pill as StatusData } from "../personal/api";

export type Translate = (zh: string, en: string) => string;

export function Card({ children, className = "", label }: { children: ReactNode; className?: string; label?: string }) {
  return <section className={"cx-card " + className} aria-label={label}>{children}</section>;
}
export type Tone = "" | "good" | "warn" | "bad" | "info" | "accent";
export function Pill({ children, tone = "", icon: Icon }: { children: ReactNode; tone?: Tone | string; icon?: LucideIcon }) {
  return <span className={"cx-pill " + tone}>{Icon && <Icon aria-hidden="true" />}{children}</span>;
}
export function PageHead({ title, description, actions }: { title: string; description?: ReactNode; actions?: ReactNode }) {
  return <div className="cx-page-head"><div><h1>{title}</h1>{description && <p>{description}</p>}</div>{actions && <div className="cx-actions">{actions}</div>}</div>;
}
export function Section({ title, description, actions, children, label }: { title: string; description?: ReactNode; actions?: ReactNode; children: ReactNode; label?: string }) {
  return <section className="cx-section" aria-label={label || title}><div className="cx-section-head"><div><h2>{title}</h2>{description && <p>{description}</p>}</div>{actions && <div className="cx-actions">{actions}</div>}</div>{children}</section>;
}
export type TabItem = { to: string; label: string; count?: number; end?: boolean };
export function Tabs({ items, label }: { items: TabItem[]; label: string }) {
  return <nav className="cx-tabs" aria-label={label}>{items.map(item => <NavLink key={item.to} to={item.to} end={item.end ?? true} className="cx-tab">{item.label}{!!item.count && <span className="cx-count">{item.count}</span>}</NavLink>)}</nav>;
}
export function SubTabs({ items, label }: { items: TabItem[]; label: string }) {
  return <nav className="cx-subtabs" aria-label={label}>{items.map(item => <NavLink key={item.to} to={item.to} end={item.end ?? true}>{item.label}</NavLink>)}</nav>;
}
export function Empty({ icon: Icon, title, children, actions }: { icon?: LucideIcon; title: string; children?: ReactNode; actions?: ReactNode }) {
  return <div className="cx-empty">{Icon && <Icon size={34} aria-hidden="true" />}<h2>{title}</h2>{children && <p>{children}</p>}{actions && <div className="cx-actions">{actions}</div>}</div>;
}
export function Loading({ label }: { label: string }) {
  return <div className="cx-loading" role="status"><LoaderCircle className="cx-spin" aria-hidden="true" /><p>{label}</p></div>;
}
export function Alert({ children, tone = "", action }: { children: ReactNode; tone?: "" | "warn" | "info"; action?: ReactNode }) {
  return <div className={"cx-alert " + tone} role={tone === "info" ? "status" : "alert"}><span>{children}</span>{action}</div>;
}
export function Switch({ label, detail, checked, disabled, onChange }: { label: string; detail?: ReactNode; checked: boolean; disabled?: boolean; onChange: (value: boolean) => void }) {
  return <label className="cx-toggle-row cx-switch"><span>{label}{detail && <small>{detail}</small>}</span><input type="checkbox" role="switch" checked={checked} disabled={disabled} onChange={event => onChange(event.target.checked)} /><span className="cx-switch-track" aria-hidden="true" /></label>;
}
export function Sheet({ title, children, onClose, t }: { title: string; children: ReactNode; onClose: () => void; t: Translate }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    const element = dialog.current, restore = document.activeElement as HTMLElement | null;
    element?.showModal();
    return () => { element?.close(); if (restore?.isConnected) restore.focus(); };
  }, []);
  return <dialog aria-labelledby={titleId} className="cx-sheet" ref={dialog}
    onCancel={event => { event.preventDefault(); close.current(); }}
    onClick={event => { if (event.target === event.currentTarget) close.current(); }}>
    <div className="cx-sheet-head"><h2 id={titleId}>{title}</h2><button className="cx-icon-btn" aria-label={t("关闭", "Close")} onClick={() => close.current()}><X size={18} aria-hidden="true" /></button></div>
    <div className="cx-sheet-body">{children}</div>
  </dialog>;
}

export const capabilityIcons: Record<string, LucideIcon> = { terminal: SquareTerminal, terminals: SquareTerminal, desktop: Monitor, computer: MousePointerClick, browser: Globe };
/** Device services answer in English; show the known messages in the interface language. */
const messages: [RegExp, string][] = [
  [/^Turn on Screen Sharing in System Settings → General → Sharing\.?$/, "在「系统设置 → 通用 → 共享」中打开「屏幕共享」。"],
  [/^macOS Screen Sharing on (.+)$/, "macOS 屏幕共享（$1）"],
  [/^Desktop server on (.+)$/, "桌面服务（$1）"],
  [/^Grant Screen Recording and Accessibility to Cranibox in System Settings, then check again\.?$/, "请在系统设置中为 Cranibox 打开「屏幕录制」和「辅助功能」，然后重新检测。"],
  [/^Grant Screen Recording and Accessibility to CuaDriver in System Settings, then reopen CuaDriver\.?$/, "请在系统设置中为 CuaDriver 打开「屏幕录制」和「辅助功能」，然后重新打开 CuaDriver。"],
  [/^Installing cua-driver…$/, "正在安装 cua-driver…"],
  [/^The Computer Use bridge is not answering\.?$/, "Computer Use 桥接没有响应。"],
  [/^Install Google Chrome, Chromium, Microsoft Edge or Brave\.?(.*)$/, "请先安装 Google Chrome、Chromium、Microsoft Edge 或 Brave。$1"],
  [/^Update the browser to Chrome (\d+) or later, then retry\.?$/, "请把浏览器更新到 Chrome $1 或更高版本后重试。"],
  [/^In your browser, allow remote debugging at (\S+), keep the browser open, then retry\.?$/, "请在浏览器中打开 $1 并允许远程调试，保持浏览器开启后重试。"],
  [/^Open Chrome and allow remote debugging at (\S+); Chrome asks you to approve each new controller\.?$/, "请打开 Chrome，在 $1 允许远程调试；每次有新的控制方连接，Chrome 都会请你确认。"],
  [/^This action does not apply in the current state; refresh and retry\.?$/, "当前状态下不能执行此操作，请刷新后重试。"],
  [/^Open your system settings to change this\.?$/, "请在系统设置中修改。"],
  [/^cua-driver is not answering; start it in your desktop session, then retry\.?$/, "cua-driver 没有响应，请在桌面会话中启动它后重试。"],
  [/^The installation plan changed, expired or was already used\. Review a new plan\.?$/, "安装方案已变更、过期或已使用，请重新查看方案。"],
  [/^The cua-driver download does not match its published checksum\.?$/, "下载的 cua-driver 与官方校验值不一致，已停止安装。"],
  [/^Browser sharing was changed elsewhere; the managed browser stopped\.?$/, "浏览器共享在别处被修改，托管浏览器已停止。"],
  [/^The managed browser exited; restarting it\.?$/, "托管浏览器已退出，正在重新启动。"],
];
export function localizeMessage(text: string | null | undefined, t: Translate) {
  if (!text || t("zh", "en") === "en") return text || "";
  for (const [pattern, chinese] of messages) if (pattern.test(text)) return text.replace(pattern, chinese);
  return text;
}
/** What provides a capability, in words rather than internal identifiers. */
export function providerName(kind: string, t: Translate) {
  return ({
    pty: t("内置终端", "Built-in terminal"), screen_sharing: t("macOS 屏幕共享", "macOS Screen Sharing"), x11vnc: "x11vnc", wayvnc: "wayvnc", vnc: "VNC", selkies: "Selkies",
    cua: "cua-driver", "cua-embedded": t("内置 Computer Use（cua-driver）", "Built-in Computer Use (cua-driver)"), "cua-driver": "cua-driver", http: t("桌面桥接", "Desktop bridge"),
    chrome: "Google Chrome", chromium: "Chromium", edge: "Microsoft Edge", brave: "Brave", profile: t("你的 Chrome", "Your Chrome"),
  } as Record<string, string>)[kind] || kind;
}
export function capabilityName(id: string, t: Translate) {
  return ({
    terminal: t("终端", "Terminal"), terminals: t("终端", "Terminal"), desktop: t("远程桌面", "Remote desktop"),
    computer: "Computer Use", browser: t("浏览器", "Browser"), exec: t("执行命令", "Run commands"),
    projects: t("项目", "Projects"), messages: t("消息", "Messages"), activity: t("活动记录", "Activity"),
    spaces: t("虚拟空间", "Virtual spaces"), llm: t("模型调用", "Model access"), vault: t("凭据同步", "Credential sync"),
  } as Record<string, string>)[id] || id;
}
export function DeviceIcon({ kind, size = 22 }: { kind: string; size?: number }) {
  const Icon = kind === "server" ? Server : kind === "phone" || kind === "mobile" ? Smartphone : kind === "desktop" ? Monitor : Laptop;
  return <Icon size={size} aria-hidden="true" />;
}
export function isThis(device: Device) { return device.badges.includes("this"); }
/** One sentence about how this browser reaches the device. Never names a central server. */
export function pathLabel(device: Device, t: Translate) {
  if (isThis(device)) return t("本机", "This device");
  if (!device.online) return t("离线", "Offline");
  const path = device.path === "direct" ? t("直连", "Direct") : device.path === "relay" ? t("中继", "Relayed") : t("已连接", "Connected");
  return path + (device.rtt_ms !== null && device.rtt_ms !== undefined ? " · " + device.rtt_ms + " ms" : "");
}
export function StatusButton({ status, t, onClick }: { status: StatusData | null; t: Translate; onClick: () => void }) {
  const labels: Record<StatusData["state"], string> = {
    connected: t("已连接", "Connected"),
    local: t("仅本机可用", "This device only"),
    attention: t("需要你确认", "Needs your attention"),
    no_models: t("添加模型即可开始", "Add a model to begin"),
    offline: t("暂时离线", "Offline"),
  };
  return <button className={"cx-status " + (status?.state || "offline")} onClick={onClick}><span className="cx-dot" aria-hidden="true" />{status ? labels[status.state] : t("正在读取状态…", "Checking status…")}</button>;
}
export function QrPanel({ invite, t }: { invite: Invite; t: Translate }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [copied, setCopied] = useState(false);
  // SVG is decoded as an image: scripts and foreignObject never enter the page DOM.
  useEffect(() => {
    let alive = true;
    const image = new Image();
    image.onload = () => {
      if (!alive) return;
      const context = canvas.current?.getContext("2d");
      context?.clearRect(0, 0, 240, 240);
      context?.drawImage(image, 0, 0, 240, 240);
    };
    image.src = "data:image/svg+xml;charset=utf-8," + encodeURIComponent(invite.qr_svg);
    return () => { alive = false; };
  }, [invite.qr_svg]);
  return <div className="cx-qr">
    <canvas ref={canvas} width={240} height={240} role="img" aria-label={t("设备配对二维码", "Device pairing QR code")} />
    <label className="cx-field">{t("邀请链接", "Invitation link")}<input readOnly value={invite.link} onFocus={event => event.target.select()} /></label>
    <button className="cx-btn full" onClick={async () => { try { await navigator.clipboard.writeText(invite.link); setCopied(true); } catch { setCopied(false); } }}>{copied ? t("已复制", "Copied") : t("复制链接", "Copy link")}</button>
    <small className="cx-muted">{t("15 分钟内有效，仅可使用一次。", "Valid for 15 minutes. Can be used once.")}</small>
  </div>;
}
export function approvalTitle(card: Approval, t: Translate) {
  if (card.title_key === "approval.join_identity") return t("邀请这台设备加入它的身份", "Invites this device to join its identity");
  return ({
    pairing: t("核对新设备", "Verify a new device"),
    message: t("一条设备消息等待确认", "A device message needs your approval"),
    capability: t("设备请求新的权限", "A device requests access"),
    agent_tool: t("Agent 请求使用工具", "Agent requests a tool"),
    computer_use: t("允许操作你的屏幕？", "Allow access to your screen?"),
    model_write: t("确认修改模型配置", "Confirm model settings"),
  } as Record<string, string>)[card.kind] || t("需要你确认", "Needs your attention");
}
export function ApprovalCard({ card, t, busy, deviceName, onDecide }: { card: Approval; t: Translate; busy: boolean; deviceName: (node: string) => string; onDecide: (action: string) => void }) {
  const expired = !!card.expires_at && card.expires_at * 1000 <= Date.now();
  const source = card.from_node ? deviceName(card.from_node) : card.agent || deviceName(card.on_node);
  return <Card className="cx-approval pa-approval">
    <div className="cx-card-head">
      <span className="cx-card-icon"><Bell size={19} aria-hidden="true" /></span>
      <div className="cx-grow"><h3>{approvalTitle(card, t)}</h3><small className="cx-muted">{source}</small></div>
      <Pill tone={card.risk === "high" ? "warn" : ""}>{card.risk === "high" ? t("高风险", "High risk") : t("待确认", "Pending")}</Pill>
    </div>
    {Object.entries(card.title_args).map(([key, value]) => <p className="cx-untrusted" key={key}>{key === "command" ? <code>{value}</code> : key === "code" ? <strong>{value}</strong> : value}</p>)}
    {card.detail && <p className="cx-untrusted">{card.detail}</p>}
    <div className="cx-between">
      <small className="cx-muted">{expired ? t("已过期，默认拒绝", "Expired. Denied by default.") : t("仅在你同意后继续。", "Continues only with your permission.")}</small>
      <div className="cx-actions">{card.actions.map(action => <button key={action} disabled={busy || expired} className={"cx-btn " + (action === "deny" ? "" : "primary")} onClick={() => onDecide(action)}>
        {action === "deny" ? <X size={16} aria-hidden="true" /> : <Check size={16} aria-hidden="true" />}
        {action === "deny" ? t("拒绝", "Deny") : action === "allow_always" ? t("始终允许", "Always allow") : card.kind === "pairing" ? t("两边一致", "Codes match") : t("允许一次", "Allow once")}
      </button>)}</div>
    </div>
  </Card>;
}
