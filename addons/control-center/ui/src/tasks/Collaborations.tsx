import { useRef, useState } from 'react';
import { Archive, CheckCheck, Layers, ListTodo, Plus, Square } from 'lucide-react';
import './collaborations.css';

export type CollaborationScope = { id: string; owner: string; name: string; goal: string; project_id?: string };
export type Collaboration = { scope: CollaborationScope; members: string[]; state: 'open' | 'stopping' | 'closed'; revision: number; created_at: number; result: string | null; stats: { total: number; active: number; succeeded: number; failed: number; cancelled: number; questions: number; blocked: number; waiting?: number } };
export type CollaborationCommand = { operation: 'collaboration'; command: { operation: 'create'; id: string; name: string; goal: string; members: string[]; project_id?: string } | { operation: 'stop' | 'archive'; id: string; revision: number } | { operation: 'close'; id: string; revision: number; result: string } };
type Mutate = (command: CollaborationCommand, after?: () => void) => Promise<void>;
const bytes = (value: string) => new TextEncoder().encode(value).length;
const uuid = (value: string) => /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value) && value.replace(/[-0]/g, '') !== '';
export const collaborationKey = (scope: CollaborationScope) => scope.owner + '/' + scope.id;

function CollaborationCard({ run, language, disabled, selected, mutate, onSelect, onDelegate }: { run: Collaboration; language: string; disabled: boolean; selected: boolean; mutate: Mutate; onSelect: () => void; onDelegate: () => void }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const [closing, setClosing] = useState(false), [result, setResult] = useState('');
  const stats = run.stats, finished = stats.total - stats.active;
  return <article className="collaboration-card" aria-label={t('协作 ', 'Collaboration ') + run.scope.name}>
    <div className="collaboration-title"><h4>{run.scope.name}</h4><span>{({ open: t('进行中', 'Open'), stopping: t('等待停止确认', 'Awaiting cancellation'), closed: t('已结束', 'Closed') })[run.state]}</span></div>
    <p>{run.scope.goal}</p>
    <div className="collaboration-progress"><progress max={Math.max(stats.total, 1)} value={finished} aria-label={t('已报告最终结果的任务', 'Tasks with final reports')} /><span>{finished} / {stats.total} {t('项已有最终报告', 'with final reports')}</span></div>
    <div className="collaboration-stats"><span>{t('成功', 'Succeeded')} {stats.succeeded}</span><span>{t('失败', 'Failed')} {stats.failed}</span><span>{t('取消', 'Cancelled')} {stats.cancelled}</span>{!!stats.waiting && <strong>{stats.waiting} {t('项等待前置任务', 'waiting for prerequisites')}</strong>}{stats.questions > 0 && <strong>{stats.questions} {t('项等待回复', 'awaiting replies')}</strong>}{stats.blocked > 0 && <strong>{stats.blocked} {t('项连接需处理', 'connections need attention')}</strong>}</div>
    <details><summary>{t('参与设备与项目', 'Devices and project')}</summary><ul>{run.members.map(node => <li key={node}><code>{node}</code></li>)}</ul><dl><div><dt>RunID</dt><dd>{run.scope.id}</dd></div><div><dt>{t('协调节点', 'Coordinating device')}</dt><dd>{run.scope.owner}</dd></div>{run.scope.project_id && <div><dt>ProjectID</dt><dd>{run.scope.project_id}</dd></div>}</dl></details>
    {run.result && <div className="collaboration-result"><strong>{t('协作总结', 'Reviewed summary')}</strong><p>{run.result}</p></div>}
    <div className="direct-actions"><button type="button" aria-pressed={selected} onClick={onSelect}><ListTodo size={16} aria-hidden="true" />{t('查看协作任务', 'View collaboration tasks')}</button>{run.state === 'open' && <button type="button" disabled={disabled} onClick={onDelegate}><Plus size={16} aria-hidden="true" />{t('添加委托', 'Add delegation')}</button>}
      {run.state === 'open' && <button type="button" disabled={disabled} onClick={() => { if (confirm(t('请求停止这次协作中的所有未完成任务？新的委托与重试将关闭；执行方仍需在安全边界停止并报告结果。', 'Request cancellation of every unfinished task in this collaboration? New delegations and retries will close. Workers must stop at a safe boundary and report their results.'))) void mutate({ operation: 'collaboration', command: { operation: 'stop', id: run.scope.id, revision: run.revision } }); }}><Square size={16} aria-hidden="true" />{t('请求停止全部', 'Request cancellation of all')}</button>}
      {run.state !== 'closed' && <button type="button" disabled={disabled || stats.active > 0} aria-expanded={closing} onClick={() => setClosing(value => !value)}><CheckCheck size={16} aria-hidden="true" />{t('核对并结束协作', 'Review and close collaboration')}</button>}
      {run.state === 'closed' && <button type="button" disabled={disabled} onClick={() => { if (confirm(t('归档这次协作及其已结束的委托？历史回执将保留到保存期限结束。', 'Archive this collaboration and its finished delegations? Receipts remain until retention expires.'))) void mutate({ operation: 'collaboration', command: { operation: 'archive', id: run.scope.id, revision: run.revision } }); }}><Archive size={16} aria-hidden="true" />{t('归档协作', 'Archive collaboration')}</button>}
    </div>
    {run.state !== 'closed' && stats.active > 0 && <small>{t('所有任务收到最终报告后，才能核对并结束协作。', 'Review and close after every task has a confirmed final report.')}</small>}
    {closing && run.state !== 'closed' && <form className="collaboration-close" onSubmit={event => { event.preventDefault(); if (result.trim() && bytes(result) <= 8192 && stats.active === 0) void mutate({ operation: 'collaboration', command: { operation: 'close', id: run.scope.id, revision: run.revision, result } }, () => setClosing(false)); }}><label>{t('核对后的协作总结', 'Reviewed collaboration summary')}<textarea autoFocus required rows={3} value={result} disabled={disabled} onChange={event => setResult(event.target.value)} /></label><small>{bytes(result)} / 8192 {t('字节', 'bytes')}</small><div className="direct-actions"><button type="submit" disabled={disabled || stats.active > 0 || !result.trim() || bytes(result) > 8192}>{t('确认结束协作', 'Confirm close collaboration')}</button><button type="button" disabled={disabled} onClick={() => setClosing(false)}>{t('返回', 'Back')}</button></div></form>}
  </article>;
}

