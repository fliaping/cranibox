import { Grip, Maximize2, Minimize2, Minus, Terminal, X } from 'lucide-react';
import { useEffect, useRef, useState, type PointerEvent } from 'react';
import { TerminalPage } from '../pages/TerminalPage';
import { useTerminalEnvironment } from './TerminalEnvironment';
import { TerminalMenu } from './TerminalMenu';
import { useTerminalWindow } from './TerminalWindowContext';

type Rect = { x: number; y: number; width: number; height: number };
type Viewport = Rect & { mobile: boolean };
const geometryKey = 'cranibox.terminal.geometry';
function viewport(): Viewport {
  const visual = window.visualViewport, mobile = innerWidth <= 900, margin = mobile ? 0 : 12;
  const top = mobile ? 68 : 84;
  return { x: (visual?.offsetLeft || 0) + margin, y: (visual?.offsetTop || 0) + top, width: Math.max(1, (visual?.width || innerWidth) - margin * 2), height: Math.max(1, (visual?.height || innerHeight) - top - margin), mobile };
}
function constrain(rect: Rect, area = viewport()): Rect {
  const width = Math.min(area.width, Math.max(Math.min(620, area.width), rect.width));
  const height = Math.min(area.height, Math.max(Math.min(360, area.height), rect.height));
  return { width, height, x: Math.max(area.x, Math.min(area.x + area.width - width, rect.x)), y: Math.max(area.y, Math.min(area.y + area.height - height, rect.y)) };
}
function defaultRect(width = 1100, height = 690): Rect {
  const area = viewport();
  return constrain({ width, height, x: area.x + (area.width - Math.min(width, area.width)) / 2, y: area.y + (area.height - Math.min(height, area.height)) / 2 }, area);
}
function savedRect(): Rect {
  try {
    const value = JSON.parse(localStorage.getItem(geometryKey) || 'null');
    if (value && value.width > 0 && value.height > 0 && ['x', 'y', 'width', 'height'].every(key => typeof value[key] === 'number' && Number.isFinite(value[key]))) return viewport().mobile ? value : constrain(value);
  } catch { /* Geometry persistence is optional. */ }
  const rect = defaultRect();
  return viewport().mobile ? { ...rect, width: 1100, height: 690 } : rect;
}

export function TerminalWindowToggle() {
  const { t } = useTerminalEnvironment();
  const terminal = useTerminalWindow(), minimized = terminal.mode === 'minimized';
  return <button id="terminal-window-toggle" className={'terminal-window-toggle' + (terminal.mode !== 'closed' ? ' has-terminals' : '')} aria-label={minimized ? t('恢复终端窗口', 'Restore terminal window') : t('打开终端窗口', 'Open terminal window')} title={(terminal.title ? terminal.title + ' · ' : '') + (minimized ? t('点击恢复终端', 'Restore terminal') : t('终端', 'Terminal'))} aria-expanded={terminal.mode === 'open'} aria-controls="floating-terminal" onClick={() => terminal.openTerminal()}><Terminal size={18} aria-hidden="true" /><span className="terminal-window-toggle-label">{t('终端', 'Terminal')}</span>{terminal.mode !== 'closed' && <span className="terminal-window-count">{terminal.count}</span>}{minimized && <span className="terminal-window-minimized-dot" aria-hidden="true" />}</button>;
}

