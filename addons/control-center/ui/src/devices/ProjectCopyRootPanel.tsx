import { useEffect, useId, useState } from 'react';
import type { DirectCatalog, DirectClient } from './DirectDevicesView';

export function ProjectCopyRootPanel({ client, catalog, language, busy, run, onCatalog }: { client: DirectClient; catalog: DirectCatalog; language: string; busy: boolean; run: (action: () => Promise<void>) => Promise<void>; onCatalog: (value: DirectCatalog) => void }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const fieldId = useId();
  const [path, setPath] = useState(catalog.copy_root || ''), [saved, setSaved] = useState(false);
  useEffect(() => { setPath(catalog.copy_root || ''); }, [catalog.copy_root]);
  return <form className="direct-grant-form" aria-label={t('项目复制接收目录', 'Project copy receiving directory')} onSubmit={event => { event.preventDefault(); setSaved(false); void run(async () => { onCatalog(await client({ operation: 'copy_root', path: path.trim() || null }) as DirectCatalog); setSaved(true); }); }}>
    <label><span id={fieldId}>{t('项目复制接收目录', 'Project copy receiving directory')}</span><input aria-labelledby={fieldId} aria-describedby={fieldId + '-hint'} placeholder={t('已有的绝对目录；留空关闭接收', 'Existing absolute directory; leave empty to disable')} value={path} disabled={busy} onChange={e => { setPath(e.target.value); setSaved(false); }} /><span id={fieldId + '-hint'} className="direct-note">{t('由本机指定。每次复制创建独立目录；调用设备还需同时拥有项目目录和执行命令权限。', 'Chosen locally. Each copy creates a separate directory. The caller also needs both project catalog and command access.')}</span></label>
    <button type="submit" disabled={busy || (path.trim() || null) === (catalog.copy_root || null)}>{t('保存接收目录', 'Save receiving directory')}</button>{saved && <p role="status">{path.trim() ? t('接收目录已保存。', 'Receiving directory saved.') : t('已关闭项目复制接收。', 'Receiving project copies disabled.')}</p>}
  </form>;
}
