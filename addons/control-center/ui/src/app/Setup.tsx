import { Check, CircleAlert, Globe, KeyRound, Monitor, MousePointerClick, Plus, Power, ShieldCheck, Sparkles } from "lucide-react";
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { useNavigate } from "react-router-dom";
import type { Capabilities, Capability, CapabilityAction, InstallPlan } from "../personal/api";
import { InstallPlanSheet } from "./Capabilities";
import { useIdentity } from "./Network";
import { RecoveryCreator } from "./Identity";
import { inShell, shellCall, type ShellStatus } from "./shell";
import { savePreferences, store, useApp } from "./store";
import { Alert, Card, formatDeviceLabel, localizeMessage, Pill, type Translate } from "./ui";

type Step = "permissions" | "identity" | "devices";
const steps: Step[] = ["permissions", "identity", "devices"];
type Row = { id: string; icon: typeof Monitor; title: string; detail: string; state: "ready" | "todo" | "manual" | "unavailable"; note?: string; fix?: () => Promise<void> };

/** First launch: everything Cranibox needs from this device, asked for in one place. */
export function SetupGuide() {
  const { t, api, setPairing, thisDevice } = useApp();
  const navigate = useNavigate();
  const [step, setStep] = useState<Step>("permissions");
  const finish = async (pair = false) => {
    store("cranibox.onboarded", "1");
    navigate("/app/home", { replace: true });
    await savePreferences(api, { onboarded: true });
    if (pair) setPairing(true);
  };
  const labels: Record<Step, string> = { permissions: t("权限与能力", "Permissions"), identity: t("身份与恢复口令", "Identity"), devices: t("添加设备", "Add devices") };
  return <div className="cx-setup">
    <div className="cx-page-head"><div><h1>{t("设置 Cranibox", "Set up Cranibox")}</h1>
      <p>{t("一次把这台设备需要的权限和设置都准备好。之后随时可以在「本机设置」里调整。", "Get everything this device needs ready in one go. You can change any of it later in Settings.")}{thisDevice ? " · " + formatDeviceLabel(thisDevice.name, thisDevice.node_id, t).label : ""}</p></div>
      <div className="cx-actions"><button className="cx-btn ghost" onClick={() => void finish()}>{t("跳过向导", "Skip setup")}</button></div></div>
    <ol className="cx-steps" aria-label={t("设置步骤", "Setup steps")}>
      {steps.map((item, index) => <li key={item} aria-current={item === step ? "step" : undefined} className={steps.indexOf(step) > index ? "done" : ""}>
        <button onClick={() => setStep(item)}><span>{steps.indexOf(step) > index ? <Check size={14} aria-hidden="true" /> : index + 1}</span>{labels[item]}</button></li>)}
    </ol>
    {step === "permissions" && <PermissionsStep next={() => setStep("identity")} />}
    {step === "identity" && <IdentityStep back={() => setStep("permissions")} next={() => setStep("devices")} />}
    {step === "devices" && <Card label={labels.devices}>
      <div className="cx-card-head"><span className="cx-card-icon"><Plus size={20} aria-hidden="true" /></span><div className="cx-grow"><h3>{t("连接你的其他设备", "Connect your other devices")}</h3>
        <p className="cx-muted">{t("在另一台设备上安装 Cranibox，然后在这里生成配对链接或二维码；也可以输入运行 Cranibox 的服务器地址。同一身份的设备会自动互相可见。", "Install Cranibox on another device, then create a pairing link or QR code here — or enter the address of a server running Cranibox. Devices of the same identity see each other automatically.")}</p></div></div>
      <div className="cx-actions" style={{ marginTop: 12 }}>
        <button className="cx-btn" onClick={() => setStep("identity")}>{t("上一步", "Back")}</button>
        <button className="cx-btn" onClick={() => void finish()}>{t("稍后再说", "Later")}</button>
        <button className="cx-btn primary" onClick={() => void finish(true)}><Plus size={15} aria-hidden="true" />{t("添加设备并完成", "Add a device and finish")}</button>
      </div>
    </Card>}
  </div>;
}

