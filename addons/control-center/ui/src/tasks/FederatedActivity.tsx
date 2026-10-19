import { useDeferredValue, useEffect, useId, useRef, useState } from 'react';
import { ChevronDown, ChevronUp, Layers, RefreshCw } from 'lucide-react';
import type { DirectCatalog, DirectClient } from '../devices/DirectDevicesView';
import { ProjectSync } from '../projects/ProjectSync';
import { activityCatalog, type Activity } from './activity-catalog';
import './activity.css';

type Props = { client: DirectClient; devices?: DirectCatalog; owner: string; language: string; paused: boolean; onDevices?: () => void };
type SourceView = { reading: boolean; snapshot?: Activity; error?: string };
type Row = { key: string; owner: string; kind: 'task' | 'message' | 'run'; title: string; text: string; status: string; created: number; detail: string };

export function FederatedActivity(props: Props) {
  const [open, setOpen] = useState(false), id = useId(), cn = props.language.startsWith('zh');
  return <section className="activity-panel" aria-label={cn ? '跨设备协作记录' : 'Collaboration records across devices'}>
    <div className="direct-heading"><div><h3><Layers size={19} aria-hidden="true" />{cn ? '跨设备协作记录' : 'Collaboration records across devices'}</h3><p>{cn ? '按来源查看任务、消息和协作 Run。远端需要单独的只读协作记录授权。' : 'View tasks, messages and collaboration Runs by source. Peers require a separate read collaboration records grant.'}</p></div><button type="button" aria-expanded={open} aria-controls={id} onClick={() => setOpen(value => !value)}>{open ? <ChevronUp size={16} aria-hidden="true" /> : <ChevronDown size={16} aria-hidden="true" />}{cn ? open ? '收起协作记录' : '查看协作记录' : open ? 'Close collaboration records' : 'View collaboration records'}</button></div>
    <div id={id}>{open && <Records {...props} />}</div>
  </section>;
}

