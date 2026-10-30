import { Copy, KeyRound, PlugZap, RotateCcw, ShieldAlert } from "lucide-react";
import { useState } from "react";
import type { Identity } from "../personal/api";
import { copyText, IdentityCard, useIdentity } from "./Network";
import { inShell } from "./shell";
import { DesktopAppSettings } from "./DesktopAppSettings";
import { useApp } from "./store";
import { Alert, Card, isThis } from "./ui";

/** Recovery code and owner-key rotation: the two layers that protect the identity. */
export function IdentitySettings() {
  const { t, api, devices } = useApp();
  const { identity, setIdentity } = useIdentity();
  const [recovery, setRecovery] = useState(""), [remove, setRemove] = useState<string[]>([]);
  const [busy, setBusy] = useState(false), [error, setError] = useState(""), [done, setDone] = useState("");
  async function step(action: () => Promise<void>) {
    setBusy(true); setError(""); setDone("");
    try { await action(); } catch (problem) { setError(problem instanceof Error ? problem.message : String(problem)); } finally { setBusy(false); }
  }
  const others = devices.filter(device => !isThis(device) && !device.guest);
  return <div className="cx-stack loose">
    <IdentityCard />
    <Card label={t("恢复口令", "Recovery passphrase")}>
      <div className="cx-card-head"><span className="cx-card-icon"><KeyRound size={20} aria-hidden="true" /></span>
        <div className="cx-grow"><h3>{t("恢复口令", "Recovery passphrase")}</h3>
          <p className="cx-muted cx-small">{t("设备丢失或被盗时，用它移除那台设备并更换身份密钥；在一台设备上批准另一台设备的请求时，也要输入它。它不会保存在任何设备上，像账号密码一样只记在你心里。", "If a device is lost or stolen, use it to remove that device and replace the identity key; you also type it to approve one device’s request from another. It is never stored on any device — keep it in your head, like an account password.")}</p></div></div>
      {identity?.recovery ? <p className="cx-muted" style={{ marginTop: 12 }}>{identity.recovery_kind === "passphrase" ? t("已设置恢复口令。", "A recovery passphrase is set.") : t("已设置恢复码。", "A recovery code is set.")}</p>
        : <RecoveryCreator onCreated={setIdentity} />}
    </Card>
    {identity?.recovery && <RecoveryRotator kind={identity.recovery_kind === "code" ? "code" : "passphrase"} onRotated={setIdentity} />}
    {identity?.recovery && <Card label={t("移除丢失的设备", "Remove a lost device")}>
      <div className="cx-card-head"><span className="cx-card-icon"><ShieldAlert size={20} aria-hidden="true" /></span>
        <div className="cx-grow"><h3>{t("移除丢失的设备", "Remove a lost device")}</h3>
          <p className="cx-muted cx-small">{t("用恢复口令（或恢复码）更换身份密钥，并把选中的设备移出身份。其余设备会自动收到新密钥。", "Replace the identity key with your recovery passphrase (or code) and remove the selected devices. Your other devices receive the new key automatically.")}</p></div></div>
      <form className="cx-stack" style={{ marginTop: 12 }} onSubmit={event => { event.preventDefault(); void step(async () => {
        setIdentity(await api.call<Identity>("POST", "/identity/rotate", { code: recovery.trim(), remove }));
        setRecovery(""); setRemove([]); setDone(t("身份密钥已更换。", "The identity key was replaced."));
      }); }}>
        <fieldset className="cx-toggle-list"><legend className="cx-small cx-muted">{t("要移除的设备", "Devices to remove")}</legend>
          {others.length ? others.map(device => <label key={device.node_id} className="cx-check"><input type="checkbox" checked={remove.includes(device.node_id)}
            onChange={event => setRemove(value => event.target.checked ? [...value, device.node_id] : value.filter(id => id !== device.node_id))} />{device.name}</label>)
            : <p className="cx-muted cx-small">{t("没有其他设备。", "No other devices.")}</p>}</fieldset>
        <label className="cx-field">{identity.recovery_kind === "passphrase" ? t("恢复口令", "Recovery passphrase") : t("恢复码", "Recovery code")}<input type={identity.recovery_kind === "passphrase" ? "password" : "text"} autoComplete="off" spellCheck={false} value={recovery} onChange={event => setRecovery(event.target.value)} required /></label>
        <div className="cx-actions"><button className="cx-btn danger" disabled={busy || !recovery.trim()}><RotateCcw size={15} aria-hidden="true" />{t("更换身份密钥", "Replace identity key")}</button></div>
      </form>
    </Card>}
    {done && <div className="cx-notice" role="status"><span>{done}</span></div>}
    {error && <Alert>{error}</Alert>}
  </div>;
}
function passphraseProblem(first: string, second: string, t: (zh: string, en: string) => string) {
  const length = [...first.trim()].length, distinct = new Set(first.trim()).size;
  return !first ? "" : length < 12 ? t("至少 12 个字符（可以是一句话）", "At least 12 characters (a sentence works)")
    : distinct < 5 ? t("太简单了，换一些不同的字符", "Too simple; mix more characters")
    : second && second.trim() !== first.trim() ? t("两次输入不一致", "The two entries differ") : "";
}
/** Replace the recovery passphrase or code, for example after typing it on a device you
 * do not trust. The old one signs the hand-over, so your other devices follow it. */
