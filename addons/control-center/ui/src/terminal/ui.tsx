import { Check, Circle, TriangleAlert, X, type LucideIcon } from 'lucide-react';
import { useEffect, useId, useRef, type ButtonHTMLAttributes, type ReactNode } from 'react';
import { useTerminalEnvironment } from './TerminalEnvironment';

export function Button({ children, icon: Icon, local: _local, label, tone = 'secondary', type = 'button', ...attributes }: ButtonHTMLAttributes<HTMLButtonElement> & { icon?: LucideIcon; local?: boolean; label?: string; tone?: string }) {
  return <button {...attributes} type={type} className={'button ' + tone + (!children ? ' icon-button' : '')} aria-label={label} title={label}>{Icon && <Icon size={18} strokeWidth={1.8} aria-hidden="true" />}{children}</button>;
}
export function Status({ state, label }: { state: string; label?: string }) {
  const { t } = useTerminalEnvironment(), connected = state === 'connected', detached = state === 'disconnected';
  const Icon = connected ? Check : detached ? TriangleAlert : Circle;
  return <span className={'status ' + (connected ? 'good' : detached ? 'warning' : 'neutral')}><Icon size={18} aria-hidden="true" />{label || t('已连接', 'Connected')}</span>;
}
export function Modal({ title, children, onCancel, className = '' }: { title: string; children: ReactNode; onCancel: () => void; className?: string }) {
  const { t } = useTerminalEnvironment(), id = useId(), ref = useRef<HTMLDialogElement>(null);
  const cancel = useRef(onCancel); cancel.current = onCancel;
  useEffect(() => {
    const restore = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const dialog = ref.current; dialog?.showModal();
    return () => { dialog?.close(); if (restore?.isConnected) restore.focus(); };
  }, []);
  return <dialog ref={ref} className={'modal ' + className} aria-labelledby={id} onCancel={event => { event.preventDefault(); cancel.current(); }}><div className="modal-head"><h2 id={id}>{title}</h2><Button icon={X} label={t('关闭', 'Close')} onClick={onCancel} /></div>{children}</dialog>;
}