function Records({ client, devices, owner, language, paused, onDevices }: Props) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const kindGroup = useId();
  const sources = [owner, ...Array.from(new Set((devices?.routes || []).map(route => route.node))).filter(node => node !== owner).sort()];
  const [selected, setSelected] = useState<string[]>([owner]), [generation, setGeneration] = useState(0);
  const [state, setState] = useState<{ scope: string; views: Record<string, SourceView> }>({ scope: '', views: {} });
  const [search, setSearch] = useState(''), [kind, setKind] = useState('all'), [page, setPage] = useState(0);
  const query = useDeferredValue(search).trim().toLocaleLowerCase();
  const pausedRef = useRef(paused), read = useRef<() => void>(() => undefined);
  pausedRef.current = paused;
  const chosen = sources.filter(source => selected.includes(source)).slice(0, 16);
  const routeKey = JSON.stringify([owner, devices?.node_id, devices?.routes.map(route => [route.node, route.url, route.allow_private_http, route.paths])]);
  const scope = JSON.stringify([routeKey, chosen, generation, language]);
  const views = state.scope === scope ? state.views : {};
  useEffect(() => {
    const changed = () => setGeneration(value => value + 1);
    window.addEventListener('cranibox:peers-changed', changed); window.addEventListener('cranibox:network-routes-changed', changed);
    window.addEventListener('cranibox:activity-sync-changed', changed);
    return () => { window.removeEventListener('cranibox:peers-changed', changed); window.removeEventListener('cranibox:network-routes-changed', changed); window.removeEventListener('cranibox:activity-sync-changed', changed); };
  }, []);
  useEffect(() => {
    let disposed = false, running = false;
    setState({ scope, views: {} });
    const refresh = async (automatic = false) => {
      if (disposed || running || document.hidden || automatic && pausedRef.current) return;
      running = true;
      const queue = [...chosen];
      const publish = (node: string, view: SourceView) => { if (!disposed) setState(previous => previous.scope === scope ? { scope, views: { ...previous.views, [node]: view } } : previous); };
      const worker = async () => {
        while (!disposed && !document.hidden && !(automatic && pausedRef.current) && queue.length) {
          const node = queue.shift()!;
          if (!disposed) setState(previous => previous.scope === scope ? { scope, views: { ...previous.views, [node]: { ...previous.views[node], reading: true } } } : previous);
          try {
            const response = await client({ operation: 'activity', node: node === owner ? null : node });
            const snapshot = activityCatalog(response, node, t('协作记录的来源或格式无效，请检查设备服务。', 'Invalid collaboration records or source identity. Check the device service.'));
            publish(node, { reading: false, snapshot });
          } catch (error) { publish(node, { reading: false, error: error instanceof Error ? error.message : String(error) }); }
        }
      };
      try { await Promise.all([worker(), worker()]); } finally { running = false; }
    };
    read.current = () => void refresh(); void refresh();
    const timer = setInterval(() => void refresh(true), 10000);
    const resume = () => void refresh(true);
    document.addEventListener('visibilitychange', resume);
    return () => { disposed = true; read.current = () => undefined; clearInterval(timer); document.removeEventListener('visibilitychange', resume); };
    // Stable route and selection serialization prevents local task polling from restarting reads.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, scope]);
  useEffect(() => { if (!paused) read.current(); }, [paused]);
  useEffect(() => { setPage(0); }, [query, kind, scope]);
  const labels = { task: t('任务', 'Task'), message: t('消息', 'Message'), run: t('协作 Run', 'Collaboration Run') };
  const rows: Row[] = chosen.flatMap(node => {
    const snapshot = views[node]?.snapshot;
    if (!snapshot) return [];
    return [
      ...snapshot.tasks.map(item => ({ key: node + ':task:' + item.task.id, owner: node, kind: 'task' as const, title: item.task.title, text: item.task.description, status: item.current.state + (item.cancel_requested ? ' · ' + t('已请求取消', 'Cancellation requested') : ''), created: item.task.created_at, detail: [item.task.sender + ' → ' + item.task.recipient, 'Task: ' + item.task.id, 'Dispatch: ' + item.current.dispatch, 'Revision: ' + item.current.revision, item.current.progress, ...(item.current.questions?.flatMap(question => [t('问题：', 'Question: ') + question.text, question.answer ? t('回复：', 'Answer: ') + question.answer : t('尚未收到回复', 'No answer received')]) || []), item.current.result].filter(Boolean).join('\n\n') })),
      ...snapshot.messages.map(item => ({ key: node + ':message:' + item.message.id, owner: node, kind: 'message' as const, title: t('设备消息', 'Device message'), text: item.message.text, status: item.status, created: item.message.created_at, detail: item.message.sender + ' → ' + item.message.recipient + '\nMessage: ' + item.message.id })),
      ...snapshot.runs.map(item => ({ key: node + ':run:' + item.scope.id, owner: node, kind: 'run' as const, title: item.scope.name, text: item.scope.goal, status: item.state, created: item.created_at, detail: ['Run: ' + item.scope.id, 'Revision: ' + item.revision, t('参与设备：', 'Members: ') + item.members.join(', '), item.result].filter(Boolean).join('\n\n') })),
    ];
  }).filter(row => (kind === 'all' || row.kind === kind) && (!query || [row.owner, row.title, row.text, row.detail].some(value => value.toLocaleLowerCase().includes(query)))).sort((a, b) => b.created - a.created || a.key.localeCompare(b.key));
  const pages = Math.max(1, Math.ceil(rows.length / 20)), currentPage = Math.min(page, pages - 1);
  const reading = chosen.some(node => views[node]?.reading);
  const date = (time: number) => new Date(time * 1000).toLocaleString(language);
  return <div className="activity-content">
    <p className="direct-note">{t('来源节点保存权威记录。这里仅汇总最近读取的内容，同一任务可在不同来源各有记录；不会合并进本机账本，也不代表 Agent 正在运行。收起后停止页面读取；已启用的后台同步继续。', 'Each source owns its records. These are the last observations; the same task can appear under multiple sources. Nothing is merged into this device’s ledger, and reports do not prove a running Agent. Closing stops page reads; enabled background sync continues.')}</p>
    <ProjectSync client={client} owner={owner} language={language} kind="activity" />
    <fieldset className="activity-sources"><legend>{t('选择来源设备（最多 16 个）', 'Select source devices (up to 16)')}</legend>{sources.map(node => <label className="direct-check" key={node}><input type="checkbox" checked={chosen.includes(node)} disabled={!chosen.includes(node) && chosen.length >= 16} onChange={event => setSelected(value => event.target.checked ? [...value.filter(source => sources.includes(source)), node] : value.filter(source => source !== node))} /><span>{node === owner ? t('本机 · ', 'This device · ') : ''}<code>{node}</code></span></label>)}</fieldset>
    <div className="direct-heading"><p role="status">{rows.length} {t('条匹配记录', 'matching records')}{reading ? t(' · 正在读取', ' · Reading') : ''}{paused ? t(' · 自动刷新已暂停', ' · Automatic updates paused') : t(' · 每 10 秒刷新', ' · Refreshes every 10 seconds')}</p><button type="button" disabled={reading || !chosen.length} onClick={() => read.current()}><RefreshCw size={16} aria-hidden="true" />{t('刷新协作记录', 'Refresh collaboration records')}</button></div>
    <div className="activity-source-status">{chosen.map(node => { const view = views[node], snapshot = view?.snapshot; return <div key={node} aria-label={t('记录来源 ', 'Record source ') + node}><strong>{node}</strong>{view?.error ? <p className="direct-error" role="alert">{view.error}</p> : snapshot ? <><p>{t('来源读取时间：', 'Source observation: ')}<time dateTime={new Date(snapshot.observed_at * 1000).toISOString()}>{date(snapshot.observed_at)}</time><small>{t('内容指纹：', 'Content digest: ')}<code>{snapshot.digest.slice(0, 12)}</code></small></p>{snapshot.sync && <p className={snapshot.sync.state === 'offline' ? 'activity-stale' : 'direct-note'}>{snapshot.sync.state === 'offline' ? t('离线快照 · 当前状态未经确认', 'Offline snapshot · current status unconfirmed') : snapshot.sync.warning ? t('实时记录 · 本机快照保存失败', 'Live records · local snapshot unavailable') : t('已保存本机快照', 'Local snapshot saved')}<small>{t('上次同步：', 'Last synced: ')}<time dateTime={new Date(snapshot.sync.synced_at).toISOString()}>{new Date(snapshot.sync.synced_at).toLocaleString(language)}</time></small></p>}{snapshot.sync?.warning && <p className="direct-error" role="alert">{snapshot.sync.warning}</p>}</> : <p role="status">{t('等待读取', 'Waiting to read')}</p>}</div>; })}</div>
    {chosen.some(node => views[node]?.error) && <p className="direct-note">{t('请检查设备连接，并在目标设备授予“只读协作记录”。原有消息授权不足以读取完整内容。', 'Check connectivity and grant Read collaboration records on the target. Message access alone does not allow reading all content.')} {onDevices && <button type="button" onClick={onDevices}>{t('管理设备授权', 'Manage device access')}</button>}</p>}
    <div className="activity-filters"><label>{t('搜索协作记录', 'Search collaboration records')}<input type="search" value={search} onChange={event => setSearch(event.target.value)} placeholder={t('标题、正文、结果或设备 ID', 'Title, content, result or device ID')} /></label><fieldset className="activity-kind"><legend>{t('记录类型', 'Record type')}</legend><div>{[['all', t('全部类型', 'All types')], ...Object.entries(labels)].map(([value, label]) => <label className={kind === value ? 'is-selected' : undefined} key={value}><input type="radio" name={kindGroup} value={value} checked={kind === value} onChange={() => setKind(value)} /><span>{label}</span></label>)}</div></fieldset></div>
    {!chosen.length ? <p>{t('请选择要查看的来源设备。', 'Select a source device to view its records.')}</p> : !rows.length && !reading ? <p>{t('没有匹配记录。没有可用离线快照时，读取失败的来源不会显示旧内容。', 'No matching records. Failed sources show previous content only when an explicitly saved offline snapshot is available.')}</p> : null}
    <div className="activity-list">{rows.slice(currentPage * 20, currentPage * 20 + 20).map(row => <article key={row.key} aria-label={labels[row.kind] + ': ' + row.title}><div className="direct-heading"><h4>{row.title}</h4><span>{labels[row.kind]} · {row.status}</span></div><p className="direct-note">{t('来源：', 'Source: ')}<code>{row.owner}</code> · {date(row.created)}</p><details><summary>{t('查看内容与报告', 'View content and reports')}</summary><pre>{row.text}</pre><pre>{row.detail}</pre></details></article>)}</div>
    {rows.length > 20 && <nav className="direct-actions" aria-label={t('协作记录分页', 'Collaboration records pages')}><button type="button" disabled={currentPage === 0} onClick={() => setPage(currentPage - 1)}>{t('上一页', 'Previous page')}</button><span>{currentPage + 1} / {pages}</span><button type="button" disabled={currentPage + 1 >= pages} onClick={() => setPage(currentPage + 1)}>{t('下一页', 'Next page')}</button></nav>}
  </div>;
}
