import { useEffect, useId, useState } from 'react';
import { Monitor, OctagonX, RefreshCw } from 'lucide-react';
import type { DirectCatalog, DirectClient } from './DirectDevicesView';

type Status = { paused: boolean; backend_available: boolean };
export function ComputerSharingPanel({ client, catalog, language, busy, run, onCatalog }: { client: DirectClient; catalog: DirectCatalog; language: string; busy: boolean; run: (action: () => Promise<void>) => Promise<void>; onCatalog: (value: DirectCatalog) => void }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const field = useId();
  const configured = catalog.computer?.address || '';
  const [address, setAddress] = useState(configured), [saved, setSaved] = useState(false), [status, setStatus] = useState<Status>();
  const [stopping, setStopping] = useState(false), [stopError, setStopError] = useState('');
  useEffect(() => { setAddress(configured); setStatus(undefined); }, [configured]);
  async function control(action: 'status' | 'emergency_stop' | 'resume') {
    await run(async () => {
      setStatus(await client({ operation: 'computer_owner', command: { action } }) as Status);
      onCatalog(await client({ operation: 'list' }) as DirectCatalog);
    });
  }
  async function stopInput() {
    setStopping(true); setStopError('');
    try {
      setStatus(await client({ operation: 'computer_owner', command: { action: 'emergency_stop' } }) as Status);
      onCatalog(await client({ operation: 'list' }) as DirectCatalog);
    } catch (error) { setStopError(error instanceof Error ? error.message : String(error)); }
    finally { setStopping(false); }
  }
  return <div className="direct-grant-form" role="region" aria-label={t('Agent 桌面操作', 'Agent Computer Use')}>
    <div className="direct-heading"><div><h3><Monitor size={18} aria-hidden="true" />{t('Agent 桌面操作', 'Agent Computer Use')}</h3><p className="direct-note">{t('将本机桌面桥接提供给已授权节点的 Agent，用于截图、键鼠和窗口操作。', 'Share the local desktop bridge with authorized Agents for screenshots, input and window actions.')}</p></div></div>
    <form onSubmit={event => { event.preventDefault(); setSaved(false); void run(async () => { onCatalog(await client({ operation: 'computer_sharing', computer: address.trim() ? { address: address.trim() } : null }) as DirectCatalog); setSaved(true); setStatus(undefined); }); }}>
      <label htmlFor={field}>{t('本机桥接地址', 'Local bridge address')}</label><input id={field} aria-describedby={field + '-hint'} placeholder="127.0.0.1:8765" value={address} disabled={busy} maxLength={64} autoComplete="off" spellCheck={false} onChange={event => { setAddress(event.target.value); setSaved(false); }} />
      <p id={field + '-hint'} className="direct-note">{t('仅接受回环 IP 和端口。需先运行 agent-desktop-bridge；留空关闭共享。共享开启后，还需在入站授权中单独勾选 Agent 桌面操作。', 'Loopback IP and port only. Run agent-desktop-bridge first; leave empty to disable sharing. Each caller also needs the separate Computer Use grant below.')}</p>
      <button type="submit" disabled={busy || address.trim() === configured}>{busy ? t('处理中…', 'Working…') : t('保存桌面桥接', 'Save desktop bridge')}</button>
    </form>
    {saved && <p role="status">{configured ? t('桌面桥接已保存。', 'Desktop bridge saved.') : t('已关闭 Agent 桌面操作共享。', 'Agent Computer Use sharing disabled.')}</p>}
    {configured && <><p role="status">{catalog.computer_paused || status?.paused ? t('已紧急停止。只有此设备可以恢复 Agent 输入。', 'Emergency stop active. Agent input can only be resumed on this device.') : status ? status.backend_available ? t('桌面桥接就绪。', 'Desktop bridge ready.') : t('桌面后端不可用，请检查本机桥接服务。', 'Desktop backend unavailable. Check the local bridge service.') : t('尚未检查桌面桥接状态。', 'Desktop bridge status not checked.')}</p>
      <div className="direct-actions"><button disabled={busy || stopping} onClick={() => void control('status')}><RefreshCw size={16} aria-hidden="true" />{t('检查桥接状态', 'Check bridge status')}</button><button disabled={stopping} onClick={() => void stopInput()}><OctagonX size={16} aria-hidden="true" />{stopping ? t('正在停止…', 'Stopping…') : t('紧急停止 Agent 输入', 'Stop Agent input')}</button>{(catalog.computer_paused || status?.paused) && <button disabled={busy || stopping} onClick={() => { if (confirm(t('恢复已授权 Agent 的桌面输入？之后 Agent 可重新取得短期控制权。', 'Resume desktop input for authorized Agents? They can acquire a new short control lease.'))) void control('resume'); }}>{t('在此设备恢复', 'Resume on this device')}</button>}</div>
      {stopError && <p className="direct-error" role="alert">{stopError}</p>}
      <p className="direct-note">{t('Agent 输入需取得 1–300 秒的控制权，与桌面查看器互斥；紧急停止会保留到明确恢复。已发给桌面后端的操作无法撤回。', 'Agent input requires a 1–300 second control lease, shared with the desktop viewer. Emergency stop persists until explicitly resumed. Actions already sent to the desktop backend cannot be recalled.')}</p></>}
  </div>;
}
