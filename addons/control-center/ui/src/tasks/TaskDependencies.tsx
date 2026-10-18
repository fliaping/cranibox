import { useId } from 'react';

export type DependencyGate = { state: 'waiting' | 'released' | 'cancelled_before_dispatch' | null; prerequisites: { id: string; title: string; node: string; current: { state: string; dispatch: string; result: string | null } }[] };
type Candidate = { task: { id: string; title: string; recipient: string }; current: { state: string } };

export function DependencyPicker({ items, selected, onChange, language, disabled, status }: { items: Candidate[]; selected: string[]; onChange: (ids: string[]) => void; language: string; disabled: boolean; status: (state: string) => string }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en, help = useId();
  const missing = selected.filter(id => !items.some(item => item.task.id === id));
  return <details className="task-dependency-picker"><summary>{t('前置任务（可选）', 'Prerequisite tasks (optional)')}{selected.length > 0 && <span> · {selected.length} / 8 {t('项已选', 'selected')}</span>}</summary>
    <fieldset disabled={disabled} aria-describedby={help}><legend>{t('全部成功后再委托', 'Delegate after all succeed')}</legend>
      <p id={help} className="direct-note">{t('最多选择同一协作中已保存的 8 项任务。失败、取消或离线都会继续等待。', 'Choose up to 8 saved tasks in this collaboration. Failure, cancellation or an outage keeps this task waiting.')}</p>
      <p className="direct-note">{t('不会自动复制前置产物，请在任务说明中写明要检查的版本或路径。', 'Artifacts are not copied automatically. Describe the revision or path to check in the task instructions.')}</p>
      {items.map(item => <label key={item.task.id} className="task-dependency-option"><input type="checkbox" aria-label={t('等待 ', 'Wait for ') + item.task.title + ' (' + item.task.recipient + ')'} checked={selected.includes(item.task.id)} disabled={!selected.includes(item.task.id) && selected.length >= 8} onChange={event => onChange(event.target.checked ? [...selected, item.task.id] : selected.filter(id => id !== item.task.id))} /><span><strong>{item.task.title}</strong><small>{item.task.recipient} · {status(item.current.state)}</small></span></label>)}
      {missing.map(id => <label key={id} className="task-dependency-option"><input type="checkbox" checked onChange={() => onChange(selected.filter(value => value !== id))} /><span>{t('原前置任务不可用，请取消勾选：', 'Prerequisite unavailable; deselect: ')}<code>{id}</code></span></label>)}
      {!items.length && !missing.length && <p className="direct-note">{t('先保存第一项任务，再添加依赖它的后续任务。', 'Save the first task, then add tasks that depend on it.')}</p>}
      {missing.length > 0 && <p className="direct-error">{t('请先取消已不可用的前置任务，才能保存。', 'Deselect unavailable prerequisites before saving.')}</p>}
    </fieldset>
  </details>;
}

export function TaskDependencies({ gate, language, status, expired }: { gate?: DependencyGate; language: string; status: (state: string) => string; expired: boolean }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  if (!gate?.state || !gate.prerequisites.length) return null;
  const failed = gate.prerequisites.some(item => ['failed', 'cancelled'].includes(item.current.state));
  const successful = gate.prerequisites.filter(item => item.current.state === 'succeeded').length;
  return <div className="task-dependencies">
    <strong>{gate.state === 'waiting' ? t('等待前置任务', 'Waiting for prerequisites') : gate.state === 'released' ? t('前置条件已满足', 'Prerequisites satisfied') : t('已在发出前取消', 'Cancelled before delegation')}</strong>
    <p>{gate.state === 'cancelled_before_dispatch' ? t('此尝试从未发给目标设备，已在本机取消。', 'This attempt was never sent to the target and was cancelled locally.') : gate.state === 'released' ? t('前置任务均已报告成功，后续任务已允许派发，仍需目标 Agent 领取。', 'All prerequisites reported success. Delegation is released and still needs a claim by the target Agent.') : expired ? t('领取期限已过，此任务尚未发出。请取消并重新安排。', 'The claim deadline passed and this task remains unsent. Cancel and schedule a new task.') : failed ? t('前置任务失败或已取消。请核对或重试前置任务，此任务仍未发出。', 'A prerequisite failed or was cancelled. Review or retry it; this task remains unsent.') : successful === gate.prerequisites.length ? t('前置任务均已成功，等待本机服务检查并派发。', 'All prerequisites succeeded. Waiting for this device to check and release delegation.') : t('此任务尚未发出，等待全部前置任务明确报告成功。', 'This task remains unsent until every prerequisite explicitly reports success.')}</p>
    <details><summary>{t('前置任务与报告', 'Prerequisites and reports')} · {successful} / {gate.prerequisites.length} {t('项成功', 'succeeded')}</summary><ul>{gate.prerequisites.map(item => <li key={item.id}><strong>{item.title}</strong><span>{item.node} · {status(item.current.state)}</span><code>TaskID: {item.id}</code><code>DispatchID: {item.current.dispatch}</code>{item.current.result && <p>{item.current.result}</p>}</li>)}</ul></details>
  </div>;
}
