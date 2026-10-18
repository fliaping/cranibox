import { useEffect, useId, useRef, useState } from 'react';
import { ArrowRight, Check, Copy, RefreshCw, X } from 'lucide-react';
import type { Device, Replica } from './ProjectsView';
import { ProjectContinuation } from './ProjectContinuation';
import { ProjectCopyManifest } from './ProjectCopyManifest';

export type Handoff = { goal: string; progress: string; next_steps: string; verification: string };
export type AgentAdapter = 'codex' | 'claude_code';
export type AgentAccess = 'read_only' | 'workspace_write' | 'edit_files';
export type CopyCommand = { operation: 'copy_list' | 'copy_activity' } | { operation: 'copy_prepare'; source: string; target: string; replica_id: string; handoff?: Handoff } | { operation: 'copy_commit'; id: string; digest: string } | { operation: 'copy_status' | 'copy_discard' | 'copy_agent_stop' | 'copy_prepare_resume'; id: string } | { operation: 'copy_agent_prepare'; id: string; adapter: AgentAdapter; access: AgentAccess } | { operation: 'copy_agent_start'; id: string; adapter: AgentAdapter; access: AgentAccess; digest: string } | { operation: 'copy_agent_status'; id: string; stdout_offset?: number; stderr_offset?: number };
export type CopyClient = (command: CopyCommand) => Promise<unknown>;
export type CopyJob = { id: string; protocol?: number; progress?: { phase: string; bytes: number; total: number }; source: string; target: string; digest: string; target_root: string; state: string; created_at: number; summary: { name: string; files: { path: string; size: number }[]; directories: string[]; skipped: string[]; bytes: number; handoff?: Handoff; handoff_path?: string }; result?: { path: string }; error?: string };
const emptyHandoff = (): Handoff => ({ goal: '', progress: '', next_steps: '', verification: '' });
const handoffFields = ['goal', 'progress', 'next_steps', 'verification'] as const;
const limits = { goal: 2048, progress: 4096, next_steps: 4096, verification: 2048 };
const message = (error: unknown) => error instanceof Error ? error.message : String(error);
const physical = (node: string | null | undefined) => node?.startsWith('direct:') ? node.slice(7) : node?.startsWith('local:') ? node.slice(6) : node || '';
const size = (bytes: number) => bytes < 1024 ? bytes + ' B' : bytes < 1024 * 1024 ? (bytes / 1024).toFixed(1) + ' KiB' : bytes < 1024 ** 3 ? (bytes / 1024 / 1024).toFixed(1) + ' MiB' : (bytes / 1024 ** 3).toFixed(2) + ' GiB';
const moving = (value: string) => ['preparing', 'sending', 'receiving', 'publishing'].includes(value);

