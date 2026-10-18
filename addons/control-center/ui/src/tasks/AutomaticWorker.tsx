import { useCallback, useEffect, useId, useRef, useState, type FormEvent } from 'react';
import { Bot, Pause, RefreshCw, ShieldCheck } from 'lucide-react';
import type { DirectClient } from '../devices/DirectDevicesView';
import './automatic-worker.css';

type Options = { enabled: boolean; senders: string[]; agent: { adapter: 'codex' | 'claude_code'; access: 'read_only' | 'workspace_write' | 'edit_files'; cwd: string; communication: boolean }; parallel: number; report: 'human_review' | 'agent_proposal' };
type Review = { owner: string; revision: number; options: Options; digest: string; available: boolean; launch: { program: string; args: string[]; cwd: string } };
type Status = { owner: string; revision: number; policy: Options | null; senders: { node: string; selected: boolean; binding_active: boolean }[]; ready: { directory: boolean; agent: boolean } | null; runs: { task: string; title: string; phase: string; state: string | null; error: string | null }[] };
export type AutomaticCommand = { operation: 'automatic'; command: { operation: 'get' } | { operation: 'prepare'; revision: number; options: Options } | { operation: 'save'; revision: number; options: Options; digest: string } | { operation: 'pause'; revision: number } };
const message = (error: unknown) => error instanceof Error ? error.message : String(error);

