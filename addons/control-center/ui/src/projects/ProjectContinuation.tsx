import { useCallback, useEffect, useRef, useState } from 'react';
import { Pause, Play, RefreshCw, Square, Terminal } from 'lucide-react';
import type { AgentAccess, AgentAdapter, CopyClient, CopyJob } from './ProjectCopyPanel';

type Launch = { name: string; program: string; args: string[]; cwd: string; stdin?: string };
type Review = { launch: Launch; available: boolean; digest: string; execution_id: string };
type Stream = { text: string; next_offset: number };
type Detail = { owner: string; execution: { id: string; state: string; exit_code: number | null; error?: string; stdout_truncated?: boolean; stderr_truncated?: boolean }; launch: Launch; stdout: Stream; stderr: Stream };
const active = (state: string) => ['queued', 'starting', 'running', 'stopping'].includes(state);
const message = (error: unknown) => error instanceof Error ? error.message : String(error);

/** This observer owns no process. A copy UUID also identifies its single continuation run. */
export function ProjectContinuation({ client, job, language, disabled }: { client: CopyClient; job: CopyJob; language: string; disabled: boolean }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const [adapter, setAdapter] = useState<AgentAdapter>('codex'), [access, setAccess] = useState<AgentAccess>('read_only');
  const [review, setReview] = useState<Review>(), [detail, setDetail] = useState<Detail>();
  const [busy, setBusy] = useState(false), [watching, setWatching] = useState(false), [error, setError] = useState('');
  const alive = useRef(true), working = useRef(false), generation = useRef(0), current = useRef<Detail | undefined>(undefined);
  useEffect(() => {
    alive.current = true; generation.current++; current.current = undefined;
    setReview(undefined); setDetail(undefined); setWatching(false); setError('');
    return () => { alive.current = false; generation.current++; };
  }, [client, job.id]);
  const validDetail = useCallback((value: Detail) => {
    if (value?.owner !== job.target || value?.execution?.id !== job.id || value?.launch?.cwd !== job.result?.path || typeof value.stdout?.text !== 'string' || typeof value.stderr?.text !== 'string') throw Error('Unexpected continuation identity or working directory. Recheck the target device.');
    return value;
  }, [job.id, job.target, job.result?.path]);
  const perform = useCallback(async (action: (request: number) => Promise<void>) => {
    if (working.current) return;
    working.current = true; const request = generation.current; setBusy(true); setError('');
    try { await action(request); }
    catch (error) { if (alive.current && request === generation.current) { setError(message(error)); setWatching(false); } }
    finally { working.current = false; if (alive.current) setBusy(false); }
  }, []);
  const read = useCallback(() => perform(async request => {
    const previous = current.current;
    const value = validDetail(await client({ operation: 'copy_agent_status', id: job.id, stdout_offset: previous?.stdout.next_offset || 0, stderr_offset: previous?.stderr.next_offset || 0 }) as Detail);
    if (!alive.current || request !== generation.current) return;
    const next = { ...value, stdout: { ...value.stdout, text: (previous?.stdout.text || '') + value.stdout.text }, stderr: { ...value.stderr, text: (previous?.stderr.text || '') + value.stderr.text } };
    current.current = next; setDetail(next); setReview(undefined);
    // Drain paginated output even after process exit; offsets are bytes, not characters.
    if (!active(value.execution.state) && !value.stdout.text && !value.stderr.text) setWatching(false);
  }), [client, job.id, perform, validDetail]);
  useEffect(() => {
    if (!watching || disabled) return;
    const tick = () => { if (!document.hidden) void read(); };
    const timer = setInterval(tick, 2000);
    document.addEventListener('visibilitychange', tick);
    return () => { clearInterval(timer); document.removeEventListener('visibilitychange', tick); };
  }, [watching, disabled, read]);
  const accessLabel = (value: AgentAccess) => value === 'read_only' ? t('只读', 'Read only') : value === 'workspace_write' ? t('允许修改项目', 'Allow project changes') : t('允许编辑文件', 'Allow file edits');
  const state = (value: string) => ({ queued: t('排队中', 'Queued'), starting: t('启动中', 'Starting'), running: t('运行中', 'Running'), stopping: t('停止中', 'Stopping'), succeeded: t('进程正常退出', 'Process exited successfully'), failed: t('运行失败', 'Execution failed'), cancelled: t('已停止', 'Stopped'), interrupted: t('已中断，请检查', 'Interrupted · inspect results') }[value] || value);
  const locked = busy || disabled;
  return <section className="projects-continuation" aria-label={t('在目标设备接续 Agent', 'Continue Agent on destination')}>
    <div className="projects-heading"><div><h3><Terminal size={18} aria-hidden="true" />{t('在目标设备接续 Agent', 'Continue Agent on destination')}</h3><p>{t('项目已复制。检查交接说明后，在目标设备启动一个新任务。', 'The project is copied. Review the handoff, then start a new run on the destination.')}</p></div></div>
    <dl><dt>{t('目标设备', 'Destination device')}</dt><dd><code>{job.target}</code></dd><dt>{t('接续工作目录', 'Continuation directory')}</dt><dd><code>{job.result?.path}</code></dd><dt>{t('接续任务 ID', 'Continuation run ID')}</dt><dd><code>{job.id}</code></dd></dl>
    <p>{t('每个副本可从这里启动一次接续任务。关闭页面后进程继续；重开后可查询原任务。使用目标节点账户的 CLI 登录与环境，可能产生模型费用。', 'Each copy can launch one continuation here. It keeps running after this page closes; look it up after reopening. Uses the destination account’s CLI login and environment and may incur model charges.')}</p>
    {!detail && <>
      {!review ? <form onSubmit={event => { event.preventDefault(); void perform(async request => {
        const value = await client({ operation: 'copy_agent_prepare', id: job.id, adapter, access }) as Review;
        if (value?.execution_id !== job.id || value?.launch?.cwd !== job.result?.path || typeof value.available !== 'boolean' || typeof value.digest !== 'string') throw Error('Invalid continuation launch review.');
        if (alive.current && request === generation.current) setReview(value);
      }); }}>
        <div className="projects-continuation-fields"><label>{t('接续 Agent', 'Continuation Agent')}<select value={adapter} disabled={locked} onChange={event => { setAdapter(event.target.value as AgentAdapter); setAccess('read_only'); }}><option value="codex">Codex</option><option value="claude_code">Claude Code</option></select></label>
        <label>{t('接续权限', 'Continuation access')}<select value={access} disabled={locked} onChange={event => setAccess(event.target.value as AgentAccess)}><option value="read_only">{accessLabel('read_only')}</option><option value={adapter === 'codex' ? 'workspace_write' : 'edit_files'}>{accessLabel(adapter === 'codex' ? 'workspace_write' : 'edit_files')}</option></select></label></div>
        <p>{adapter === 'codex' ? t('Codex 使用所选沙箱权限，以非交互方式运行。', 'Codex runs noninteractively with the selected sandbox access.') : t('Claude Code 的编辑授权不提供系统级隔离。请先确认目标项目可信。', 'Claude Code edit permissions do not provide system isolation. Review the destination project before running.')}</p>
        <button type="submit" disabled={locked}>{t('检查目标 Agent', 'Review destination Agent')}</button>
      </form> : <div className="projects-continuation-review">
        <h4>{t('确认接续内容', 'Confirm continuation')}</h4><p>{adapter === 'codex' ? 'Codex' : 'Claude Code'} · {accessLabel(access)}</p>
        <pre className="projects-handoff-text" tabIndex={0} aria-label={t('完整接续指令', 'Full continuation instructions')}>{review.launch.stdin}</pre>
        <details><summary>{t('查看接续命令', 'View continuation command')}</summary><code>{review.launch.program}</code><code>{JSON.stringify(review.launch.args)}</code><code>{review.digest}</code></details>
        {!review.available && <p className="projects-error" role="alert">{t('目标节点未找到此 CLI。请在目标节点账户下安装并登录，再返回重新检查。', 'This CLI is missing on the destination. Install it and sign in with the destination account, then review again.')}</p>}
        <div className="projects-copy-actions"><button type="button" className="projects-primary" disabled={locked || !review.available} onClick={() => void perform(async request => {
          const value = validDetail(await client({ operation: 'copy_agent_start', id: job.id, adapter, access, digest: review.digest }) as Detail);
          if (alive.current && request === generation.current) { current.current = value; setDetail(value); setReview(undefined); setWatching(true); }
        })}><Play size={16} aria-hidden="true" />{t('确认在目标启动', 'Confirm start on destination')}</button><button type="button" disabled={locked} onClick={() => { setReview(undefined); setError(''); }}>{t('返回选择 Agent', 'Back to Agent selection')}</button></div>
        <p>{t('如启动结果未知，先查询原任务或用相同内容重试。接续 ID 固定，不会因重新打开页面而创建第二个进程。', 'If the outcome is unknown, look up the original run or retry unchanged. The fixed run ID prevents a second process after reopening.')}</p>
      </div>}
    </>}
    <div className="projects-copy-actions"><button type="button" disabled={locked} onClick={() => { setWatching(true); void read(); }}><RefreshCw size={16} aria-hidden="true" />{detail ? t('刷新接续任务', 'Refresh continuation') : t('查找已有接续任务', 'Find existing continuation')}</button>
      {detail && <button type="button" disabled={locked} aria-pressed={watching} onClick={() => setWatching(value => !value)}>{watching ? <Pause size={16} aria-hidden="true" /> : <Play size={16} aria-hidden="true" />}{watching ? t('暂停更新', 'Pause updates') : t('恢复更新', 'Resume updates')}</button>}
      {detail && active(detail.execution.state) && <button type="button" disabled={locked || detail.execution.state === 'stopping'} onClick={() => { if (confirm(t('停止这个接续任务及其子进程？未保存的工作可能丢失。', 'Stop this continuation and its child processes? Unsaved work may be lost.'))) void perform(async request => {
        const value = validDetail(await client({ operation: 'copy_agent_stop', id: job.id }) as Detail);
        if (alive.current && request === generation.current) { current.current = value; setDetail(value); setWatching(true); }
      }); }}><Square size={16} aria-hidden="true" />{t('停止接续任务', 'Stop continuation')}</button>}
    </div>
    {busy && <p role="status">{t('正在联系目标设备…', 'Contacting destination…')}</p>}
    {error && <div className="projects-error" role="alert">{error}{detail && t(' · 当前显示上次读取结果。', ' · Showing the previous result.')}</div>}
    {detail && <div className="projects-continuation-output"><p role="status" aria-atomic="true">{t('接续任务：', 'Continuation: ')}{state(detail.execution.state)}{detail.execution.exit_code !== null && (' · ' + t('退出码', 'Exit code') + ' ' + detail.execution.exit_code)}</p>{detail.execution.error && <p className="projects-error">{detail.execution.error}</p>}
      {(['stdout', 'stderr'] as const).map(stream => <details key={stream} open={stream === 'stdout'}><summary>{stream}</summary><pre className="projects-handoff-text" tabIndex={0} aria-label={t('接续 ', 'Continuation ') + stream}>{detail[stream].text || t('暂无输出', 'No output yet')}</pre></details>)}
      {(detail.execution.stdout_truncated || detail.execution.stderr_truncated) && <p>{t('每条输出流最多保存 1 MiB，后续输出未保存。', 'Each stream retains up to 1 MiB; later output was not saved.')}</p>}
      <p>{t('进程退出并不代表原协作任务完成。请检查结果；原设备的 Agent 和任务派发状态仍需单独处理。', 'Process exit does not complete the original collaboration task. Inspect the results; the source Agent and task dispatch still require separate handling.')}</p>
    </div>}
  </section>;
}
