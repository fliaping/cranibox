import { useCallback, useEffect, useRef, useState, type FormEvent } from 'react';
import { Cable, Check, Plus, RefreshCw, Terminal, Trash2, X } from 'lucide-react';
import './devices.css';
import { BrowserTargetPanel } from './BrowserTargetPanel';
import { DirectAccessPanel } from './DirectAccessPanel';
import { DirectPathsPanel, type PathConnection } from './DirectPathsPanel';
import type { MessageCommand } from '../messages/DeviceMessagesView';
import type { TaskCommand } from '../tasks/DeviceTasksView';
import type { AgentCommand } from '../tasks/AgentSessions';
import type { ExecutionCommand } from '../tasks/ManagedRuns';
import type { CopyCommand } from '../projects/ProjectCopyPanel';

export type DirectRoute = { node: string; url: string; allow_private_http: boolean; paths?: { url: string; allow_private_http: boolean; managed?: boolean }[] };
type Route = DirectRoute;
export type DirectListener = { address: string; allow_insecure: boolean };
export type DirectCatalog = { node_id: string | null; copy_root?: string | null; routes: Route[]; grants?: { node: string; permissions: { exec: boolean; terminals: boolean; projects?: boolean; desktop?: boolean; computer?: boolean; browser?: boolean; messages?: boolean; spaces?: boolean }; require_verified?: boolean; peer?: { fingerprint: string; verified: boolean; pairing_code?: string | null } | null }[]; computer?: { address: string } | null; computer_paused?: boolean; browser?: { address: string } | null; browser_paused?: boolean; desktop?: { address: string; protocol?: 'vnc' | 'selkies' } | null; listener?: DirectListener | null; listener_runtime?: { active: boolean; address: string | null; applied: boolean; error?: string | null } };
type Catalog = DirectCatalog;
type Peer = { id: string; os: string; arch: string; exec: boolean; terminal_sessions: boolean; projects?: boolean; desktop?: boolean; computer?: boolean; browser?: boolean; messages?: boolean; spaces?: boolean };
type Job = { id: string; node: string; outcome: string; stdout?: string; stderr?: string; exit_code?: number | null; truncated?: boolean; error?: string | null };
export type DirectCommand = { operation: 'browser_sharing'; browser: { address: string } | null } | { operation: 'browser_owner'; command: { action: 'status' | 'emergency_stop' | 'resume' } } | { operation: 'browser'; node: string; command: { operation: 'status' | 'emergency_stop' } } | { operation: 'computer_sharing'; computer: { address: string } | null } | { operation: 'computer_owner'; command: { action: 'status' | 'emergency_stop' | 'resume' } } | { operation: 'project_sync'; command: Record<string, unknown> } | { operation: 'executions'; node?: string | null; command: ExecutionCommand } | { operation: 'agent_sessions'; node: string | null; command: AgentCommand } | { operation: 'tasks'; command: TaskCommand } | { operation: 'messages'; command: MessageCommand } | CopyCommand | { operation: 'copy_root'; path: string | null } | { operation: 'list' | 'identity' } | { operation: 'listener'; listener: DirectListener | null } | { operation: 'desktop'; desktop: { address: string; protocol?: 'vnc' | 'selkies' } | null } | { operation: 'grant'; node: string; require_verified?: boolean; permissions: { exec: boolean; terminals: boolean; projects?: boolean; desktop?: boolean; computer?: boolean; browser?: boolean; messages?: boolean; spaces?: boolean } } | { operation: 'verify'; node: string; code: string } | { operation: 'revoke'; node: string } | { operation: 'connect'; node: string; url: string; token: string; allow_private_http: boolean } | { operation: 'add_path'; node: string; url: string; token: string; allow_private_http: boolean } | { operation: 'prefer_path' | 'remove_path'; node: string; url: string } | { operation: 'forget' | 'probe' | 'space_catalog'; node: string } | { operation: 'exec'; node: string; request: { program: string; args: string[]; cwd: string | null; env: Record<string, string>; timeout_secs: number } } | { operation: 'job' | 'cancel'; node: string; id: string } | { operation: 'projects' | 'sessions'; node: string; command: Record<string, unknown> };
export type DirectClient = (command: DirectCommand) => Promise<unknown>;
const blank = { node: '', url: '', token: '', allow_private_http: false };
const message = (error: unknown) => error instanceof Error ? error.message : String(error);
const storageKey = 'cranibox.direct.jobs';
function savedJobs(): Job[] {
  try {
    const value: unknown = JSON.parse(sessionStorage.getItem(storageKey) || '[]');
    if (Array.isArray(value)) return value.filter(j => typeof j?.id === 'string' && typeof j?.node === 'string').slice(-20).map(j => ({ id: j.id, node: j.node, outcome: 'running' }));
  } catch { /* Session storage is optional. */ }
  return [];
}