function rows(data: Capabilities | null, shell: ShellStatus | null, act: (item: Capability, action: CapabilityAction) => Promise<void>, setAutostart: (on: boolean) => Promise<void>, t: Translate): Row[] {
  const item = (id: string) => data?.items.find(entry => entry.id === id);
  const has = (entry: Capability | undefined, action: CapabilityAction) => !!entry?.actions.includes(action);
  const result: Row[] = [];
  const computer = item("computer");
  if (computer) {
    const missing = computer.permissions.some(p => p.state === "denied" || p.state === "unknown");
    result.push({
      id: "computer", icon: MousePointerClick, title: "Computer Use",
      detail: t("让 Agent 看到屏幕并使用鼠标键盘（macOS：屏幕录制与辅助功能）。每次会话仍会先问你。", "Let agents see the screen and use mouse and keyboard (macOS: Screen Recording and Accessibility). Each session still asks you first."),
      state: !computer.provider.installed ? "manual" : missing ? "todo" : computer.sharing.enabled ? "ready" : "todo",
      note: !computer.provider.installed ? t("需要先安装组件，点击「安装」后会先给你看下载来源。", "Needs a component first. Install shows you the download before anything runs.") : missing ? t("macOS 会弹出授权窗口或打开系统设置，请为 Cranibox 打开开关。", "macOS shows a prompt or opens System Settings — turn Cranibox on there.") : undefined,
      fix: async () => {
        if (!computer.provider.installed) return;
        if (missing && has(computer, "request_permissions")) await act(computer, "request_permissions");
        if (!computer.sharing.enabled && has(computer, "enable")) await act(computer, "enable").catch(() => undefined);
      },
    });
  }
  const desktop = item("desktop");
  if (desktop) result.push({
    id: "desktop", icon: Monitor, title: t("远程桌面", "Remote desktop"),
    detail: t("在你的其他设备上查看和操作这块屏幕。", "View and control this screen from your other devices."),
    state: desktop.sharing.enabled && desktop.provider.running ? "ready" : desktop.provider.installed ? "todo" : "manual",
    note: !desktop.provider.running ? (data?.platform === "macos" ? t("需要在「系统设置 → 通用 → 共享」中打开「屏幕共享」。", "Turn on Screen Sharing in System Settings → General → Sharing.") : localizeMessage(desktop.error, t) || undefined) : undefined,
    fix: async () => {
      if (!desktop.provider.running && has(desktop, "open_settings")) { await act(desktop, "open_settings"); return; }
      if (!desktop.sharing.enabled && has(desktop, "enable")) await act(desktop, "enable");
    },
  });
  const browser = item("browser");
  if (browser) result.push({
    id: "browser", icon: Globe, title: t("浏览器", "Browser"),
    detail: t("让 Agent 使用你正在用的 Chrome，保留登录状态；每次连接 Chrome 会请你允许。", "Let agents use the Chrome you already use, signed in; Chrome asks you to allow each connection."),
    state: browser.sharing.enabled && browser.provider.running ? "ready" : browser.provider.installed ? "todo" : "unavailable",
    note: browser.sharing.mode === "profile" && !browser.provider.running ? t("在打开的 Chrome 页面中勾选「允许远程调试」。", "In the Chrome page that opens, allow remote debugging.") : !browser.provider.installed ? t("没有找到 Chrome、Edge、Brave 或 Chromium。", "No Chrome, Edge, Brave or Chromium found.") : undefined,
    fix: async () => {
      if (has(browser, "use_profile")) await act(browser, "use_profile");
      else if (browser.sharing.mode === "profile" && has(browser, "open_settings")) await act(browser, "open_settings");
      else if (has(browser, "enable")) await act(browser, "enable");
      else if (has(browser, "start")) await act(browser, "start");
    },
  });
  if (shell) result.push({
    id: "autostart", icon: Power, title: t("登录时启动", "Start at login"),
    detail: t("开机后在后台保持这台设备在线，其他设备和 Agent 随时能用。", "Keep this device online in the background after you sign in, so your other devices and agents can always reach it."),
    state: shell.autostart ? "ready" : "todo",
    fix: () => setAutostart(true),
  });
  return result;
}

