import { Copy,Square,Terminal,X } from 'lucide-react';
import { useEffect,useRef,useState,type ReactNode } from 'react';
import { Button } from './components';
import type { Prompt } from './types';
import { openUrl,request,t,useWorkspace } from './workspace';

export function Modal({ title, children, onCancel, className = '' }: { title: string; children: ReactNode; onCancel: () => void; className?: string }) {
  const ref = useRef<HTMLDialogElement>(null);
  const restore = useRef<HTMLElement | null>(null);
  const cancelRef = useRef(onCancel); cancelRef.current = onCancel;
  useEffect(() => {
    restore.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    ref.current?.showModal();
    return () => { ref.current?.close(); if (restore.current?.isConnected) restore.current.focus(); };
  }, []);
  return <dialog ref={ref} className={'modal ' + className} aria-labelledby="dialog-title" onCancel={event => { event.preventDefault(); cancelRef.current(); }}><div className="modal-head"><h2 id="dialog-title">{title}</h2><Button local icon={X} label={t('关闭', 'Close')} onClick={onCancel} /></div>{children}</dialog>;
}
function EmergencyStop() {
  const { snapshot } = useWorkspace();
  return snapshot?.desktop?.computer_use?.running ? <div className="modal-emergency"><Button action="desktopEmergencyStop" icon={Square} tone="danger">{t('紧急停止桌面控制', 'Emergency stop desktop control')}</Button></div> : null;
}
function PromptModal({ prompt }: { prompt: Prompt }) {
  const { refresh } = useWorkspace();
  const [value, setValue] = useState(prompt.value || '');
  const [selected, setSelected] = useState<number[]>(() => prompt.canPickMany ? (prompt.items || []).flatMap((item, index) => item.picked ? [index] : []) : prompt.items?.length ? [0] : []);
  const [error, setError] = useState(''), [pending, setPending] = useState(false), [answered, setAnswered] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  async function answer(cancel = false) {
    if (pending || answered) return;
    setPending(true); setError('');
    try {
      const response = prompt.kind === 'input' ? value : prompt.kind === 'pick' ? prompt.canPickMany ? selected : selected[0] : prompt.choices?.[0];
      await request('/api/answer', { id: prompt.id, cancel, value: cancel ? null : response });
      setAnswered(true); refresh();
    } catch (problem) { setError(problem instanceof Error ? problem.message : String(problem)); inputRef.current?.focus(); }
    finally { setPending(false); }
  }
  if (answered) return null;
  return <Modal title={prompt.title || t('配置', 'Configure')} onCancel={() => void answer(true)}><form onSubmit={event => { event.preventDefault(); void answer(); }}>{prompt.prompt && <p className="muted" id="prompt-help">{prompt.prompt}</p>}{prompt.kind === 'input' && <label className="field"><span>{t('输入内容', 'Value')}</span><input ref={inputRef} autoFocus required autoComplete="off" value={value} placeholder={prompt.placeHolder} onChange={event => setValue(event.target.value)} aria-describedby={error ? 'prompt-error' : prompt.prompt ? 'prompt-help' : undefined} aria-invalid={!!error} /></label>}{prompt.kind === 'pick' && <fieldset className="choices"><legend className="sr-only">{prompt.title || t('选择项目', 'Choose items')}</legend>{(prompt.items || []).map((item, i) => <label key={i}><input type={prompt.canPickMany ? 'checkbox' : 'radio'} name="selection" checked={selected.includes(i)} onChange={() => setSelected(current => prompt.canPickMany ? current.includes(i) ? current.filter(index => index !== i) : [...current, i] : [i])} /><span><strong>{item.label}</strong>{item.description && <small>{item.description}</small>}</span></label>)}</fieldset>}{error && <p className="form-error" id="prompt-error" role="alert">{error}</p>}<div className="modal-actions"><button type="button" className="button secondary" disabled={pending} onClick={() => void answer(true)}>{t('取消', 'Cancel')}</button><button className="button primary" type="submit" disabled={pending}>{pending ? t('正在提交…', 'Submitting…') : prompt.kind === 'confirm' ? prompt.choices?.[0] || t('确认', 'Confirm') : t('继续', 'Continue')}</button></div></form><EmergencyStop /></Modal>;
}
function CommandModal() {
  const { localDialog, setLocalDialog, notify } = useWorkspace();
  const ref = useRef<HTMLTextAreaElement>(null);
  if (!localDialog) return null;
  const copy = async () => {
    try { await navigator.clipboard.writeText(localDialog.command); notify(t('命令已复制', 'Command copied')); }
    catch { ref.current?.select(); document.execCommand('copy'); }
  };
  return <Modal title={localDialog.title} onCancel={() => setLocalDialog(null)}><p className="muted">{t('打开工作区终端执行以下命令，交互式 Agent 和登录流程在终端中完成。', 'Run this command in the workspace terminal. Interactive Agents and sign-in flows continue there.')}</p>{localDialog.command && <label className="field"><span>{t('终端命令', 'Terminal command')}</span><textarea ref={ref} readOnly rows={3} value={localDialog.command} /></label>}<div className="modal-actions">{localDialog.command && <Button local icon={Copy} onClick={() => void copy()}>{t('复制命令', 'Copy command')}</Button>}<Button local icon={Terminal} tone="primary" onClick={() => openUrl(window.workspaceWeb.workspaceUrl)}>{t('打开工作区', 'Open workspace')}</Button></div><EmergencyStop /></Modal>;
}
function OutputModal() {
  const { state, setOutputOpen, cancel } = useWorkspace();
  const pre = useRef<HTMLPreElement>(null), pinned = useRef(true);
  useEffect(() => { if (pre.current && pinned.current) pre.current.scrollTop = pre.current.scrollHeight; }, [state.output]);
  return <Modal title={t('操作日志', 'Activity log')} className="output-modal" onCancel={() => setOutputOpen(false)}><div className="output-status"><span>{state.busy ? t('操作正在执行', 'Operation running') : t('没有正在执行的操作', 'No operation running')}</span>{state.busy && <Button local onClick={cancel} tone="danger-ghost">{t('取消当前操作', 'Cancel operation')}</Button>}</div><pre ref={pre} onScroll={() => { if (pre.current) pinned.current = pre.current.scrollHeight - pre.current.scrollTop - pre.current.clientHeight < 60; }}>{state.output || t('暂无操作日志。安装、服务操作等输出会显示在这里。', 'No operations yet. Installation and service output will appear here.')}</pre><p className="muted output-footnote">{t('关闭日志窗口不会取消操作。', 'Closing this window does not cancel the operation.')}</p><EmergencyStop /></Modal>;
}
export function Overlays() {
  const { state, outputOpen, localDialog } = useWorkspace();
  if (state.prompt) return <PromptModal key={state.prompt.id} prompt={state.prompt} />;
  if (localDialog) return <CommandModal />;
  if (outputOpen) return <OutputModal />;
  return null;
}
