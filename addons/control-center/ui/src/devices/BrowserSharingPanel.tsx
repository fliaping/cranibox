import { useEffect, useId, useRef, useState } from 'react';
import { Globe, OctagonX, RefreshCw } from 'lucide-react';
import type { DirectCatalog, DirectClient } from './DirectDevicesView';

type Status = { paused: boolean; backend_available?: boolean; browser?: { version: string } | null };
export function BrowserSharingPanel({ client, catalog, language, busy, run, onCatalog }: { client: DirectClient; catalog: DirectCatalog; language: string; busy: boolean; run: (action: () => Promise<void>) => Promise<void>; onCatalog: (value: DirectCatalog) => void }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const id = useId(), configured = catalog.browser?.address || '';
  const [address, setAddress] = useState(configured), [status, setStatus] = useState<Status>(), [saved, setSaved] = useState(false);
  const [stopping, setStopping] = useState(false), [error, setError] = useState('');
  const serial = useRef(0), mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; ++serial.current; }; }, []);
  useEffect(() => { ++serial.current; setAddress(configured); setStatus(undefined); }, [configured]);
  async function control(action: 'status' | 'resume') {
    const current = ++serial.current;
    await run(async () => {
      const value = await client({ operation: 'browser_owner', command: { action } }) as Status;
      if (!mounted.current || current !== serial.current) return;
      setStatus(value);
      const next = await client({ operation: 'list' }) as DirectCatalog;
      if (mounted.current && current === serial.current) onCatalog(next);
    });
  }
  async function stop() {
    const current = ++serial.current; setStopping(true); setError('');
    try {
      const value = await client({ operation: 'browser_owner', command: { action: 'emergency_stop' } }) as Status;
      if (mounted.current && current === serial.current) {
        setStatus(value);
        const next = await client({ operation: 'list' }) as DirectCatalog;
        if (mounted.current && current === serial.current) onCatalog(next);
      }
    } catch (error) { if (mounted.current) setError(error instanceof Error ? error.message : String(error)); }
    finally { if (mounted.current) setStopping(false); }
  }
  const paused = catalog.browser_paused || status?.paused;
  return <details className="direct-grant-form browser-sharing" open={configured ? true : undefined}>
    <summary><Globe size={18} aria-hidden="true" /> {t('Agent 浏览器操作', 'Agent Browser Use')}</summary>
    <div role="region" aria-label={t('Agent 浏览器操作', 'Agent Browser Use')}>
      <p className="direct-note">{t('共享当前接入端所在设备的 Chromium 浏览器。浏览器网页所在的设备可能与接入端不同。', 'Share a Chromium browser on this access point’s device. This may differ from the device displaying this page.')}</p>
      <form onSubmit={event => { event.preventDefault(); const current = ++serial.current; setSaved(false); void run(async () => { const value = await client({ operation: 'browser_sharing', browser: address.trim() ? { address: address.trim() } : null }) as DirectCatalog; if (mounted.current && current === serial.current) { onCatalog(value); setStatus(undefined); setSaved(true); } }); }}>
        <label htmlFor={id}>{t('本机 Chromium CDP 地址', 'Local Chromium CDP address')}</label>
        <input id={id} aria-describedby={id + '-hint'} placeholder="127.0.0.1:9222" value={address} disabled={busy} maxLength={64} autoComplete="off" spellCheck={false} onChange={event => { setAddress(event.target.value); setSaved(false); }} />
        <p id={id + '-hint'} className="direct-note">{t('先在此设备启动支持远程调试的 Chromium 浏览器。仅接受回环 IP 和端口；留空关闭共享。每个调用方还需要独立的浏览器操作授权。', 'Start a Chromium browser with remote debugging on this device first. Loopback IP and port only; leave empty to disable sharing. Each caller also needs a separate Browser Use grant.')}</p>
        <button type="submit" disabled={busy || address.trim() === configured}>{busy ? t('处理中…', 'Working…') : t('保存浏览器共享', 'Save browser sharing')}</button>
      </form>
      {saved && <p role="status">{configured ? t('浏览器共享已保存。', 'Browser sharing saved.') : t('浏览器共享已关闭。', 'Browser sharing disabled.')}</p>}
      {configured && <><p role="status">{paused ? t('已紧急停止。只能在此设备恢复浏览器共享。', 'Emergency stop active. Browser sharing can only be resumed on this device.') : status ? status.backend_available ? t('浏览器已就绪：', 'Browser ready: ') + (status.browser?.version || 'Chromium') : t('浏览器不可用，请检查远程调试设置。', 'Browser unavailable. Check remote debugging settings.') : t('尚未检查浏览器状态。', 'Browser status not checked.')}</p>
        <div className="direct-actions"><button disabled={busy || stopping} onClick={() => void control('status')}><RefreshCw size={16} aria-hidden="true" />{t('检查浏览器状态', 'Check browser status')}</button><button disabled={stopping} onClick={() => void stop()}><OctagonX size={16} aria-hidden="true" />{stopping ? t('正在停止…', 'Stopping…') : t('紧急停止浏览器操作', 'Stop browser control')}</button>{paused && <button disabled={busy || stopping} onClick={() => { if (confirm(t('恢复浏览器共享？已授权 Agent 将可以重新连接并操作全部标签页。', 'Resume browser sharing? Authorized Agents will be able to reconnect and control all tabs.'))) void control('resume'); }}>{t('在此设备恢复浏览器共享', 'Resume browser sharing on this device')}</button>}</div>
        <p className="direct-note">{t('每次仅允许一个远程控制连接。紧急停止会断开连接并保留到明确恢复；已发送给浏览器的操作无法撤回，也不会自动重试。', 'One remote controller at a time. Emergency stop disconnects it and persists until explicitly resumed. Actions already sent to the browser cannot be recalled and are never automatically replayed.')}</p></>}
      {error && <p className="direct-error" role="alert">{error}</p>}
    </div>
  </details>;
}