export function TerminalWindow({ suspended = false }: { suspended?: boolean }) {
  const terminal = useTerminalWindow(), { t, available } = useTerminalEnvironment();
  const [rect, setRect] = useState(savedRect), [area, setArea] = useState(viewport), [maximized, setMaximized] = useState(false), [moving, setMoving] = useState(false);
  const gesture = useRef<{ pointer: number; edge: string; startX: number; startY: number; rect: Rect } | null>(null);
  const frame = useRef(0), queued = useRef<Rect | null>(null), windowRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const resize = () => { const next = viewport(); setArea(next); if (!next.mobile) setRect(previous => constrain(previous, next)); };
    window.addEventListener('resize', resize); window.visualViewport?.addEventListener('resize', resize); window.visualViewport?.addEventListener('scroll', resize);
    return () => { window.removeEventListener('resize', resize); window.visualViewport?.removeEventListener('resize', resize); window.visualViewport?.removeEventListener('scroll', resize); cancelAnimationFrame(frame.current); };
  }, []);
  useEffect(() => {
    if (area.mobile) return;
    const timer = window.setTimeout(() => { try { localStorage.setItem(geometryKey, JSON.stringify(rect)); } catch { /* Optional. */ } }, 200);
    return () => window.clearTimeout(timer);
  }, [rect, area.mobile]);
  useEffect(() => {
    if (terminal.mode !== 'open') return;
    const task = requestAnimationFrame(() => {
      if (!document.querySelector('dialog[open], [popover]:popover-open')) windowRef.current?.focus({ preventScroll: true });
    });
    return () => cancelAnimationFrame(task);
  }, [terminal.mode]);
  function start(event: PointerEvent<HTMLButtonElement>, edge: string) {
    if (area.mobile || maximized || event.button !== 0) return;
    event.preventDefault(); event.currentTarget.setPointerCapture(event.pointerId);
    gesture.current = { pointer: event.pointerId, edge, startX: event.clientX, startY: event.clientY, rect }; setMoving(true);
  }
  function move(event: PointerEvent<HTMLButtonElement>) {
    const current = gesture.current; if (!current || event.pointerId !== current.pointer) return;
    const dx = event.clientX - current.startX, dy = event.clientY - current.startY, initial = current.rect;
    if (current.edge === 'move') queued.current = constrain({ ...initial, x: initial.x + dx, y: initial.y + dy }, area);
    else {
      let left = initial.x, right = initial.x + initial.width, top = initial.y, bottom = initial.y + initial.height;
      const minWidth = Math.min(620, area.width), minHeight = Math.min(360, area.height);
      if (current.edge.includes('e')) right = Math.max(left + minWidth, Math.min(area.x + area.width, right + dx));
      if (current.edge.includes('w')) left = Math.max(area.x, Math.min(right - minWidth, left + dx));
      if (current.edge.includes('s')) bottom = Math.max(top + minHeight, Math.min(area.y + area.height, bottom + dy));
      if (current.edge.includes('n')) top = Math.max(area.y, Math.min(bottom - minHeight, top + dy));
      queued.current = constrain({ x: left, y: top, width: right - left, height: bottom - top }, area);
    }
    cancelAnimationFrame(frame.current); frame.current = requestAnimationFrame(() => { if (queued.current) setRect(queued.current); });
  }
  function finish(event: PointerEvent<HTMLButtonElement>) {
    if (event.pointerId !== gesture.current?.pointer) return;
    cancelAnimationFrame(frame.current); if (queued.current) setRect(queued.current);
    queued.current = null; gesture.current = null; setMoving(false);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
  }
  function preset(width: number, height: number) { setMaximized(false); setRect(defaultRect(width, height)); }
  if (terminal.mode === 'closed' || !available) return null;
  const size = area.mobile || maximized ? area : constrain(rect, area);
  const hidden = terminal.mode === 'minimized' || suspended;
  return <div ref={windowRef} id="floating-terminal" role="dialog" aria-modal="false" aria-labelledby="terminal-window-title" tabIndex={-1} hidden={hidden} inert={hidden} className={'terminal-window' + (moving ? ' is-moving' : '') + (maximized ? ' is-maximized' : '')} style={{ left: size.x, top: size.y, width: size.width, height: size.height }}>
    <header className="terminal-window-head">
      <button className="terminal-window-drag" aria-label={t('移动终端窗口，使用方向键调整位置', 'Move terminal window, use arrow keys to adjust position')} title={t('拖动移动 · 方向键移动 · 双击最大化', 'Drag to move · Arrow keys to move · Double click to maximize')} onPointerDown={event => start(event, 'move')} onPointerMove={move} onPointerUp={finish} onPointerCancel={finish} onDoubleClick={() => setMaximized(value => !value)} onKeyDown={event => {
        if (area.mobile || maximized || !['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) return;
        event.preventDefault(); const step = event.shiftKey ? 32 : 8;
        setRect(previous => constrain({ ...previous, x: previous.x + (event.key === 'ArrowRight' ? step : event.key === 'ArrowLeft' ? -step : 0), y: previous.y + (event.key === 'ArrowDown' ? step : event.key === 'ArrowUp' ? -step : 0) }, area));
      }}><Terminal size={18} aria-hidden="true" /><strong id="terminal-window-title">{t('终端', 'Terminal')}</strong><span>{terminal.title || t('持久会话', 'Persistent sessions')}</span></button>
      <div className="terminal-window-actions">
        <TerminalMenu label={t('终端窗口布局', 'Terminal window layout')}><button role="menuitem" disabled={area.mobile} onClick={() => preset(700, 520)}>{t('紧凑窗口', 'Compact window')}</button><button role="menuitem" disabled={area.mobile} onClick={() => preset(1100, 690)}>{t('标准窗口', 'Standard window')}</button><button role="menuitem" disabled={area.mobile} onClick={() => preset(1320, 820)}>{t('大窗口', 'Large window')}</button><button role="menuitem" disabled={area.mobile} onClick={() => { setMaximized(false); setRect(defaultRect(rect.width, rect.height)); }}>{t('居中', 'Center window')}</button><button role="menuitem" disabled={area.mobile} onClick={() => { setMaximized(false); setRect(constrain({ ...rect, x: area.x, y: area.y }, area)); }}>{t('靠左', 'Align left')}</button><button role="menuitem" disabled={area.mobile} onClick={() => { setMaximized(false); setRect(constrain({ ...rect, x: area.x + area.width - rect.width, y: area.y }, area)); }}>{t('靠右', 'Align right')}</button></TerminalMenu>
        <button aria-label={t('最小化终端窗口', 'Minimize terminal window')} title={t('最小化到右上角，保持连接', 'Minimize to the top right, keep connections')} onClick={terminal.minimize}><Minus size={18} aria-hidden="true" /></button>
        <button className="terminal-window-maximize" aria-label={maximized ? t('还原终端窗口', 'Restore terminal size') : t('最大化终端窗口', 'Maximize terminal window')} title={maximized ? t('还原', 'Restore size') : t('最大化', 'Maximize')} onClick={() => setMaximized(value => !value)}>{maximized ? <Minimize2 size={17} aria-hidden="true" /> : <Maximize2 size={17} aria-hidden="true" />}</button>
        <button aria-label={t('关闭终端窗口', 'Close terminal window')} title={t('关闭窗口，保留后台会话', 'Close window, keep sessions running')} onClick={terminal.close}><X size={18} aria-hidden="true" /></button>
      </div>
    </header>
    <div className="terminal-window-body"><TerminalPage visible={!hidden} /></div>
    {!area.mobile && !maximized && ['n', 's', 'e', 'w', 'ne', 'nw', 'se', 'sw'].map(edge => <button key={edge} className={'terminal-window-resize edge-' + edge} tabIndex={edge === 'se' ? 0 : -1} aria-hidden={edge === 'se' ? undefined : true} aria-label={edge === 'se' ? t('调整终端窗口大小，使用方向键调整', 'Resize terminal window, use arrow keys to adjust') : undefined} title={edge === 'se' ? t('拖动调整大小 · 方向键调整', 'Drag or use arrow keys to resize') : undefined} onPointerDown={event => start(event, edge)} onPointerMove={move} onPointerUp={finish} onPointerCancel={finish} onKeyDown={event => {
      if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) return;
      event.preventDefault(); const step = event.shiftKey ? 32 : 8;
      setRect(previous => constrain({ ...previous, width: previous.width + (event.key === 'ArrowRight' ? step : event.key === 'ArrowLeft' ? -step : 0), height: previous.height + (event.key === 'ArrowDown' ? step : event.key === 'ArrowUp' ? -step : 0) }, area));
    }}>{edge === 'se' && <Grip size={14} aria-hidden="true" />}</button>)}
  </div>;
}