/** Both apps use the same controls; credentials only travel to their local backend. */
export function DirectDevicesView({ client, language = 'en', onOpenTerminal }: { client: DirectClient; language?: string; onOpenTerminal?: (node: string) => void }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const [catalog, setCatalog] = useState<Catalog>(), [selected, setSelected] = useState('');
  const [editor, setEditor] = useState(false), [form, setForm] = useState(blank), [editing, setEditing] = useState(false);
  const [error, setError] = useState(''), [loading, setLoading] = useState(true), [busy, setBusy] = useState(false), [notice, setNotice] = useState('');
  const [probe, setProbe] = useState<{ node: string; peer?: Peer; connection?: PathConnection; checked: number; error?: string }>(), [checking, setChecking] = useState(false);
  const [program, setProgram] = useState(''), [args, setArgs] = useState('[]'), [cwd, setCwd] = useState(''), [timeout, setTimeoutSeconds] = useState('60');
  const [jobs, setJobs] = useState(savedJobs), [jobId, setJobId] = useState(''), [jobError, setJobError] = useState(''), [jobRefresh, setJobRefresh] = useState(0);
  const mounted = useRef(true), mutation = useRef(false), probeSerial = useRef(0), catalogSerial = useRef(0), firstField = useRef<HTMLInputElement>(null);
  const route = catalog?.routes.find(r => r.node === selected) || catalog?.routes[0];
  const routeJobs = jobs.filter(j => j.node === route?.node);
  const job = routeJobs.find(j => j.id === jobId) || routeJobs.at(-1);
  const checked = probe?.node === route?.node ? probe : undefined;
  const apply = (value: Catalog) => { ++catalogSerial.current; setCatalog(value); setProbe(undefined); ++probeSerial.current; setChecking(false); window.dispatchEvent(new Event('cranibox:peers-changed')); };
  const reload = useCallback(async () => {
    const serial = ++catalogSerial.current;
    try { const value = await client({ operation: 'list' }) as Catalog; if (mounted.current && serial === catalogSerial.current) { setCatalog(value); setProbe(undefined); ++probeSerial.current; setChecking(false); setError(''); } }
    catch (error) { if (mounted.current && serial === catalogSerial.current) setError(message(error)); }
    finally { if (mounted.current && serial === catalogSerial.current) setLoading(false); }
  }, [client]);
  useEffect(() => { mounted.current = true; void reload(); return () => { mounted.current = false; ++probeSerial.current; }; }, [reload]);
  useEffect(() => { const changed = () => void reload(); window.addEventListener('cranibox:network-routes-changed', changed); return () => window.removeEventListener('cranibox:network-routes-changed', changed); }, [reload]);
  useEffect(() => { if (editor) firstField.current?.focus(); }, [editor]);
  useEffect(() => { try { sessionStorage.setItem(storageKey, JSON.stringify(jobs.map(({ node, id }) => ({ node, id })))); } catch { /* Never persist credentials or output. */ } }, [jobs]);
  // Poll only the viewed job. A failed read pauses polling; retry never resubmits a command.
  useEffect(() => {
    if (!job || !route) return;
    let disposed = false, active = false, timer: ReturnType<typeof setTimeout>;
    const { id, node } = job;
    async function read() {
      if (active || disposed || document.hidden) return;
      active = true;
      try {
        const value = await client({ operation: 'job', node, id }) as Job;
        if (disposed) return;
        setJobs(previous => previous.map(j => j.id === id && j.node === node ? value : j)); setJobError('');
        if (value.outcome === 'running') timer = setTimeout(read, 1000);
      } catch (error) { if (!disposed) setJobError(message(error)); }
      finally { active = false; }
    }
    setJobError(''); void read();
    const resume = () => { clearTimeout(timer); void read(); };
    document.addEventListener('visibilitychange', resume);
    return () => { disposed = true; clearTimeout(timer); document.removeEventListener('visibilitychange', resume); };
  }, [client, job?.id, route?.node, jobRefresh]);
  async function run(action: () => Promise<void>) {
    if (mutation.current) return;
    mutation.current = true; setBusy(true); setError(''); setNotice('');
    try { await action(); } catch (error) { if (mounted.current) setError(message(error)); }
    finally { mutation.current = false; if (mounted.current) setBusy(false); }
  }
  async function save(event: FormEvent) {
    event.preventDefault();
    await run(async () => {
      const node = form.node.trim();
      const value = await client({ operation: 'connect', ...form, node, url: form.url.trim() }) as Catalog;
      if (!mounted.current) return;
      apply(value); setSelected(node); setEditor(false); setForm(blank);
      setNotice(t('连接已保存。检查连接后即可查看目标设备授予的能力。', 'Connection saved. Check it to see the capabilities granted by the target.'));
    });
  }
  async function check() {
    if (!route || checking) return;
    const node = route.node, serial = ++probeSerial.current;
    setChecking(true);
    try {
      const value = await client({ operation: 'probe', node }) as { nodes: Peer[]; connection?: PathConnection };
      if (mounted.current && serial === probeSerial.current) setProbe({ node, peer: value.nodes[0], connection: value.connection, checked: Date.now() });
    } catch (error) {
      if (mounted.current && serial === probeSerial.current) setProbe({ node, error: message(error), checked: Date.now() });
    } finally { if (mounted.current && serial === probeSerial.current) setChecking(false); }
  }
  async function execute(event: FormEvent) {
    event.preventDefault(); if (!route) return;
    await run(async () => {
      const values: unknown = JSON.parse(args);
      if (!Array.isArray(values) || !values.every(v => typeof v === 'string')) throw Error(t('参数必须是字符串 JSON 数组。', 'Arguments must be a JSON array of strings.'));
      const value = await client({ operation: 'exec', node: route.node, request: { program, args: values, cwd: cwd || null, env: {}, timeout_secs: Number(timeout) } }) as Job;
      if (!mounted.current) return;
      setJobs(previous => [...previous, value].slice(-20)); setJobId(value.id);
    });
  }
  return <section className="direct-devices" aria-label={t('直连设备', 'Direct devices')}>
    <div className="direct-heading"><div><h2>{t('直连设备', 'Direct devices')}</h2><p>{t('通过局域网、EasyTier 或 Tailscale 可达地址访问，无需 Workspace 中继在线。', 'Use reachable LAN, EasyTier or Tailscale addresses. The Workspace relay can be offline.')}</p></div><div className="direct-actions"><button disabled={busy || loading} onClick={() => void reload()} aria-label={t('刷新直连设备', 'Refresh direct devices')}><RefreshCw size={16} aria-hidden="true" /></button><button disabled={busy} onClick={() => { setEditor(true); setEditing(false); setForm(blank); }}><Plus size={16} aria-hidden="true" />{t('添加直连设备', 'Add direct device')}</button></div></div>
    {catalog && <div className="direct-identity">{catalog.node_id ? <><span>{t('当前接入端的直连身份', 'Direct identity of this access point')}</span><code>{catalog.node_id}</code><small>{t('请让目标设备对这个 ID 授权。它与 Workspace 中继的节点别名不同。', 'Ask the target to grant this ID access. It differs from the Workspace relay alias.')}</small></> : <><p>{t('先创建稳定身份，再让目标设备授权。创建身份不会开启监听端口。', 'Create a stable identity to receive target grants. This does not open a listener.')}</p><button disabled={busy} onClick={() => void run(async () => { const value = await client({ operation: 'identity' }) as Catalog; if (mounted.current) apply(value); })}>{t('创建直连身份', 'Create direct identity')}</button></>}</div>}
    {error && <p className="direct-error" role="alert">{error}</p>}
    {notice && <p className="direct-note" role="status"><Check size={16} aria-hidden="true" />{notice}</p>}
    {catalog?.node_id && <DirectAccessPanel client={client} catalog={catalog} language={language} busy={busy} run={run} onCatalog={apply} />}
    {editor && <form className="direct-editor" onSubmit={save}><div className="direct-heading"><h3>{editing ? t('编辑连接', 'Edit connection') : t('添加直连设备', 'Add direct device')}</h3><button type="button" disabled={busy} aria-label={t('关闭连接表单', 'Close connection form')} onClick={() => { setEditor(false); setForm(blank); }}><X size={16} aria-hidden="true" /></button></div>
      <p className="direct-note">{t('在目标设备的“入站授权”中为当前接入端的 ID 创建 Token，也可以使用 peer grant。这里保存的是出站连接。', 'In the target’s Incoming access panel, grant this access point’s ID a token, or use peer grant. This configures an outbound connection.')}</p>
      <div className="direct-fields"><label>{t('目标节点 ID', 'Target node ID')}<input ref={firstField} required readOnly={editing} pattern="[A-Za-z0-9_-]{1,64}" maxLength={64} value={form.node} onChange={e => setForm({ ...form, node: e.target.value })} /></label><label>{t('直连地址', 'Direct URL')}<input required type="url" placeholder="https://device.example.com" value={form.url} onChange={e => setForm({ ...form, url: e.target.value })} /></label></div>
      <label>{t('直连 Token', 'Direct token')}<input type="password" autoComplete="off" required={!editing} pattern="[A-Fa-f0-9]{64}" maxLength={64} value={form.token} onChange={e => setForm({ ...form, token: e.target.value })} /><span className="direct-note">{editing ? t('留空保留凭据和备用路径；填写 Token 会替换授权并清除备用路径。', 'Leave blank to keep the grant and paths. Entering a token replaces the grant and clears alternative paths.') : t('保存在当前接入端的配置目录，保存后不再返回页面。', 'Stored in the current access point’s configuration directory and never returned to the page.')}</span></label>
      <label className="direct-check"><input type="checkbox" checked={form.allow_private_http} onChange={e => setForm({ ...form, allow_private_http: e.target.checked })} />{t('允许可信私网使用 HTTP', 'Allow HTTP on a trusted private network')}</label>
      <button className="direct-primary" type="submit" disabled={busy}>{busy ? t('正在保存…', 'Saving…') : t('保存连接', 'Save connection')}</button>
    </form>}
    {loading ? <p role="status">{t('正在读取连接…', 'Loading connections…')}</p> : !catalog?.routes.length ? <div className="direct-empty"><Cable size={28} aria-hidden="true" /><h3>{t('尚未配置直连设备', 'No direct devices configured')}</h3><p>{t('可以从设备组网中接入已发现的节点，或通过监听地址和目标授权 Token 添加。', 'Connect a discovered node in Device networks, or add a listener address and a target access token.')}</p></div> : <div className="direct-layout">
      <div className="direct-list" aria-label={t('直连设备列表', 'Direct device list')}>{catalog.routes.map(r => <button key={r.node} disabled={busy} aria-pressed={route?.node === r.node} onClick={() => { setSelected(r.node); setJobId(''); setEditor(false); setForm(blank); ++probeSerial.current; setChecking(false); }}><Cable size={18} aria-hidden="true" /><span><strong>{r.node}</strong><small>{r.url}</small></span></button>)}</div>
      {route && <div className="direct-detail"><div className="direct-heading"><div><h3>{route.node}</h3><p>{route.url}</p></div><button disabled={busy || checking} onClick={() => void check()}><RefreshCw size={16} aria-hidden="true" />{checking ? t('检查中…', 'Checking…') : t('检查连接', 'Check connection')}</button></div>
        {checked ? <div className="direct-probe"><p role={checked.error ? 'alert' : 'status'}>{checked.error || [checked.peer?.os, checked.peer?.arch, checked.peer?.exec ? t('允许执行命令', 'Commands granted') : t('未授予命令权限', 'Commands not granted'), checked.peer?.terminal_sessions ? t('允许终端会话', 'Terminal sessions granted') : t('未授予终端权限', 'Terminal sessions not granted'), checked.peer?.computer && t('允许 Agent 桌面操作', 'Computer Use granted'), checked.peer?.browser && t('允许 Agent 浏览器操作', 'Browser Use granted'), checked.peer?.projects ? t('允许项目目录', 'Project catalog granted') : t('未授予项目目录权限', 'Project catalog not granted')].filter(Boolean).join(' · ')}</p><small>{t('上次检查：', 'Last checked: ')}{new Date(checked.checked).toLocaleTimeString()}</small></div> : <p className="direct-note">{t('尚未检查连接；授权和在线情况以目标设备实时响应为准。', 'Connection not checked. The target determines current availability and permissions.')}</p>}
        <div className="direct-actions">{onOpenTerminal && <button disabled={busy || !checked?.peer?.terminal_sessions || !!checked.error} onClick={() => onOpenTerminal(route.node)}><Terminal size={16} aria-hidden="true" />{t('打开终端', 'Open terminal')}</button>}<button disabled={busy} onClick={() => { setEditing(true); setForm({ node: route.node, url: route.url, allow_private_http: route.allow_private_http, token: '' }); setEditor(true); }}>{t('编辑连接', 'Edit connection')}</button><button disabled={busy} onClick={() => { if (confirm(t('移除此接入端保存的连接？目标设备的授权和后台 Shell 不受影响。', 'Remove this saved connection? Target grants and background shells remain active.'))) void run(async () => { const value = await client({ operation: 'forget', node: route.node }) as Catalog; if (mounted.current) apply(value); }); }}><Trash2 size={16} aria-hidden="true" />{t('移除连接', 'Remove connection')}</button></div>
        {checked?.peer?.browser && <BrowserTargetPanel key={route.node} client={client} node={route.node} language={language} />}
        <DirectPathsPanel key={route.node} route={route} connection={checked?.connection} client={client} busy={busy} run={run} onCatalog={apply} t={t} />
        <form className="direct-command" onSubmit={execute}><h3>{t('执行直连任务', 'Run direct task')}</h3><p className="direct-note">{t('程序与参数分别传入；不会自动使用 Shell。提交失败时不会自动重试。', 'Program and arguments are separate; no implicit shell. Failed submissions are never automatically retried.')}</p><div className="direct-fields"><label>{t('程序', 'Program')}<input required value={program} onChange={e => setProgram(e.target.value)} placeholder="git" /></label><label>{t('工作目录（可选）', 'Working directory (optional)')}<input value={cwd} onChange={e => setCwd(e.target.value)} /></label></div><label>{t('参数（JSON 数组）', 'Arguments (JSON array)')}<textarea rows={2} required value={args} onChange={e => setArgs(e.target.value)} /></label><label>{t('超时（秒）', 'Timeout (seconds)')}<input type="number" min={1} max={3600} required value={timeout} onChange={e => setTimeoutSeconds(e.target.value)} /></label><button className="direct-primary" type="submit" disabled={busy || checked?.peer?.exec === false}><Terminal size={16} aria-hidden="true" />{busy ? t('处理中…', 'Working…') : t('执行直连任务', 'Run direct task')}</button></form>
        {!!routeJobs.length && <div className="direct-jobs"><label>{t('任务记录', 'Task records')}<select value={job?.id || ''} onChange={e => setJobId(e.target.value)}>{[...routeJobs].reverse().map(j => <option key={j.id} value={j.id}>{j.id}</option>)}</select></label><p className="direct-note">{t('仅在当前浏览器标签页保存最近 20 个任务 ID；刷新可恢复查询，关闭页面不会取消任务。节点退出后任务记录丢失，完成的任务记录至少保留 10 分钟。', 'This tab keeps the latest 20 task IDs across reloads. Closing it does not cancel tasks. Node exit clears records; completed records are kept for at least 10 minutes.')}</p>{job && <><div className="direct-heading"><p role="status">{job.outcome}{job.exit_code != null ? ' · exit ' + job.exit_code : ''}</p><div className="direct-actions"><button aria-label={t('刷新任务输出', 'Refresh task output')} onClick={() => setJobRefresh(v => v + 1)}><RefreshCw size={16} aria-hidden="true" /></button><button disabled={busy || job.outcome !== 'running'} onClick={() => void run(async () => { await client({ operation: 'cancel', node: job.node, id: job.id }); if (mounted.current) setJobRefresh(v => v + 1); })}>{t('取消任务', 'Cancel task')}</button></div></div>{(jobError || job.error) && <p className="direct-error" role="alert">{jobError || job.error}</p>}{job.truncated && <p role="status">{t('输出超过 64 KiB，已截断。', 'Output exceeded 64 KiB and was truncated.')}</p>}<label>{t('标准输出', 'Standard output')}<pre tabIndex={0} aria-label={t('标准输出', 'Standard output')}>{job.stdout || '—'}</pre></label><label>{t('错误输出', 'Standard error')}<pre tabIndex={0} aria-label={t('错误输出', 'Standard error')}>{job.stderr || '—'}</pre></label></>}</div>}
      </div>}
    </div>}
  </section>;
}
