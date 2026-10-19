import { invoke } from '@tauri-apps/api/core';
import { createRoot } from 'react-dom/client';
import { useEffect, useMemo, useRef, useState } from 'react';
import { Boxes, Check, Copy, FolderGit2, ListTodo, MessageSquare, Monitor, Network, RefreshCw, Settings as SettingsIcon, ShieldCheck, Terminal, X } from 'lucide-react';
import './style.css';
import { DeviceTasksView } from '../../../addons/control-center/ui/src/tasks/DeviceTasksView';
import { DeviceMessagesView } from '../../../addons/control-center/ui/src/messages/DeviceMessagesView';
import { DirectDevicesView, type DirectClient } from '../../../addons/control-center/ui/src/devices/DirectDevicesView';
import { NetworkProvidersView, type NetworkClient } from '../../../addons/control-center/ui/src/devices/NetworkProvidersView';
import type { ProjectClient } from '../../../addons/control-center/ui/src/projects/ProjectsView';
import { FederatedProjectsView } from '../../../addons/control-center/ui/src/projects/FederatedProjectsView';
import { TerminalWindow, TerminalWindowToggle } from '../../../addons/control-center/ui/src/terminal/TerminalWindow';
import { TerminalWindowProvider, useTerminalWindow } from '../../../addons/control-center/ui/src/terminal/TerminalWindowContext';
import { directTarget, localTarget } from '../../../addons/control-center/ui/src/terminal/TerminalEnvironment';
import { DesktopTerminalEnvironment } from './TerminalEnvironment';
import { RemoteDesktopsView } from '../../../addons/control-center/ui/src/desktop/RemoteDesktopsView';
import type { DesktopServiceClient } from '../../../addons/control-center/ui/src/desktop/DesktopServicePanel';
import { connectDesktop } from './DesktopTransport';
import { SpacesView, type SpacesClient } from '../../../addons/control-center/ui/src/spaces/SpacesView';
import '../../../addons/control-center/ui/src/terminal/terminal.css';
import '../../../addons/control-center/ui/src/terminal/window.css';
import './terminal.css';