/** Only manifests cross into the UI. The Rust controller owns the reviewed bytes and receipts. */
export function ProjectCopyPanel({ client, devices, replica, language, onComplete }: { client: CopyClient; devices: Device[]; replica?: Replica; language: string; onComplete: () => Promise<void> }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const [open, setOpen] = useState(false), [target, setTarget] = useState(''), [jobs, setJobs] = useState<CopyJob[]>([]), [selected, setSelected] = useState('');
  const handoffId = useId();
  const [includeHandoff, setIncludeHandoff] = useState(false), [handoff, setHandoff] = useState<Handoff>(emptyHandoff);
  const [busy, setBusy] = useState(''), [error, setError] = useState(''), [reviewed, setReviewed] = useState(false);
  const [unavailable, setUnavailable] = useState<string[]>([]);
  const mounted = useRef(true), active = useRef(false), targetField = useRef<HTMLSelectElement>(null);
  const revision = useRef(0), historyRequest = useRef(0), clientRef = useRef(client), jobsRef = useRef(jobs);
  clientRef.current = client; jobsRef.current = jobs;
  const source = physical(replica?.node), eligible = devices.filter((d, index, all) => d.online && d.can_copy && physical(d.id) && physical(d.id) !== source && all.findIndex(other => other.online && other.can_copy && physical(other.id) === physical(d.id)) === index);
  const sourceReady = !!replica && !replica.offline_synced_at && devices.some(d => d.id === replica.node && d.online && d.can_copy) && !!source;
  const job = jobs.find(j => j.id === selected);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => { setReviewed(false); }, [selected]);
  useEffect(() => { setSelected(''); setReviewed(false); setTarget(''); setHandoff(emptyHandoff()); setIncludeHandoff(false); }, [replica?.id]);
  useEffect(() => { if (open) targetField.current?.focus(); }, [open]);
  const polling = !!busy || jobs.some(j => moving(j.state));
  useEffect(() => {
    if (!open || !polling) return;
    let closed = false, pending = false;
    const timer = window.setInterval(async () => {
      if (pending) return;
      pending = true;
      const before = revision.current;
      try {
        const value = await clientRef.current({ operation: 'copy_activity' }) as { copies: Pick<CopyJob, 'id' | 'state' | 'progress' | 'error'>[] };
        if (closed || !mounted.current || before !== revision.current || !Array.isArray(value.copies)) return;
        const changed = value.copies.some(item => !jobsRef.current.some(previous => previous.id === item.id && previous.state === item.state));
        if (changed) {
          await list();
          if (!closed && active.current) { const latest = value.copies.find(item => moving(item.state)); if (latest) setSelected(latest.id); }
        } else {
          setJobs(previous => previous.map(item => ({ ...item, ...value.copies.find(update => update.id === item.id) })));
        }
      } catch { /* An explicit action reports errors; polling never retries a write. */ }
      finally { pending = false; }
    }, 1500);
    return () => { closed = true; window.clearInterval(timer); };
  }, [open, polling]);
  async function run(label: string, action: () => Promise<void>) {
    if (active.current) return;
    active.current = true; setBusy(label); setError('');
    try { await action(); } catch (error) { if (mounted.current) setError(message(error)); try { await list(true); } catch { /* Keep the original actionable error. */ } }
    finally { active.current = false; if (mounted.current) setBusy(''); }
  }
  async function list(selectPaused = false) {
    const request = ++historyRequest.current, before = revision.current;
    const value = await clientRef.current({ operation: 'copy_list' }) as { copies: CopyJob[]; unavailable?: string[] };
    if (!Array.isArray(value.copies)) throw Error('Unexpected copy history');
    if (mounted.current && before === revision.current && request === historyRequest.current) { setJobs(value.copies); setUnavailable(value.unavailable || []); if (selectPaused) { const paused = value.copies.find(item => item.state === 'prepare_paused'); if (paused) setSelected(paused.id); } }
  }
  function apply(value: CopyJob) { revision.current++; if (mounted.current) { setJobs(previous => [value, ...previous.filter(j => j.id !== value.id)]); setSelected(value.id); setReviewed(false); } }
  const fieldLabel = (field: keyof Handoff) => ({ goal: t('目标与约束', 'Goal and constraints'), progress: t('当前进展与决定', 'Progress and decisions'), next_steps: t('接下来要做的事', 'Next steps'), verification: t('验证结果与待解决问题', 'Verification and unresolved issues') })[field];
  const handoffInvalid = includeHandoff && (!handoff.goal.trim() || handoffFields.some(field => new TextEncoder().encode(handoff[field]).length > limits[field] || /[\x00-\x08\x0b\x0c\x0e-\x1f\x7f-\x9f]/.test(handoff[field])));
  const state = (value: string) => ({ preparing: t('正在生成快照', 'Preparing snapshot'), prepare_paused: t('快照生成已暂停', 'Snapshot preparation paused'), receiving: t('正在传输文件', 'Transferring files'), publishing: t('正在校验并登记', 'Verifying and registering'), prepared: t('等待确认', 'Awaiting review'), sending: t('等待核对结果', 'Check target receipt'), uncertain: t('结果待确认', 'Outcome unknown'), copied: t('文件已复制，待登记', 'Files copied; registration pending'), incomplete: t('未完成，请检查目标目录', 'Incomplete; inspect target directory'), completed: t('已完成', 'Completed') }[value] || value);
  return <section className="projects-copy" aria-label={t('项目文件复制', 'Project file copies')}>
    <button type="button" aria-expanded={open} onClick={() => { setOpen(v => !v); if (!open) void run(t('读取记录…', 'Loading history…'), list); }}><Copy size={17} aria-hidden="true" />{t('复制项目到设备', 'Copy project to device')}</button>
    {open && <div className="projects-register">
      <div className="projects-heading"><div><h2>{t('项目文件复制', 'Project file copies')}</h2><p>{t('准备快照 → 检查清单 → 确认复制', 'Prepare snapshot → Review manifest → Confirm copy')}</p></div><button type="button" aria-label={t('关闭复制面板', 'Close copy panel')} onClick={() => setOpen(false)}><X size={17} aria-hidden="true" /></button></div>
      <p>{t('包含当前文件和未提交修改；常见凭据、Git 历史、依赖和构建目录会跳过。请检查清单，确认没有其他敏感文件。新版节点支持分块续传，每次最多 2 GiB、单文件 1 GiB。Shell 和 Agent 运行状态不随文件复制。', 'Includes current files and uncommitted changes. Common credentials, Git history, dependencies and build folders are skipped. Review the manifest for other sensitive files. Updated nodes support resumable chunks up to 2 GiB total and 1 GiB per file. Running shells and agents stay on their current device.')}</p>
      <form onSubmit={event => { event.preventDefault(); if (handoffInvalid || !sourceReady || !eligible.some(d => physical(d.id) === target)) return; void run(t('生成快照…', 'Preparing snapshot…'), async () => apply(await client({ operation: 'copy_prepare', source, target, replica_id: replica!.id.slice(replica!.id.lastIndexOf('/') + 1), ...(includeHandoff ? { handoff } : {}) }) as CopyJob)); }}>
        <p><strong>{t('来源：', 'Source: ')}</strong>{devices.find(d => d.id === replica?.node)?.name || source || t('请选择设备上的工作副本', 'Select a working copy on a device')}{source && <code>{replica?.path}</code>}</p>
        {!sourceReady && <p>{t('请选择本机或在线直连设备的工作副本。远端需同时授予项目目录和执行命令权限。', 'Choose a working copy on this device or an online direct device. Remote sources must grant both project catalog and command access.')}</p>}
        <label>{t('目标设备', 'Copy destination')}<select ref={targetField} required value={target} disabled={!!busy} onChange={event => setTarget(event.target.value)}><option value="">{t('选择设备', 'Choose a device')}</option>{eligible.map(d => <option key={d.id} value={physical(d.id)}>{d.name || physical(d.id)}</option>)}</select></label>
        <p>{t('目标需在“设备 → 入站授权”设置接收目录；远端还需授予项目和命令权限。复制将创建独立的新目录。', 'Set a receiving directory in Devices → Incoming access on the target. Remote destinations also need project and command access. Each copy creates a separate new directory.')}</p>
        <button type="button" aria-expanded={includeHandoff} disabled={!!busy} onClick={() => setIncludeHandoff(value => !value)}>{includeHandoff ? t('取消携带 Agent 交接说明', 'Remove Agent handoff') : t('携带 Agent 交接说明（可选）', 'Include Agent handoff (optional)')}</button>
        {includeHandoff && <fieldset className="projects-handoff-fields"><legend>{t('Agent 交接说明', 'Agent handoff')}</legend><p>{t('将作为 Markdown 文件随快照复制。请填写工作背景和下一步，避免填写密钥；目标设备不会自动启动 Agent。', 'Copied as a Markdown file inside the snapshot. Describe the work and next steps; keep secrets out. No Agent starts automatically on the target.')}</p>{handoffFields.map(field => { const bytes = new TextEncoder().encode(handoff[field]).length; return <div key={field}><label>{fieldLabel(field)}{field === 'goal' && t('（必填）', ' (required)')}<textarea rows={field === 'goal' ? 3 : 4} required={field === 'goal'} aria-describedby={handoffId + field} value={handoff[field]} disabled={!!busy} aria-invalid={bytes > limits[field] || undefined} onChange={event => setHandoff(value => ({ ...value, [field]: event.target.value }))} /></label><small id={handoffId + field}>{bytes} / {limits[field]} {t('字节', 'UTF-8 bytes')}{bytes > limits[field] && t(' · 请缩短内容', ' · Shorten this text')}</small></div>; })}</fieldset>}
        <button type="submit" disabled={!!busy || handoffInvalid || !sourceReady || !eligible.some(d => physical(d.id) === target)}>{t('生成并预览快照', 'Prepare and preview snapshot')}<ArrowRight size={16} aria-hidden="true" /></button>
      </form>
      <div className="projects-heading"><h3>{t('复制记录', 'Copy history')}</h3><button type="button" disabled={!!busy} aria-label={t('刷新复制记录', 'Refresh copy history')} onClick={() => void run(t('读取记录…', 'Loading history…'), list)}><RefreshCw size={16} aria-hidden="true" /></button></div>
      {!!jobs.length && <label>{t('查看复制记录', 'View copy record')}<select value={selected} disabled={!!busy} onChange={e => setSelected(e.target.value)}><option value="">{t('选择记录', 'Choose a record')}</option>{jobs.map(j => <option key={j.id} value={j.id}>{j.summary.name} · {j.source} → {j.target} · {state(j.state)}</option>)}</select></label>}
      {!jobs.length && <p>{t('暂无记录。记录保存在当前接入端，重新打开后可继续查看。', 'No copies yet. Records stay on this endpoint and remain available after reopening.')}</p>}
      {!!unavailable.length && <div className="projects-copy-unavailable"><p>{t('部分旧记录因连接变化或文件损坏不可用。仅显示本机记录 ID；可移除暂存数据以释放空间，不影响目标文件。', 'Some old records are unavailable after connection changes or file damage. Only local record IDs are shown. Remove staged data to free space; target files remain.')}</p>{unavailable.map(id => <div key={id}><code>{id}</code><button type="button" disabled={!!busy} aria-label={t('移除不可用记录 ', 'Remove unavailable record ') + id} onClick={() => { if (confirm(t('删除该本机记录和暂存快照？目标文件仍保留，这不会取消已经发出的复制。', 'Delete this local record and staged snapshot? Target files remain. This does not cancel a copy already sent.'))) void run(t('移除记录…', 'Removing record…'), async () => { await client({ operation: 'copy_discard', id }); revision.current++; await list(); }); }}>{t('移除本机记录', 'Remove local record')}</button></div>)}</div>}
      {job && <div className="projects-copy-review">
        <div role="status" aria-atomic="true"><strong>{job.summary.name} · {state(job.state)}</strong></div>
        <p>{job.source} → {job.target}<code>{job.result?.path || job.target_root}</code></p>
        <p>{job.summary.files.length} {t('个文件', 'files')} · {size(job.summary.bytes)} · {new Date(job.created_at).toLocaleString()}</p>
        {job.progress && job.state !== 'completed' && job.state !== 'prepared' && <div className="projects-copy-progress"><div><strong>{job.progress.phase === 'preparing' ? t('准备快照', 'Preparing snapshot') : t('传输到目标设备', 'Transferring to destination')}</strong><span>{size(job.progress.bytes)} / {size(job.progress.total)}</span></div><progress aria-label={t('项目复制进度', 'Project copy progress')} max={job.progress.total || 1} value={Math.min(job.progress.bytes, job.progress.total)} /></div>}
        {job.protocol === 1 && <p>{t('本次使用旧版节点协议：最多 16 MiB、单文件 4 MiB。两端升级后即可使用分块续传。', 'This copy uses the legacy node protocol: 16 MiB total and 4 MiB per file. Update both endpoints to enable resumable chunks.')}</p>}
        <ProjectCopyManifest key={job.id + job.digest} job={job} language={language} formatSize={size} />
        {job.summary.handoff && <details className="projects-handoff-review"><summary>{t('检查 Agent 交接说明', 'Review Agent handoff')}</summary><code>{job.summary.handoff_path}</code>{handoffFields.map(field => <div key={field}><h4>{fieldLabel(field)}</h4><p>{job.summary.handoff![field] || t('未填写', 'Not provided')}</p></div>)}</details>}
        {job.digest && <p>{t('快照校验值：', 'Snapshot checksum: ')}<code>{job.digest}</code></p>}
        {job.error && <div className="projects-error" role="alert">{job.error}</div>}
        {job.state === 'prepared' && <label className="projects-copy-check"><input type="checkbox" checked={reviewed} disabled={!!busy} onChange={e => setReviewed(e.target.checked)} />{t('已检查文件及跳过项，同意复制到上述设备的新目录。', 'I reviewed the files and exclusions and approve copying into a new directory on this device.')}</label>}
        {['preparing', 'prepare_paused', 'sending', 'receiving', 'publishing', 'uncertain', 'copied', 'incomplete'].includes(job.state) && <p>{t('可以关闭页面，接入端会继续处理。进程退出或网络中断后，重新打开本记录继续；传输阶段请先查询目标回执，沿用原 ID。暂存快照保留 24 小时。', 'You can close this page while the endpoint continues. After a process restart or network interruption, reopen this record to resume. Check the target receipt before resuming a transfer with the same ID. Staged snapshots expire after 24 hours.')}</p>}
        <div className="projects-copy-actions">
          {['preparing', 'prepare_paused'].includes(job.state) && <button type="button" className="projects-primary" disabled={!!busy} onClick={() => void run(t('继续生成快照…', 'Resuming snapshot…'), async () => apply(await client({ operation: 'copy_prepare_resume', id: job.id }) as CopyJob))}>{t('继续生成快照', 'Resume snapshot preparation')}</button>}
          {['prepared', 'copied', 'uncertain', 'sending', 'receiving', 'publishing'].includes(job.state) && <button type="button" className="projects-primary" disabled={!!busy || job.state === 'prepared' && !reviewed} onClick={() => { if (job.state !== 'prepared' && !confirm(t('使用相同复制 ID 查询并继续？目标会复用原回执，不覆盖已完成的目录。', 'Resume using the same copy ID? The target reuses its receipt and preserves an already completed directory.'))) return; void run(t('正在复制…', 'Copying…'), async () => { const value = await client({ operation: 'copy_commit', id: job.id, digest: job.digest }) as CopyJob; apply(value); if (value.state === 'completed') await onComplete(); }); }}><Check size={16} aria-hidden="true" />{job.state === 'prepared' ? t('确认复制此快照', 'Copy reviewed snapshot') : t('使用原 ID 继续', 'Resume original copy')}</button>}
          {['sending', 'uncertain', 'copied', 'receiving', 'publishing'].includes(job.state) && <button type="button" disabled={!!busy} onClick={() => void run(t('查询回执…', 'Checking receipt…'), async () => { const value = await client({ operation: 'copy_status', id: job.id }) as CopyJob; apply(value); if (value.state === 'completed') await onComplete(); })}>{t('查询目标回执', 'Check target receipt')}</button>}
          <button type="button" disabled={!!busy} onClick={() => { if (confirm(t('删除本机复制记录和暂存快照？目标上的文件仍保留；这不会取消已经发送的复制。', 'Delete this local record and staged snapshot? Target files remain. This does not cancel a copy already sent.'))) void run(t('移除记录…', 'Removing record…'), async () => { await client({ operation: 'copy_discard', id: job.id }); revision.current++; if (mounted.current) setSelected(''); await list(); }); }}>{t('移除本机记录', 'Remove local record')}</button>
        </div>
        {job.state === 'completed' && job.summary.handoff && <ProjectContinuation key={job.id} client={client} job={job} language={language} disabled={!!busy} />}
      </div>}
      {busy && <p role="status">{busy}</p>}{error && <div className="projects-error" role="alert">{error}</div>}
    </div>}
  </section>;
}
