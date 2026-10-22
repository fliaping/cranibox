import { Laptop, ShieldCheck } from "lucide-react";
import { useState } from "react";
import type { Accepted, Invite } from "../personal/api";
import { store, useApp } from "./store";
import { QrPanel, Sheet } from "./ui";

/** Pairing works the same from every device: any peer can invite, any peer can accept. */
export function PairingFlow({ onPaired }: { onPaired?: () => void }) {
  const { api, t, refresh, setNotice } = useApp();
  const [purpose, setPurpose] = useState("owner_device"), [invite, setInvite] = useState<Invite | null>(null);
  const [link, setLink] = useState(""), [accepted, setAccepted] = useState<Accepted | null>(null);
  const [busy, setBusy] = useState(false), [error, setError] = useState("");
  async function step(action: () => Promise<void>) {
    setBusy(true); setError("");
    try { await action(); } catch (problem) { setError(problem instanceof Error ? problem.message : String(problem)); } finally { setBusy(false); }
  }
  if (accepted) return <div className="cx-sas">
    <ShieldCheck size={40} aria-hidden="true" />
    <h3>{t("两台设备上的数字一致吗？", "Do the codes match on both devices?")}</h3>
    <p className="cx-muted">{t("请在另一台设备上查看，确认每一位都相同。", "Check the other device. Every digit must match.")}</p>
    <code>{accepted.sas}</code>
    <div className="cx-actions">
      <button className="cx-btn" disabled={busy} onClick={() => void step(async () => {
        await api.call("POST", "/pair/" + encodeURIComponent(accepted.flow_id) + "/confirm", { match: false });
        setAccepted(null); setError(t("配对已取消。请重新生成邀请。", "Pairing cancelled. Create a new invitation."));
      })}>{t("不一致，取消", "Different, cancel")}</button>
      <button className="cx-btn primary" disabled={busy} onClick={() => void step(async () => {
        await api.call("POST", "/pair/" + encodeURIComponent(accepted.flow_id) + "/confirm", { match: true });
        setNotice(t("设备已配对", "Device paired")); await refresh(); onPaired?.();
      })}>{t("两边一致", "Codes match")}</button>
    </div>
    {error && <p className="cx-error" role="alert">{error}</p>}
  </div>;
  return <>
    <div className="cx-segment" role="group" aria-label={t("设备类型", "Device type")}>
      {[["owner_device", t("我的设备", "My device")], ["guest", t("访客设备", "Guest device")]].map(([value, label]) =>
        <button key={value} aria-pressed={purpose === value} onClick={() => { setPurpose(value); setInvite(null); }}>{label}</button>)}
    </div>
    <p className="cx-muted">{purpose === "guest"
      ? t("访客只能使用你逐项允许的功能，无法访问模型订阅。配对后在设备详情的「授权」中选择。", "Guests can use only the capabilities you allow, never your model subscriptions. Choose them under Access on the device page after pairing.")
      : t("你的设备彼此对等：共享设备名单，接收加密的模型订阅，可以互相打开终端、桌面和浏览器。", "Your devices are equal peers: they share a device list, receive encrypted model subscriptions and can open each other's terminal, desktop and browser.")}</p>
    {invite ? <QrPanel invite={invite} t={t} /> : <button className="cx-btn primary full" disabled={busy} onClick={() => void step(async () => setInvite(await api.call<Invite>("POST", "/pair/invite", { purpose })))}>{t("生成二维码和链接", "Create QR code and link")}</button>}
    <div className="cx-divider">{t("或打开另一台设备发来的邀请", "Or open an invitation from another device")}</div>
    <form onSubmit={event => { event.preventDefault(); void step(async () => setAccepted(await api.call<Accepted>("POST", "/pair/accept", { link: link.trim() }))); }}>
      <label className="cx-field">{t("配对链接", "Pairing link")}<input placeholder="cranibox://pair/…" value={link} onChange={event => setLink(event.target.value)} required /></label>
      <button className="cx-btn full" disabled={busy || !link.trim()}>{t("继续核对", "Continue to verification")}</button>
    </form>
    {error && <p className="cx-error" role="alert">{error}</p>}
  </>;
}
export function PairingSheet() {
  const { t, pairing, setPairing } = useApp();
  if (!pairing) return null;
  return <Sheet title={t("添加设备", "Add device")} t={t} onClose={() => setPairing(false)}><PairingFlow onPaired={() => setPairing(false)} /></Sheet>;
}
export function Onboarding({ onModels }: { onModels: () => void }) {
  const { t, loading } = useApp();
  const [open, setOpen] = useState(() => { try { return localStorage.getItem("cranibox.onboarded") !== "1"; } catch { return false; } });
  const [step, setStep] = useState(0);
  if (!open || loading) return null;
  const finish = () => { store("cranibox.onboarded", "1"); setOpen(false); };
  const titles = [t("欢迎使用 Cranibox", "Welcome to Cranibox"), t("连接你的另一台设备", "Connect another device"), t("给 Agent 一个模型", "Give your agents a model")];
  return <Sheet title={titles[step]} t={t} onClose={finish}>
    <div className="cx-progress" aria-label={t("引导进度", "Setup progress")}>{[0, 1, 2].map(index => <span key={index} className={index <= step ? "done" : ""} />)}</div>
    {step === 0 ? <>
      <span className="cx-card-icon"><Laptop size={22} aria-hidden="true" /></span>
      <p>{t("Cranibox 把你的每台设备连成一个 Agent 工作台：终端、桌面、浏览器与 Computer Use 都能跨设备使用。设备之间端到端加密、对等直连，没有中心服务器；重要操作始终先问你。",
        "Cranibox turns every device you own into one workspace for your agents: terminal, desktop, browser and Computer Use work across devices. Devices connect peer to peer with end-to-end encryption and no central server. Important actions always ask you first.")}</p>
      <button className="cx-btn primary full" onClick={() => setStep(1)}>{t("开始设置", "Get started")}</button>
      <button className="cx-btn full" onClick={() => setStep(2)}>{t("先在这台设备使用", "Use on this device first")}</button>
    </> : step === 1 ? <>
      <p>{t("在另一台运行 Cranibox 的设备上打开「添加设备」，把邀请链接粘贴到这里，或用相机扫描二维码。", "On another device running Cranibox, open Add device. Paste its invitation link here, or scan the QR code with your camera.")}</p>
      <PairingFlow onPaired={() => setStep(2)} />
      <button className="cx-btn ghost full" onClick={() => setStep(2)}>{t("暂时跳过", "Skip for now")}</button>
    </> : <>
      <p>{t("连接你已有的订阅，之后随时可以更改。", "Connect a subscription you already have. You can change it any time.")}</p>
      <button className="cx-btn primary full" onClick={() => { finish(); onModels(); }}>{t("添加订阅", "Add subscription")}</button>
      <button className="cx-btn full" onClick={finish}>{t("稍后添加，进入首页", "Add later, go home")}</button>
    </>}
  </Sheet>;
}