function RecoveryRotator({ kind, onRotated }: { kind: "passphrase" | "code"; onRotated: (identity: Identity) => void }) {
  const { t, api } = useApp();
  const [open, setOpen] = useState(false), [mode, setMode] = useState<"passphrase" | "code">("passphrase");
  const [current, setCurrent] = useState(""), [first, setFirst] = useState(""), [second, setSecond] = useState("");
  const [code, setCode] = useState(""), [busy, setBusy] = useState(false), [error, setError] = useState(""), [done, setDone] = useState(""), [copied, setCopied] = useState(false);
  const problem = mode === "passphrase" ? passphraseProblem(first, second, t) : "";
  const ready = current.trim() && (mode === "code" || (!problem && !!second));
  async function rotate() {
    setBusy(true); setError(""); setDone("");
    try {
      const value = await api.call<{ code: string | null; identity: Identity }>("POST", "/identity/recovery/rotate", mode === "passphrase" ? { current: current.trim(), passphrase: first.trim() } : { current: current.trim() });
      setCurrent(""); setFirst(""); setSecond("");
      if (value.code) setCode(value.code); else { onRotated(value.identity); setOpen(false); setDone(t("恢复口令已更换。", "The recovery passphrase was replaced.")); }
    } catch (failure) { setError(failure instanceof Error ? failure.message : String(failure)); } finally { setBusy(false); }
  }
  const old = kind === "code" ? t("当前恢复码", "Current recovery code") : t("当前恢复口令", "Current recovery passphrase");
  return <Card label={t("更换恢复口令", "Replace recovery passphrase")}>
    <div className="cx-card-head"><span className="cx-card-icon"><RotateCcw size={20} aria-hidden="true" /></span>
      <div className="cx-grow"><h3>{t("更换恢复口令", "Replace recovery passphrase")}</h3>
        <p className="cx-muted cx-small">{t("如果你在不放心的设备上输入过它，或怀疑被别人看到，就换一个。你的其他设备会自动改用新的；旧的立刻失效。身份码会随之改变。离线太久、错过两次更换的设备需要重新配对。", "Replace it if you typed it on a device you do not trust or think someone saw it. Your other devices switch to the new one automatically and the old one stops working at once. Your identity code changes too. A device that was offline through two replacements must be paired again.")}</p></div>
      {!open && !code && <button className="cx-btn" onClick={() => setOpen(true)}>{t("更换", "Replace")}</button>}</div>
    {code ? <div className="cx-stack" style={{ marginTop: 12 }}>
      <Alert tone="warn">{t("这是新的恢复码，只显示这一次。请抄写或存进密码管理器，旧的已经失效。", "This is your new recovery code, shown only once. Write it down or keep it in a password manager; the old one no longer works.")}</Alert>
      <code className="cx-identity-code">{code}</code>
      <div className="cx-actions"><button className="cx-btn" onClick={() => void copyText(code).then(setCopied)}><Copy size={14} aria-hidden="true" />{copied ? t("已复制", "Copied") : t("复制", "Copy")}</button>
        <button className="cx-btn primary" onClick={() => { setCode(""); setOpen(false); void api.call<Identity>("GET", "/identity").then(onRotated); }}>{t("我已保存", "I have saved it")}</button></div>
    </div> : open && <form className="cx-stack" style={{ marginTop: 12 }} onSubmit={event => { event.preventDefault(); if (ready) void rotate(); }}>
      <label className="cx-field">{old}<input type="password" autoComplete="off" spellCheck={false} value={current} onChange={event => setCurrent(event.target.value)} /></label>
      <div className="cx-segment" role="group" aria-label={t("新的恢复方式", "New recovery method")}>
        <button type="button" aria-pressed={mode === "passphrase"} onClick={() => setMode("passphrase")}>{t("自己设置口令", "Choose a passphrase")}</button>
        <button type="button" aria-pressed={mode === "code"} onClick={() => setMode("code")}>{t("生成随机恢复码", "Generate a code")}</button></div>
      {mode === "passphrase" ? <>
        <label className="cx-field">{t("新的恢复口令", "New recovery passphrase")}<input type="password" autoComplete="new-password" value={first} onChange={event => setFirst(event.target.value)} /></label>
        <label className="cx-field">{t("再输入一次", "Enter it again")}<input type="password" autoComplete="new-password" value={second} onChange={event => setSecond(event.target.value)} /></label>
        <small className={problem ? "cx-error" : "cx-muted"}>{problem || t("用一句只有你知道的话，至少 12 个字符。", "Use a sentence only you know, at least 12 characters.")}</small>
      </> : <small className="cx-muted">{t("新的恢复码只显示一次。", "The new code is shown once.")}</small>}
      <div className="cx-actions"><button className="cx-btn primary" disabled={busy || !ready}>{busy ? t("正在更换…", "Replacing…") : t("更换", "Replace")}</button>
        <button type="button" className="cx-btn ghost" onClick={() => { setOpen(false); setError(""); }}>{t("取消", "Cancel")}</button></div>
    </form>}
    {done && <p className="cx-muted" role="status">{done}</p>}
    {error && <Alert>{error}</Alert>}
  </Card>;
}
/** Set the identity's recovery key: a passphrase the owner chooses (default) or a
 * generated code shown once. */
