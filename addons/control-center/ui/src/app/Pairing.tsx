import { ShieldCheck } from "lucide-react";
import { useEffect, useState } from "react";
import type { Accepted, Invite } from "../personal/api";
import { useApp } from "./store";
import { QrPanel, Sheet } from "./ui";

type ServerRequest = { request_id: string; fingerprint: string; server: { name?: string; node_id: string; fingerprint: string } };
/** Pairing works the same from every device: any peer can invite, any peer can accept. */
export function PairingFlow({ onPaired }: { onPaired?: () => void }) {
  const { api, t, refresh, setNotice } = useApp();
  const [purpose, setPurpose] = useState("owner_device"), [invite, setInvite] = useState<Invite | null>(null);
  const [link, setLink] = useState(""), [accepted, setAccepted] = useState<Accepted | null>(null);
  const [busy, setBusy] = useState(false), [error, setError] = useState("");
  const [waiting, setWaiting] = useState<ServerRequest | null>(null);
  useEffect(() => {
    if (!waiting) return;
    let alive = true;
    const timer = setInterval(() => {
      void api.call<{ state: string } & Partial<Accepted>>("GET", "/pair/server/" + encodeURIComponent(waiting.request_id)).then(value => {
        if (!alive) return;
        if (value.state === "accepted" && value.flow_id && value.sas) { setWaiting(null); setAccepted({ flow_id: value.flow_id, sas: value.sas }); }
        else if (value.state === "rejected") { setWaiting(null); setError(t("服务器主人拒绝了这个请求。", "The server's owner declined the request.")); }
      }, problem => { if (alive) { setWaiting(null); setError(problem instanceof Error ? problem.message : String(problem)); } });
    }, 2000);
    return () => { alive = false; clearInterval(timer); };
  }, [waiting, api, t]);
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
  if (waiting) return <div className="cx-sas">
    <ShieldCheck size={40} aria-hidden="true" />
    <h3>{t("等待服务器主人批准", "Waiting for the server's owner")}</h3>
    <p className="cx-muted">{t("请服务器主人在其「待办」中核对下面的指纹并批准。", "Ask the server's owner to check this fingerprint in their Inbox and approve.")}</p>
    <dl className="cx-facts">
      <div><dt>{t("服务器", "Server")}</dt><dd>{waiting.server.name || waiting.server.node_id}</dd></div>
      <div><dt>{t("服务器指纹", "Server fingerprint")}</dt><dd className="cx-mono">{waiting.server.fingerprint}</dd></div>
      <div><dt>{t("本机指纹", "This device's fingerprint")}</dt><dd className="cx-mono">{waiting.fingerprint}</dd></div>
    </dl>
    <button className="cx-btn" onClick={() => setWaiting(null)}>{t("取消", "Cancel")}</button>
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
    <div className="cx-divider">{t("或打开邀请、连接服务器", "Or open an invitation, or connect to a server")}</div>
    <form onSubmit={event => { event.preventDefault(); void step(async () => {
      const value = link.trim();
      if (value.startsWith("cranibox://")) { setAccepted(await api.call<Accepted>("POST", "/pair/accept", { link: value })); return; }
      const answer = await api.call<Accepted & ServerRequest>("POST", "/pair/server", { url: value });
      if (answer.flow_id) setAccepted(answer); else setWaiting(answer);
    }); }}>
      <label className="cx-field">{t("配对链接或服务器地址", "Pairing link or server address")}<input placeholder="cranibox://pair/…  ·  https://box.example.com" value={link} onChange={event => setLink(event.target.value)} required /></label>
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
