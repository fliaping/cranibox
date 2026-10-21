import { CircleAlert, Download, ExternalLink, Play, RefreshCw, ShieldCheck, Square } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { notAvailable, type Capabilities as CapabilityList, type Capability, type CapabilityAction, type InstallPlan, type PermissionState } from "../personal/api";
import { useApp } from "./store";
import { Alert, capabilityIcons, Card, Empty, Loading, Pill, Sheet, Switch, type Translate } from "./ui";

const order = ["terminal", "desktop", "computer", "browser"];
function describe(id: string, t: Translate) {
  return ({
    terminal: [t("终端", "Terminal"), t("你的其他设备和 Agent 可以在这里打开持久终端会话。", "Your other devices and agents can open persistent terminal sessions here.")],
    desktop: [t("远程桌面", "Remote desktop"), t("通过 VNC 共享这块屏幕，在其他设备上查看和操作。", "Share this screen over VNC to view and control it from your other devices.")],
    computer: ["Computer Use", t("让 Agent 看到屏幕并使用鼠标和键盘，每次会话都需要你确认。", "Let agents see the screen and use the mouse and keyboard. Each session asks you first.")],
    browser: [t("浏览器", "Browser"), t("一个独立配置的浏览器，供 Agent 操作，不影响你日常使用的浏览器。", "A browser with its own profile for agents to drive, separate from your everyday browser.")],
  } as Record<string, string[]>)[id] || [id, ""];
}
function permissionName(id: string, t: Translate) {
  return ({ screen_recording: t("屏幕录制", "Screen recording"), accessibility: t("辅助功能", "Accessibility") } as Record<string, string>)[id] || id;
}
const permissionTone: Record<PermissionState, string> = { granted: "good", denied: "bad", unknown: "warn", not_required: "" };
function permissionState(state: PermissionState, t: Translate) {
  return { granted: t("已授权", "Granted"), denied: t("未授权", "Not granted"), unknown: t("待检测", "Unknown"), not_required: t("无需授权", "Not required") }[state] || state;
}
function platformName(platform: string) {
  return ({ macos: "macOS", darwin: "macOS", linux: "Linux", windows: "Windows" } as Record<string, string>)[platform.toLowerCase()] || platform;
}
export function Capabilities() {
  const { t, api } = useApp();
  const [data, setData] = useState<CapabilityList | null>(null), [state, setState] = useState<"loading" | "ready" | "unsupported" | "error">("loading");
  const [error, setError] = useState(""), [pending, setPending] = useState(""), [plan, setPlan] = useState<{ id: string; plan: InstallPlan } | null>(null);
  const load = useCallback(async () => {
    try { setData(await api.call<CapabilityList>("GET", "/capabilities")); setState("ready"); setError(""); }
    catch (problem) { if (notAvailable(problem)) setState("unsupported"); else { setError(problem instanceof Error ? problem.message : String(problem)); setState(current => current === "ready" ? current : "error"); } }
  }, [api]);
  useEffect(() => { void load(); }, [load]);
  async function act(item: Capability, action: CapabilityAction, body?: unknown) {
    setPending(item.id + ":" + action); setError("");
    try {
      const value = await api.call<Capability | InstallPlan>("POST", "/capabilities/" + encodeURIComponent(item.id) + "/" + action, body ?? {});
      if (action === "install_plan") setPlan({ id: item.id, plan: value as InstallPlan });
      else if (value && typeof value === "object" && "id" in value) setData(current => current && { ...current, items: current.items.map(entry => entry.id === item.id ? value as Capability : entry) });
      else await load();
      if (action === "install") setPlan(null);
    } catch (problem) { setError(problem instanceof Error ? problem.message : String(problem)); }
    finally { setPending(""); }
  }
  if (state === "loading") return <Loading label={t("正在检测本机能力…", "Checking this device's capabilities…")} />;
  if (state === "unsupported") return <Empty icon={ShieldCheck} title={t("这个节点还不支持能力管理", "Capability management is not available on this node yet")}
    actions={<><Link className="cx-btn" to="/desktop">{t("桌面共享设置", "Desktop sharing settings")}</Link><Link className="cx-btn" to="/nodes">{t("设备与网络", "Devices & network")}</Link></>}>
    {t("更新 Cranibox 后，可以在这里统一安装、授权和共享终端、桌面、Computer Use 与浏览器。在此之前，请使用以下页面。", "After updating Cranibox you can install, authorize and share terminal, desktop, Computer Use and browser here. Until then, use these pages.")}</Empty>;
  if (state === "error" && !data) return <Alert action={<button className="cx-btn small" onClick={() => void load()}>{t("重试", "Retry")}</button>}>{error}</Alert>;
  const items = [...(data?.items || [])].sort((a, b) => (order.indexOf(a.id) + 1 || 99) - (order.indexOf(b.id) + 1 || 99));
  return <>
    <div className="cx-between" style={{ marginBottom: 16 }}>
      <p className="cx-muted">{t("运行环境：", "Platform: ")}<strong>{platformName(data?.platform || "")}</strong>{t("。每项能力依次经过：提供者 → 系统权限 → 共享开关 → 对端授权（在设备详情的「授权」里）。", ". Each capability goes through provider → system permission → sharing → per-device access (on each device's Access tab).")}</p>
      <button className="cx-btn" onClick={() => void load()}><RefreshCw size={15} aria-hidden="true" />{t("重新检测", "Check again")}</button>
    </div>
    {error && <Alert>{error}</Alert>}
    <div className="cx-grid two">{items.map(item => <CapabilityCard key={item.id} item={item} pending={pending} act={act} />)}</div>
    {plan && <Sheet title={t("审阅安装方案", "Review the install plan")} t={t} onClose={() => setPlan(null)}>
      <p className="cx-muted">{t("以下命令会在本机执行一次。请确认来源可信后再继续。", "These commands run once on this device. Continue only if you trust their source.")}</p>
      <div className="cx-plan"><pre aria-label={t("安装命令", "Install commands")}>{plan.plan.commands.join("\n")}</pre>
        {plan.plan.url && <p className="cx-small">{t("来源：", "Source: ")}<span className="cx-mono">{plan.plan.url}</span></p>}
        {plan.plan.expires_at && <p className="cx-small cx-muted">{t("方案有效期至 ", "Plan valid until ")}{new Date(plan.plan.expires_at * 1000).toLocaleTimeString()}</p>}</div>
      <div className="cx-actions"><button className="cx-btn" onClick={() => setPlan(null)}>{t("取消", "Cancel")}</button>
        <button className="cx-btn primary" disabled={!!pending} onClick={() => { const item = data?.items.find(entry => entry.id === plan.id); if (item) void act(item, "install", { plan_id: plan.plan.plan_id }); }}><Download size={15} aria-hidden="true" />{t("执行安装", "Run install")}</button></div>
    </Sheet>}
  </>;
}
function CapabilityCard({ item, pending, act }: { item: Capability; pending: string; act: (item: Capability, action: CapabilityAction, body?: unknown) => Promise<void> }) {
  const { t } = useApp();
  const [title, detail] = describe(item.id, t), Icon = capabilityIcons[item.id] || ShieldCheck;
  const busy = pending.startsWith(item.id + ":"), has = (action: CapabilityAction) => item.actions.includes(action);
  const missing = item.permissions.filter(permission => permission.state === "denied" || permission.state === "unknown");
  const ready = item.provider.installed && item.provider.running && !missing.length;
  return <Card className="cx-cap-card" label={title}>
    <div className="cx-card-head"><span className={"cx-card-icon " + (ready ? "" : "muted")}><Icon size={20} aria-hidden="true" /></span>
      <div className="cx-grow"><h3>{title}</h3><p className="cx-muted cx-small">{detail}</p></div>
      <Pill tone={ready ? (item.sharing.enabled ? "good" : "accent") : "warn"}>{ready ? (item.sharing.enabled ? t("已共享", "Shared") : t("就绪", "Ready")) : !item.provider.installed ? t("未安装", "Not installed") : !item.provider.running ? t("未运行", "Not running") : t("需要授权", "Needs permission")}</Pill></div>
    <div className="cx-cap-section"><h4>{t("提供者", "Provider")}</h4>
      <div className="cx-between"><div><strong>{item.provider.kind}</strong>{item.provider.version && <span className="cx-muted cx-small"> · {item.provider.version}</span>}
        {item.provider.detail && <p className="cx-muted cx-small">{item.provider.detail}</p>}</div>
        <div className="cx-actions"><Pill tone={item.provider.installed ? "good" : ""}>{item.provider.installed ? t("已安装", "Installed") : t("未安装", "Not installed")}</Pill>{item.provider.installed && <Pill tone={item.provider.running ? "good" : ""}>{item.provider.running ? t("运行中", "Running") : t("已停止", "Stopped")}</Pill>}</div></div>
      <div className="cx-actions">
        {has("install_plan") && <button className="cx-btn small primary" disabled={busy} onClick={() => void act(item, "install_plan")}><Download size={14} aria-hidden="true" />{t("安装…", "Install…")}</button>}
        {has("start") && <button className="cx-btn small" disabled={busy} onClick={() => void act(item, "start")}><Play size={14} aria-hidden="true" />{t("启动", "Start")}</button>}
        {has("stop") && <button className="cx-btn small" disabled={busy} onClick={() => void act(item, "stop")}><Square size={14} aria-hidden="true" />{t("停止", "Stop")}</button>}
        {has("refresh") && <button className="cx-btn small ghost" disabled={busy} onClick={() => void act(item, "refresh")}><RefreshCw size={14} aria-hidden="true" />{t("刷新", "Refresh")}</button>}
      </div></div>
    {item.permissions.length > 0 && <div className="cx-cap-section"><h4>{t("系统权限", "System permissions")}</h4>
      {item.permissions.map(permission => <div className="cx-between" key={permission.id}><span>{permissionName(permission.id, t)}</span><Pill tone={permissionTone[permission.state]}>{permissionState(permission.state, t)}</Pill></div>)}
      {missing.length > 0 && <div className="cx-actions">
        {has("request_permissions") && <button className="cx-btn small primary" disabled={busy} onClick={() => void act(item, "request_permissions")}><ShieldCheck size={14} aria-hidden="true" />{t("去授权", "Grant access")}</button>}
        {has("open_settings") && <button className="cx-btn small" disabled={busy} onClick={() => void act(item, "open_settings")}><ExternalLink size={14} aria-hidden="true" />{t("打开系统设置", "Open System Settings")}</button>}
      </div>}
      {missing.length > 0 && <p className="cx-muted cx-small">{t("权限授予提供者应用本身，与终端或浏览器无关；授权后点击「重新检测」。", "Permission belongs to the provider app itself, not your terminal or browser. Check again after granting.")}</p>}</div>}
    <div className="cx-cap-section"><h4>{t("共享", "Sharing")}</h4>
      <Switch label={t("允许我的其他设备使用", "Available to my other devices")} detail={item.sharing.enabled && item.sharing.address ? <span className="cx-mono">{item.sharing.address}</span> : undefined}
        checked={item.sharing.enabled} disabled={busy || !(item.sharing.enabled ? has("disable") : has("enable"))} onChange={on => void act(item, on ? "enable" : "disable")} />
      <p className="cx-muted cx-small">{t("每台设备能否使用，在该设备详情的「授权」中单独设置。", "Choose which devices may use it on each device's Access tab.")}</p></div>
    {item.error && <p className="cx-error" role="alert"><CircleAlert size={14} aria-hidden="true" /> {item.error}</p>}
  </Card>;
}