export function RecoveryCreator({ onCreated }: { onCreated: (identity: Identity) => void }) {
  const { t, api } = useApp();
  const [mode, setMode] = useState<"passphrase" | "code">("passphrase");
  const [first, setFirst] = useState(""), [second, setSecond] = useState(""), [code, setCode] = useState("");
  const [busy, setBusy] = useState(false), [error, setError] = useState(""), [copied, setCopied] = useState(false);
  const length = [...first.trim()].length, distinct = new Set(first.trim()).size;
  const problem = !first ? "" : length < 12 ? t("至少 12 个字符（可以是一句话）", "At least 12 characters (a sentence works)")
    : distinct < 5 ? t("太简单了，换一些不同的字符", "Too simple; mix more characters")
    : second && second.trim() !== first.trim() ? t("两次输入不一致", "The two entries differ") : "";
  const strength = length >= 20 && distinct >= 10 ? t("强", "Strong") : length >= 14 ? t("中", "Fair") : t("弱", "Weak");
  async function create(passphrase?: string) {
    setBusy(true); setError("");
    try {
      const value = await api.call<{ code: string | null; identity: Identity }>("POST", "/identity/recovery", passphrase ? { passphrase } : {});
      if (value.code) setCode(value.code); else onCreated(value.identity);
      if (value.code) onCreated({ ...value.identity, recovery: false });
    } catch (failure) { setError(failure instanceof Error ? failure.message : String(failure)); } finally { setBusy(false); }
  }
  if (code) return <div className="cx-stack" style={{ marginTop: 12 }}>
    <Alert tone="warn">{t("这是唯一一次显示。请抄写或存进密码管理器。", "This is the only time it is shown. Write it down or keep it in a password manager.")}</Alert>
    <code className="cx-identity-code">{code}</code>
    <div className="cx-actions"><button className="cx-btn" onClick={() => void copyText(code).then(setCopied)}><Copy size={14} aria-hidden="true" />{copied ? t("已复制", "Copied") : t("复制", "Copy")}</button>
      <button className="cx-btn primary" onClick={() => { setCode(""); void api.call<Identity>("GET", "/identity").then(onCreated); }}>{t("我已保存", "I have saved it")}</button></div>
  </div>;
  return <div className="cx-stack" style={{ marginTop: 12 }}>
    {mode === "passphrase" ? <form className="cx-stack" onSubmit={event => { event.preventDefault(); if (!problem && second) void create(first.trim()); }}>
      <label className="cx-field">{t("恢复口令", "Recovery passphrase")}<input type="password" autoComplete="new-password" value={first} onChange={event => setFirst(event.target.value)} aria-describedby="recovery-hint" /></label>
      <label className="cx-field">{t("再输入一次", "Enter it again")}<input type="password" autoComplete="new-password" value={second} onChange={event => setSecond(event.target.value)} /></label>
      <small id="recovery-hint" className={problem ? "cx-error" : "cx-muted"}>{problem || (first ? t("强度：", "Strength: ") + strength + t("。记住它：忘记后无法找回。", ". Remember it: it cannot be recovered if forgotten.") : t("用一句只有你知道的话，至少 12 个字符。", "Use a sentence only you know, at least 12 characters."))}</small>
      <div className="cx-actions"><button className="cx-btn primary" disabled={busy || !!problem || !second}>{busy ? t("正在设置…", "Setting…") : t("设置恢复口令", "Set recovery passphrase")}</button>
        <button type="button" className="cx-btn ghost" onClick={() => setMode("code")}>{t("改用随机恢复码", "Use a generated code instead")}</button></div>
    </form> : <div className="cx-stack">
      <p className="cx-muted cx-small">{t("生成一串随机恢复码，只显示一次，需要你抄写或存进密码管理器。", "Generate a random recovery code. It is shown once; write it down or keep it in a password manager.")}</p>
      <div className="cx-actions"><button className="cx-btn primary" disabled={busy} onClick={() => void create()}>{t("生成恢复码", "Create recovery code")}</button>
        <button className="cx-btn ghost" onClick={() => setMode("passphrase")}>{t("改为自己设置口令", "Choose a passphrase instead")}</button></div>
    </div>}
    {error && <Alert>{error}</Alert>}
  </div>;
}
/** How agents reach your devices: the desktop app writes MCP configuration; the CLI serves the same tools. */
export function AgentAccess() {
  const { t } = useApp();
  if (inShell()) return <div className="cx-stack loose"><DesktopAppSettings section="agents" /></div>;
  const commands = "cranibox nodes\ncranibox exec --node <NodeID> -- uname -a\ncranibox mcp";
  return <Card label={t("Agent 接入", "Agent access")}>
    <div className="cx-card-head"><span className="cx-card-icon"><PlugZap size={20} aria-hidden="true" /></span>
      <div className="cx-grow"><h3>{t("Agent 接入", "Agent access")}</h3>
        <p className="cx-muted cx-small">{t("Agent 通过 Cranibox 命令行或 MCP 使用你已配对和同一身份的设备。把 `cranibox mcp` 加为 MCP 服务器即可。", "Agents use your paired and same-identity devices through the Cranibox CLI or MCP. Add `cranibox mcp` as an MCP server.")}</p></div></div>
    <div className="cx-plan" style={{ marginTop: 12 }}><pre aria-label={t("命令", "Commands")}>{commands}</pre></div>
  </Card>;
}