function PermissionsStep({ next }: { next: () => void }) {
  const { t, api } = useApp();
  const [data, setData] = useState<Capabilities | null>(null), [shell, setShell] = useState<ShellStatus | null>(null);
  const [busy, setBusy] = useState(""), [error, setError] = useState(""), [plan, setPlan] = useState<InstallPlan | null>(null);
  const load = useCallback(async () => {
    try { setData(await api.call<Capabilities>("GET", "/capabilities")); } catch (problem) { setError(problem instanceof Error ? problem.message : String(problem)); }
    if (inShell()) try { setShell(await shellCall<ShellStatus>("status")); } catch { /* Older desktop apps. */ }
  }, [api]);
  useEffect(() => { void load(); }, [load]);
  // Grants happen in System Settings; keep the list current while it is open.
  useEffect(() => { const timer = setInterval(() => { if (!document.hidden) void load(); }, 4000); return () => clearInterval(timer); }, [load]);
  const act = useCallback(async (item: Capability, action: CapabilityAction) => {
    await api.call("POST", "/capabilities/" + encodeURIComponent(item.id) + "/" + action, {});
  }, [api]);
  const setAutostart = useCallback(async (enabled: boolean) => { setShell(await shellCall<ShellStatus>("autostart", { enabled })); }, []);
  const list = rows(data, shell, act, setAutostart, t);
  async function run(ids: string[]) {
    setError("");
    for (const id of ids) {
      const row = list.find(entry => entry.id === id);
      if (!row?.fix || row.state === "ready") continue;
      setBusy(id);
      try { await row.fix(); } catch (problem) { setError((row.title + ": ") + localizeMessage(problem instanceof Error ? problem.message : String(problem), t)); }
    }
    setBusy("");
    await load();
  }
  // The plan is reviewed in the page: the app's macOS webview shows no window.confirm.
  async function install() {
    setBusy("computer"); setError("");
    try { setPlan(await api.call<InstallPlan>("POST", "/capabilities/computer/install_plan", {})); }
    catch (problem) { setError(problem instanceof Error ? problem.message : String(problem)); }
    setBusy("");
  }
  async function runInstall(plan: InstallPlan) {
    setBusy("computer"); setError("");
    try { await api.call("POST", "/capabilities/computer/install", { plan_id: plan.plan_id }); setPlan(null); }
    catch (problem) { setError(problem instanceof Error ? problem.message : String(problem)); }
    setBusy(""); await load();
  }
  const pending = list.filter(row => row.state === "todo");
  return <section className="cx-setup-section" aria-label={t("权限与能力", "Permissions")}>
    <div className="cx-card-head cx-setup-head"><span className="cx-card-icon"><ShieldCheck size={20} aria-hidden="true" /></span><div className="cx-grow"><h3>{t("这台设备需要的权限", "What this device needs")}</h3>
      <p className="cx-muted">{t("点「全部开启」会依次请求下面所有权限；系统弹窗出现时选择允许。已经开启的会自动跳过。", "Turn on all asks for everything below, one after another — allow each system prompt as it appears. Anything already on is skipped.")}</p></div>
      <button className="cx-btn pill" disabled={!!busy || !pending.length} onClick={() => void run(pending.map(row => row.id))}><Sparkles size={15} aria-hidden="true" />{pending.length ? t("全部开启", "Turn on all") : t("已全部开启", "All on")}</button></div>
    {error && <Alert>{error}</Alert>}
    {!data && !error && <p className="cx-muted" role="status">{t("正在检测…", "Checking…")}</p>}
    <ul className="cx-setup-list cx-grouped-list">{list.map(row => <li key={row.id}>
      <span className={"cx-card-icon " + (row.state === "ready" ? "" : "muted")}><row.icon size={18} aria-hidden="true" /></span>
      <div className="cx-grow"><strong>{row.title}</strong><small className="cx-muted">{row.detail}</small>{row.note && row.state !== "ready" && <small className="cx-setup-note"><CircleAlert size={13} aria-hidden="true" /> {row.note}</small>}</div>
      <SetupState state={row.state} t={t}>
        {row.id === "computer" && row.state === "manual" ? <button className="cx-btn small" disabled={!!busy} onClick={() => void install()}>{t("安装…", "Install…")}</button>
          : row.state !== "ready" && row.state !== "unavailable" && <button className="cx-btn small" disabled={!!busy} onClick={() => void run([row.id])}>{busy === row.id ? t("处理中…", "Working…") : t("开启", "Turn on")}</button>}
      </SetupState>
    </li>)}</ul>
    <div className="cx-actions cx-setup-foot"><button className="cx-btn primary" onClick={next}>{t("下一步", "Next")}</button></div>
    {plan && <InstallPlanSheet plan={plan} busy={!!busy} t={t} onClose={() => setPlan(null)} onRun={() => void runInstall(plan)} />}
  </section>;
}
function SetupState({ state, t, children }: { state: Row["state"]; t: Translate; children: ReactNode }) {
  if (state === "ready") return <Pill tone="good" icon={Check}>{t("已开启", "On")}</Pill>;
  if (state === "unavailable") return <Pill>{t("不可用", "Unavailable")}</Pill>;
  return <>{children}</>;
}

function IdentityStep({ back, next }: { back: () => void; next: () => void }) {
  const { t } = useApp();
  const { identity, setIdentity } = useIdentity();
  return <Card label={t("身份与恢复口令", "Identity")}>
    <div className="cx-card-head"><span className="cx-card-icon"><KeyRound size={20} aria-hidden="true" /></span><div className="cx-grow"><h3>{t("你的身份", "Your identity")}{identity && <span className="cx-mono"> · {identity.code}</span>}</h3>
      <p className="cx-muted">{t("不需要注册账号：这台设备已经创建了你的身份，之后配对的设备会加入同一身份。再设置一个恢复口令：设备丢失或被盗时，用它移除那台设备。口令不保存在任何设备上。", "No account needed: this device already created your identity, and devices you pair join it. Set a recovery passphrase too: if a device is lost or stolen, it removes that device. The passphrase is never stored on any device.")}</p></div></div>
    {identity?.recovery ? <p className="cx-muted" style={{ marginTop: 12 }}><Check size={14} aria-hidden="true" /> {identity.recovery_kind === "passphrase" ? t("已设置恢复口令。", "A recovery passphrase is set.") : t("已设置恢复码。", "A recovery code is set.")}</p>
      : identity && <RecoveryCreator onCreated={setIdentity} />}
    <div className="cx-actions" style={{ marginTop: 16 }}>
      <button className="cx-btn" onClick={back}>{t("上一步", "Back")}</button>
      <button className="cx-btn primary" onClick={next}>{identity?.recovery ? t("下一步", "Next") : t("暂不设置，下一步", "Skip for now")}</button>
    </div>
  </Card>;
}
