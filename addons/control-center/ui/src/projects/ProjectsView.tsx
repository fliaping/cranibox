import { useCallback, useEffect, useRef, useState, type FormEvent, type ReactNode } from 'react';
import { FolderGit2, Link2, Monitor, Plus, RefreshCw, Search, Terminal, Unlink, WifiOff, X } from 'lucide-react';
import './projects.css';
import { ProjectCopyPanel, type CopyClient } from './ProjectCopyPanel';
import { ask } from '../confirm';

export type Device = { id: string; name?: string; online: boolean; terminal_sessions?: boolean; projects?: boolean; can_copy?: boolean };
type Binding = { id: string; terminal_id: string };
export type Replica = { id: string; node: string | null; path: string; sessions: Binding[]; offline_synced_at?: number };
type Project = { id: string; name: string; replicas: Replica[] };
export type Catalog = { revision: number; projects: Project[] };
export type TerminalSession = { id: string; name: string; cwd: string; status: string };
type Terminals = { installed: boolean; sessions: TerminalSession[]; session?: TerminalSession };
export type ProjectClient = {
  copy?: CopyClient;
  projects: (command: Record<string, unknown>) => Promise<Catalog>;
  terminals: (node: string | null, command: Record<string, unknown>) => Promise<Terminals>;
  open: (node: string | null, session: TerminalSession) => void | Promise<void>;
};
const message = (error: unknown) => error instanceof Error ? error.message : String(error);

