import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { CheckCheck, Play, RefreshCw, Square, Terminal } from 'lucide-react';
import type { DirectClient } from '../devices/DirectDevicesView';

type Options = { adapter: 'codex' | 'claude_code'; access: 'read_only' | 'workspace_write' | 'edit_files'; cwd: string; communication?: boolean };
type Launch = { program: string; args: string[]; cwd: string; stdin: string };
type Review = { owner: string; task: string; dispatch: string; digest: string; options: Options; launch: Launch; available: boolean };
type Proposal = { outcome: 'succeeded' | 'failed' | 'cancelled'; result: string; created_at: number };
type Stream = { text: string; next_offset: number };
type Detail = { owner: string; task: string; dispatch: string; run: { id: string; session: string; options: Options; digest: string; proposal?: Proposal | null; automatic?: { generation: string; report: 'human_review' | 'agent_proposal' } | null }; launch: Launch; execution: { id: string; session_id: string; state: string; exit_code: number | null; error?: string; stdout_truncated?: boolean; stderr_truncated?: boolean } | null; stdout: Stream; stderr: Stream };
export type TaskRunCommand = { operation: 'run'; id: string; dispatch: string; command: { operation: 'prepare'; options: Options } | { operation: 'start'; options: Options; digest: string } | { operation: 'get'; stdout_offset?: number; stderr_offset?: number } | { operation: 'stop' } | { operation: 'finish'; outcome: 'succeeded' | 'failed' | 'cancelled'; result: string } };
const active = (state: string) => ['queued', 'starting', 'running', 'stopping'].includes(state);
const message = (error: unknown) => error instanceof Error ? error.message : String(error);

