import { useCallback, useEffect, useRef, useState } from 'react';
import { ExternalLink, Maximize, Monitor, RefreshCw, Unplug } from 'lucide-react';
import { request } from '../workspace';
import type { DesktopTarget } from './transport';

type Page = { url: string; protocol: 'selkies' | 'vnc' };
/** The shared desktop's own web interface: Selkies as served by the target, or noVNC
 * for VNC. The node forwards it on a separate origin, so it never sees this console. */
export function DesktopViewer({ target, language = 'en', openExternal }: { target: DesktopTarget; language?: string; openExternal?: (url: string) => Promise<unknown> }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const shell = useRef<HTMLDivElement>(null);
  const [page, setPage] = useState<Page | null>(null), [phase, setPhase] = useState<'connecting' | 'connected' | 'browser' | 'closed'>('connecting');
  const [error, setError] = useState(''), [attempt, setAttempt] = useState(0), [busy, setBusy] = useState(false);
  const issue = useCallback(() => request('/api/desktop/page', { node: target.node, local: target.local }) as Promise<Page>, [target.node, target.local]);
  useEffect(() => {
    let gone = false;
    setPhase('connecting'); setError(''); setPage(null);
    issue().then(value => { if (!gone) { setPage(value); setPhase('connected'); } })
      .catch(e => { if (!gone) { setError(e instanceof Error ? e.message : String(e)); setPhase('closed'); } });
    return () => { gone = true; };
  }, [issue, attempt]);
  function disconnect() { setPage(null); setPhase('closed'); setError(''); }
  async function fullscreen() {
    try {
      if (document.fullscreenElement) await document.exitFullscreen();
      else if (shell.current?.requestFullscreen) await shell.current.requestFullscreen();
      else throw Error('Fullscreen unavailable');
    } catch { setError(t('当前浏览器不支持全屏，请使用“在浏览器打开”。', 'Fullscreen is unavailable here. Use Open in browser instead.')); }
  }
  async function openInBrowser() {
    // A fresh one-time link: the browser keeps its own session. Close the embedded
    // page first, since a desktop accepts one controller at a time.
    setBusy(true); setError(''); setPage(null);
    const tab = openExternal ? null : window.open('about:blank', '_blank');
    try {
      let value: Page | undefined, failure: unknown;
      for (let tries = 0; tries < 6 && !value; tries++) {
        if (tries) await new Promise(resolve => setTimeout(resolve, 500));
        try { value = await issue(); } catch (e) { failure = e; }
      }
      if (!value) throw failure;
      if (openExternal) await openExternal(value.url);
      else if (tab) { tab.opener = null; tab.location.replace(value.url); }
      else throw Error(t('浏览器阻止了新窗口，请允许弹出窗口后重试。', 'The browser blocked the new window. Allow pop-ups and retry.'));
      setPhase('browser');
    } catch (e) { tab?.close(); setError(e instanceof Error ? e.message : String(e)); setPhase('closed'); }
    finally { setBusy(false); }
  }
  const status = phase === 'connected' ? t('已连接', 'Connected') : phase === 'connecting' ? t('正在连接…', 'Connecting…') : phase === 'browser' ? t('已在浏览器中打开', 'Opened in browser') : t('已断开', 'Disconnected');
  return <div className="vnc-viewer" ref={shell}>
    <div className="vnc-toolbar"><div className="vnc-title"><p role="status"><span className={'vnc-dot ' + phase} />{status}</p>{page && <small className="vnc-protocol">{page.protocol === 'selkies' ? 'Selkies' : 'noVNC'}</small>}</div><div className="direct-actions">
      <button disabled={phase !== 'connected'} aria-label={t('全屏桌面', 'Fullscreen desktop')} title={t('全屏', 'Fullscreen')} onClick={() => void fullscreen()}><Maximize size={16} aria-hidden="true" /></button>
      <button disabled={busy || phase === 'connecting'} onClick={() => void openInBrowser()}><ExternalLink size={16} aria-hidden="true" />{t('在浏览器打开', 'Open in browser')}</button>
      {phase === 'connected' || phase === 'connecting' ? <button disabled={busy} onClick={disconnect}><Unplug size={16} aria-hidden="true" />{t('断开', 'Disconnect')}</button> : <button disabled={busy} onClick={() => setAttempt(value => value + 1)}><RefreshCw size={16} aria-hidden="true" />{phase === 'browser' ? t('在此显示', 'Show here') : t('重新连接', 'Reconnect')}</button>}
    </div></div>
    {error && <p className="direct-error" role="alert">{error}</p>}
    {page ? <iframe className="vnc-screen vnc-frame" src={page.url} title={t('远程桌面画面', 'Remote desktop display')} allow="fullscreen; clipboard-read; clipboard-write; autoplay; microphone; camera; gamepad; keyboard-map" />
      : <div className="vnc-placeholder"><Monitor size={36} aria-hidden="true" /><p>{phase === 'connecting' ? t('正在建立桌面连接', 'Opening desktop connection') : phase === 'browser' ? t('桌面已在浏览器中打开。需要时可在此重新显示。', 'The desktop is open in your browser. Show it here again any time.') : t('桌面仍在目标设备运行', 'The desktop keeps running on the target')}</p></div>}
    <p className="vnc-footnote">{t('显示的是目标桌面服务自己的界面（Selkies 或 noVNC），经授权通道原样转发。关闭查看器不会注销目标桌面。', 'This is the desktop server’s own interface (Selkies or noVNC), forwarded unchanged through the authorized connection. Closing the viewer keeps the target desktop running.')}</p>
  </div>;
}
