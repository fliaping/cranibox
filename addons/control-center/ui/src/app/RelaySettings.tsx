import { useEffect, useState } from "react";
import { request } from "../workspace";
import { useApp } from "./store";
import { Alert, Pill, Switch } from "./ui";

export type RelayConfig = { enabled: boolean; public_url: string; listen: string };
export type HostedRelayStatus = {
  enabled: boolean;
  public_url: string | null;
  listen?: string | null;
  settings?: RelayConfig;
  restart_required?: boolean;
};

export function RelaySettings({ status, onSaved }: { status: HostedRelayStatus; onSaved: (value: HostedRelayStatus) => void }) {
  const { t } = useApp();
  const [draft, setDraft] = useState<RelayConfig>({ enabled: status.enabled, public_url: status.public_url || "", listen: status.listen || "127.0.0.1:7378" });
  const [busy, setBusy] = useState(false), [error, setError] = useState(""), [saved, setSaved] = useState(false);
  useEffect(() => {
    setDraft(status.settings || { enabled: status.enabled, public_url: status.public_url || "", listen: status.listen || "127.0.0.1:7378" });
  }, [status]);
  async function save() {
    setBusy(true); setError(""); setSaved(false);
    try {
      const value = await request("/api/network", { operation: "relay_settings", ...draft, public_url: draft.public_url.trim(), listen: draft.listen.trim() }) as { iroh?: { this_relay: HostedRelayStatus } };
      if (!value.iroh?.this_relay) throw new Error(t("未能读取保存后的中继配置，请重试。", "Could not read the saved relay configuration. Please retry."));
      onSaved(value.iroh.this_relay); setSaved(true);
    } catch (problem) { setError(problem instanceof Error ? problem.message : String(problem)); }
    finally { setBusy(false); }
  }
  function change(value: Partial<RelayConfig>) { setDraft(current => ({ ...current, ...value })); setSaved(false); }
  return <section className="cx-settings-group" aria-label={t("这台设备作为中继", "This device as a relay")}>
    <h2>{t("这台设备作为中继", "This device as a relay")}</h2>
    <div className="cx-setting-row">
      <p className="cx-grow cx-muted cx-small">{t("将这台设备的中继入口暴露到公网后，其他已配对设备就能通过它中继连接。连接始终端到端加密。", "Expose this device’s relay to the public internet so your paired devices can use it to relay connections. Connections remain end-to-end encrypted.")}</p>
      <Pill tone={status.enabled ? "good" : ""}>{status.enabled ? t("正在运行", "Running") : t("未运行", "Not running")}</Pill>
    </div>
    <form className="cx-setting-row cx-stack" onSubmit={event => { event.preventDefault(); void save(); }}>
      <Switch label={t("启用本机中继", "Enable this device’s relay")} detail={t("保存后重启 Cranibox 生效。", "Save and restart Cranibox to apply.")} checked={draft.enabled} disabled={busy} onChange={enabled => change({ enabled })} />
      <label className="cx-field">{t("公网 HTTPS 地址", "Public HTTPS address")}<input type="url" placeholder="https://relay.example.com" spellCheck={false} autoComplete="off" required={draft.enabled} value={draft.public_url} disabled={busy} onChange={event => change({ public_url: event.target.value })} /></label>
      <p className="cx-muted cx-small">{t("填写域名根地址。将该域名通过 HTTPS 反向代理或隧道转发到下面的中继监听地址，并开启 WebSocket 转发。", "Enter the domain’s root address. Forward it through an HTTPS reverse proxy or tunnel to the relay listener below, with WebSocket forwarding enabled.")}</p>
      <label className="cx-field">{t("中继监听地址与端口", "Relay listener address and port")}<input type="text" placeholder="127.0.0.1:7378" spellCheck={false} autoComplete="off" required value={draft.listen} disabled={busy} onChange={event => change({ listen: event.target.value })} /></label>
      <p className="cx-muted cx-small">{t("代理在本机时使用 127.0.0.1:7378；代理在其他设备时，可使用 0.0.0.0:7378。该端口只提供中继和配对入口，管理界面使用自己的地址。", "Use 127.0.0.1:7378 for a proxy on this device, or 0.0.0.0:7378 for a proxy on another device. This port serves relay and pairing requests; the management interface uses its own address.")}</p>
      <p className="cx-muted cx-small">{t("运行状态仅表示本机中继服务已启动；公网可用性取决于代理和网络配置。", "Running means the local relay service has started; public access depends on your proxy and network configuration.")}</p>
      {status.enabled && status.public_url && <p className="cx-muted cx-small">{t("当前运行地址：", "Currently running at: ")}<span className="cx-mono">{status.public_url}</span></p>}
      {error && <Alert>{error}</Alert>}
      {(status.restart_required || saved) && <p className="cx-notice" role="status">{status.restart_required ? t("配置已保存，重启 Cranibox 后生效。当前运行状态如上。", "Configuration saved. Restart Cranibox to apply. The current running state is shown above.") : t("中继配置已保存。", "Relay configuration saved.")}</p>}
      <div className="cx-actions"><button className="cx-btn primary" type="submit" disabled={busy}>{busy ? t("正在保存…", "Saving…") : t("保存中继配置", "Save relay configuration")}</button></div>
    </form>
  </section>;
}
