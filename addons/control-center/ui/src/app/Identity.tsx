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
  const [code, setCode] = useState(""), [copied, setCopied] = useState(false);
  const [recovery, setRecovery] = useState(""), [remove, setRemove] = useState<string[]>([]);
  const [busy, setBusy] = useState(false), [error, setError] = useState(""), [done, setDone] = useState("");
  async function step(action: () => Promise<void>) {
    setBusy(true); setError(""); setDone("");
    try { await action(); } catch (problem) { setError(problem instanceof Error ? problem.message : String(problem)); } finally { setBusy(false); }
  }
  const others = devices.filter(device => !isThis(device) && !device.guest);
  return <div className="cx-stack loose">
    <IdentityCard />
    <Card label={t("恢复码", "Recovery code")}>
      <div className="cx-card-head"><span className="cx-card-icon"><KeyRound size={20} aria-hidden="true" /></span>
        <div className="cx-grow"><h3>{t("恢复码", "Recovery code")}</h3>
          <p className="cx-muted cx-small">{t("恢复码只用来更换设备上的身份密钥：设备丢失或被盗时，用它移除那台设备。它不会保存在任何设备上，只显示一次，请抄写或存进密码管理器。", "The recovery code only replaces the identity key your devices hold: if a device is lost or stolen, use it to remove that device. It is never stored on any device and is shown once — write it down or keep it in a password manager.")}</p></div></div>
      {code ? <div className="cx-stack" style={{ marginTop: 12 }}>
        <Alert tone="warn">{t("这是唯一一次显示。关闭后无法再次查看。", "This is the only time it is shown. It cannot be viewed again.")}</Alert>
        <code className="cx-identity-code">{code}</code>
        <div className="cx-actions"><button className="cx-btn" onClick={() => void copyText(code).then(setCopied)}><Copy size={14} aria-hidden="true" />{copied ? t("已复制", "Copied") : t("复制", "Copy")}</button>
          <button className="cx-btn primary" onClick={() => setCode("")}>{t("我已保存", "I have saved it")}</button></div>
      </div> : identity?.recovery ? <p className="cx-muted" style={{ marginTop: 12 }}>{t("这个身份已经有恢复码。", "This identity already has a recovery code.")}</p>
        : <div className="cx-actions" style={{ marginTop: 12 }}><button className="cx-btn primary" disabled={busy || !identity} onClick={() => void step(async () => {
          const value = await api.call<{ code: string; identity: Identity }>("POST", "/identity/recovery");
          setCode(value.code); setIdentity(value.identity);
        })}>{t("生成恢复码", "Create recovery code")}</button></div>}
    </Card>
    {identity?.recovery && <Card label={t("移除丢失的设备", "Remove a lost device")}>
      <div className="cx-card-head"><span className="cx-card-icon"><ShieldAlert size={20} aria-hidden="true" /></span>
        <div className="cx-grow"><h3>{t("移除丢失的设备", "Remove a lost device")}</h3>
          <p className="cx-muted cx-small">{t("用恢复码更换身份密钥，并把选中的设备移出身份。其余设备会自动收到新密钥。", "Replace the identity key with your recovery code and remove the selected devices. Your other devices receive the new key automatically.")}</p></div></div>
      <form className="cx-stack" style={{ marginTop: 12 }} onSubmit={event => { event.preventDefault(); void step(async () => {
        setIdentity(await api.call<Identity>("POST", "/identity/rotate", { code: recovery.trim(), remove }));
        setRecovery(""); setRemove([]); setDone(t("身份密钥已更换。", "The identity key was replaced."));
      }); }}>
        <fieldset className="cx-toggle-list"><legend className="cx-small cx-muted">{t("要移除的设备", "Devices to remove")}</legend>
          {others.length ? others.map(device => <label key={device.node_id} className="cx-check"><input type="checkbox" checked={remove.includes(device.node_id)}
            onChange={event => setRemove(value => event.target.checked ? [...value, device.node_id] : value.filter(id => id !== device.node_id))} />{device.name}</label>)
            : <p className="cx-muted cx-small">{t("没有其他设备。", "No other devices.")}</p>}</fieldset>
        <label className="cx-field">{t("恢复码", "Recovery code")}<input autoComplete="off" spellCheck={false} value={recovery} onChange={event => setRecovery(event.target.value)} required /></label>
        <div className="cx-actions"><button className="cx-btn danger" disabled={busy || !recovery.trim()}><RotateCcw size={15} aria-hidden="true" />{t("更换身份密钥", "Replace identity key")}</button></div>
      </form>
    </Card>}
    {done && <div className="cx-notice" role="status"><span>{done}</span></div>}
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
