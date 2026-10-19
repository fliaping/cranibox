import { listen } from '@tauri-apps/api/event';
import { PersonalApp } from '../../../addons/control-center/ui/src/personal/PersonalApp';
import { Sheet } from '../../../addons/control-center/ui/src/personal/components';
import type { Api } from '../../../addons/control-center/ui/src/personal/api';
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
function App() {
  const { openTerminal } = useTerminalWindow();
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [tab, setTab] = useState<'desktops' | 'devices' | 'spaces' | 'projects' | 'tasks' | 'messages' | 'settings' | 'agent'>(() => location.hash === '#/tasks' ? 'tasks' : 'devices');
  const [peers, setPeers] = useState<Peer[]>([]), [peerError, setPeerError] = useState('');
  const [error, setError] = useState(''), [busy, setBusy] = useState(false);
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
    try { await invoke('set_autostart', { enabled }); setSnapshot(previous => previous ? {...previous,autostart:enabled}:previous); }
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

    <nav aria-label="客户端页面">{([['desktops', Monitor, '远程桌面'], ['spaces', Boxes, '虚拟空间'], ['devices', Network, '设备网络'], ['projects', FolderGit2, '项目与会话'], ['tasks', ListTodo, '协作任务'], ['messages', MessageSquare, '设备消息'], ['settings', SettingsIcon, '本机设置'], ['agent', ShieldCheck, 'Agent 接入']] as const).map(([id, Icon, title]) => <button key={id} aria-current={tab === id ? 'page' : undefined} onClick={() => setTab(id)}><Icon size={18} aria-hidden="true" />{title}</button>)}</nav>
    <main>
      {tab === 'spaces' && <SpacesView onOpenApplication={(id, application) => invoke<void>('space_application', { id, application })} direct={directClient} onDevices={() => setTab('devices')} client={spacesClient} language="zh-cn" onOpenTerminal={node => openTerminal({ node: directTarget(node), create: true })} onProjects={() => setTab('projects')} />}
      {tab === 'desktops' && <RemoteDesktopsView relayNodes={devices} client={directClient} serviceClient={desktopServiceClient} connect={connectDesktop} language="zh-cn" openGuide={guide => invoke('open_desktop_guide', { guide })} onAccess={() => setTab('devices')} local={snapshot?.local?.online ? { id: snapshot.local.id, name: '这台设备' } : undefined} />}
      {tab === 'devices' && <>{snapshot?.local && <section aria-label="这台设备"><div className="section-heading"><div><h2>这台设备</h2><p>本机项目与 Shell 独立运行，无需配置 Workspace 中继。</p></div><button disabled={!snapshot.local.online} onClick={() => openTerminal({ node: localTarget(snapshot.local!.id), create: true })}><Terminal size={16} aria-hidden="true" />本机终端</button></div><p className="muted">{snapshot.local.id}</p></section>}<NetworkProvidersView client={networkClient} language="zh-cn" openExternal={url => invoke('open_network_link', { url })} /><DirectDevicesView client={directClient} language="zh-cn" onOpenTerminal={node => openTerminal({ node: directTarget(node), create: true })} /><section><div className="section-heading"><div><h2>可访问的设备</h2><p>同组设备通过 Workspace 中继互访；支持公网入口或 Tailscale 私网入口。</p></div><button disabled={!online} onClick={() => void refreshPeers()} aria-label="刷新设备"><RefreshCw size={16} aria-hidden="true" /></button></div>
        {peerError && online && <p className="error" role="alert">{peerError}</p>}
        {devices.length ? <ul className="devices">{devices.map(peer => <li key={peer.id}><span className="device-icon"><Monitor aria-hidden="true" /></span><div className="device-detail"><strong>{peer.name || peer.id}{(peer.id === snapshot?.local?.id || peer.id === snapshot?.settings?.id || (!!snapshot?.settings?.id && peer.aliases?.includes(snapshot.settings.id))) && <span className="tag">本机</span>}</strong><p>{[peer.id, peer.os, peer.arch].filter(Boolean).join(' · ')}</p></div><div className="device-status"><span className={'pill ' + (peer.online ? 'online' : '')}>{peer.online ? '在线' : '离线'}</span><span>{peer.mesh_group || '未分组'}</span></div><button disabled={!peer.online || !peer.terminal_sessions} aria-label={'打开 ' + (peer.name || peer.id) + ' 的终端'} title={peer.terminal_sessions ? undefined : '此设备需要升级客户端以支持持久终端'} onClick={() => openTerminal({ node: peer.id, create: true })}><Terminal size={16} aria-hidden="true" />终端</button></li>)}</ul> : <div className="empty"><Network size={32} aria-hidden="true" /><h3>{online ? '暂无可访问的设备' : '连接后显示设备网络'}</h3><p>{online ? '在 Workspace 网页将可信设备加入同一设备组。' : '在设备页面添加设备，完成核对后即可访问。'}</p><button onClick={() => setTab(online ? 'agent' : 'settings')}>{online ? '配置 Agent 接入' : '本机设置'}</button></div>}
        <aside><ShieldCheck size={20} aria-hidden="true" /><p>设备组由 Workspace 管理。同组 Agent 可以在其他设备执行命令；客户端使用自己的节点凭据。</p></aside></section></>}
      {tab === 'projects' && <FederatedProjectsView client={projectClient} direct={directClient} local={local} gatewayEnabled={!!snapshot?.settings} language="zh-cn" devices={devices} />}
      {tab === 'tasks' && <DeviceTasksView client={directClient} language="zh-cn" onDevices={() => setTab('devices')} onMessages={() => setTab('messages')} />}
      {tab === 'messages' && <DeviceMessagesView client={directClient} language="zh-cn" onDevices={() => setTab('devices')} />}
      {tab === 'settings' && <section><h2>本机设置</h2><label className="checkbox"><input type="checkbox" disabled={busy} checked={!!snapshot?.autostart} onChange={event => void setAutostart(event.target.checked)} /><div><strong>登录系统后自动启动</strong><span>在托盘运行本机服务。</span></div></label></section>}
      {tab === 'agent' && <section><h2>让这台设备上的 Agent 访问其他设备</h2><p className="muted">将下面的 MCP 配置添加到本机 Agent。cranibox-messages 提供本机会话登记、设备消息与协作任务；cranibox-devices 提供通过 Workspace 接入的设备、命令和任务工具。</p>
        <label className="checkbox"><input type="checkbox" checked={includeRuns} onChange={event => setIncludeRuns(event.target.checked)} aria-describedby="runs-mcp-help" /><div><strong>添加 Agent 后台运行工具</strong><span id="runs-mcp-help">加入 cranibox-runs，让 Agent 明确选择设备后启动程序、查看输出和停止运行。目标须已启动节点服务；远端须授予命令执行权限。添加配置本身不会启动程序。</span></div></label>
        <div className="code-heading"><span>{mcpLoading ? '正在生成 MCP 配置…' : 'MCP 配置'}</span><button disabled={!mcp || mcpLoading} onClick={async () => { try { await navigator.clipboard.writeText(mcp); setCopied(true); setMcpError(''); } catch { setMcpError('复制失败，请在下方选中配置并复制。'); } }}><Copy size={16} aria-hidden="true" />{copied ? '已复制' : '复制配置'}</button></div>
        {mcpError && <p className="error" role="alert">{mcpError}</p>}<textarea className="code" aria-label="MCP 配置" aria-busy={mcpLoading} readOnly value={mcp} rows={13} spellCheck={false} />
        <aside><ShieldCheck size={20} aria-hidden="true" /><p>消息使用本机 NodeID 和目标单独授予的消息权限，可独立于 Workspace 使用。任务需由 Agent 明确领取并报告结果，消息与任务不会自动执行；通过 Workspace 的命令访问范围仍由设备组决定。{includeRuns && ' 后台运行工具使用目标节点账户执行程序；每次调用需指定设备 ID，关闭 Agent 或页面后目标节点继续持有运行。'}</p></aside></section>}
    </main><footer><span>{snapshot?.autostart ? '已启用登录自启动' : '登录自启动未开启'}</span><button className="text-button" onClick={() => setQuitOpen(true)}>退出客户端</button></footer>
    <TerminalWindow suspended={quitOpen} /><dialog ref={dialog} onCancel={event => { event.preventDefault(); void cancelQuit(); }}><h2>退出节点客户端？</h2><p>退出会结束本机的 Shell 会话，其他设备也将无法访问这台设备。</p><p className="muted">只想关闭窗口，请选择“继续运行”。</p><div className="dialog-actions"><button autoFocus onClick={() => void cancelQuit()}>继续运行</button><button className="danger" onClick={() => void invoke('quit').catch(error => setError(String(error)))}>退出并结束会话</button></div></dialog>
  </div></DesktopTerminalEnvironment>;
}
const personalApi:Api={
 async call<T>(method:string,path:string,body?:unknown){
  const reads:Record<string,string>={'/status':'app_status','/devices':'app_devices','/approvals':'app_approvals','/agents':'app_agents'};
  let command=method==='GET'?reads[path]:undefined;let args:Record<string,unknown>={};
  if(path.startsWith('/models/')){command='app_models';args={method,path:path.slice('/models'.length),body:body??null};}
  else if(method==='POST'&&path==='/pair/invite'){command='app_pair_invite';args={req:body};}
  else if(method==='POST'&&path==='/pair/accept'){command='app_pair_accept';args={req:body};}
  else if(method==='POST'&&/^\/pair\/[^/]+\/confirm$/.test(path)){command='app_pair_confirm';args={flow:decodeURIComponent(path.split('/')[2]),req:body};}
  else if(method==='POST'&&path.startsWith('/approvals/')){command='app_approval_decide';args={id:decodeURIComponent(path.slice('/approvals/'.length)),req:body};}
  else if(method==='PUT'&&path.startsWith('/agents/')){command='app_agent_update';args={id:decodeURIComponent(path.slice('/agents/'.length)),req:body};}
  // Device administration extends C6 without changing any shared request shape.
  if(!command){command='app_call';args={method,path,body:body??null};}
  try{return await invoke<T>(command,args);}catch(error){throw Error(typeof error==='object'&&error&&'message' in error?String(error.message):String(error));}
 },
 subscribe(refresh){let closed=false;let unlisten:(()=>void)|undefined;void listen('app://event',refresh).then(stop=>{if(closed)stop();else unlisten=stop;}).catch(()=>{});void invoke('app_events').catch(()=>{});return()=>{closed=true;unlisten?.();};}
};
function DesktopApp(){
 const [quitRequested,setQuitRequested]=useState(false);
 useEffect(()=>{let disposed=false;const timer=setInterval(()=>{void invoke<Snapshot>('snapshot').then(s=>{if(!disposed)setQuitRequested(s.quit_requested);}).catch(()=>{});},1500);return()=>{disposed=true;clearInterval(timer);};},[]);
 return <><PersonalApp api={personalApi} language="zh-cn" advanced={<App/>} openExternal={url=>invoke('app_open_external',{url})} chatUrl="http://127.0.0.1:3080/"/>{quitRequested&&<Sheet title="退出 Cranibox？" t={zh=>zh} onClose={()=>{void invoke('dismiss_quit');setQuitRequested(false);}}><p>退出会结束本机的 Shell 会话，其他设备将无法访问这台设备。</p><button className="pa-full" onClick={()=>{void invoke('dismiss_quit');setQuitRequested(false);}}>继续在托盘运行</button><button className="pa-full" onClick={()=>void invoke('quit')}>退出并结束会话</button></Sheet>}</>;
}
createRoot(document.getElementById('root')!).render(<TerminalWindowProvider><DesktopApp /></TerminalWindowProvider>);