type Settings = { server: string; id: string; name: string | null; token_file: string; allow_insecure: boolean };
type Snapshot = { settings: Settings | null; local?: { id: string; online: boolean }; status: { state: string; message: string | null } | null; error: string | null; autostart: boolean; quit_requested: boolean };
type Peer = { desktop_relay?: boolean; id: string; aliases?: string[]; name?: string; os?: string; arch?: string; online: boolean; terminal_sessions?: boolean; mesh_group?: string | null };
const stateLabel: Record<string, string> = { connected: '已连接', connecting: '正在连接', reconnecting: '等待网络恢复', failed: '启动失败', stopped: '已停止' };
const initialForm = { server: '', id: '', name: '', token: '', allow_insecure: false, autostart: true };
function App() {
  const { openTerminal } = useTerminalWindow();
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null), [form, setForm] = useState(initialForm);
  const [tab, setTab] = useState<'desktops' | 'devices' | 'spaces' | 'projects' | 'tasks' | 'messages' | 'settings' | 'agent'>('devices');
  const [peers, setPeers] = useState<Peer[]>([]), [peerError, setPeerError] = useState('');
  const [error, setError] = useState(''), [busy, setBusy] = useState(false), [saved, setSaved] = useState(false);
  const [mcp, setMcp] = useState(''), [copied, setCopied] = useState(false), [quitOpen, setQuitOpen] = useState(false);
  const [includeRuns, setIncludeRuns] = useState(false), [mcpLoading, setMcpLoading] = useState(true), [mcpError, setMcpError] = useState('');
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
          setForm({ ...initialForm, ...value.settings, id: value.settings?.id || value.local?.id || '', name: value.settings?.name || '', autostart: value.autostart });

        }
      } catch (error) { if (!disposed) setError(String(error)); }
      finally { active = false; }
    }
    void poll(); const timer = setInterval(poll, 1000); document.addEventListener('visibilitychange', poll);
    return () => { disposed = true; clearInterval(timer); document.removeEventListener('visibilitychange', poll); };
  }, []);
  useEffect(() => {
    if (snapshot?.status?.state !== 'connected' || (tab !== 'devices' && tab !== 'projects' && tab !== 'desktops')) return;
    void refreshPeers(); const timer = setInterval(() => { if (!document.hidden) void refreshPeers(); }, 10000);
    return () => clearInterval(timer);
  }, [snapshot?.status?.state, tab]);
  useEffect(() => {
    let disposed = false;
    setMcp(''); setCopied(false); setMcpLoading(true); setMcpError('');
    void invoke('mcp_config', { includeRuns }).then(value => { if (!disposed) setMcp(JSON.stringify(value, null, 2)); })
      .catch(error => { if (!disposed) setMcpError(String(error)); }).finally(() => { if (!disposed) setMcpLoading(false); });
    return () => { disposed = true; };
  }, [includeRuns]);
  useEffect(() => {
    const element = dialog.current; if (!element) return;
    if (quitOpen && !element.open) element.showModal();
    else if (!quitOpen && element.open) element.close();
  }, [quitOpen]);
  async function cancelQuit() { cancelling.current = true; try { await invoke('dismiss_quit'); setQuitOpen(false); } finally { cancelling.current = false; } }
  async function save(event: React.FormEvent) {
    event.preventDefault(); if (busy) return;
    setBusy(true); setSaved(false); setError('');
    try { await invoke('configure', { input: form }); setForm(previous => ({ ...previous, token: '' })); setSaved(true); setTab('devices'); }
    catch (error) { setError(String(error)); } finally { setBusy(false); }
  }
  async function reconnect() { setBusy(true); setError(''); try { await invoke('reconnect'); } catch (error) { setError(String(error)); } finally { setBusy(false); } }
  const projectClient = useMemo<ProjectClient>(() => ({
    projects: command => invoke('projects', { command }),
    terminals: (node, command) => invoke('terminal_sessions', { node, command }),
    open: (node, session) => { if (!node) throw Error('此项目没有可访问的设备'); openTerminal({ node, session: session.id }); },
  }), [openTerminal]);
  const directClient = useMemo<DirectClient>(() => command => invoke('direct_peers', { command }), []);
  const networkClient = useMemo<NetworkClient>(() => command => invoke('network', { command }), []);
  const spacesClient = useMemo<SpacesClient>(() => command => invoke('spaces', { command }), []);
  const desktopServiceClient = useMemo<DesktopServiceClient>(() => command => invoke('desktop_service', { command }), []);
  const local = useMemo(() => snapshot?.local?.online ? { id: snapshot.local.id, name: '这台设备', client: {
    projects: (command: Record<string, unknown>) => invoke('local_projects', { command }),
    terminals: (_node: string | null, command: Record<string, unknown>) => invoke('local_sessions', { command }),
    open: projectClient.open,
  } as ProjectClient } : undefined, [snapshot?.local?.id, snapshot?.local?.online, projectClient]);
  async function setAutostart(enabled: boolean) {
    setBusy(true); setError('');
    try { await invoke('set_autostart', { enabled }); setForm(previous => ({ ...previous, autostart: enabled })); }
    catch (error) { setError(String(error)); } finally { setBusy(false); }
  }
  const online = snapshot?.status?.state === 'connected';
  const standalone = !snapshot?.settings && !!snapshot?.local?.online;
  const devices = online ? peers : [];
  return <DesktopTerminalEnvironment devices={devices} local={snapshot?.local}><div className="app">
    <header><div className="brand"><span className="brand-icon"><Network aria-hidden="true" /></span><div><strong>Cranibox Node</strong><span>这台设备的 Agent 接入点</span></div></div><div className="app-header-actions"><TerminalWindowToggle /><button className="icon-button" aria-label="收起到系统托盘" title="收起到系统托盘" onClick={() => void invoke('hide_window').catch(error => setError(String(error)))}><X aria-hidden="true" /></button></div></header>
    <section className="connection" aria-label="节点连接状态"><div className="connection-main"><span className={'status-dot ' + (online || standalone ? 'online' : '')} /><div><h1>{snapshot?.settings?.name || snapshot?.settings?.id || (standalone ? '这台设备' : '连接你的设备')}</h1><p role="status">{standalone ? '本机服务运行中 · Workspace 中继为可选连接' : stateLabel[snapshot?.status?.state || ''] || 'Workspace 中继未配置，可使用直连设备'}{snapshot?.settings && <span className="endpoint"> · {snapshot.settings.server}</span>}</p></div></div><button disabled={!snapshot?.settings || busy} onClick={reconnect}><RefreshCw aria-hidden="true" size={16} />重新连接</button></section>
    <p className="service-note">关闭窗口后继续在托盘运行。断网会自动重连，现有 Shell 保持运行。</p>
    {(error || snapshot?.error || snapshot?.status?.message) && <p className="error" role="alert">{error || snapshot?.error || snapshot?.status?.message}</p>}
    {saved && <p className="success" role="status"><Check size={16} aria-hidden="true" />设置已保存，后台节点正在连接。</p>}
    <nav aria-label="客户端页面">{([['desktops', Monitor, '远程桌面'], ['spaces', Boxes, '虚拟空间'], ['devices', Network, '设备网络'], ['projects', FolderGit2, '项目与会话'], ['tasks', ListTodo, '协作任务'], ['messages', MessageSquare, '设备消息'], ['settings', SettingsIcon, '连接设置'], ['agent', ShieldCheck, 'Agent 接入']] as const).map(([id, Icon, title]) => <button key={id} aria-current={tab === id ? 'page' : undefined} onClick={() => setTab(id)}><Icon size={18} aria-hidden="true" />{title}</button>)}</nav>
    <main>
      {tab === 'spaces' && <SpacesView onOpenApplication={(id, application) => invoke<void>('space_application', { id, application })} direct={directClient} onDevices={() => setTab('devices')} client={spacesClient} language="zh-cn" onOpenTerminal={node => openTerminal({ node: directTarget(node), create: true })} onProjects={() => setTab('projects')} />}
      {tab === 'desktops' && <RemoteDesktopsView relayNodes={devices} client={directClient} serviceClient={desktopServiceClient} connect={connectDesktop} language="zh-cn" openGuide={guide => invoke('open_desktop_guide', { guide })} onAccess={() => setTab('devices')} local={snapshot?.local?.online ? { id: snapshot.local.id, name: '这台设备' } : undefined} />}
      {tab === 'devices' && <>{snapshot?.local && <section aria-label="这台设备"><div className="section-heading"><div><h2>这台设备</h2><p>本机项目与 Shell 独立运行，无需配置 Workspace 中继。</p></div><button disabled={!snapshot.local.online} onClick={() => openTerminal({ node: localTarget(snapshot.local!.id), create: true })}><Terminal size={16} aria-hidden="true" />本机终端</button></div><p className="muted">{snapshot.local.id}</p></section>}<NetworkProvidersView client={networkClient} language="zh-cn" openExternal={url => invoke('open_network_link', { url })} /><DirectDevicesView client={directClient} language="zh-cn" onOpenTerminal={node => openTerminal({ node: directTarget(node), create: true })} /><section><div className="section-heading"><div><h2>可访问的设备</h2><p>同组设备通过 Workspace 中继互访；支持公网入口或 Tailscale 私网入口。</p></div><button disabled={!online} onClick={() => void refreshPeers()} aria-label="刷新设备"><RefreshCw size={16} aria-hidden="true" /></button></div>
        {peerError && online && <p className="error" role="alert">{peerError}</p>}
        {devices.length ? <ul className="devices">{devices.map(peer => <li key={peer.id}><span className="device-icon"><Monitor aria-hidden="true" /></span><div className="device-detail"><strong>{peer.name || peer.id}{(peer.id === snapshot?.local?.id || peer.id === snapshot?.settings?.id || (!!snapshot?.settings?.id && peer.aliases?.includes(snapshot.settings.id))) && <span className="tag">本机</span>}</strong><p>{[peer.id, peer.os, peer.arch].filter(Boolean).join(' · ')}</p></div><div className="device-status"><span className={'pill ' + (peer.online ? 'online' : '')}>{peer.online ? '在线' : '离线'}</span><span>{peer.mesh_group || '未分组'}</span></div><button disabled={!peer.online || !peer.terminal_sessions} aria-label={'打开 ' + (peer.name || peer.id) + ' 的终端'} title={peer.terminal_sessions ? undefined : '此设备需要升级客户端以支持持久终端'} onClick={() => openTerminal({ node: peer.id, create: true })}><Terminal size={16} aria-hidden="true" />终端</button></li>)}</ul> : <div className="empty"><Network size={32} aria-hidden="true" /><h3>{online ? '暂无可访问的设备' : '连接后显示设备网络'}</h3><p>{online ? '在 Workspace 网页将可信设备加入同一设备组。' : '填写服务端地址和节点凭据，客户端会保持后台连接。'}</p><button onClick={() => setTab(online ? 'agent' : 'settings')}>{online ? '配置 Agent 接入' : '连接设置'}</button></div>}
        <aside><ShieldCheck size={20} aria-hidden="true" /><p>设备组由 Workspace 管理。同组 Agent 可以在其他设备执行命令；客户端使用自己的节点凭据。</p></aside></section></>}
      {tab === 'projects' && <FederatedProjectsView client={projectClient} direct={directClient} local={local} gatewayEnabled={!!snapshot?.settings} language="zh-cn" devices={devices} />}
      {tab === 'tasks' && <DeviceTasksView client={directClient} language="zh-cn" onDevices={() => setTab('devices')} onMessages={() => setTab('messages')} />}
      {tab === 'messages' && <DeviceMessagesView client={directClient} language="zh-cn" onDevices={() => setTab('devices')} />}
      {tab === 'settings' && <section><h2>连接设置</h2><p className="muted">Workspace 中继为可选连接。本机项目、Shell 和直连设备无需中继；修改连接设置会保留本机 Shell。</p><label className="checkbox"><input type="checkbox" disabled={busy} checked={form.autostart} onChange={event => void setAutostart(event.target.checked)} /><div><strong>登录系统后自动启动</strong><span>无需配置中继，静默进入托盘并启动本机服务。</span></div></label><p className="muted">需要接入 Workspace 时，先在网页的“远程节点”页面注册设备，再填入地址和 Token。</p><form onSubmit={save}>
        <label>服务端地址<input aria-label="服务端地址" type="url" required placeholder="wss://workspace.example.com" value={form.server} onChange={event => setForm({ ...form, server: event.target.value })} /><span>公网使用 wss://；Tailscale 可填写 Workspace 的私网地址。</span></label>
        <div className="form-row"><label>节点 ID<input aria-label="节点 ID" aria-describedby={snapshot?.local ? 'registration-id-help' : undefined} required pattern="[A-Za-z0-9_-]{1,64}" maxLength={64} placeholder="node-…" value={form.id} onChange={event => setForm({ ...form, id: event.target.value })} />{snapshot?.local && <span id="registration-id-help">新注册请使用本机 NodeID：{snapshot.local.id}。已有旧 ID 可在 Workspace 网页迁移。</span>}</label><label>显示名称<input maxLength={128} placeholder="我的笔记本" value={form.name} onChange={event => setForm({ ...form, name: event.target.value })} /></label></div>
        <label>节点 Token<input aria-label="节点 Token" type="password" autoComplete="off" required={!snapshot?.settings} pattern="[A-Fa-f0-9]{64}" maxLength={64} placeholder={snapshot?.settings ? '已保存；留空保留当前 Token' : '注册节点获得的 64 位 Token'} value={form.token} onChange={event => setForm({ ...form, token: event.target.value })} /><span>仅保存在本机配置目录；不会放进命令参数或 MCP 配置。</span></label>
        <label className="checkbox"><input type="checkbox" checked={form.allow_insecure} onChange={event => setForm({ ...form, allow_insecure: event.target.checked })} /><div><strong>允许可信私网的 ws:// 连接</strong><span>用于 Tailscale 私网或可信局域网。公网保持 TLS。</span></div></label>
        <div className="form-actions"><button className="primary" disabled={busy} type="submit">{busy ? '正在保存…' : '保存并连接'}</button></div></form></section>}
      {tab === 'agent' && <section><h2>让这台设备上的 Agent 访问其他设备</h2><p className="muted">将下面的 MCP 配置添加到本机 Agent。cranibox-messages 提供本机会话登记、设备消息与协作任务；cranibox-devices 提供通过 Workspace 接入的设备、命令和任务工具。</p>
        <label className="checkbox"><input type="checkbox" checked={includeRuns} onChange={event => setIncludeRuns(event.target.checked)} aria-describedby="runs-mcp-help" /><div><strong>添加 Agent 后台运行工具</strong><span id="runs-mcp-help">加入 cranibox-runs，让 Agent 明确选择设备后启动程序、查看输出和停止运行。目标须已启动节点服务；远端须授予命令执行权限。添加配置本身不会启动程序。</span></div></label>
        <div className="code-heading"><span>{mcpLoading ? '正在生成 MCP 配置…' : 'MCP 配置'}</span><button disabled={!mcp || mcpLoading} onClick={async () => { try { await navigator.clipboard.writeText(mcp); setCopied(true); setMcpError(''); } catch { setMcpError('复制失败，请在下方选中配置并复制。'); } }}><Copy size={16} aria-hidden="true" />{copied ? '已复制' : '复制配置'}</button></div>
        {mcpError && <p className="error" role="alert">{mcpError}</p>}<textarea className="code" aria-label="MCP 配置" aria-busy={mcpLoading} readOnly value={mcp} rows={13} spellCheck={false} />
        <aside><ShieldCheck size={20} aria-hidden="true" /><p>消息使用本机 NodeID 和目标单独授予的消息权限，可独立于 Workspace 使用。任务需由 Agent 明确领取并报告结果，消息与任务不会自动执行；通过 Workspace 的命令访问范围仍由设备组决定。{includeRuns && ' 后台运行工具使用目标节点账户执行程序；每次调用需指定设备 ID，关闭 Agent 或页面后目标节点继续持有运行。'}</p></aside></section>}
    </main><footer><span>{snapshot?.autostart ? '已启用登录自启动' : '登录自启动未开启'}</span><button className="text-button" onClick={() => setQuitOpen(true)}>退出客户端</button></footer>
    <TerminalWindow suspended={quitOpen} /><dialog ref={dialog} onCancel={event => { event.preventDefault(); void cancelQuit(); }}><h2>退出节点客户端？</h2><p>退出会结束本机的 Shell 会话，其他设备也将无法访问这台设备。</p><p className="muted">只想关闭窗口，请选择“继续运行”。</p><div className="dialog-actions"><button autoFocus onClick={() => void cancelQuit()}>继续运行</button><button className="danger" onClick={() => void invoke('quit').catch(error => setError(String(error)))}>退出并结束会话</button></div></dialog>
  </div></DesktopTerminalEnvironment>;
}
createRoot(document.getElementById('root')!).render(<TerminalWindowProvider><App /></TerminalWindowProvider>);