/** Shared by the browser and tray client. Transport and terminal presentation are injected. */
export function ProjectsView({ client, devices, language = 'en', sources }: { client: ProjectClient; devices: Device[]; language?: string; sources?: ReactNode }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const [catalog, setCatalog] = useState<Catalog>();
  const [selected, setSelected] = useState(''), [replicaId, setReplicaId] = useState(''), [filter, setFilter] = useState('');
  const [error, setError] = useState(''), [loading, setLoading] = useState(true), [busy, setBusy] = useState(false);
  const [registration, setRegistration] = useState<'new' | 'replica' | null>(null);
  const [name, setName] = useState(''), [path, setPath] = useState(''), [node, setNode] = useState('');
  const [terminals, setTerminals] = useState<Terminals>(), [terminalError, setTerminalError] = useState('');
  const [terminalLoading, setTerminalLoading] = useState(false), [refresh, setRefresh] = useState(0);
  const [existing, setExisting] = useState(''), [sessionName, setSessionName] = useState('');
  const mounted = useRef(true), request = useRef(0), mutation = useRef(false), polling = useRef(false);
  const registrationTitle = useRef<HTMLInputElement>(null);
  const terminalTarget = useRef('');
  const project = catalog?.projects.find(p => p.id === selected) || catalog?.projects[0];
  const replica = project?.replicas.find(r => r.id === replicaId) || project?.replicas[0];
  const device = replica && devices.find(d => d.id === (replica.node || ''));
  const offline = replica?.offline_synced_at !== undefined;
  const online = !offline && !!device?.online && device.terminal_sessions !== false;
  const apply = (value: Catalog) => { ++request.current; setCatalog(value); };
  const reload = useCallback(async () => {
    if (polling.current || mutation.current) return;
    polling.current = true; const serial = ++request.current;
    try {
      const value = await client.projects({ operation: 'list' });
      if (mounted.current && serial === request.current) { setCatalog(value); setError(''); }
    } catch (problem) { if (mounted.current && serial === request.current) setError(message(problem)); }
    finally { polling.current = false; if (mounted.current) setLoading(false); }
  }, [client]);
  useEffect(() => {
    mounted.current = true; void reload();
    const poll = () => { if (!document.hidden) { void reload(); setRefresh(v => v + 1); } };
    const timer = setInterval(poll, 10000);
    window.addEventListener('focus', poll); document.addEventListener('visibilitychange', poll);
    return () => { mounted.current = false; ++request.current; clearInterval(timer); window.removeEventListener('focus', poll); document.removeEventListener('visibilitychange', poll); };
  }, [reload]);
  useEffect(() => { if (registration) registrationTitle.current?.focus(); }, [registration]);
  useEffect(() => {
    let active = true;
    const target = replica?.id || '';
    if (terminalTarget.current !== target || !online) { setTerminals(undefined); setExisting(''); }
    terminalTarget.current = target; setTerminalError('');
    if (!replica || !online) { setTerminalLoading(false); return; }
    setTerminalLoading(true);
    void client.terminals(replica.node, { operation: 'list' }).then(value => { if (active) setTerminals(value); })
      .catch(problem => { if (active) setTerminalError(message(problem)); }).finally(() => { if (active) setTerminalLoading(false); });
    return () => { active = false; };
  }, [client, replica?.id, replica?.node, online, refresh]);
  async function run(action: () => Promise<void>) {
    if (mutation.current) return;
    mutation.current = true; ++request.current; setBusy(true); setError('');
    try { await action(); } catch (problem) { if (mounted.current) setError(message(problem)); }
    finally { mutation.current = false; if (mounted.current) { setBusy(false); setRefresh(v => v + 1); } }
  }
  function register(event: FormEvent) {
    event.preventDefault();
    if (!devices.some(d => d.id === node && d.projects !== false)) return;
    const previousIds = new Set(catalog?.projects.map(p => p.id));
    void run(async () => {
      const value = await client.projects({ operation: 'register', name: registration === 'replica' ? project!.name : name.trim(), node: node || null, path: path.trim(), project_id: registration === 'replica' ? project!.id : null });
      if (!mounted.current) return;
      apply(value);
      const registered = value.projects.find(p => p.replicas.some(r => r.node === (node || null) && r.path === path.trim())) || value.projects.find(p => !previousIds.has(p.id));
      if (registered) { setSelected(registered.id); setReplicaId(registered.replicas.find(r => r.node === (node || null) && r.path === path.trim())?.id || ''); }
      setRegistration(null); setName(''); setPath('');
    });
  }
  function startRegistration(kind: 'new' | 'replica') {
    if (!devices.some(d => d.projects !== false)) return;
    setNode(devices.find(d => d.online && d.projects !== false)?.id || devices.find(d => d.projects !== false)?.id || ''); setRegistration(kind); setName(''); setPath('');
  }
  async function bind(session: TerminalSession) {
    const value = await client.projects({ operation: 'bind', replica_id: replica!.id, terminal_id: session.id });
    if (mounted.current) { apply(value); setExisting(''); }
  }
  function createSession(event: FormEvent) {
    event.preventDefault(); if (!replica) return;
    const owner = replica;
    void run(async () => {
      const value = await client.terminals(owner.node, { operation: 'create', name: sessionName.trim() || Array.from(project!.name).slice(0, 64).join('') + ' · Shell', cwd: owner.path });
      if (!value.session) throw Error(t('节点没有返回会话。请刷新会话列表后检查，避免重复创建。', 'No session returned. Refresh the session list before creating another.'));
      try { await bind(value.session); }
      catch (problem) { throw Error(t('Shell 已创建，但关联失败。请从“关联已有会话”找回：', 'Shell created, but linking failed. Recover it with “Link existing session”: ') + message(problem)); }
      if (mounted.current) { setSessionName(''); await client.open(owner.node, value.session); }
    });
  }
  const available = terminals?.sessions.filter(s => !replica?.sessions.some(b => b.terminal_id === s.id)) || [];
  const filtered = catalog?.projects.filter(p => (p.name + ' ' + p.replicas.map(r => r.path).join(' ')).toLowerCase().includes(filter.toLowerCase())) || [];
  return <section className="projects-view" aria-label={t('项目与会话', 'Projects and sessions')}>
    <div className="projects-heading"><div><h1>{t('项目与会话', 'Projects & sessions')}</h1><p>{t('按项目找回工作目录和 Shell，明确每个会话运行在哪台设备。', 'Find each working directory and shell, with its device always in view.')}</p></div><div className="projects-actions"><button type="button" disabled={busy || loading} aria-label={t('刷新项目', 'Refresh projects')} onClick={() => { void reload(); setRefresh(v => v + 1); }}><RefreshCw size={17} aria-hidden="true" /></button><button type="button" className="projects-primary" disabled={busy || !devices.some(d => d.projects !== false)} onClick={() => startRegistration('new')}><Plus size={17} aria-hidden="true" />{t('注册项目', 'Register project')}</button></div></div>
    {sources}
    {client.copy && <ProjectCopyPanel client={client.copy} devices={devices} replica={replica} language={language} onComplete={reload} />}
    {error && <div className="projects-error" role="alert">{error}<button type="button" disabled={busy} onClick={() => void reload()}>{t('重新读取', 'Reload')}</button></div>}
    {registration && <form className="projects-register" onSubmit={register} aria-label={registration === 'new' ? t('注册项目', 'Register project') : t('添加工作副本', 'Add working copy')}>
      <div className="projects-heading"><h2>{registration === 'new' ? t('注册已有目录', 'Register an existing directory') : t('为项目添加工作副本', 'Add a working copy')}</h2><button type="button" disabled={busy} aria-label={t('取消注册', 'Cancel registration')} onClick={() => setRegistration(null)}><X size={17} aria-hidden="true" /></button></div>
      {registration === 'new' && <label>{t('项目名称', 'Project name')}<input ref={registrationTitle} required maxLength={80} value={name} disabled={busy} onChange={e => setName(e.target.value)} /></label>}
      <label>{t('所在设备', 'Device')}<select aria-label={t("所在设备", "Device")} value={node} disabled={busy} onChange={e => setNode(e.target.value)}>{devices.map(d => <option key={d.id} value={d.id} disabled={d.projects === false}>{d.name || d.id}{!d.online ? t(' · 离线', ' · Offline') : ''}</option>)}</select></label>
      <label>{t('设备上的绝对目录', 'Absolute directory on device')}<input ref={registration === 'replica' ? registrationTitle : undefined} required maxLength={4096} placeholder={t('例如 /home/me/project 或 C:/Projects/app', 'e.g. /home/me/project or C:/Projects/app')} value={path} disabled={busy} onChange={e => setPath(e.target.value)} /></label>
      <p>{t('仅记录已有目录；不会复制文件。目录是否存在，会在创建 Shell 时由目标设备检查。', 'Records an existing directory. No files are copied. The target checks the directory when creating a shell.')}</p><button className="projects-primary" disabled={busy || !devices.some(d => d.id === node && d.projects !== false)} type="submit">{busy ? t('正在保存…', 'Saving…') : t('保存目录', 'Save directory')}</button>
    </form>}
    {loading && !catalog ? <p role="status">{t('正在读取项目…', 'Loading projects…')}</p> : <div className="projects-layout">
      <div className="projects-sidebar"><label className="projects-search"><Search size={17} aria-hidden="true" /><span className="projects-sr">{t('搜索项目', 'Search projects')}</span><input type="search" value={filter} placeholder={t('搜索项目或路径', 'Search projects or paths')} onChange={e => setFilter(e.target.value)} /></label><div className="projects-list" aria-label={t('项目列表', 'Project list')}>{filtered.map(p => <button type="button" key={p.id} aria-pressed={project?.id === p.id} disabled={busy || !!registration} onClick={() => { setSelected(p.id); setReplicaId(''); }}><FolderGit2 size={19} aria-hidden="true" /><span><strong>{p.name}</strong><small>{p.replicas.length} {t('个工作副本', 'working copies')} · {p.replicas.reduce((sum, r) => sum + r.sessions.length, 0)} {t('个关联会话', 'linked sessions')}</small></span></button>)}</div>{!filtered.length && <p className="projects-note">{filter ? t('没有匹配的项目', 'No matching projects') : t('注册你正在使用的项目目录，开始整理会话。', 'Register a working directory to organize its sessions.')}</p>}</div>
      <div className="projects-detail">{project && replica ? <><div className="projects-heading"><div><h2>{project.name} <span className="cx-id-pill">#{project.id.slice(0, 6)}</span></h2><code className="projects-id projects-sr" title={project.id}>{project.id}</code></div><button type="button" disabled={busy || !devices.some(d => d.projects !== false)} onClick={() => startRegistration('replica')}><Plus size={16} aria-hidden="true" />{t('添加副本', 'Add working copy')}</button></div>
        <label className="projects-replica-label">{t('工作副本', 'Working copy')}<select aria-label={t("工作副本", "Working copy")} value={replica.id} disabled={busy || !!registration} onChange={e => setReplicaId(e.target.value)}>{project.replicas.map(r => <option key={r.id} value={r.id}>{devices.find(d => d.id === (r.node || ''))?.name || r.node || t('Workspace 主机', 'Workspace host')} · {r.path}</option>)}</select></label>
        <div className="projects-location"><Monitor size={19} aria-hidden="true" /><div><strong>{device?.name || replica.node || t('Workspace 主机', 'Workspace host')}</strong><code>{replica.path}</code></div><span className={'projects-status ' + (online ? 'is-online' : '')}>{online ? t('在线', 'Online') : t('离线或终端不可用', 'Offline or terminals unavailable')}</span></div>
        {offline && <div className="projects-snapshot"><WifiOff size={18} aria-hidden="true" /><div><strong>{t('离线快照', 'Offline snapshot')}</strong><p>{t('上次同步：', 'Last synced: ')}<time dateTime={new Date(replica.offline_synced_at!).toISOString()}>{new Date(replica.offline_synced_at!).toLocaleString()}</time></p><p>{t('目录和关联会话可能已变化。恢复连接并刷新后，才能编辑记录或操作 Shell。', 'Directories and linked sessions may have changed. Reconnect and refresh before editing records or using shells.')}</p></div></div>}
        <div className="projects-heading"><h3>{t('关联的 Shell 会话', 'Linked shell sessions')}</h3><span className="projects-note">{replica.sessions.length}</span></div>
        {terminalLoading && <p className="projects-note" role="status">{t('正在查询设备上的会话…', 'Checking sessions on the device…')}</p>}{terminalError && <p className="projects-error" role="alert">{terminalError}</p>}
        <ul className="projects-sessions">{replica.sessions.map(binding => { const session = terminals?.sessions.find(s => s.id === binding.terminal_id); const running = online && session?.status === 'running'; return <li key={binding.id}><Terminal size={18} aria-hidden="true" /><div><strong>{session?.name || binding.terminal_id}</strong><small>{!online || terminalError || terminalLoading ? t('等待设备确认状态', 'Waiting for device status') : !terminals?.installed ? t('终端服务不可用', 'Terminal service unavailable') : !session ? t('原会话已不在设备上', 'Session no longer on device') : session.status === 'running' ? t('运行中', 'Running') : t('已结束', 'Ended')}</small></div><div className="projects-actions"><button type="button" disabled={busy || !running || terminalLoading || !!terminalError} onClick={() => void run(async () => { await client.open(replica.node, session!); })}>{t('打开', 'Open')}</button><button type="button" aria-label={t('解除关联 ', 'Unlink ') + (session?.name || binding.terminal_id)} disabled={busy || offline} onClick={() => void run(async () => { const value = await client.projects({ operation: 'unbind', replica_id: replica.id, terminal_id: binding.terminal_id }); if (mounted.current) apply(value); })}><Unlink size={16} aria-hidden="true" /></button></div></li>; })}</ul>
        {!replica.sessions.length && <p className="projects-note">{t('还没有关联会话。可以新建 Shell，或关联这个设备上的已有会话。', 'Create a shell or link an existing session on this device.')}</p>}
        <form className="projects-session-form" onSubmit={createSession}><label>{t('新会话名称', 'New session name')}<input maxLength={80} value={sessionName} placeholder={project.name.slice(0, 64) + ' · Shell'} disabled={busy || !online} onChange={e => setSessionName(e.target.value)} /></label><button type="submit" className="projects-primary" disabled={busy || !online || !terminals?.installed || terminalLoading}><Plus size={16} aria-hidden="true" />{t('在此目录新建 Shell', 'Create shell here')}</button></form>
        <form className="projects-session-form" onSubmit={e => { e.preventDefault(); const session = available.find(s => s.id === existing); if (session) void run(() => bind(session)); }}><label>{t('关联已有会话', 'Link existing session')}<select aria-label={t("关联已有会话", "Link existing session")} value={existing} disabled={busy || !online || !available.length || terminalLoading || !!terminalError} onChange={e => setExisting(e.target.value)}><option value="">{t('选择此设备上的会话', 'Choose a session on this device')}</option>{available.map(s => <option key={s.id} value={s.id}>{s.name} · {s.cwd}</option>)}</select></label><button type="submit" disabled={busy || !existing || !online || terminalLoading || !!terminalError}><Link2 size={16} aria-hidden="true" />{t('关联', 'Link')}</button></form>
        <button type="button" disabled={busy || offline || !!registration} onClick={async () => { if (await ask(t('仅移除这个工作副本的记录。文件和 Shell 都会保留。继续？', 'Remove this working copy from the catalog? Files and shells will be kept.'))) void run(async () => { const value = await client.projects({ operation: 'forget', replica_id: replica.id }); if (mounted.current) apply(value); }); }}>{t('移除目录记录', 'Remove directory record')}</button>
        <p className="projects-note">{t('关闭页面或解除关联不会结束 Shell。节点进程退出后，原生 Shell 无法保留；此处不会自动重建。', 'Closing this page or unlinking leaves shells running. Native shells end when the node process exits; they are never silently recreated here.')}</p>
      </> : <div className="projects-empty"><FolderGit2 size={36} aria-hidden="true" /><h2>{t('你的项目，分布在不同设备', 'Your projects, across devices')}</h2><p>{t('从一个已有目录开始。在另一入口打开同一会话时，仍然连接原设备上的 Shell。', 'Start with an existing directory. Opening a session elsewhere reconnects to its original shell.')}</p></div>}</div>
    </div>}
  </section>;
}