/** A Run is a durable coordination namespace; each target retains its own execution consent. */
export function Collaborations({ runs, routes, language, disabled, used, capacity, selected, error, identifier, mutate, onSelect, onDelegate }: { runs: Collaboration[]; routes: string[]; language: string; disabled: boolean; used: number; capacity: number; selected: string; error?: string; identifier: () => string; mutate: Mutate; onSelect: (key: string) => void; onDelegate: (id: string) => void }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const [open, setOpen] = useState(false), [name, setName] = useState(''), [goal, setGoal] = useState(''), [members, setMembers] = useState<string[]>([]), [project, setProject] = useState('');
  const draft = useRef<{ key: string; id: string }>(undefined), nameField = useRef<HTMLInputElement>(null);
  const invalid = !name.trim() || !goal.trim() || bytes(name) > 240 || bytes(goal) > 4096 || !members.length || members.length > 16 || members.some(node => !routes.includes(node)) || (!!project && !uuid(project));
  return <section className="collaborations" aria-label={t('多设备协作', 'Collaboration runs')}>
    <div className="collaborations-heading"><div><h3><Layers size={20} aria-hidden="true" />{t('多设备协作', 'Collaboration runs')}</h3><p>{t('围绕同一目标组织多项委托，汇总各设备的进度与结果。', 'Organize delegations around one goal and follow progress and results across devices.')}</p></div><button type="button" disabled={disabled} aria-expanded={open} onClick={() => { setOpen(value => !value); requestAnimationFrame(() => nameField.current?.focus()); }}><Plus size={16} aria-hidden="true" />{t('新建协作', 'New collaboration')}</button></div>
    {open && <form className="collaboration-create" aria-label={t('新建多设备协作', 'Create collaboration run')} onSubmit={event => {
      event.preventDefault(); if (invalid || disabled) return;
      const key = JSON.stringify([name, goal, [...members].sort(), project]);
      if (draft.current?.key !== key) draft.current = { key, id: identifier() };
      void mutate({ operation: 'collaboration', command: { operation: 'create', id: draft.current.id, name, goal, members: [...members].sort(), ...(project ? { project_id: project } : {}) } }, () => { setName(''); setGoal(''); setMembers([]); setProject(''); draft.current = undefined; setOpen(false); });
    }}>
      <label>{t('协作名称', 'Collaboration name')}<input ref={nameField} required value={name} disabled={disabled} onChange={event => setName(event.target.value)} /></label><label>{t('共同目标与完成条件', 'Shared goal and completion criteria')}<textarea required rows={3} value={goal} disabled={disabled} onChange={event => setGoal(event.target.value)} /></label>
      <small>{t('名称', 'Name')} {bytes(name)} / 240 · {t('目标', 'Goal')} {bytes(goal)} / 4096 {t('字节', 'bytes')}</small>
      <fieldset disabled={disabled}><legend>{t('参与设备（最多 16 台）', 'Participating devices (up to 16)')}</legend>{[...new Set(routes)].map(node => <label className="collaboration-member" key={node}><input type="checkbox" checked={members.includes(node)} disabled={!members.includes(node) && members.length >= 16} onChange={event => setMembers(previous => event.target.checked ? [...previous, node] : previous.filter(member => member !== node))} /><code>{node}</code></label>)}{!routes.length && <p>{t('先在设备页面添加直连连接，并由目标授予消息权限。', 'Add a direct connection in Devices and have the target grant message access first.')}</p>}</fieldset>
      <details><summary>{t('关联项目（可选）', 'Link a project (optional)')}</summary><label>ProjectID<input value={project} disabled={disabled} aria-invalid={!!project && !uuid(project)} onChange={event => setProject(event.target.value.trim())} placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" /></label><small>{t('可从项目页面复制项目 ID。关联用于整理工作，不授予目录或执行权限。', 'Copy the project ID from Projects. Linking organizes work and grants no file or execution access.')}</small></details>
      <p className="direct-note">{t('名称、目标和参与设备在创建后固定。协作保存在发起节点；发起节点离线时，已领取的工作继续，新的协调决策等待其恢复。', 'The name, goal and members are fixed after creation. This device stores the collaboration. Claimed work continues while it is offline; new coordination decisions wait for it to return.')}</p>
      <div className="direct-actions"><button type="submit" disabled={disabled || invalid}>{t('创建协作', 'Create collaboration')}</button><button type="button" disabled={disabled} onClick={() => setOpen(false)}>{t('返回', 'Back')}</button></div>
      {used >= capacity && <p className="direct-error">{t('协作记录已满。归档结束的协作，保存期限到期后可释放空间。', 'Collaboration history is full. Archive closed runs; space is reclaimed after retention expires.')}</p>}
    </form>}
    {error && <p className="direct-error" role="alert">{error}</p>}
    {!runs.length && !open && <p className="direct-note">{t('例如将实现、跨平台验证和审查分给不同设备，放在同一次协作中跟进。', 'Keep implementation, platform checks and review on different devices together in one collaboration.')}</p>}
    <div className="collaborations-grid">{runs.map(run => <CollaborationCard key={run.scope.id} run={run} language={language} disabled={disabled} selected={selected === collaborationKey(run.scope)} mutate={mutate} onSelect={() => onSelect(collaborationKey(run.scope))} onDelegate={() => onDelegate(run.scope.id)} />)}</div>
  </section>;
}
