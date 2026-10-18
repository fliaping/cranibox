import { useEffect, useId, useRef, useState, type FormEvent } from 'react';
import { ArrowUp, Plus, Trash2, X } from 'lucide-react';
import type { DirectCatalog, DirectClient, DirectRoute } from './DirectDevicesView';

export type PathConnection = { url: string; fallback: boolean; path_count: number };

/** Paths belong to one device and use the same target-issued grant. */
export function DirectPathsPanel({ route, connection, client, busy, run, onCatalog, t }: {
  route: DirectRoute; connection?: PathConnection; client: DirectClient; busy: boolean;
  run: (action: () => Promise<void>) => Promise<void>; onCatalog: (catalog: DirectCatalog) => void;
  t: (cn: string, en: string) => string;
}) {
  const [open, setOpen] = useState(false), [url, setUrl] = useState(''), [token, setToken] = useState('');
  const [privateHttp, setPrivateHttp] = useState(false), [error, setError] = useState(''), [notice, setNotice] = useState('');
  const formId = useId();
  const first = useRef<HTMLInputElement>(null), trigger = useRef<HTMLButtonElement>(null), panel = useRef<HTMLElement>(null), restoreFocus = useRef(false), mounted = useRef(true);
  const paths = route.paths || [{ url: route.url, allow_private_http: route.allow_private_http }];
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => { if (open) first.current?.focus(); }, [open]);
  useEffect(() => {
    if (!busy && restoreFocus.current) {
      restoreFocus.current = false;
      (trigger.current?.disabled ? panel.current : trigger.current)?.focus();
    }
  });
  function close() {
    setOpen(false); setUrl(''); setToken(''); setPrivateHttp(false); setError('');
    restoreFocus.current = true;
  }
  async function change(operation: 'prefer_path' | 'remove_path', path: string) {
    await run(async () => {
      setError(''); setNotice('');
      try {
        const catalog = await client({ operation, node: route.node, url: path }) as DirectCatalog;
        if (!mounted.current) return;
        onCatalog(catalog);
        setNotice(operation === 'prefer_path' ? t('首选路径已更新，下次连接时生效。', 'Preferred path updated. It applies to the next connection.') : t('路径已移除，已连接的会话不受影响。', 'Path removed. Existing connections remain active.'));
        restoreFocus.current = true;
      } catch (error) { if (mounted.current) setError(error instanceof Error ? error.message : String(error)); }
    });
  }
  async function save(event: FormEvent) {
    event.preventDefault();
    await run(async () => {
      setError(''); setNotice('');
      try {
        const catalog = await client({ operation: 'add_path', node: route.node, url: url.trim(), token: token.trim(), allow_private_http: privateHttp }) as DirectCatalog;
        if (!mounted.current) return;
        onCatalog(catalog); close();
        setNotice(t('目标身份与授权检查通过，路径已保存。', 'Target identity and access verified. Path saved.'));
      } catch (error) {
        if (mounted.current) { setError(error instanceof Error ? error.message : String(error)); first.current?.focus(); }
      }
    });
  }
  return <section ref={panel} tabIndex={-1} className="direct-paths" aria-label={t('设备连接路径', 'Device connection paths')}>
    <div className="direct-heading"><div><h4>{t('连接路径', 'Connection paths')} <span>{paths.length}/4</span></h4><p>{t('局域网、EasyTier 和 Tailscale 地址可接入同一设备。', 'LAN, EasyTier and Tailscale addresses can reach the same device.')}</p></div><button ref={trigger} aria-expanded={open} aria-controls={open ? formId : undefined} disabled={busy || paths.length >= 4} onClick={() => { setOpen(true); setError(''); setNotice(''); }}><Plus size={16} aria-hidden="true" />{t('添加路径', 'Add path')}</button></div>
    <ol>{paths.map((path, index) => <li key={path.url}><div className="direct-path-address"><div className="direct-path-label"><strong>{index === 0 ? t('首选路径', 'Preferred') : t('备用路径 ', 'Alternative ') + index}</strong>{connection?.url.replace(/[/]$/, '') === path.url.replace(/[/]$/, '') && <span>{t('上次检查可用', 'Available at last check')}</span>}</div><code>{path.url}</code>{path.managed && <small>{t('由 EasyTier 管理', 'Managed by EasyTier')}</small>}</div><div className="direct-actions">{index > 0 && <button disabled={busy} aria-label={t('设为首选路径：', 'Prefer path: ') + path.url} onClick={() => void change('prefer_path', path.url)}><ArrowUp size={16} aria-hidden="true" />{t('设为首选', 'Prefer')}</button>}{paths.length > 1 && <button disabled={busy} aria-label={t('移除路径：', 'Remove path: ') + path.url} onClick={() => void change('remove_path', path.url)}><Trash2 size={16} aria-hidden="true" /></button>}</div></li>)}</ol>
    {connection?.fallback && <p role="status" className="direct-note">{t('上次检查通过备用路径连接。首选路径恢复后，新连接会重新优先使用它。', 'The last check used an alternative. New connections prefer the first path again when it recovers.')}</p>}
    <p className="direct-note">{t('新连接按顺序检查路径。命令只发送一次，不会因断线换路径重发；重新附着终端时会再次选路。', 'New connections check paths in order. Commands are sent once and never replayed after a disconnect. Reattaching a terminal selects a path again.')}</p>
    {open && <form id={formId} onSubmit={save} aria-label={t('添加设备路径', 'Add device path')}><div className="direct-heading"><h4>{t('添加设备路径', 'Add device path')}</h4><button type="button" disabled={busy} aria-label={t('关闭路径表单', 'Close path form')} onClick={close}><X size={16} aria-hidden="true" /></button></div><label>{t('路径地址', 'Path address')}<input ref={first} type="url" required value={url} onChange={event => setUrl(event.target.value)} placeholder="http://100.64.0.2:9091" autoComplete="off" /></label><label>{t('现有目标授权 Token', 'Existing target access token')}<input type="password" required pattern="[0-9a-fA-F]{64}" maxLength={64} value={token} onChange={event => setToken(event.target.value)} autoComplete="off" /><span className="direct-note">{t('必须与此设备已保存的授权相同。保存前只检查身份和授权，不执行命令。', 'Use the same grant already saved for this device. Saving only checks identity and access; it runs no commands.')}</span></label><label className="direct-check"><input type="checkbox" checked={privateHttp} onChange={event => setPrivateHttp(event.target.checked)} />{t('允许可信私网使用 HTTP', 'Allow HTTP on a trusted private network')}</label><button type="submit" className="direct-primary" disabled={busy}>{busy ? t('正在检查…', 'Checking…') : t('检查并保存路径', 'Check and save path')}</button></form>}
    {error && <p className="direct-error" role="alert">{error}</p>}
    {notice && <p className="direct-note" role="status">{notice}</p>}
  </section>;
}