export function AutomaticWorker(props: { client: DirectClient; owner: string; language: string; paused: boolean; disabled: boolean }) {
  const [open, setOpen] = useState(false), zh = props.language.startsWith('zh');
  return <details className="automatic-worker" onToggle={event => setOpen(event.currentTarget.open)}>
    <summary><Bot size={19} aria-hidden="true" />{zh ? '本机自动接单' : 'Automatic task worker'}<span>{zh ? '默认关闭 · 按设备授权' : 'Off by default · trusted senders only'}</span></summary>
    {open && <WorkerPolicy key={props.owner} {...props} />}
  </details>;
}
function WorkerPolicy({ client, owner, language, paused, disabled }: { client: DirectClient; owner: string; language: string; paused: boolean; disabled: boolean }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const [status, setStatus] = useState<Status>(), [draft, setDraft] = useState<{ revision: number; options: Options }>(), [review, setReview] = useState<Review>();
  const [busy, setBusy] = useState(false), [reading, setReading] = useState(false), [readError, setReadError] = useState(''), [error, setError] = useState(''), [notice, setNotice] = useState(''), [consent, setConsent] = useState(false);
  const alive = useRef(true), fetching = useRef(false), writing = useRef(false), serial = useRef(0), help = useId(), reviewHeading = useRef<HTMLHeadingElement>(null);
  const call = useCallback((command: AutomaticCommand['command']) => client({ operation: 'tasks', command: { operation: 'automatic', command } }), [client]);
  const accept = useCallback((value: Status) => {
    if (value?.owner !== owner || !Number.isSafeInteger(value.revision) || !Array.isArray(value.senders) || !Array.isArray(value.runs)) throw Error('Unexpected worker policy identity. Refresh the intended device.');
    return value;
  }, [owner]);
  const read = useCallback(async () => {
    if (fetching.current || writing.current || document.hidden) return;
    fetching.current = true; const request = ++serial.current; setReading(true);
    try { const value = accept(await call({ operation: 'get' }) as Status); if (alive.current && request === serial.current) { setStatus(value); setReadError(''); } }
    catch (error) { if (alive.current && request === serial.current) setReadError(message(error)); }
    finally { fetching.current = false; if (alive.current) setReading(false); }
  }, [accept, call]);
  useEffect(() => { alive.current = true; void read(); return () => { alive.current = false; ++serial.current; }; }, [read]);
  useEffect(() => {
    if (paused) return;
    const tick = () => void read(), timer = setInterval(tick, 3000);
    document.addEventListener('visibilitychange', tick); window.addEventListener('cranibox:peers-changed', tick);
    return () => { clearInterval(timer); document.removeEventListener('visibilitychange', tick); window.removeEventListener('cranibox:peers-changed', tick); };
  }, [paused, read]);
  useEffect(() => { if (review) reviewHeading.current?.focus(); }, [review]);
  async function mutate(action: () => Promise<void>) {
    if (writing.current) return; writing.current = true; ++serial.current; setBusy(true); setError(''); setNotice('');
    try { await action(); } catch (error) { if (alive.current) setError(message(error)); }
    finally { writing.current = false; if (alive.current) setBusy(false); }
  }
  function edit() {
    if (!status) return;
    setDraft({ revision: status.revision, options: status.policy ? structuredClone(status.policy) : { enabled: false, senders: [], agent: { adapter: 'codex', access: 'read_only', cwd: '', communication: true }, parallel: 1, report: 'human_review' } }); setReview(undefined); setConsent(false); setError(''); setNotice('');
  }
  function change(value: Partial<Options>) { if (draft) { setDraft({ ...draft, options: { ...draft.options, ...value } }); setReview(undefined); setConsent(false); } }
  function prepare(event: FormEvent) {
    event.preventDefault(); if (!draft) return;
    reviewDraft(draft);
  }
  function reviewDraft(value: { revision: number; options: Options }) {
    void mutate(async () => {
      const reviewed = await call({ operation: 'prepare', ...value }) as Review;
      if (reviewed?.owner !== owner || reviewed?.revision !== value.revision || typeof reviewed.digest !== 'string' || !reviewed.options || typeof reviewed.available !== 'boolean') throw Error('Unexpected worker policy review. Reload and review again.');
      if (alive.current) { setReview(reviewed); setConsent(false); }
    });
  }
  const locked = busy || disabled || !!readError, enabled = !!status?.policy?.enabled, active = status?.runs.filter(run => run.phase === 'active').length || 0;
  const senderIds = [...new Set([...(status?.senders.map(sender => sender.node) || []), ...(draft?.options.senders || [])])];
  const phase = (value: string) => ({ active: t('正在执行', 'Running'), reserved: t('等待进程入队', 'Waiting to enqueue'), paused: t('已暂停启动', 'Launch paused'), review_required: t('需要核对结果', 'Review needed'), grant_revoked: t('授权已失效', 'Grant no longer valid'), reported: t('已回报', 'Reported') }[value] || value);
  const reporting = (value: Options['report']) => value === 'human_review' ? t('人工核对后回报', 'Human review before reporting') : t('自动回报 Agent 提案', 'Report Agent proposals automatically');
  return <div className="automatic-worker-body">
    <p className="direct-note">{t('本机是运行此界面的节点。浏览器访问时，是 Workspace 服务所在的设备。', 'This device runs the node behind this interface. In a browser, it is the device hosting Workspace.')}<code>{owner}</code></p>
    <p className="direct-note">{t('明确启用后，可信设备发来的普通任务会在后台启动 Agent，已有待领取任务也会处理。发送方不能指定本机目录、程序或权限。指定给已有会话的任务仍由该会话领取。', 'When enabled, ordinary tasks from trusted senders start an Agent in the background, including tasks already waiting. Senders cannot choose this device’s directory, program or access. Tasks addressed to existing sessions remain with those sessions.')}</p>
    <div className="automatic-worker-status"><strong>{enabled ? t('自动接单已启用', 'Automatic acceptance enabled') : t('自动接单已关闭', 'Automatic acceptance off')}</strong><span>{active} / {status?.policy?.parallel || 1} {t('个进程执行中', 'processes running')}</span></div>
    {readError && <p className="direct-error" role="alert">{readError}{status && t(' · 显示上次读取的设置。', ' · Showing previously read settings.')}</p>}
    {status?.policy && <div className="automatic-worker-current"><p><strong>{status.policy.agent.adapter === 'codex' ? 'Codex' : 'Claude Code'}</strong> · {status.policy.agent.access === 'read_only' ? t('只读', 'Read only') : t('允许修改文件', 'File changes allowed')} · {reporting(status.policy.report)}</p><code>{status.policy.agent.cwd}</code>
      {status.policy.senders.map(node => <p key={node}><code>{node}</code> · {status.senders.some(sender => sender.node === node && sender.binding_active) ? t('授权有效', 'Grant valid') : t('授权已移除或更换；需重新检查并确认', 'Grant removed or rotated; review and confirm again')}</p>)}
      {status.ready && (!status.ready.agent || !status.ready.directory) && <p className="direct-error" role="alert">{t('Agent 程序或工作目录不可用，暂不启动新任务。请检查节点环境并重新确认配置。', 'Agent executable or directory unavailable. New launches are blocked. Check the device and review the configuration again.')}</p>}
    </div>}
    <div className="direct-actions"><button disabled={locked || !status} onClick={edit}>{t('配置自动接单', 'Configure automatic worker')}</button>{enabled && <button disabled={locked} onClick={() => { if (!status) return; void mutate(async () => { const value = accept(await call({ operation: 'pause', revision: status.revision }) as Status); if (alive.current) { setStatus(value); setDraft(undefined); setReview(undefined); setNotice(t('已停止接收新任务。已启动的进程继续运行，可在各任务中停止。', 'New launches paused. Existing processes continue; stop them in their task controls.')); } }); }}><Pause size={16} aria-hidden="true" />{t('暂停新任务', 'Pause new launches')}</button>}<button disabled={busy || reading} onClick={() => void read()}><RefreshCw size={16} aria-hidden="true" />{t('刷新接单状态', 'Refresh worker status')}</button></div>
    {busy && <p role="status">{t('正在检查或保存…', 'Checking or saving…')}</p>}{error && <p className="direct-error" role="alert">{error}</p>}{notice && <p className="messages-notice" role="status">{notice}</p>}
    {draft && !review && <form onSubmit={prepare} aria-label={t('配置本机自动接单', 'Configure this device’s automatic worker')}>
      {status && status.revision !== draft.revision && <div className="automatic-worker-current"><p role="status">{t('接单配置已在其他位置更改。你的修改仍保留；请按当前版本重新核对，保存后将替换其他位置的设置。', 'The worker policy changed elsewhere. Your edits are preserved. Review against the current version; saving will replace those settings.')}</p><div className="direct-actions"><button type="button" disabled={locked || !draft.options.senders.length || !draft.options.agent.cwd.trim()} onClick={() => { const next = { ...draft, revision: status.revision }; setDraft(next); reviewDraft(next); }}>{t('保留修改并重新检查', 'Keep edits and review again')}</button></div></div>}
      <label className="automatic-worker-check"><input type="checkbox" checked={draft.options.enabled} disabled={locked} onChange={event => change({ enabled: event.target.checked })} /><span>{t('确认保存后启用自动接单', 'Enable automatic acceptance after confirmation')}</span></label>
      <fieldset disabled={locked}><legend>{t('允许自动接单的发送设备', 'Trusted task senders')}</legend>{!senderIds.length && <p>{t('请先在设备页面向另一台设备授予消息权限。', 'First grant another device messages access on the Devices page.')}</p>}{senderIds.map(node => <label className="automatic-worker-check" key={node}><input type="checkbox" checked={draft.options.senders.includes(node)} onChange={event => change({ senders: event.target.checked ? [...draft.options.senders, node] : draft.options.senders.filter(value => value !== node) })} /><span><code>{node}</code>{!status?.senders.some(sender => sender.node === node) && <small>{t('消息授权已移除，请取消选择', 'Messages grant removed; deselect this device')}</small>}</span></label>)}</fieldset>
      <div className="automatic-worker-fields"><label>{t('接单 Agent', 'Worker Agent')}<select disabled={locked} value={draft.options.agent.adapter} onChange={event => change({ agent: { ...draft.options.agent, adapter: event.target.value as Options['agent']['adapter'], access: 'read_only' } })}><option value="codex">Codex</option><option value="claude_code">Claude Code</option></select></label><label>{t('接单权限', 'Worker access')}<select disabled={locked} value={draft.options.agent.access} onChange={event => change({ agent: { ...draft.options.agent, access: event.target.value as Options['agent']['access'] } })}><option value="read_only">{t('只读', 'Read only')}</option><option value={draft.options.agent.adapter === 'codex' ? 'workspace_write' : 'edit_files'}>{t('允许修改文件', 'Allow file changes')}</option></select></label></div>
      <label>{t('固定工作目录', 'Fixed working directory')}<input required disabled={locked} value={draft.options.agent.cwd} onChange={event => change({ agent: { ...draft.options.agent, cwd: event.target.value } })} placeholder="/home/user/project" /></label>
      <div className="automatic-worker-fields"><label>{t('最大并发进程', 'Maximum parallel processes')}<input type="number" required min={1} max={4} step={1} value={Number.isNaN(draft.options.parallel) ? '' : draft.options.parallel} disabled={locked} onChange={event => change({ parallel: event.target.valueAsNumber })} /></label><label>{t('结果回报方式', 'Result reporting')}<select disabled={locked} value={draft.options.report} onChange={event => change({ report: event.target.value as Options['report'] })}><option value="human_review">{reporting('human_review')}</option><option value="agent_proposal">{reporting('agent_proposal')}</option></select></label></div>
      <p className="direct-note">{t('CLI 需预先安装并登录。Codex 使用所选沙箱；Claude 仅使用文件工具，不开放 Shell。每个 Agent 仅加入本任务的沟通工具。', 'Install and sign in to the CLI first. Codex uses the selected sandbox; Claude uses file tools without Shell. Each Agent receives communication tools for its own task only.')}</p>
      <div className="direct-actions"><button type="submit" disabled={locked || (status && status.revision !== draft.revision) || !draft.options.senders.length || !draft.options.agent.cwd.trim()}><ShieldCheck size={16} aria-hidden="true" />{t('检查接单配置', 'Review worker policy')}</button><button type="button" disabled={busy} onClick={() => setDraft(undefined)}>{t('取消修改', 'Cancel changes')}</button></div>
    </form>}
    {review && <section className="automatic-worker-review" aria-label={t('确认自动接单配置', 'Confirm automatic worker policy')}><h3 ref={reviewHeading} tabIndex={-1}>{review.options.enabled ? t('确认启用后台接单', 'Confirm background acceptance') : t('确认保存停用的配置', 'Save disabled configuration')}</h3><p>{review.options.senders.length} {t('台可信发送设备', 'trusted sender devices')} · {review.options.parallel} {t('个并发进程', 'parallel processes')} · {reporting(review.options.report)}</p><p><strong>{review.options.agent.adapter === 'codex' ? 'Codex' : 'Claude Code'}</strong> · {review.options.agent.access === 'read_only' ? t('只读', 'Read only') : t('允许修改文件', 'File changes allowed')}</p><ul aria-label={t('已核对的发送设备', 'Reviewed sender devices')}>{review.options.senders.map(node => <li key={node}><code>{node}</code></li>)}</ul><code>{review.launch.cwd}</code><details><summary>{t('查看 Agent 命令', 'View Agent command')}</summary><code>{review.launch.program}</code><code>{JSON.stringify(review.launch.args)}</code></details>
      <p id={help} className="direct-note">{t('启用后无需逐个确认任务，可能产生模型费用并按所选权限修改文件。关闭页面不停止执行；暂停只停止新任务。撤销或更换发送方的消息授权会停止相关自动进程，重新授权后也需重新确认配置。', 'Enabling removes per-task launch confirmation, may incur model charges and allows file changes according to the selected access. Closing the page does not stop execution; pausing stops new launches. Removing or rotating a sender’s messages grant stops its automatic processes and requires a fresh policy review.')}</p>
      {review.options.report === 'agent_proposal' && <p className="direct-note">{t('只有 Agent 明确提交候选结果且进程已正常停止，才会回报；成功提案要求退出码 0。无提案、未答疑问或节点重启中断都需要人工处理。报告属于 Agent 声明，不能替代实际验证。', 'Reporting requires an explicit Agent proposal and a stopped process; success proposals require exit code 0. Missing proposals, unanswered questions or interrupted runs need human review. Reports are Agent claims, not independent verification.')}</p>}
      {!review.available && <p className="direct-error" role="alert">{t('未找到 Agent CLI，请安装登录后重新检查。', 'Agent CLI not found. Install it, sign in, then review again.')}</p>}
      <label className="automatic-worker-check"><input type="checkbox" checked={consent} disabled={locked} aria-describedby={help} onChange={event => setConsent(event.target.checked)} /><span>{t('我已核对发送设备、目录、权限与回报方式', 'I reviewed the senders, directory, access and reporting policy')}</span></label><div className="direct-actions"><button className="direct-primary" disabled={locked || !consent || !review.available} onClick={() => void mutate(async () => { const value = accept(await call({ operation: 'save', revision: review.revision, options: review.options, digest: review.digest }) as Status); if (alive.current) { setStatus(value); setReview(undefined); setDraft(undefined); setConsent(false); setNotice(t('自动接单配置已保存。', 'Automatic worker policy saved.')); } })}>{review.options.enabled ? t('确认启用自动接单', 'Confirm and enable worker') : t('确认保存配置', 'Confirm and save policy')}</button><button disabled={busy} onClick={() => { setReview(undefined); setConsent(false); }}>{t('返回修改', 'Back to edit')}</button></div>
    </section>}
    {!!status?.runs.filter(run => run.phase !== 'reported').length && <ul className="automatic-worker-runs" aria-label={t('自动接单运行', 'Automatic task runs')}>{status.runs.filter(run => run.phase !== 'reported').map(run => <li key={run.task}><strong>{run.title}</strong><span>{phase(run.phase)}{run.state && ` · ${run.state}`}</span><code>{run.task}</code>{run.error && <p className="direct-error">{run.error}</p>}</li>)}</ul>}
  </div>;
}
