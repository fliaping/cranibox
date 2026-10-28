import { Bot, MessagesSquare, Plus, Sparkles } from "lucide-react";
import { useState, type FormEvent } from "react";
import { agentNames, type SignIn } from "../personal/api";
import { useApp } from "./store";
import { Card, Empty, formatDeviceLabel, Pill, Section, Sheet } from "./ui";

const agentStyle: Record<string, { code: string; bg: string; fg: string }> = {
  "deepseek-harness": { code: "DS", bg: "rgba(37, 99, 235, 0.12)", fg: "#2563eb" },
  "codex": { code: "CX", bg: "rgba(16, 185, 129, 0.12)", fg: "#059669" },
  "claude-code": { code: "CC", bg: "rgba(217, 119, 6, 0.14)", fg: "#d97706" },
  "pi": { code: "PI", bg: "rgba(124, 58, 237, 0.12)", fg: "#7c3aed" },
  "hermes": { code: "HM", bg: "rgba(225, 29, 72, 0.12)", fg: "#e11d48" },
  "openclaw": { code: "OC", bg: "rgba(8, 145, 178, 0.12)", fg: "#0891b2" },
};

export function AgentPicker() {
  const { t, agents, models, busy, run, api } = useApp();
  const available = models.filter(model => model.availability.kind !== "unavailable");
  return <div className="cx-list">{agents.map(agent => <div className="cx-list-row pa-agent-row" key={agent.id}>
    {(() => { const s = agentStyle[agent.id]; return s ? <span className="cx-card-icon" style={{ background: s.bg, color: s.fg, fontWeight: 700, fontSize: 12, fontFamily: "var(--mono)" }} aria-hidden="true">{s.code}</span> : <span className="cx-card-icon"><Bot size={19} aria-hidden="true" /></span>; })()}
    <div className="cx-grow"><strong>{agentNames[agent.id] || agent.id}</strong>
      <div className="cx-row cx-wrap">{agent.recommended && <Pill tone="good">{t("推荐 · 默认", "Recommended · Default")}</Pill>}<small className="cx-muted">{agent.installed ? t("已安装", "Installed") : t("未安装", "Not installed")}</small></div></div>
    <label><span className="cx-sr">{(agentNames[agent.id] || agent.id) + t("使用的模型", " model")}</span>
      <select className="cx-select" disabled={busy || (!available.length && !agent.managed)} value={agent.managed ? agent.model || "auto" : "unmanaged"}
        onChange={event => void run(() => api.call("PUT", "/agents/" + agent.id, { managed: event.target.value !== "unmanaged", model: ["auto", "unmanaged"].includes(event.target.value) ? null : event.target.value }))}>
        <option value="auto">{t("自动", "Automatic")}</option>
        <option value="unmanaged">{t("自行管理", "Manage myself")}</option>
        {available.map(model => <option key={model.id} value={model.id}>{model.display || model.id}</option>)}
      </select></label>
  </div>)}</div>;
}
export function Models({ chatUrl, adding, setAdding }: { chatUrl: string; adding: boolean; setAdding: (value: boolean) => void }) {
  const { t, accounts, devices, thisDevice, agents, busy, run, api } = useApp();
  const [chat, setChat] = useState(false), [preset, setPreset] = useState<string | null>(null);
  const dsh = agents.find(agent => agent.id === "deepseek-harness");
  const mine = devices.filter(device => !device.guest);
  const enable = (account: string, body: object) => run(() => api.call("POST", "/models/accounts/" + account + "/enable", body));
  return <>
    {dsh?.installed && <Section title={t("对话", "Conversation")} description={t("DeepSeek Harness 是默认 Agent，可以调用你所有设备上的能力。", "DeepSeek Harness is the default agent. It can use capabilities on all your devices.")}
      actions={<button className="cx-btn" aria-expanded={chat} onClick={() => setChat(!chat)}><MessagesSquare size={16} aria-hidden="true" />{chat ? t("收起对话", "Hide conversation") : t("打开对话", "Open conversation")}</button>}>
      {chat && <iframe className="cx-frame" title={t("与 DeepSeek Harness 对话", "Chat with DeepSeek Harness")} src={chatUrl} allow="clipboard-write" sandbox="allow-scripts allow-same-origin allow-forms allow-downloads allow-popups" />}
    </Section>}
    <Section title={t("模型订阅", "Model subscriptions")} description={t("连接你的订阅，再选择哪些设备可以使用。凭据加密同步，只在你的设备之间流转。", "Connect your subscriptions, then choose which devices may use them. Credentials sync encrypted, only between your devices.")}
      actions={<button className="cx-btn primary" onClick={() => { setPreset(null); setAdding(true); }}><Plus size={17} aria-hidden="true" />{t("添加订阅", "Add subscription")}</button>}>
      {accounts.length ? <div className="cx-grid">{accounts.map(account => <Card key={account.id} className="pa-card">
        <div className="cx-card-head"><span className="cx-card-icon"><Sparkles size={19} aria-hidden="true" /></span>
          <div className="cx-grow"><h3>{account.label || account.provider}</h3><small className="cx-muted">{account.provider}</small></div>
          <Pill tone={account.state.kind === "ready" ? "good" : "warn"}>{account.state.kind === "ready" ? t("可用", "Ready") : account.state.kind === "needs_sign_in" ? t("需重新登录", "Sign in again") : account.state.kind === "quota_exhausted" ? t("额度已用完", "Quota used") : t("需要检查", "Needs attention")}</Pill></div>
        {account.kind === "claude_oauth" ? <>
          <p className="cx-muted">{t("凭据只保留在一台设备上，不同步到其他设备。", "Credentials stay on one device and never sync to the others.")}</p>
          <label className="cx-field">{t("保存 Claude 凭据的设备", "Device holding Claude credentials")}<select disabled={busy} value={devices.find(device => device.badges.includes("claude"))?.node_id || account.home_node}
            onChange={event => void run(() => api.call("POST", "/models/claude/set-node", { node: event.target.value }))}>{mine.map(device => <option key={device.node_id} value={device.node_id}>{formatDeviceLabel(device.name, device.node_id, t).label}</option>)}</select></label>
        </> : <div style={{ marginTop: 8 }}>
          <div className="cx-segment" role="group" aria-label={t("可用设备范围", "Device scope")} style={{ marginBottom: 8 }}>
            <button type="button" disabled={busy || !thisDevice} aria-pressed={!account.all_nodes && account.enabled_on.includes(thisDevice?.node_id || "") && account.enabled_on.length <= 1} onClick={() => void enable(account.id, { node: thisDevice?.node_id, all_nodes: false, on: true })}>{t("仅本机", "This device")}</button>
            <button type="button" disabled={busy} aria-pressed={account.all_nodes} onClick={() => void enable(account.id, { node: null, all_nodes: true, on: true })}>{t("所有设备", "All devices")}</button>
            <button type="button" disabled={busy} aria-pressed={!account.all_nodes && account.enabled_on.length > 1} onClick={() => void enable(account.id, { node: null, all_nodes: false, on: false })}>{t("指定设备", "Selected devices")}</button>
          </div>
          <div className="cx-sr">
            <label className="cx-check"><input type="checkbox" disabled={busy || !thisDevice} checked={account.all_nodes || account.enabled_on.includes(thisDevice?.node_id || "")} onChange={event => void enable(account.id, { node: thisDevice?.node_id, all_nodes: null, on: event.target.checked })} />{t("在本机启用", "Enable on this device")}</label>
            <label className="cx-check"><input type="checkbox" disabled={busy} checked={account.all_nodes} onChange={event => void enable(account.id, { node: null, all_nodes: true, on: event.target.checked })} />{t("在所有设备启用", "Enable on all devices")}</label>
          </div>
          <details open={!account.all_nodes && account.enabled_on.length > 1}><summary>{t("逐台选择设备", "Choose devices individually")}</summary>
            {mine.map(device => { const info = formatDeviceLabel(device.name, device.node_id, t); return <label className="cx-check" key={device.node_id}><input type="checkbox" disabled={busy || account.all_nodes} checked={account.all_nodes || account.enabled_on.includes(device.node_id)} onChange={event => void enable(account.id, { node: device.node_id, all_nodes: null, on: event.target.checked })} />{info.label}{info.rawHidden && <span className="cx-sr"> {info.rawHidden}</span>}</label>; })}
          </details>
        </div>}
        {account.state.kind === "needs_sign_in" && <button className="cx-btn" style={{ marginTop: 12 }} onClick={() => { setPreset(account.provider); setAdding(true); }}>{t("重新登录", "Sign in again")}</button>}
      </Card>)}</div> : <Empty icon={Sparkles} title={t("还没有订阅", "No subscriptions yet")} actions={<button className="cx-btn primary" onClick={() => setAdding(true)}>{t("添加第一个订阅", "Add your first subscription")}</button>}>{t("从你已经在用的账号开始。", "Start with an account you already use.")}</Empty>}
    </Section>
    <Section title={t("各 Agent 用哪个模型", "Models for your agents")}><AgentPicker /></Section>
    {adding && <SubscriptionSheet provider={preset} onClose={() => setAdding(false)} />}
  </>;
}
function SubscriptionSheet({ provider: initial, onClose }: { provider: string | null; onClose: () => void }) {
  const { t, api, presets, refresh, openExternal, run, busy: running } = useApp();
  const [tab, setTab] = useState("signin"), [provider, setProvider] = useState(initial || "codex"), [region, setRegion] = useState("");
  const [key, setKey] = useState(""), [label, setLabel] = useState(""), [endpoint, setEndpoint] = useState(""), [customModel, setCustomModel] = useState("");
  const [signIn, setSignIn] = useState<SignIn | null>(null), [callback, setCallback] = useState(""), [error, setError] = useState(""), [pending, setPending] = useState(false);
  const busy = pending || running;
  async function submit(event: FormEvent) {
    event.preventDefault(); setPending(true); setError("");
    try {
      if (tab === "signin") {
        const flow = await api.call<SignIn>("POST", "/models/signin/start", { provider, region: region || null });
        setSignIn(flow); await openExternal(flow.url);
      } else {
        const body = tab === "custom"
          ? { kind: "custom", label, key, endpoints: { chat: endpoint }, models: customModel.split(",").map(item => item.trim()).filter(Boolean) }
          : { kind: "api_key", preset: provider, key, label: label || null, region: region || null };
        await api.call("POST", "/models/accounts", body);
        setKey(""); onClose(); await refresh();
      }
    } catch (problem) { setError(problem instanceof Error ? problem.message : String(problem)); }
    finally { setPending(false); }
  }
  const regions = presets.find(item => item.id === provider)?.region || [];
  return <Sheet title={t("添加订阅", "Add subscription")} t={t} onClose={() => { setKey(""); onClose(); }}>
    <div className="cx-segment" role="group" aria-label={t("添加方式", "How to add")}>{[["signin", t("账号登录", "Sign in")], ["plan", t("国内套餐", "Regional plans")], ["key", t("粘贴 Key", "API key")], ["custom", t("自定义", "Custom")]].map(([id, title]) =>
      <button key={id} aria-pressed={tab === id} onClick={() => { setTab(id); setSignIn(null); setRegion(""); setProvider(id === "plan" ? presets.find(item => item.region.includes("cn"))?.id || "deepseek" : id === "key" ? "deepseek" : "codex"); }}>{title}</button>)}</div>
    {!signIn ? <form onSubmit={event => void submit(event)}>
      {tab !== "custom" && <label className="cx-field">{t("服务商", "Provider")}<select value={provider} onChange={event => { setProvider(event.target.value); setRegion(""); }}>
        {(tab === "signin" ? [["codex", "ChatGPT"], ["copilot", "GitHub Copilot"], ["gemini", "Gemini"], ["kimi-code", "Kimi Code"], ["claude", "Claude"]] : presets.filter(item => tab !== "plan" || item.region.includes("cn")).map(item => [item.id, item.label])).map(([id, name]) => <option key={id} value={id}>{name}</option>)}
      </select></label>}
      {tab !== "signin" && tab !== "custom" && regions.length > 1 && <label className="cx-field">{t("服务区域", "Service region")}<select value={region} onChange={event => setRegion(event.target.value)}>
        <option value="">{t("默认区域", "Default region")}</option>{regions.map(item => <option key={item} value={item}>{item === "cn" ? t("中国大陆", "China") : item === "global" ? t("国际", "Global") : item}</option>)}</select></label>}
      {provider === "claude" && tab === "signin" && <p className="cx-muted">{t("Claude 凭据只保存在本机，不会同步到其他设备。", "Claude credentials stay on this device and never sync.")}</p>}
      {tab !== "signin" && <><label className="cx-field">{t("名称（可选）", "Name (optional)")}<input value={label} onChange={event => setLabel(event.target.value)} autoComplete="off" /></label>
        <label className="cx-field">{t("API Key", "API key")}<input value={key} type="password" autoComplete="off" onChange={event => setKey(event.target.value)} required /></label></>}
      {tab === "custom" && <><label className="cx-field">{t("服务地址", "API base URL")}<input type="url" required value={endpoint} onChange={event => setEndpoint(event.target.value)} placeholder="https://api.example.com" /></label>
        <label className="cx-field">{t("模型 ID（逗号分隔）", "Model IDs (comma separated)")}<input required value={customModel} onChange={event => setCustomModel(event.target.value)} /></label></>}
      <button className="cx-btn primary full" disabled={busy}>{busy ? t("请稍候…", "Please wait…") : tab === "signin" ? t("在浏览器中继续", "Continue in browser") : t("保存订阅", "Save subscription")}</button>
    </form> : <div className="cx-stack">
      <h3>{t("完成浏览器中的登录", "Finish signing in in your browser")}</h3>
      {signIn.code && <div className="cx-sas"><code>{signIn.code}</code></div>}
      <button className="cx-btn" onClick={() => void openExternal(signIn.url)}>{t("再次打开登录页", "Open sign-in page again")}</button>
      <form onSubmit={event => { event.preventDefault(); void run(async () => {
        await api.call("POST", "/models/signin/" + encodeURIComponent(signIn.flow_id) + "/complete", signIn.kind === "device_code" ? { callback_url: null, code: null } : /^https?:/.test(callback) ? { callback_url: callback, code: null } : { callback_url: null, code: callback });
        onClose();
      }); }}>
        {signIn.kind !== "device_code" && <label className="cx-field">{t("回调链接或授权码", "Callback URL or authorization code")}<input required value={callback} onChange={event => setCallback(event.target.value)} /></label>}
        <button className="cx-btn primary full" disabled={busy}>{t("完成登录", "Complete sign-in")}</button>
      </form>
    </div>}
    {error && <p className="cx-error" role="alert">{error}</p>}
  </Sheet>;
}
