// Touch input and pane interactions adapted from devswha/herdr-web-ui (MIT).
// See docs/licenses/herdr-web-ui.txt for the upstream notice.
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import { ArrowDown, ArrowLeft, ArrowRight, ArrowUp, CornerDownLeft, Keyboard, Maximize, Minimize, Play, SendHorizontal, Unplug, X } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { Button, Status } from './ui';
import { useTerminalEnvironment, type TerminalConnection } from './TerminalEnvironment';
import type { OpenTerminal } from './types';
import { TerminalMenu } from './TerminalMenu';

export function TerminalPane({ entry, visible, online, machine, onClose, onRename, onEnd }: { entry: OpenTerminal; visible: boolean; online: boolean; machine: string; onClose: () => void; onRename: () => void; onEnd: () => void }) {
  const { t, connect } = useTerminalEnvironment();
  const host = useRef<HTMLDivElement>(null), pane = useRef<HTMLElement>(null), inputBox = useRef<HTMLTextAreaElement>(null);
  const terminal = useRef<Terminal | null>(null), fit = useRef<FitAddon | null>(null), socket = useRef<TerminalConnection | null>(null);
  const alive = useRef(false), generation = useRef(0), attaching = useRef(false), timeout = useRef<number | undefined>(undefined);
  const wanted = useRef(true), retry = useRef<number | undefined>(undefined), backoff = useRef(0);
  const reconnect = useRef<() => void>(() => {});
  const [status, setStatus] = useState('disconnected'), [error, setError] = useState('');
  const [draft, setDraft] = useState(entry.initialCommand || ''), [showInput, setShowInput] = useState(!!entry.initialCommand), [fontSize, setFontSize] = useState(14);
  const [fullscreen, setFullscreen] = useState(false);
  const composing = useRef(false);
  const connected = status === 'connected', busy = connected || status === 'connecting';

  const detach = useCallback((manual = true) => {
    if (manual) wanted.current = false;
    generation.current++; attaching.current = false; window.clearTimeout(timeout.current); window.clearTimeout(retry.current);
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
    if (attaching.current || socket.current || !online || !terminal.current) return;
    wanted.current = true; window.clearTimeout(retry.current);
    attaching.current = true; setStatus('connecting'); setError('');
    const current = ++generation.current, term = terminal.current;
    try {
      if (host.current?.clientWidth) fit.current?.fit();
      const valid = () => alive.current && current === generation.current;
      term.reset();
      timeout.current = window.setTimeout(() => { if (valid()) { setError(t('接入超时，请重试或检查节点。', 'Attachment timed out. Retry or check the node.')); detach(false); scheduleReconnect(); } }, 20000);
      let closed = false;
      const connection = await connect({ node: entry.node, session_id: entry.id, takeover, cols: term.cols, rows: term.rows }, {
        ready: () => { if (valid()) { window.clearTimeout(timeout.current); attaching.current = false; backoff.current = 0; setError(''); setStatus('connected'); } },
        output: data => { if (valid()) term.write(data); },
        error: (message, fatal) => { if (valid()) { if (fatal) wanted.current = false; setError(message); } },
        ended: () => { if (valid()) wanted.current = false; },
        closed: () => { closed = true; if (valid()) { socket.current = null; window.clearTimeout(timeout.current); attaching.current = false; setStatus('disconnected'); scheduleReconnect(); } },
      });
      if (!valid() || closed) { connection.close(); return; }
      socket.current = connection;
      connection.send({ type: 'resize', cols: term.cols, rows: term.rows });
    } catch (problem) {
      if (alive.current && current === generation.current) { window.clearTimeout(timeout.current); attaching.current = false; setError(problem instanceof Error ? problem.message : String(problem)); setStatus('disconnected'); scheduleReconnect(); }
    }
  }, [entry.node, entry.id, online, connect, detach, scheduleReconnect]);
  useEffect(() => { reconnect.current = () => { if (wanted.current) void attach(); }; }, [attach]);
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
    const resized = term.onResize(({ cols, rows }) => send({ type: 'resize', cols, rows }));
    let frame = 0;
    const resize = () => { cancelAnimationFrame(frame); frame = requestAnimationFrame(() => { if (host.current?.clientWidth && host.current?.clientHeight) fitted.fit(); }); };
    const observer = new ResizeObserver(resize); observer.observe(host.current!); resize();
    const fullscreenChanged = () => setFullscreen(document.fullscreenElement === pane.current);
    document.addEventListener('fullscreenchange', fullscreenChanged);
    return () => { alive.current = false; detach(); observer.disconnect(); cancelAnimationFrame(frame); document.removeEventListener('fullscreenchange', fullscreenChanged); field?.removeEventListener('compositionend', composed); input.dispose(); resized.dispose(); term.dispose(); terminal.current = null; fit.current = null; };
  }, [detach]);
  useEffect(() => { if (online && wanted.current) void attach(); else if (!online) detach(false); }, [online, attach, detach]);
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
        <Status state={status} label={status === 'connecting' ? t('接入中', 'Attaching') : status === 'disconnected' ? t('已脱离', 'Detached') : undefined} />
        <Button local icon={busy ? Unplug : Play} disabled={!online && !busy} label={busy ? t('脱离', 'Detach') : t('重新接回', 'Reattach')} onClick={() => busy ? detach() : void attach()} />
        <TerminalMenu label={t('终端操作 ', 'Terminal actions ') + entry.name}>
          <button role="menuitem" onClick={onRename}>{t('重命名会话', 'Rename session')}</button>
          <button role="menuitem" disabled={busy || !online} onClick={() => void attach(true)}>{t('接管已有连接', 'Take over attachment')}</button>
          <button role="menuitem" onClick={() => terminal.current?.clear()}>{t('清空显示', 'Clear display')}</button>
          <button role="menuitem" onClick={() => setShowInput(value => !value)}>{t('显示命令输入栏', 'Toggle command input')}</button>
          <button role="menuitem" disabled={fontSize >= 22} onClick={() => setFontSize(value => Math.min(22, value + 1))}>{t('放大字号', 'Increase font size')}</button>
          <button role="menuitem" disabled={fontSize <= 10} onClick={() => setFontSize(value => Math.max(10, value - 1))}>{t('缩小字号', 'Decrease font size')}</button>
          <button role="menuitem" onClick={() => void toggleFullscreen()}>{fullscreen ? t('退出全屏', 'Exit fullscreen') : t('全屏', 'Fullscreen')}</button>
          <button role="menuitem" onClick={onClose}>{t('关闭标签，保留会话', 'Close tab, keep session')}</button>
          <button role="menuitem" className="danger-text" onClick={onEnd}>{t('结束会话…', 'End session…')}</button>
        </TerminalMenu>
      </div>
    </header>
    {error && <div className="terminal-alert" role="alert"><span>{error}</span><Button local icon={X} label={t('关闭错误提示', 'Dismiss error')} onClick={() => setError('')} /></div>}
    <div className="terminal-canvas" onClick={() => terminal.current?.focus()}>
      <div className="terminal-screen" ref={host} aria-label={t('交互式终端 ', 'Interactive terminal ') + entry.name} />
      {!busy && <div className="terminal-detached"><span>{online ? t('后台会话继续运行', 'Your session keeps running') : t('节点离线，恢复后可重新接回', 'Node offline. Reattach when it returns.')}</span><Button local tone="primary" icon={Play} disabled={!online} onClick={() => void attach()}>{t('重新接回', 'Reattach')}</Button></div>}
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
