// Touch input and pane interactions adapted from devswha/herdr-web-ui (MIT).
// See docs/licenses/herdr-web-ui.txt for the upstream notice.
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import { ArrowDown, ArrowLeft, ArrowRight, ArrowUp, CornerDownLeft, Keyboard, Maximize, Minimize, Play, Plus, SendHorizontal, Unplug, X } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { Button, Status } from './ui';
import { useTerminalEnvironment, type TerminalConnection } from './TerminalEnvironment';
import type { OpenTerminal } from './types';
import { TerminalMenu } from './TerminalMenu';
import { RequesterStatus } from '../access/Access';

// The node accepts 2–500 columns and 2–300 rows; a very short pane still attaches.
const size = (cols: number, rows: number) => ({ cols: Math.min(500, Math.max(2, cols)), rows: Math.min(300, Math.max(2, rows)) });
// The node refuses a second attachment unless the client asks to take over.
const heldElsewhere = (message: string) => /already attached|take over|\b409\b/i.test(message);
const sessionGone = (message: string) => /^not found$|unknown (?:terminal )?session|no such (?:session|terminal)|(?:session|terminal|会话).*(?:not found|ended|does not exist|已结束|不存在)|(?:not found|ended).*session/i.test(message);

export function TerminalPane({ entry, visible, online, machine, pending, onClose, onRename, onEnd, onEnded, onCheckSession, onRecreate }: { entry: OpenTerminal; visible: boolean; online: boolean; machine: string; pending: boolean; onClose: () => void; onRename: () => void; onEnd: () => void; onEnded: () => void; onCheckSession: () => void; onRecreate: () => void }) {
  const { t, connect } = useTerminalEnvironment();
  const host = useRef<HTMLDivElement>(null), pane = useRef<HTMLElement>(null), inputBox = useRef<HTMLTextAreaElement>(null);
  const terminal = useRef<Terminal | null>(null), fit = useRef<FitAddon | null>(null), socket = useRef<TerminalConnection | null>(null);
  const alive = useRef(false), generation = useRef(0), attaching = useRef(false), timeout = useRef<number | undefined>(undefined);
  const wanted = useRef(true), retry = useRef<number | undefined>(undefined), backoff = useRef(0);
  const reconnect = useRef<() => void>(() => {});
  const endedCallback = useRef(onEnded), checkCallback = useRef(onCheckSession), started = useRef(false);
  endedCallback.current = onEnded; checkCallback.current = onCheckSession;
  const [status, setStatus] = useState('disconnected'), [error, setError] = useState('');
  const [draft, setDraft] = useState(entry.initialCommand || ''), [showInput, setShowInput] = useState(!!entry.initialCommand), [fontSize, setFontSize] = useState(14);
  const [fullscreen, setFullscreen] = useState(false), [held, setHeld] = useState(false);
  // Set while this pane holds the attachment, so a reconnect after a transport loss
  // replaces our own stale attachment instead of reporting a conflict.
  const owned = useRef(false);
  const approvalController = useRef<AbortController | null>(null);
  const [waiting, setWaiting] = useState(false);
  const composing = useRef(false);
  const connected = online && !entry.ended && status === 'connected', busy = online && !entry.ended && (connected || status === 'connecting');
  const markEnded = useCallback(() => { wanted.current = false; endedCallback.current(); }, []);

  const detach = useCallback((manual = true) => {
    if (manual) { wanted.current = false; owned.current = false; }
    generation.current++; attaching.current = false; window.clearTimeout(timeout.current); window.clearTimeout(retry.current);
    approvalController.current?.abort(); approvalController.current = null; setWaiting(false);
    socket.current?.close(); socket.current = null;
    if (alive.current) setStatus('disconnected');
  }, []);
  const scheduleReconnect = useCallback(() => {
    if (!alive.current || !wanted.current) return;
    window.clearTimeout(retry.current);
    setStatus('connecting');
    retry.current = window.setTimeout(() => reconnect.current(), Math.min(1000 * 2 ** backoff.current++, 15000));
  }, []);
  const attach = useCallback(async (takeover = false) => {
    if (attaching.current || socket.current || !online || entry.ended || !terminal.current) return;
    wanted.current = true; window.clearTimeout(retry.current);
    attaching.current = true; setStatus('connecting'); setError(''); setHeld(false);
    const current = ++generation.current, term = terminal.current;
    const conflict = () => { wanted.current = false; owned.current = false; setHeld(true); };
    try {
      if (host.current?.clientWidth) fit.current?.fit();
      const valid = () => alive.current && current === generation.current;
      term.reset();
      timeout.current = window.setTimeout(() => { if (valid()) { setError(t('接入超时，请重试或检查节点。', 'Attachment timed out. Retry or check the node.')); detach(false); scheduleReconnect(); } }, 20000);
      let closed = false;
      approvalController.current = new AbortController();
      const connection = await connect({ node: entry.node, session_id: entry.id, takeover, signal: approvalController.current.signal, ...size(term.cols, term.rows) }, {
        waiting: value => { if (valid()) { window.clearTimeout(timeout.current); setWaiting(value); if (!value && attaching.current) timeout.current = window.setTimeout(() => { if (valid()) { detach(false); scheduleReconnect(); } }, 20000); } },
        ready: () => { if (valid()) { window.clearTimeout(timeout.current); attaching.current = false; backoff.current = 0; owned.current = true; setError(''); setStatus('connected'); } },
        output: data => { if (valid()) term.write(data); },
        error: (message, fatal) => { if (valid()) { if (fatal) wanted.current = false; if (heldElsewhere(message)) conflict(); else { if (sessionGone(message)) markEnded(); setError(message); } } },
        // The attach client can also exit after another client takes over. Check
        // the session catalog before calling the underlying session ended.
        ended: () => { if (valid()) { wanted.current = false; owned.current = false; checkCallback.current(); } },
        closed: () => { closed = true; if (valid()) { socket.current = null; window.clearTimeout(timeout.current); attaching.current = false; setStatus('disconnected'); scheduleReconnect(); } },
      });
      if (!valid() || closed) { connection.close(); return; }
      socket.current = connection;
      connection.send({ type: 'resize', ...size(term.cols, term.rows) });
    } catch (problem) {
      if (alive.current && current === generation.current) {
        window.clearTimeout(timeout.current); attaching.current = false;
        const message = problem instanceof Error ? problem.message : String(problem);
        setStatus('disconnected');
        if (problem instanceof DOMException && problem.name === 'AbortError') { wanted.current = false; return; }
        if (heldElsewhere(message)) conflict();
        else { setError(message); if (sessionGone(message)) markEnded(); else scheduleReconnect(); }
      }
    }
  }, [entry.node, entry.id, entry.ended, online, connect, detach, scheduleReconnect, markEnded]);
  useEffect(() => { reconnect.current = () => { if (wanted.current) void attach(owned.current); }; }, [attach]);
  useEffect(() => {
    alive.current = true;
    const term = new Terminal({ cursorBlink: true, fontSize: 14, fontFamily: 'ui-monospace, SFMono-Regular, Consolas, monospace', scrollback: 3000, screenReaderMode: true, minimumContrastRatio: 4.5, theme: { background: '#0f141c', foreground: '#f1f5f9', cursor: '#a3e635', selectionBackground: '#303d50' } });
    const fitted = new FitAddon(); term.loadAddon(fitted); term.open(host.current!); terminal.current = term; fit.current = fitted;
    const send = (value: Parameters<TerminalConnection['send']>[0]) => socket.current?.send(value);
    let delivered = 0;
    const input = term.onData(data => { delivered++; send({ type: 'input', data }); });
    // Some IMEs (notably third-party ones in macOS WebKit) end a composition without
    // xterm producing data. xterm reads committed text on a 0 ms timer after
    // compositionend; ours runs after it and forwards the text only if xterm stayed silent.
    const field = term.textarea;
    const composed = (event: CompositionEvent) => {
      const text = event.data, before = delivered;
      if (text) window.setTimeout(() => { if (delivered === before) { delivered++; send({ type: 'input', data: text }); } }, 0);
    };
    field?.addEventListener('compositionend', composed);
    const resized = term.onResize(({ cols, rows }) => send({ type: 'resize', ...size(cols, rows) }));
    let frame = 0;
    const resize = () => { cancelAnimationFrame(frame); frame = requestAnimationFrame(() => { if (host.current?.clientWidth && host.current?.clientHeight) fitted.fit(); }); };
    const observer = new ResizeObserver(resize); observer.observe(host.current!); resize();
    const fullscreenChanged = () => setFullscreen(document.fullscreenElement === pane.current);
    document.addEventListener('fullscreenchange', fullscreenChanged);
    return () => { alive.current = false; detach(); observer.disconnect(); cancelAnimationFrame(frame); document.removeEventListener('fullscreenchange', fullscreenChanged); field?.removeEventListener('compositionend', composed); input.dispose(); resized.dispose(); term.dispose(); terminal.current = null; fit.current = null; };
  }, [detach]);
  useEffect(() => {
    if (entry.ended) { wanted.current = false; detach(false); }
    else if (online && wanted.current) void attach();
    else if (!online) detach(false);
  }, [online, entry.ended, attach, detach]);
  useEffect(() => {
    if (connected && entry.startCommand && !started.current && socket.current) {
      started.current = true;
      socket.current.send({ type: 'input', data: entry.startCommand + '\r' });
    }
  }, [connected, entry.startCommand]);
  useEffect(() => {
    if (!visible) return;
    const frame = requestAnimationFrame(() => {
      if (host.current?.clientWidth) fit.current?.fit();
      if (host.current?.clientWidth && status === 'connected' && !window.matchMedia('(pointer: coarse)').matches && !document.querySelector('dialog[open], [popover]:popover-open') && document.activeElement?.getAttribute('role') !== 'tab') {
        if (entry.initialCommand) inputBox.current?.focus(); else terminal.current?.focus();
      }
    });
    return () => cancelAnimationFrame(frame);
  }, [visible, status]);
  useEffect(() => { if (terminal.current) { terminal.current.options.fontSize = fontSize; if (host.current?.clientWidth) fit.current?.fit(); } }, [fontSize]);
  function key(data: string) {
    if (!socket.current || !connected) return false;
    socket.current.send({ type: 'input', data }); return true;
  }
  function sendDraft() {
    if (!connected || composing.current || !socket.current) return;
    if (draft) terminal.current?.paste(draft);
    if (key('\r')) setDraft('');
    inputBox.current?.focus();
  }
  async function toggleFullscreen() {
    try { if (document.fullscreenElement === pane.current) await document.exitFullscreen(); else await pane.current?.requestFullscreen(); }
    catch { setError(t('当前浏览器不支持全屏。', 'This browser does not support fullscreen.')); }
  }
  const shortcut = (label: string, data: string, icon?: typeof ArrowUp) => {
    const Glyph = icon;
    return <button key={label} className="terminal-key" type="button" aria-label={label} title={label} disabled={!connected} onPointerDown={event => event.preventDefault()} onClick={() => { if (key(data) && document.activeElement !== inputBox.current) terminal.current?.focus(); }}>{Glyph ? <Glyph size={17} aria-hidden="true" /> : label}</button>;
  };
  return <section ref={pane} className={'terminal-pane' + (visible ? ' is-visible' : '')} aria-label={entry.name}>
    <header className="terminal-pane-head">
      <div className="terminal-pane-title"><strong title={entry.name}>{entry.name}</strong><span title={machine + ' · ' + entry.cwd}>{machine}<span aria-hidden="true"> · </span>{entry.cwd}</span></div>
      <div className="terminal-pane-actions">
        <Status state={connected ? 'connected' : busy ? 'connecting' : 'disconnected'} label={entry.ended ? t('会话已结束', 'Session ended') : !online ? t('设备离线', 'Device offline') : status === 'connecting' ? t('接入中', 'Attaching') : status === 'disconnected' ? t('后台运行中', 'Running in background') : undefined} />
        {!entry.ended && <Button local icon={busy ? Unplug : Play} disabled={!online} label={busy ? t('脱离', 'Detach') : t('重新接回', 'Reattach')} onClick={() => busy ? detach() : void attach()} />}
        <TerminalMenu label={t('终端操作 ', 'Terminal actions ') + entry.name}>
          <button role="menuitem" disabled={entry.ended || !online} onClick={onRename}>{t('重命名会话', 'Rename session')}</button>
          <button role="menuitem" disabled={entry.ended || busy || !online} onClick={() => void attach(true)}>{t('接管已有连接', 'Take over attachment')}</button>
          <button role="menuitem" onClick={() => terminal.current?.clear()}>{t('清空显示', 'Clear display')}</button>
          <button role="menuitem" onClick={() => setShowInput(value => !value)}>{t('显示命令输入栏', 'Toggle command input')}</button>
          <button role="menuitem" disabled={fontSize >= 22} onClick={() => setFontSize(value => Math.min(22, value + 1))}>{t('放大字号', 'Increase font size')}</button>
          <button role="menuitem" disabled={fontSize <= 10} onClick={() => setFontSize(value => Math.max(10, value - 1))}>{t('缩小字号', 'Decrease font size')}</button>
          <button role="menuitem" onClick={() => void toggleFullscreen()}>{fullscreen ? t('退出全屏', 'Exit fullscreen') : t('全屏', 'Fullscreen')}</button>
          <button role="menuitem" onClick={onClose}>{entry.ended ? t('关闭标签', 'Close tab') : t('关闭标签，保留会话', 'Close tab, keep session')}</button>
          <button role="menuitem" className="danger-text" disabled={entry.ended || !online} onClick={onEnd}>{t('结束会话…', 'End session…')}</button>
        </TerminalMenu>
      </div>
    </header>
    {error && <div className="terminal-alert" role="alert"><span>{error}</span><Button local icon={X} label={t('关闭错误提示', 'Dismiss error')} onClick={() => setError('')} /></div>}
    {waiting && <RequesterStatus target={entry.node} session={entry.id} />}
    <div className="terminal-canvas" onClick={() => terminal.current?.focus()}>
      <div className="terminal-screen" ref={host} aria-label={t('交互式终端 ', 'Interactive terminal ') + entry.name} />
      {!busy && <div className="terminal-detached" role="status"><span>{entry.ended ? t('会话已结束', 'Session ended') : !online ? t('设备离线，恢复后自动接回', 'Device offline. Reattaching automatically when it returns.') : held ? t('此会话正在另一个窗口或设备上使用。接管后，那边会断开，会话继续运行。', 'This session is open in another window or device. Taking over disconnects it there; the session keeps running.') : t('后台运行中，可重新接回', 'Running in background. Reattach to continue.')}</span>{entry.ended ? <><Button local tone="primary" icon={Plus} disabled={!online || pending} onClick={onRecreate}>{t('在相同目录新建', 'New in same directory')}</Button><Button local onClick={onClose}>{t('关闭标签', 'Close tab')}</Button></> : online && (held ? <Button local tone="primary" icon={Play} onClick={() => void attach(true)}>{t('接管到这里', 'Take over here')}</Button> : <Button local tone="primary" icon={Play} onClick={() => void attach()}>{t('重新接回', 'Reattach')}</Button>)}</div>}
    </div>
    <div className="terminal-input-area">
      <div className="terminal-keybar" aria-label={t('终端快捷键', 'Terminal shortcuts')}>
        {shortcut('Esc', '\x1b')}{shortcut('Tab', '\t')}{shortcut('Ctrl+C', '\x03')}{shortcut('Ctrl+D', '\x04')}
        {shortcut(t('上方向键', 'Arrow up'), '\x1b[A', ArrowUp)}{shortcut(t('下方向键', 'Arrow down'), '\x1b[B', ArrowDown)}{shortcut(t('左方向键', 'Arrow left'), '\x1b[D', ArrowLeft)}{shortcut(t('右方向键', 'Arrow right'), '\x1b[C', ArrowRight)}{shortcut('Enter', '\r', CornerDownLeft)}
        <button className="terminal-key terminal-input-toggle" aria-label={t('命令输入栏', 'Command input')} aria-pressed={showInput} onClick={() => { setShowInput(value => !value); requestAnimationFrame(() => inputBox.current?.focus()); }}><Keyboard size={18} aria-hidden="true" /></button>
        <button className="terminal-key terminal-fullscreen" aria-label={fullscreen ? t('退出全屏', 'Exit fullscreen') : t('全屏', 'Fullscreen')} onClick={() => void toggleFullscreen()}>{fullscreen ? <Minimize size={17} aria-hidden="true" /> : <Maximize size={17} aria-hidden="true" />}</button>
      </div>
      <form className={'terminal-composer' + (showInput ? ' is-shown' : '')} onSubmit={event => { event.preventDefault(); sendDraft(); }}>
        <label className="sr-only" htmlFor={'command-' + entry.id}>{t('命令输入', 'Command input')}</label>
        <textarea id={'command-' + entry.id} ref={inputBox} value={draft} rows={1} disabled={!connected} autoCorrect="off" autoCapitalize="off" spellCheck={false} placeholder={t('输入命令，支持中文与粘贴…', 'Write a command, then send…')} onChange={event => setDraft(event.target.value)} onCompositionStart={() => { composing.current = true; }} onCompositionEnd={() => { composing.current = false; }} onKeyDown={event => { if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing && event.nativeEvent.keyCode !== 229) { event.preventDefault(); sendDraft(); } }} />
        <button className="terminal-send" type="submit" disabled={!connected} aria-label={t('发送命令', 'Send command')} title={t('发送命令', 'Send command')}><SendHorizontal size={19} aria-hidden="true" /></button>
      </form>
    </div>
  </section>;
}
