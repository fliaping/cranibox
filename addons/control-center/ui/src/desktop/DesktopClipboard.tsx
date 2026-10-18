import { useEffect, useId, useRef, useState, type FormEvent } from 'react';
import { ArrowUpRight, Copy, RefreshCw } from 'lucide-react';
import { clipboardBytes, MAX_CLIPBOARD_BYTES } from './clipboard';

export interface ClipboardViewer {
  readonly viewOnly: boolean;
  clipboardPasteFrom(text: string): void;
  requestClipboard?(): void;
}
export function DesktopClipboard({ viewer, received, receiveError, viewOnly, language }: { viewer: ClipboardViewer | null; received: string | null; receiveError: string; viewOnly: boolean; language: string }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const id = useId(), output = useRef<HTMLTextAreaElement>(null);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const [draft, setDraft] = useState(''), [notice, setNotice] = useState(''), [error, setError] = useState('');
  const bytes = new TextEncoder().encode(draft).length, oversized = bytes > MAX_CLIPBOARD_BYTES;
  function send(event: FormEvent) {
    event.preventDefault(); if (!viewer || viewOnly || viewer.viewOnly || !draft) return;
    try {
      clipboardBytes(draft); viewer.clipboardPasteFrom(draft); setError('');
      setNotice(t('文字已发送。请在远端应用中粘贴；服务端需启用剪贴板。', 'Text sent. Paste in the remote application; the server must allow clipboard access.'));
    } catch (error) { setNotice(''); setError(error instanceof Error ? error.message : String(error)); }
  }
  async function copy() {
    if (received === null) return;
    setError(''); setNotice('');
    try {
      if (navigator.clipboard?.writeText) await navigator.clipboard.writeText(received);
      else throw Error('Clipboard API unavailable');
      if (!mounted.current) return;
      setNotice(t('已复制到本机剪贴板。', 'Copied to this device’s clipboard.'));
    } catch {
      // Keep text selectable on HTTP origins and WebViews without the async API.
      if (!mounted.current || !output.current) return;
      output.current.focus(); output.current.select();
      let copied = false;
      try { copied = document.execCommand('copy'); } catch { /* Manual selection remains available. */ }
      if (copied) setNotice(t('已复制到本机剪贴板。', 'Copied to this device’s clipboard.'));
      else setError(t('浏览器不允许自动复制。文字已选中，请使用系统复制操作。', 'Copy was blocked. The text is selected; use your system’s copy action.'));
    }
  }
  return <div className="desktop-clipboard" role="region" aria-label={t('桌面文字剪贴板', 'Desktop text clipboard')}>
    <p className="direct-note">{t('仅传递纯文本，每次最多 64 KiB。不自动读写本机剪贴板，断开连接后清除暂存文字。', 'Plain text, up to 64 KiB per transfer. This viewer never reads or automatically overwrites your clipboard. Buffered text is cleared on disconnect.')}</p>
    <div className="desktop-clipboard-columns">
      <form onSubmit={send}>
        <label htmlFor={id + '-send'}>{t('发送到远端', 'Send to remote')}</label>
        <textarea id={id + '-send'} rows={4} autoFocus autoComplete="off" autoCapitalize="off" spellCheck={false} maxLength={MAX_CLIPBOARD_BYTES + 1} value={draft} onChange={event => { setDraft(event.target.value); setNotice(''); setError(''); }} aria-describedby={id + '-size'} aria-invalid={oversized} />
        <p id={id + '-size'} className={oversized ? 'direct-error' : 'direct-note'}>{oversized ? t('文字超过 64 KiB，请缩短后发送。', 'Text exceeds 64 KiB. Shorten it before sending.') : bytes.toLocaleString() + ' / ' + MAX_CLIPBOARD_BYTES.toLocaleString() + ' ' + t('字节', 'bytes')}</p>
        {viewOnly && <p className="direct-note">{t('当前为仅查看模式。启用输入后才能发送文字。', 'View only is enabled. Enable input to send text.')}</p>}
        {!viewer?.requestClipboard && <p className="direct-note">{t('旧版 VNC 可能只支持西欧字符。中文等文字需要服务端支持扩展剪贴板。', 'Older VNC servers may only support Latin-1. Other characters require extended clipboard support on the server.')}</p>}
        <button type="submit" className="direct-primary" disabled={!viewer || viewOnly || !draft || oversized}><ArrowUpRight size={16} aria-hidden="true" />{t('发送文字', 'Send text')}</button>
      </form>
      <div className="desktop-clipboard-received">
        <label htmlFor={id + '-receive'}>{t('远端剪贴板', 'Remote clipboard')}</label>
        <textarea ref={output} id={id + '-receive'} rows={4} readOnly value={received ?? ''} placeholder={t('在远端复制文字后会显示在这里', 'Text copied on the remote desktop appears here')} autoComplete="off" spellCheck={false} />
        <p className="direct-note">{t('确认内容后再复制到本机。图片和文件不会导入。', 'Review the text before copying to this device. Images and files are not imported.')}</p>
        <div className="direct-actions">
          <button disabled={received === null} onClick={() => void copy()}><Copy size={16} aria-hidden="true" />{t('复制到本机', 'Copy to this device')}</button>
          {viewer?.requestClipboard && <button onClick={() => { viewer.requestClipboard?.(); setError(''); setNotice(t('已请求远端文字。若未更新，请检查远端剪贴板是否为空或被禁用。', 'Remote text requested. If it does not update, the remote clipboard may be empty or disabled.')); }}><RefreshCw size={16} aria-hidden="true" />{t('获取远端文字', 'Fetch remote text')}</button>}
        </div>
      </div>
    </div>
    {(receiveError || error) && <p className="direct-error" role="alert">{receiveError ? t('远端剪贴板不是有效的 64 KiB 以内纯文本，已忽略；桌面连接不受影响。', 'Remote clipboard content is invalid, unsupported or exceeds 64 KiB. It was ignored; the desktop connection is unaffected.') : error}</p>}
    {notice && <p className="direct-notice" role="status">{notice}</p>}
  </div>;
}
