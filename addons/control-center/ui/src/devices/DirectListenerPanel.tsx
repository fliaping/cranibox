import { useEffect, useId, useRef, useState, type FormEvent } from 'react';
import { Radio, Settings2, X } from 'lucide-react';
import type { DirectCatalog, DirectClient, DirectListener } from './DirectDevicesView';
import { ask } from '../confirm';

export function DirectListenerPanel({ client, catalog, language, busy, run, onCatalog }: {
  client: DirectClient; catalog: DirectCatalog; language: string; busy: boolean;
  run: (action: () => Promise<void>) => Promise<void>; onCatalog: (catalog: DirectCatalog) => void;
}) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const runtime = catalog.listener_runtime;
  const [editor, setEditor] = useState(false), [address, setAddress] = useState(''), [privateHttp, setPrivateHttp] = useState(false);
  const [error, setError] = useState(''), [pending, setPending] = useState(false);
  const field = useRef<HTMLInputElement>(null), trigger = useRef<HTMLButtonElement>(null), mounted = useRef(true);
  const formId = useId(), hintId = useId(), errorId = useId();
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => { if (editor) field.current?.focus(); }, [editor]);
  useEffect(() => { if (error && !busy) field.current?.focus(); }, [error, busy]);
  function close() { setEditor(false); setError(''); requestAnimationFrame(() => trigger.current?.focus()); }
  async function apply(listener: DirectListener | null) {
    await run(async () => {
      setPending(true); setError('');
      try {
        const value = await client({ operation: 'listener', listener }) as DirectCatalog;
        if (mounted.current) { onCatalog(value); close(); }
      } catch (error) {
        if (mounted.current) setError(error instanceof Error ? error.message : String(error));
      } finally { if (mounted.current) setPending(false); }
    });
  }
  function save(event: FormEvent) { event.preventDefault(); void apply({ address: address.trim(), allow_insecure: privateHttp }); }
  if (!runtime) return <p className="direct-note">{catalog.listener ? <>{t('配置的监听地址：', 'Configured listener: ')}<code>{catalog.listener.address}</code>{t('。重启持有此身份的 Cranibox 服务后生效。', '. Restart the Cranibox service that owns this identity to apply it.')}</> : t('此服务未提供监听管理，请升级服务或通过 peer listen 配置。', 'Listener management is unavailable. Update this service or configure it with peer listen.')}</p>;
  return <div className="direct-listener" aria-label={t('入站监听', 'Incoming listener')}>
    <div className="direct-heading"><div><h4><Radio size={16} aria-hidden="true" />{t('入站监听', 'Incoming listener')}</h4><p role="status">{runtime.active ? <>{t('正在监听 ', 'Listening on ')}<code>{runtime.address}</code></> : t('入站接入已关闭', 'Incoming access is stopped')}</p></div><button ref={trigger} disabled={busy} aria-expanded={editor} aria-controls={formId} onClick={() => {
      if (editor) { close(); return; }
      setAddress(catalog.listener?.address || runtime.address || '127.0.0.1:9020'); setPrivateHttp(catalog.listener?.allow_insecure || false); setError(''); setEditor(true);
    }}>{editor ? <X size={16} aria-hidden="true" /> : <Settings2 size={16} aria-hidden="true" />}{editor ? t('收起监听设置', 'Close listener settings') : t('配置监听', 'Configure listener')}</button></div>
    {!runtime.applied && <p className="direct-note">{t('保存的配置尚未应用。打开监听设置即可应用，无需重启。', 'Saved settings have not been applied. Open listener settings to apply them without restarting.')}</p>}
    {runtime.error && <p className="direct-error" role="alert">{runtime.error}</p>}
    <p className="direct-note">{t('启用后，仅持有授权 Token 的设备可以访问。修改端口会断开现有入站连接；后台 Shell 和任务继续运行。', 'Only devices with an access token can connect. Changing ports disconnects incoming connections; background shells and jobs keep running.')}</p>
    {editor && <form id={formId} onSubmit={save} aria-busy={pending}>
      <label>{t('监听地址', 'Listen address')}<input ref={field} required spellCheck={false} autoComplete="off" value={address} disabled={busy} aria-describedby={hintId + (error ? ' ' + errorId : '')} onChange={event => setAddress(event.target.value)} /></label>
      <p id={hintId} className="direct-note">{t('填写 IP:端口，例如 127.0.0.1:9020 或 [::1]:9020。其他设备连接时使用此设备的局域网、EasyTier 或 Tailscale 地址；0.0.0.0 和 [::] 仅表示绑定全部接口。', 'Enter IP:port, such as 127.0.0.1:9020 or [::1]:9020. Other devices connect using this device’s LAN, EasyTier or Tailscale address. 0.0.0.0 and [::] bind all interfaces.')}</p>
      <label className="direct-check"><input type="checkbox" disabled={busy} checked={privateHttp} onChange={event => setPrivateHttp(event.target.checked)} />{t('允许可信私网或 TLS 反向代理后的 HTTP 接入', 'Allow HTTP on a trusted private network or behind a TLS proxy')}</label>
      {error && <p id={errorId} className="direct-error" role="alert">{error}</p>}
      <div className="direct-actions"><button className="direct-primary" type="submit" disabled={busy}>{pending ? t('正在应用…', 'Applying…') : t('保存并启用', 'Save and enable')}</button><button type="button" disabled={busy} onClick={close}>{t('取消', 'Cancel')}</button>{(runtime.active || catalog.listener) && <button type="button" disabled={busy} onClick={async () => {
        if (await ask(t('关闭入站接入？其他设备的入站连接将断开，授权、后台 Shell 和运行中的任务会保留。', 'Stop incoming access? Other devices will disconnect. Grants, background shells and running jobs remain available.'))) void apply(null);
      }}>{t('关闭入站接入', 'Stop incoming access')}</button>}</div>
    </form>}
  </div>;
}