/** The node owns the process; this observer uses the persisted task/dispatch binding. */
export function TaskRun({ client, id, dispatch, owner, hasRun, done, canStart, disabled, paused, language, onChange }: { client: DirectClient; id: string; dispatch: string; owner: string; hasRun: boolean; done: boolean; canStart: boolean; disabled: boolean; paused: boolean; language: string; onChange: () => void }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const [open, setOpen] = useState(hasRun), [busy, setBusy] = useState(false), [watching, setWatching] = useState(hasRun), [error, setError] = useState('');
  const [options, setOptions] = useState<Options>({ adapter: 'codex', access: 'read_only', cwd: '' });
  const [review, setReview] = useState<Review>(), [detail, setDetail] = useState<Detail>();
  const [outcome, setOutcome] = useState<'' | 'succeeded' | 'failed' | 'cancelled'>(''), [result, setResult] = useState('');
  const resultInput = useRef<HTMLTextAreaElement>(null);
  const alive = useRef(true), working = useRef(false), generation = useRef(0), current = useRef<Detail | undefined>(undefined), help = useId();
  useEffect(() => { alive.current = true; ++generation.current; return () => { alive.current = false; ++generation.current; }; }, [client, id, dispatch, owner]);
  const perform = useCallback(async (action: (request: number) => Promise<void>) => {
    if (working.current) return;
    working.current = true; const request = generation.current; setBusy(true); setError('');
    try { await action(request); }
    catch (error) { if (alive.current && request === generation.current) { setError(message(error)); setWatching(false); } }
    finally { working.current = false; if (alive.current) setBusy(false); }
  }, []);
  const call = useCallback((command: TaskRunCommand['command']) => client({ operation: 'tasks', command: { operation: 'run', id, dispatch, command } }), [client, id, dispatch]);
  const accept = useCallback((value: Detail, request: number, append = false) => {
    if (value?.owner !== owner || value?.task !== id || value?.dispatch !== dispatch || !value.run?.id || !value.run?.session || (value.execution && (value.execution.id !== value.run.id || value.execution.session_id !== value.run.session)) || typeof value.stdout?.text !== 'string' || typeof value.stderr?.text !== 'string') throw Error('Unexpected task execution identity. Refresh the task before continuing.');
    if (!alive.current || request !== generation.current) return;
    if (value.run.proposal && (!['succeeded', 'failed', 'cancelled'].includes(value.run.proposal.outcome) || typeof value.run.proposal.result !== 'string' || new TextEncoder().encode(value.run.proposal.result).length > 8192)) throw Error('Invalid Agent candidate result.');
    const previous = append && current.current?.run.id === value.run.id ? current.current : undefined;
    const next = { ...value, stdout: { ...value.stdout, text: (previous?.stdout.text || '') + value.stdout.text }, stderr: { ...value.stderr, text: (previous?.stderr.text || '') + value.stderr.text } };
    current.current = next; setDetail(next); setReview(undefined);
    if ((!value.execution || !active(value.execution.state)) && !value.stdout.text && !value.stderr.text) setWatching(false);
  }, [owner, id, dispatch]);
  const read = useCallback(() => perform(async request => {
    const previous = current.current;
    accept(await call({ operation: 'get', stdout_offset: previous?.stdout.next_offset || 0, stderr_offset: previous?.stderr.next_offset || 0 }) as Detail, request, true);
  }), [perform, accept, call]);
  useEffect(() => { if (hasRun) { setOpen(true); void read(); } }, [hasRun, read]);
  useEffect(() => {
    if (!watching || paused || disabled || !open) return;
    const tick = () => { if (!document.hidden) void read(); }, timer = setInterval(tick, 2000);
    document.addEventListener('visibilitychange', tick);
    return () => { clearInterval(timer); document.removeEventListener('visibilitychange', tick); };
  }, [watching, paused, disabled, open, read]);
  const locked = busy || disabled, state = detail?.execution?.state, bytes = new TextEncoder().encode(result).length;
  const stateLabel = state ? ({ queued: t('排队中', 'Queued'), starting: t('启动中', 'Starting'), running: t('运行中', 'Running'), stopping: t('停止中', 'Stopping'), succeeded: t('进程正常退出，待核对结果', 'Process exited · review result'), failed: t('运行失败，待核对结果', 'Execution failed · review result'), cancelled: t('进程已停止，待报告', 'Process stopped · report needed'), interrupted: t('节点重启中断，请检查实际结果', 'Interrupted by node restart · inspect results') }[state] || state) : t('已保留领取记录，进程尚未入队', 'Claim reserved · process not queued');
  const changed = (request: number) => { if (alive.current && request === generation.current) { setWatching(true); onChange(); } };
  const start = (value: Pick<Review, 'options' | 'digest'>) => perform(async request => { accept(await call({ operation: 'start', options: value.options, digest: value.digest }) as Detail, request); changed(request); });
  return <div className="task-run">
    <button className="task-run-toggle" type="button" aria-expanded={open} aria-controls={help + '-panel'} disabled={disabled} onClick={() => setOpen(value => !value)}><Terminal size={17} aria-hidden="true" />{hasRun || detail ? t('任务 Agent 与结果', 'Task Agent and result') : t('用 Agent 处理', 'Run with Agent')}</button>
    {open && <section id={help + '-panel'} aria-label={t('任务 Agent 执行', 'Task Agent execution')}>
      <ol className="task-run-steps" aria-label={t('任务执行步骤', 'Task execution steps')}><li aria-current={!detail ? 'step' : undefined}>1 · {t('检查并确认', 'Review')}</li><li aria-current={detail && (!state || active(state)) ? 'step' : undefined}>2 · {t('执行与查看', 'Run')}</li><li aria-current={detail && state && !active(state) ? 'step' : undefined}>3 · {t('核对并报告', 'Report')}</li></ol>
      <p className="direct-note">{t('执行设备：', 'Executing device: ')}<code>{owner}</code></p>
      {!detail && !review && <form onSubmit={event => { event.preventDefault(); void perform(async request => {
        const value = await call({ operation: 'prepare', options }) as Review;
        if (value?.owner !== owner || value?.task !== id || value?.dispatch !== dispatch || value?.launch?.cwd !== options.cwd || typeof value.digest !== 'string' || typeof value.available !== 'boolean') throw Error('Invalid task launch review.');
        if (alive.current && request === generation.current) setReview(value);
      }); }}>
        <div className="task-run-fields"><label>{t('任务 Agent', 'Task Agent')}<select disabled={locked || !canStart} value={options.adapter} onChange={event => setOptions({ ...options, adapter: event.target.value as Options['adapter'], access: 'read_only' })}><option value="codex">Codex</option><option value="claude_code">Claude Code</option></select></label><label>{t('任务权限', 'Task access')}<select disabled={locked || !canStart} value={options.access} onChange={event => setOptions({ ...options, access: event.target.value as Options['access'] })}><option value="read_only">{t('只读', 'Read only')}</option><option value={options.adapter === 'codex' ? 'workspace_write' : 'edit_files'}>{t('允许修改文件', 'Allow file changes')}</option></select></label></div>
        <label>{t('任务工作目录', 'Task working directory')}<input required disabled={locked || !canStart} value={options.cwd} onChange={event => setOptions({ ...options, cwd: event.target.value })} placeholder="/home/user/project" /></label>
        <label className="task-run-communication"><input type="checkbox" checked={!!options.communication} disabled={locked || !canStart} onChange={event => setOptions({ ...options, communication: event.target.checked })} /><span>{t('启用任务沟通工具', 'Enable task communication tools')}</span></label>
        <p className="direct-note">{t('Agent 可读取本任务回复、报告进度、提问并提交候选结果；最终结果仍需你核对。其他任务和程序启动工具不会加入此通道。', 'The Agent can read this task’s answers, report progress, ask questions and propose a result. You review the final report. This channel adds no other tasks or program launch tools.')}</p>
        <p className="direct-note">{t('确认后以本机节点账户启动，可能产生模型费用。检查不会启动程序；CLI 需预先安装并登录。', 'Confirmation starts a process using the node account and may incur model charges. Review does not start it. Install and sign in to the CLI first.')}</p>
        <p className="direct-note">{options.adapter === 'codex' ? t('使用 Codex 只读或工作区沙箱，无法交互确认的操作会被拒绝。', 'Uses the Codex read-only or workspace sandbox. Operations requiring interactive approval are denied.') : t('Claude 仅开放 Read、Glob、Grep，允许修改后增加 Edit、Write；不开放 Shell，任务沟通工具按上方选项加入。这是工具限制，文件工具仍使用节点账户权限。', 'Claude exposes Read, Glob and Grep, adding Edit and Write for file changes. Shell is disabled; the option above adds task communication. This is a tool restriction; file tools still use the node account’s permissions.')}</p>
        <button type="submit" disabled={locked || !canStart || !options.cwd.trim()}>{t('检查任务执行', 'Review task execution')}</button>
      </form>}
      {review && <div className="task-run-review"><h4>{t('确认任务执行', 'Confirm task execution')}</h4><p>{review.options.adapter === 'codex' ? 'Codex' : 'Claude Code'} · {review.options.access === 'read_only' ? t('只读', 'Read only') : t('允许修改文件', 'Allow file changes')}</p><code>{review.launch.cwd}</code>{review.options.communication && <p className="direct-note">{t('已启用本任务 MCP 沟通；候选结果不会自动提交。', 'Task MCP communication enabled; candidate results are never submitted automatically.')}</p>}<pre tabIndex={0} aria-label={t('完整任务指令', 'Full task instructions')}>{review.launch.stdin}</pre><details><summary>{t('查看实际命令', 'View actual command')}</summary><code>{review.launch.program}</code><code>{JSON.stringify(review.launch.args)}</code></details>
        {!review.available && <p className="direct-error" role="alert">{t('节点未找到此 CLI。请安装并登录后重新检查。', 'CLI not found on this node. Install it and sign in, then review again.')}</p>}
        <div className="direct-actions"><button className="direct-primary" type="button" disabled={locked || !canStart || !review.available} onClick={() => void start(review)}><Play size={16} aria-hidden="true" />{t('确认领取并启动', 'Confirm claim and start')}</button><button type="button" disabled={locked} onClick={() => setReview(undefined)}>{t('返回修改', 'Back to edit')}</button></div>
        <p className="direct-note">{t('响应不确定时先查找原运行，或用相同内容重试；同一次尝试只启动一个 Agent。', 'If the response is uncertain, find the existing run or retry unchanged. Each dispatch starts at most one Agent.')}</p></div>}
      <div className="direct-actions"><button type="button" disabled={locked} onClick={() => { setWatching(true); void read(); }}><RefreshCw size={16} aria-hidden="true" />{t('查找或刷新任务运行', 'Find or refresh task run')}</button>{detail && <button type="button" disabled={locked} aria-pressed={watching} onClick={() => setWatching(value => !value)}>{watching ? t('暂停输出更新', 'Pause output updates') : t('恢复输出更新', 'Resume output updates')}</button>}
        {detail?.execution && active(detail.execution.state) && <button type="button" disabled={locked || state === 'stopping'} onClick={() => { if (confirm(t('停止此任务的 Agent 及其子进程？停止后仍需核对并报告结果。', 'Stop this task’s Agent and its child processes? Review and report the result afterward.'))) void perform(async request => { accept(await call({ operation: 'stop' }) as Detail, request); changed(request); }); }}><Square size={16} aria-hidden="true" />{t('停止任务进程', 'Stop task process')}</button>}</div>
      {busy && <p role="status">{t('正在读取或保存…', 'Reading or saving…')}</p>}{error && <p className="direct-error" role="alert">{error}{detail && t(' · 当前显示上次结果。', ' · Showing the previous result.')}</p>}
      {detail && <>{detail.run.automatic && <p className="direct-note">{t('由本机自动接单配置启动', 'Started by this device’s automatic worker policy')} · {detail.run.automatic.report === 'human_review' ? t('人工核对后回报', 'Human review before reporting') : t('自动回报 Agent 提案', 'Automatic Agent proposal reporting')}</p>}<p role="status" aria-atomic="true">{stateLabel}{detail.execution?.exit_code != null && (' · ' + t('退出码 ', 'Exit code ') + detail.execution.exit_code)}</p><p className="direct-note">{t('运行 ID：', 'Run ID: ')}<code>{detail.run.id}</code></p>{detail.execution?.error && <p className="direct-error">{detail.execution.error}</p>}
        {!detail.execution && !done && !detail.run.automatic && <button type="button" disabled={locked || !canStart} onClick={() => void start(detail.run)}>{t('继续提交原运行', 'Resume the same launch')}</button>}
        {(['stdout', 'stderr'] as const).map(stream => <details key={stream} open={stream === 'stdout'}><summary>{stream}</summary><pre tabIndex={0} aria-label={t('任务输出 ', 'Task output ') + stream}>{detail[stream].text || t('暂无输出', 'No output yet')}</pre></details>)}
        {(detail.execution?.stdout_truncated || detail.execution?.stderr_truncated) && <p className="direct-note">{t('每条输出流最多保存 1 MiB，后续输出未保存。', 'Each stream retains up to 1 MiB; later output was not saved.')}</p>}
        {detail.run.proposal && <section className="task-run-proposal" aria-label={t('Agent 候选结果', 'Agent candidate result')}><h4>{done ? t('Agent 候选结果', 'Agent candidate result') : t('Agent 候选结果 · 待核对', 'Agent candidate result · review needed')}</h4><p>{({ succeeded: t('建议：已完成', 'Suggested: completed'), failed: t('建议：未能完成', 'Suggested: failed'), cancelled: t('建议：已取消', 'Suggested: cancelled') })[detail.run.proposal.outcome]}</p><pre tabIndex={0}>{detail.run.proposal.result}</pre><p className="direct-note">{done ? t('任务已回报。此内容由 Agent 提交，具体结论仍应结合实际证据核对。', 'This task has been reported. The candidate is an Agent claim; inspect the actual evidence.') : t('此内容由 Agent 提交。请按本任务的回报方式核对结果；单独退出进程不会完成任务。', 'This content was proposed by the Agent. Follow this task’s reporting policy; process exit alone does not complete it.')}</p>{!done && <button type="button" disabled={locked || !state || active(state)} onClick={() => { const proposal = detail.run.proposal; if (!proposal) return; if ((result || outcome) && !confirm(t('用 Agent 候选结果替换当前报告草稿？', 'Replace the current report draft with the Agent candidate result?'))) return; setResult(proposal.result); setOutcome(proposal.outcome); resultInput.current?.focus(); }}>{t('填入报告供我核对', 'Use in report for review')}</button>}</section>}
        {!done && <form className="task-run-report" aria-label={t('核对并报告任务结果', 'Review and report task result')} onSubmit={event => { event.preventDefault(); const selected = outcome; if (!selected || !result.trim() || bytes > 8192) return; void perform(async request => { await call({ operation: 'finish', outcome: selected, result }); if (alive.current && request === generation.current) { setResult(''); onChange(); } }); }}>
          <h4>{t('核对并报告任务结果', 'Review and report task result')}</h4><p className="direct-note">{t('进程退出不等于任务完成。检查输出与实际文件后，提交明确的结果和证据。报告会同步给发起设备。', 'Process exit does not complete the task. Inspect the output and actual files, then submit a result and evidence. The report synchronizes to the sender.')}</p>
          <label>{t('任务结果', 'Task outcome')}<select required value={outcome} disabled={locked || (!!state && active(state))} onChange={event => setOutcome(event.target.value as typeof outcome)}><option value="" disabled>{t('选择核对后的结果', 'Choose a reviewed outcome')}</option><option value="succeeded">{t('已完成', 'Completed')}</option><option value="failed">{t('未能完成', 'Failed')}</option><option value="cancelled">{t('已确认取消', 'Cancellation confirmed')}</option></select></label>
          <label>{t('结果与验证证据', 'Result and verification evidence')}<textarea ref={resultInput} rows={4} required disabled={locked} value={result} aria-describedby={help} aria-invalid={bytes > 8192 || undefined} onChange={event => setResult(event.target.value)} /></label><small id={help} className={bytes > 8192 ? 'direct-error' : 'direct-note'}>{bytes} / 8192 {t('字节', 'bytes')}</small>
          <button type="submit" className="direct-primary" disabled={locked || (!!state && active(state)) || !outcome || !result.trim() || bytes > 8192}><CheckCheck size={16} aria-hidden="true" />{t('提交已核对的结果', 'Submit reviewed result')}</button>
        </form>}</>}
    </section>}
  </div>;
}
