import { invoke } from '@tauri-apps/api/core';
import { createRoot } from 'react-dom/client';
import { useEffect, useRef, useState } from 'react';
import { Check, Copy, Monitor, Network, RefreshCw, Settings as SettingsIcon, ShieldCheck, X } from 'lucide-react';
import './style.css';

type Settings = { server: string; id: string; name: string | null; token_file: string; allow_insecure: boolean };
type Snapshot = { settings: Settings | null; status: { state: string; message: string | null } | null; error: string | null; autostart: boolean; quit_requested: boolean };
type Peer = { id: string; name?: string; os?: string; arch?: string; online: boolean; mesh_group?: string | null };
const stateLabel: Record<string, string> = { connected: '已连接', connecting: '正在连接', reconnecting: '等待网络恢复', failed: '启动失败', stopped: '已停止' };
const initialForm = { server: '', id: '', name: '', token: '', allow_insecure: false, autostart: true };
function App() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null), [form, setForm] = useState(initialForm);
  const [tab, setTab] = useState<'devices' | 'settings' | 'agent'>('devices');
  const [peers, setPeers] = useState<Peer[]>([]), [peerError, setPeerError] = useState('');
  const [error, setError] = useState(''), [busy, setBusy] = useState(false), [saved, setSaved] = useState(false);
  const [mcp, setMcp] = useState(''), [copied, setCopied] = useState(false), [quitOpen, setQuitOpen] = useState(false);
  const loaded = useRef(false), dialog = useRef<HTMLDialogElement>(null), cancelling = useRef(false);
  async function refreshPeers() {
    try { const value = await invoke<{ nodes: Peer[] }>('peers'); setPeers(value.nodes); setPeerError(''); }
    catch (error) { setPeers([]); setPeerError(String(error)); }
  }
  useEffect(() => {
    let disposed = false, active = false;
    async function poll() {
      if (active || (loaded.current && document.hidden)) return;
      active = true;
      try {
        const value = await invoke<Snapshot>('snapshot');
        if (disposed) return;
        setSnapshot(value);
        if (value.quit_requested && !cancelling.current) setQuitOpen(true);
        if (!loaded.current) {
          loaded.current = true;
          if (value.settings) setForm({ ...initialForm, ...value.settings, name: value.settings.name || '', autostart: value.autostart });
          else setTab('settings');
        }
      } catch (error) { if (!disposed) setError(String(error)); }
      finally { active = false; }
    }
    void poll(); const timer = setInterval(poll, 1000); document.addEventListener('visibilitychange', poll);
    return () => { disposed = true; clearInterval(timer); document.removeEventListener('visibilitychange', poll); };
  }, []);
  useEffect(() => {
    if (snapshot?.status?.state !== 'connected' || tab !== 'devices') return;
    void refreshPeers(); const timer = setInterval(() => { if (!document.hidden) void refreshPeers(); }, 10000);
    return () => clearInterval(timer);
  }, [snapshot?.status?.state, tab]);
  useEffect(() => { void invoke('mcp_config').then(value => setMcp(JSON.stringify(value, null, 2))).catch(error => setError(String(error))); }, []);
  useEffect(() => {
    const element = dialog.current; if (!element) return;
    if (quitOpen && !element.open) element.showModal();
    else if (!quitOpen && element.open) element.close();
  }, [quitOpen]);
  async function cancelQuit() { cancelling.current = true; try { await invoke('dismiss_quit'); setQuitOpen(false); } finally { cancelling.current = false; } }
  async function save(event: React.FormEvent) {
    event.preventDefault(); if (busy) return;
    const previous = snapshot?.settings;
    const server = form.server.trim();
    const normalizedServer = server.endsWith('/') ? server.slice(0, -1) : server;
    const changesService = previous && (previous.server !== normalizedServer || previous.id !== form.id.trim() || (previous.name || '') !== form.name.trim() || previous.allow_insecure !== form.allow_insecure);
    if (changesService && !confirm('更改节点身份或服务端会结束本机现有 Shell 会话，确认保存？')) return;
    setBusy(true); setSaved(false); setError('');
    try { await invoke('configure', { input: form }); setForm(previous => ({ ...previous, token: '' })); setSaved(true); setTab('devices'); }
    catch (error) { setError(String(error)); } finally { setBusy(false); }
  }
  async function reconnect() { setBusy(true); setError(''); try { await invoke('reconnect'); } catch (error) { setError(String(error)); } finally { setBusy(false); } }
  const online = snapshot?.status?.state === 'connected';
  const devices = online ? peers : [];
  return <div className="app">
    <header><div className="brand"><span className="brand-icon"><Network aria-hidden="true" /></span><div><strong>Cranibox Node</strong><span>这台设备的 Agent 接入点</span></div></div><button className="icon-button" aria-label="收起到系统托盘" title="收起到系统托盘" onClick={() => void invoke('hide_window').catch(error => setError(String(error)))}><X aria-hidden="true" /></button></header>
    <section className="connection" aria-label="节点连接状态"><div className="connection-main"><span className={'status-dot ' + (online ? 'online' : '')} /><div><h1>{snapshot?.settings?.name || snapshot?.settings?.id || '连接你的设备'}</h1><p role="status">{stateLabel[snapshot?.status?.state || ''] || '等待配置'}{snapshot?.settings && <span className="endpoint"> · {snapshot.settings.server}</span>}</p></div></div><button disabled={!snapshot?.settings || busy} onClick={reconnect}><RefreshCw aria-hidden="true" size={16} />重新连接</button></section>
    <p className="service-note">关闭窗口后继续在托盘运行。断网会自动重连，现有 Shell 保持运行。</p>
    {(error || snapshot?.error || snapshot?.status?.message) && <p className="error" role="alert">{error || snapshot?.error || snapshot?.status?.message}</p>}
    {saved && <p className="success" role="status"><Check size={16} aria-hidden="true" />设置已保存，后台节点正在连接。</p>}
    <nav aria-label="客户端页面">{([['devices', Network, '设备网络'], ['settings', SettingsIcon, '连接设置'], ['agent', ShieldCheck, 'Agent 接入']] as const).map(([id, Icon, title]) => <button key={id} aria-current={tab === id ? 'page' : undefined} onClick={() => setTab(id)}><Icon size={18} aria-hidden="true" />{title}</button>)}</nav>
    <main>
      {tab === 'devices' && <section><div className="section-heading"><div><h2>可访问的设备</h2><p>同组设备通过 Workspace 中继互访；支持公网入口或 Tailscale 私网入口。</p></div><button disabled={!online} onClick={() => void refreshPeers()} aria-label="刷新设备"><RefreshCw size={16} aria-hidden="true" /></button></div>
        {peerError && online && <p className="error" role="alert">{peerError}</p>}
        {devices.length ? <ul className="devices">{devices.map(peer => <li key={peer.id}><span className="device-icon"><Monitor aria-hidden="true" /></span><div className="device-detail"><strong>{peer.name || peer.id}{peer.id === snapshot?.settings?.id && <span className="tag">本机</span>}</strong><p>{[peer.id, peer.os, peer.arch].filter(Boolean).join(' · ')}</p></div><div className="device-status"><span className={'pill ' + (peer.online ? 'online' : '')}>{peer.online ? '在线' : '离线'}</span><span>{peer.mesh_group || '未分组'}</span></div></li>)}</ul> : <div className="empty"><Network size={32} aria-hidden="true" /><h3>{online ? '暂无可访问的设备' : '连接后显示设备网络'}</h3><p>{online ? '在 Workspace 网页将可信设备加入同一设备组。' : '填写服务端地址和节点凭据，客户端会保持后台连接。'}</p><button onClick={() => setTab(online ? 'agent' : 'settings')}>{online ? '配置 Agent 接入' : '连接设置'}</button></div>}
        <aside><ShieldCheck size={20} aria-hidden="true" /><p>设备组由 Workspace 管理。同组 Agent 可以在其他设备执行命令；客户端使用自己的节点凭据。</p></aside></section>}
      {tab === 'settings' && <section><h2>连接设置</h2><p className="muted">先在 Workspace 的“远程节点”页面注册设备，再填入地址和 Token。</p><form onSubmit={save}>
        <label>服务端地址<input aria-label="服务端地址" type="url" required placeholder="wss://workspace.example.com" value={form.server} onChange={event => setForm({ ...form, server: event.target.value })} /><span>公网使用 wss://；Tailscale 可填写 Workspace 的私网地址。</span></label>
        <div className="form-row"><label>节点 ID<input required pattern="[A-Za-z0-9_-]{1,64}" maxLength={64} placeholder="my-laptop" value={form.id} onChange={event => setForm({ ...form, id: event.target.value })} /></label><label>显示名称<input maxLength={128} placeholder="我的笔记本" value={form.name} onChange={event => setForm({ ...form, name: event.target.value })} /></label></div>
        <label>节点 Token<input aria-label="节点 Token" type="password" autoComplete="off" required={!snapshot?.settings} pattern="[A-Fa-f0-9]{64}" maxLength={64} placeholder={snapshot?.settings ? '已保存；留空保留当前 Token' : '注册节点获得的 64 位 Token'} value={form.token} onChange={event => setForm({ ...form, token: event.target.value })} /><span>仅保存在本机配置目录；不会放进命令参数或 MCP 配置。</span></label>
        <label className="checkbox"><input type="checkbox" checked={form.autostart} onChange={event => setForm({ ...form, autostart: event.target.checked })} /><div><strong>登录系统后自动启动</strong><span>静默进入托盘并恢复节点连接。</span></div></label>
        <label className="checkbox"><input type="checkbox" checked={form.allow_insecure} onChange={event => setForm({ ...form, allow_insecure: event.target.checked })} /><div><strong>允许可信私网的 ws:// 连接</strong><span>用于 Tailscale 私网或可信局域网。公网保持 TLS。</span></div></label>
        <div className="form-actions"><button className="primary" disabled={busy} type="submit">{busy ? '正在保存…' : '保存并连接'}</button></div></form></section>}
      {tab === 'agent' && <section><h2>让这台设备上的 Agent 访问其他设备</h2><p className="muted">将下面的 MCP 配置添加到本机 Agent。节点客户端同时提供设备列表、命令执行、任务查询和取消工具。</p><div className="code-heading"><span>MCP 配置</span><button disabled={!mcp} onClick={async () => { try { await navigator.clipboard.writeText(mcp); setCopied(true); } catch { setError('复制失败，请在下方选中配置并复制。'); } }}><Copy size={16} aria-hidden="true" />{copied ? '已复制' : '复制配置'}</button></div><textarea className="code" aria-label="MCP 配置" readOnly value={mcp} rows={13} spellCheck={false} /><aside><ShieldCheck size={20} aria-hidden="true" /><p>MCP 使用本机节点的身份，访问范围由设备组决定。无需 Workspace 管理员 Token。</p></aside></section>}
    </main><footer><span>{snapshot?.autostart ? '已启用登录自启动' : '登录自启动未开启'}</span><button className="text-button" onClick={() => setQuitOpen(true)}>退出客户端</button></footer>
    <dialog ref={dialog} onCancel={event => { event.preventDefault(); void cancelQuit(); }}><h2>退出节点客户端？</h2><p>退出会结束本机的 Shell 会话，其他设备也将无法访问这台设备。</p><p className="muted">只想关闭窗口，请选择“继续运行”。</p><div className="dialog-actions"><button autoFocus onClick={() => void cancelQuit()}>继续运行</button><button className="danger" onClick={() => void invoke('quit').catch(error => setError(String(error)))}>退出并结束会话</button></div></dialog>
  </div>;
}
createRoot(document.getElementById('root')!).render(<App />);
