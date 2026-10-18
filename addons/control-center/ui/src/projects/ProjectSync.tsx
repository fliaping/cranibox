import { useCallback, useEffect, useRef, useState, type FormEvent } from 'react';
import { Pause, RefreshCw } from 'lucide-react';
import type { DirectClient } from '../devices/DirectDevicesView';
import './project-sync.css';

type Node = { node: string; connection_valid: boolean; phase: string; last_attempt: number | null; last_success: number | null; catalog_revision: number | null; project_count: number | null };
type Status = { owner: string; revision: number; enabled: boolean; interval_seconds: number; available_nodes: string[]; nodes: Node[] };
type Draft = { revision: number; nodes: string[]; interval_seconds: number };
const message = (error: unknown) => error instanceof Error ? error.message : String(error);

/** Lazy shared settings; synchronization itself runs in the Rust node service. */
export function ProjectSync({ client, owner, language }: { client: DirectClient; owner?: string; language: string }) {
  const [open, setOpen] = useState(false), zh = language.startsWith('zh');
  return <details className="project-sync" onToggle={event => setOpen(event.currentTarget.open)}>
    <summary><RefreshCw size={17} aria-hidden="true" /><strong>{zh ? '后台目录同步' : 'Background catalog sync'}</strong><span>{zh ? '按设备启用 · 关闭页面后继续' : 'Choose devices · continues after closing'}</span></summary>
    {open && <SyncSettings key={owner} client={client} owner={owner} language={language} />}
  </details>;
}
function SyncSettings({ client, owner, language }: { client: DirectClient; owner?: string; language: string }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const [status, setStatus] = useState<Status>(), [draft, setDraft] = useState<Draft>();
  const [busy, setBusy] = useState(false), [reading, setReading] = useState(false), [readError, setReadError] = useState(''), [error, setError] = useState(''), [notice, setNotice] = useState('');
  const alive = useRef(true), fetching = useRef(false), writing = useRef(false), serial = useRef(0);
  const accept = useCallback((value: Status) => {
    if (!value || typeof value.owner !== 'string' || (owner && value.owner !== owner) || !Number.isSafeInteger(value.revision) ||
      typeof value.enabled !== 'boolean' || ![15, 30, 60, 300].includes(value.interval_seconds) || !Array.isArray(value.available_nodes) || !Array.isArray(value.nodes)) throw Error('Unexpected project sync settings. Reload the intended device.');
    return value;
  }, [owner]);
  const call = useCallback((command: Record<string, unknown>) => client({ operation: 'project_sync', command }) as Promise<Status>, [client]);
  const read = useCallback(async () => {
    if (fetching.current || writing.current || document.hidden) return;
    fetching.current = true; const request = ++serial.current; setReading(true);
    try { const value = accept(await call({ operation: 'status' })); if (alive.current && request === serial.current) { setStatus(value); setReadError(''); } }
    catch (problem) { if (alive.current && request === serial.current) setReadError(message(problem)); }
    finally { fetching.current = false; if (alive.current) setReading(false); }
  }, [accept, call]);
  useEffect(() => {
    alive.current = true; void read(); const tick = () => void read(), timer = setInterval(tick, 5000);
    document.addEventListener('visibilitychange', tick); window.addEventListener('cranibox:peers-changed', tick);
    return () => { alive.current = false; ++serial.current; clearInterval(timer); document.removeEventListener('visibilitychange', tick); window.removeEventListener('cranibox:peers-changed', tick); };
  }, [read]);
  async function save(command: Record<string, unknown>, text: string) {
    if (writing.current) return; writing.current = true; ++serial.current; setBusy(true); setError(''); setNotice('');
    try { const value = accept(await call(command)); if (alive.current) { setStatus(value); setDraft(undefined); setReadError(''); setNotice(text); } }
    catch (problem) { if (alive.current) setError(message(problem)); }
    finally { writing.current = false; if (alive.current) setBusy(false); }
  }
  function submit(event: FormEvent) {
    event.preventDefault(); if (!draft) return;
    void save({ operation: 'configure', ...draft }, t('已保存，节点将在后台同步选定设备。', 'Saved. This node will synchronize the selected devices in the background.'));
  }
  const phase = (value: string) => ({ waiting: t('等待同步', 'Waiting'), checking: t('正在同步', 'Syncing'), online: t('同步成功', 'Up to date'), offline: t('设备离线，保留快照', 'Offline · snapshot kept'), unavailable: t('暂时无法连接', 'Temporarily unreachable'), error: t('读取被拒绝或目录无效，请检查目标授权', 'Read denied or catalog invalid; check target access'), cache_error: t('已读取，但本机快照保存失败', 'Read succeeded; local snapshot could not be saved'), paused: t('已暂停', 'Paused'), connection_changed: t('连接已移除或变更，需要重新选择', 'Connection removed or changed; select it again') }[value] || t('状态未知，请刷新', 'Unknown status; refresh'));
  const nodes = [...new Set([...(status?.available_nodes || []), ...(draft?.nodes || [])])];
  const stale = !!draft && status?.revision !== draft.revision, locked = busy || !!readError;
  const unavailable = draft?.nodes.some(node => !status?.available_nodes.includes(node));
  return <div className="project-sync-body">
    <p className="projects-note">{t('运行此界面的节点会定期读取选定设备的项目目录和会话关联，保存本机只读快照。需要目标的项目目录权限。', 'The node behind this interface periodically reads selected devices’ project catalogs and session links, keeping a local read-only snapshot. Target project access is required.')}</p>
    {status && <div className="project-sync-heading"><div><strong>{status.enabled ? t('后台同步已启用', 'Background sync enabled') : t('后台同步已关闭', 'Background sync off')}</strong><small>{t('同步节点：', 'Syncing node: ')}<code>{status.owner}</code></small></div><span>{status.nodes.length} {t('台设备', 'devices')} · {status.interval_seconds < 60 ? status.interval_seconds + t(' 秒', ' seconds') : status.interval_seconds / 60 + t(' 分钟', ' minutes')}</span></div>}
    {!status && reading && <p role="status">{t('正在读取同步设置…', 'Loading sync settings…')}</p>}
    {readError && <p className="projects-error" role="alert">{readError}{status && t(' · 以下为上次读取的设置。', ' · Showing previously read settings.')}</p>}
    {error && <p className="projects-error" role="alert">{error}</p>}
    {notice && <p className="project-sync-notice" role="status">{notice}</p>}
    <div className="project-sync-actions">{status && !draft && <button type="button" disabled={locked} onClick={() => { setDraft({ revision: status.revision, nodes: status.nodes.map(node => node.node), interval_seconds: status.interval_seconds }); setError(''); setNotice(''); }}>{status.enabled ? t('修改同步设备', 'Edit sync devices') : t('设置后台同步', 'Set up background sync')}</button>}{status?.enabled && <button type="button" disabled={locked} onClick={() => void save({ operation: 'pause', revision: status.revision }, t('后台同步已暂停。现有快照仍可查看。', 'Background sync paused. Existing snapshots remain available.'))}><Pause size={16} aria-hidden="true" />{t('暂停同步', 'Pause sync')}</button>}<button type="button" disabled={busy || reading} onClick={() => void read()}><RefreshCw size={16} aria-hidden="true" />{t('刷新同步状态', 'Refresh sync status')}</button></div>
    {draft && <form className="project-sync-form" onSubmit={submit} aria-label={t('后台同步设置', 'Background sync settings')}>
      {stale && <div className="projects-error" role="alert"><span>{t('设置已在其他入口变更。请重新核对所选设备。', 'Settings changed elsewhere. Review your selected devices again.')}</span><button type="button" disabled={locked || !status} onClick={() => status && setDraft({ ...draft, revision: status.revision })}>{t('使用当前版本继续修改', 'Continue with current version')}</button></div>}
      <fieldset disabled={locked}><legend>{t('要同步的设备', 'Devices to synchronize')}</legend>{!nodes.length && <p className="projects-note">{t('请先在设备页面添加直连设备并取得项目目录权限。', 'First add a direct device and obtain project catalog access on the Devices page.')}</p>}{nodes.map(node => <label key={node} className="project-sync-check"><input type="checkbox" checked={draft.nodes.includes(node)} onChange={event => setDraft({ ...draft, nodes: event.target.checked ? [...draft.nodes, node] : draft.nodes.filter(value => value !== node) })} /><span><code>{node}</code>{!status?.available_nodes.includes(node) && <small>{t('连接已移除，请取消选择', 'Connection removed; deselect this device')}</small>}</span></label>)}</fieldset>
      <label className="project-sync-interval">{t('同步间隔', 'Sync interval')}<select value={draft.interval_seconds} disabled={locked} onChange={event => setDraft({ ...draft, interval_seconds: Number(event.target.value) })}><option value={15}>{t('15 秒', '15 seconds')}</option><option value={30}>{t('30 秒', '30 seconds')}</option><option value={60}>{t('1 分钟 · 推荐', '1 minute · recommended')}</option><option value={300}>{t('5 分钟 · 节省资源', '5 minutes · fewer requests')}</option></select></label>
      <p className="projects-note">{t('最多选择 32 台设备。只同步目录记录，不复制项目文件，不启动 Shell 或 Agent。关闭页面后继续；暂停后不再开始新读取，已发出的请求会结束。', 'Choose up to 32 devices. Sync reads catalog records; it does not copy files or start shells or Agents. It continues after closing. Pausing prevents new reads; in-flight requests finish.')}</p>
      <div className="project-sync-actions"><button type="submit" className="projects-primary" disabled={locked || stale || !draft.nodes.length || draft.nodes.length > 32 || unavailable}>{busy ? t('正在保存…', 'Saving…') : status?.enabled ? t('保存同步设置', 'Save sync settings') : t('启用后台同步', 'Enable background sync')}</button><button type="button" disabled={busy} onClick={() => { setDraft(undefined); setError(''); }}>{t('取消修改', 'Cancel changes')}</button></div>
    </form>}
    {!!status?.nodes.length && <ul className="project-sync-results" aria-label={t('各设备同步状态', 'Device sync status')}>{status.nodes.map(node => <li key={node.node}><div><code>{node.node}</code><strong>{phase(node.phase)}</strong>{node.last_success && <small>{t('上次成功：', 'Last successful sync: ')}<time dateTime={new Date(node.last_success).toISOString()}>{new Date(node.last_success).toLocaleString()}</time></small>}{node.last_attempt && node.last_attempt !== node.last_success && <small>{t('最近尝试：', 'Last attempted: ')}<time dateTime={new Date(node.last_attempt).toISOString()}>{new Date(node.last_attempt).toLocaleString()}</time></small>}</div>{node.project_count !== null && <span>{node.project_count} {t('个项目', 'projects')}</span>}</li>)}</ul>}
    <p className="projects-note">{t('离线快照可能已过期，不能用于操作远端会话。目标拒绝读取或返回无效目录后，会清除相应快照。', 'Offline snapshots may be stale and cannot operate remote sessions. A denied read or invalid owner catalog removes that cached snapshot.')}</p>
  </div>;
}
