import { useEffect, useRef, useState, type FormEvent } from 'react';
import { Keyboard, Maximize, Monitor, MousePointer2, RefreshCw, Unplug } from 'lucide-react';
// @ts-expect-error noVNC publishes its documented JavaScript API without TypeScript definitions.
import RFB from '@novnc/novnc';
import type { DesktopConnector, DesktopTarget } from './transport';

interface Viewer extends EventTarget {
  scaleViewport: boolean; resizeSession: boolean; viewOnly: boolean;
  focusOnClick: boolean; qualityLevel: number; compressionLevel: number;
  disconnect(): void; focus(): void; sendCtrlAltDel(): void;
  sendCredentials(credentials: Record<string, string>): void;
  sendKey(keysym: number, code?: string, down?: boolean): void;
}
export function VncViewer({ target, connect, language = 'en' }: { target: DesktopTarget; connect: DesktopConnector; language?: string }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const host = useRef<HTMLDivElement>(null), shell = useRef<HTMLDivElement>(null), rfb = useRef<Viewer | null>(null);
  const [attempt, setAttempt] = useState(0), [phase, setPhase] = useState<'connecting' | 'connected' | 'closed'>('connecting');
  const [error, setError] = useState(''), [types, setTypes] = useState<string[]>([]), [credentials, setCredentials] = useState<Record<string, string>>({});
  const [viewOnly, setViewOnly] = useState(false), [keyboard, setKeyboard] = useState(false);
  const [enabled, setEnabled] = useState(true);
  const [text, setText] = useState(''), composing = useRef(false);
  useEffect(() => {
    if (!enabled) return;
    let disposed = false, viewer: Viewer | null = null, transport: Awaited<ReturnType<DesktopConnector>> | undefined;
    const cancellation = new AbortController();
    setPhase('connecting'); setError(''); setTypes([]); setCredentials({}); setKeyboard(false); setText(''); composing.current = false;
    async function open() {
      try {
        transport = await connect(target, cancellation.signal);
        if (disposed || !host.current) { if (typeof transport === 'object') transport.close(); return; }
        viewer = new RFB(host.current, transport, { shared: true }) as Viewer;
        rfb.current = viewer; viewer.scaleViewport = true; viewer.resizeSession = false;
        viewer.viewOnly = false; viewer.focusOnClick = true; viewer.qualityLevel = 6; viewer.compressionLevel = 2; setViewOnly(false);
        viewer.addEventListener('connect', () => { if (!disposed) { setPhase('connected'); setTypes([]); setCredentials({}); } });
        viewer.addEventListener('disconnect', event => {
          if (disposed) return;
          setPhase('closed'); setTypes([]); setCredentials({}); setKeyboard(false);
          if (!(event as CustomEvent<{ clean: boolean }>).detail.clean) setError(previous => (typeof transport === 'object' && transport.error) || previous || t('连接已断开。检查节点、VNC 服务和授权后重新连接。', 'Connection lost. Check the node, VNC server and grant, then reconnect.'));
        });
        viewer.addEventListener('credentialsrequired', event => {
          if (!disposed) setTypes((event as CustomEvent<{ types: string[] }>).detail.types);
        });
        viewer.addEventListener('securityfailure', () => { if (!disposed) setError(t('VNC 身份验证失败，请检查凭据和服务端认证方式。', 'VNC authentication failed. Check credentials and the server’s authentication method.')); });
      } catch (error) {
        if (!disposed) { setError(error instanceof Error ? error.message : String(error)); setPhase('closed'); }
      }
    }
    void open();
    return () => { disposed = true; cancellation.abort(); viewer?.disconnect(); if (typeof transport === 'object') transport.close(); rfb.current = null; };
  }, [target.node, target.local, target.relay, connect, attempt, language, enabled]);
  function authenticate(event: FormEvent) { event.preventDefault(); rfb.current?.sendCredentials(credentials); setCredentials({}); setTypes([]); }
  function sendText(value: string) {
    for (const char of value) { const code = char.codePointAt(0)!; rfb.current?.sendKey(code > 255 ? 0x01000000 | code : code); }
    setText('');
  }
  async function fullscreen() {
    try {
      if (document.fullscreenElement) await document.exitFullscreen();
      else if (shell.current?.requestFullscreen) await shell.current.requestFullscreen();
      else throw Error('Fullscreen unavailable');
    } catch { setError(t('当前浏览器不支持全屏，请使用横屏查看。', 'Fullscreen unavailable. Rotate your device for a wider view.')); }
  }
  return <div className="vnc-viewer" ref={shell}>
    <div className="vnc-toolbar"><p role="status"><span className={'vnc-dot ' + phase} />{phase === 'connected' ? t('已连接', 'Connected') : phase === 'connecting' ? t('正在连接…', 'Connecting…') : t('已断开', 'Disconnected')}</p><div className="direct-actions">
      <button disabled={phase !== 'connected'} aria-pressed={!viewOnly} onClick={() => { if (rfb.current) rfb.current.viewOnly = !viewOnly; setViewOnly(!viewOnly); }}><MousePointer2 size={16} aria-hidden="true" />{viewOnly ? t('仅查看', 'View only') : t('允许输入', 'Input enabled')}</button>
      <button disabled={phase !== 'connected' || viewOnly} aria-expanded={keyboard} onClick={() => setKeyboard(!keyboard)}><Keyboard size={16} aria-hidden="true" />{t('键盘', 'Keyboard')}</button>
      <button disabled={phase !== 'connected'} aria-label={t('全屏桌面', 'Fullscreen desktop')} onClick={() => void fullscreen()}><Maximize size={16} aria-hidden="true" /></button>
      {phase === 'closed' ? <button onClick={() => { setEnabled(true); setAttempt(value => value + 1); }}><RefreshCw size={16} aria-hidden="true" />{t('重新连接', 'Reconnect')}</button> : <button onClick={() => { setEnabled(false); rfb.current?.disconnect(); setPhase('closed'); setTypes([]); setCredentials({}); }}><Unplug size={16} aria-hidden="true" />{t('断开', 'Disconnect')}</button>}
    </div></div>
    {keyboard && phase === 'connected' && !viewOnly && <div className="vnc-keyboard"><label>{t('输入文字', 'Type text')}<input autoComplete="off" autoCapitalize="off" spellCheck={false} aria-label={t('远程键盘输入', 'Remote keyboard input')} value={text} onCompositionStart={() => { composing.current = true; }} onCompositionEnd={event => { composing.current = false; sendText(event.currentTarget.value); }} onChange={event => { setText(event.target.value); if (!composing.current) sendText(event.target.value); }} /></label><div className="direct-actions">{[['Esc', 0xff1b], ['Tab', 0xff09], ['Enter', 0xff0d], ['Backspace', 0xff08]] .map(([name, code]) => <button key={name} onClick={() => rfb.current?.sendKey(Number(code))}>{name}</button>)}<button onClick={() => rfb.current?.sendCtrlAltDel()}>Ctrl · Alt · Del</button></div></div>}
    {error && <p className="direct-error" role="alert">{error}</p>}
    {!!types.length && <form className="vnc-auth" onSubmit={authenticate}><h3>{t('VNC 身份验证', 'VNC authentication')}</h3><p className="direct-note">{t('凭据只用于本次连接，不会保存。', 'Credentials are used for this connection only and are never saved.')}</p>{types.filter(type => ['username', 'password', 'target'].includes(type)).map(type => <label key={type}>{type === 'password' ? t('VNC 密码', 'VNC password') : type === 'username' ? t('用户名', 'Username') : t('认证目标', 'Authentication target')}<input autoFocus={type === types[0]} type={type === 'password' ? 'password' : 'text'} required autoComplete={type === 'password' ? 'current-password' : 'username'} value={credentials[type] || ''} onChange={event => setCredentials({ ...credentials, [type]: event.target.value })} /></label>)}<button type="submit" className="direct-primary">{t('验证并连接', 'Authenticate')}</button></form>}
    <div className="vnc-screen" ref={host} aria-label={t('远程桌面画面', 'Remote desktop display')} />
    {phase !== 'connected' && !types.length && <div className="vnc-placeholder"><Monitor size={36} aria-hidden="true" /><p>{phase === 'connecting' ? t('正在建立桌面连接', 'Opening desktop connection') : t('桌面仍在目标设备运行', 'The desktop keeps running on the target')}</p></div>}
    <p className="vnc-footnote">{t('关闭查看器不会注销目标桌面。“仅查看”是当前查看器的输入开关，授权仍由目标设备管理。', 'Closing the viewer keeps the target desktop running. View only disables this viewer’s input; the target manages authorization.')}</p>
  </div>;
}
