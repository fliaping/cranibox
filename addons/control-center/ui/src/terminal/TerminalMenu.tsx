import { Ellipsis, type LucideIcon } from 'lucide-react';
import { useEffect, useId, useRef, useState, type ReactNode } from 'react';

// Native popovers stay above scrolling lists, modal drawers and fullscreen panes.
export function TerminalMenu({ label, children, icon: Icon = Ellipsis }: { label: string; children: ReactNode; icon?: LucideIcon }) {
  const id = useId(), trigger = useRef<HTMLButtonElement>(null), menu = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  function close(restore = true) {
    menu.current?.hidePopover();
    if (restore) trigger.current?.focus();
  }
  function show(last = false) {
    menu.current?.showPopover(); position(); setOpen(true);
    const buttons = menu.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)');
    buttons?.[last ? buttons.length - 1 : 0]?.focus();
  }
  function position() {
    if (!trigger.current || !menu.current) return;
    const anchor = trigger.current.getBoundingClientRect(), popup = menu.current;
    const height = popup.getBoundingClientRect().height;
    popup.style.left = Math.max(8, Math.min(innerWidth - popup.offsetWidth - 8, anchor.right - popup.offsetWidth)) + 'px';
    popup.style.top = Math.max(8, Math.min(innerHeight - height - 8, anchor.bottom + 4)) + 'px';
  }
  useEffect(() => {
    if (!open) return;
    window.addEventListener('resize', position); window.addEventListener('scroll', position, true);
    return () => { window.removeEventListener('resize', position); window.removeEventListener('scroll', position, true); };
  }, [open]);
  return <div className="terminal-menu">
    <button ref={trigger} type="button" popoverTarget={id} aria-label={label} title={label} aria-haspopup="menu" aria-expanded={open} aria-controls={id} onClick={event => {
      event.preventDefault();
      if (menu.current?.matches(':popover-open')) close(); else show();
    }} onKeyDown={event => { if (event.key === 'ArrowDown' || event.key === 'ArrowUp') { event.preventDefault(); show(event.key === 'ArrowUp'); } }}><Icon size={18} aria-hidden="true" /></button>
    <div ref={menu} id={id} popover="auto" role="menu" aria-label={label} className="terminal-menu-items" onToggle={event => {
      const shown = event.newState === 'open'; setOpen(shown);
      if (shown) position();
    }} onClick={event => { if ((event.target as HTMLElement).closest('button')) close(); }} onKeyDown={event => {
      if (event.key === 'Escape') { event.preventDefault(); close(); return; }
      if (event.key === 'Tab') { close(); return; }
      const buttons = [...(menu.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') || [])];
      const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
      const next = event.key === 'ArrowDown' ? (current + 1) % buttons.length : event.key === 'ArrowUp' ? (current - 1 + buttons.length) % buttons.length : event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : -1;
      if (next >= 0) { event.preventDefault(); buttons[next]?.focus(); }
    }}>{children}</div>
  </div>;
}
