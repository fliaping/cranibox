import { useEffect, useId, useRef, useState } from 'react';
import { ArrowUpRight, Bot, Copy, Laptop, RefreshCw, Search } from 'lucide-react';
import type { DirectCatalog, DirectClient } from '../devices/DirectDevicesView';
import { agentCatalog, agentRegistrationInstructions, currentAgent, type AgentReport, type AgentSnapshot } from './agent-session-catalog';
import './agent-sessions.css';

type Source = { owner: string; local: boolean };
type View = { reading: boolean; snapshot?: AgentSnapshot; error?: string };

/** Query saved, authorized peers independently. No directory data is persisted in the browser. */
export function FederatedAgentSessions({ client, devices, owner, language, paused, disabled, onDelegate }: { client: DirectClient; devices?: DirectCatalog; owner: string; language: string; paused: boolean; disabled: boolean; onDelegate: (node: string, session: string) => void }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const [generation, setGeneration] = useState(0), [search, setSearch] = useState(''), [includeInactive, setIncludeInactive] = useState(false), [notice, setNotice] = useState('');
  const [state, setState] = useState<{ scope: string; views: Record<string, View> }>({ scope: '', views: {} });
  const [clock, setClock] = useState(() => performance.now());
  const pausedRef = useRef(paused), read = useRef<(owner?: string) => void>(() => undefined), filter = useId();
  pausedRef.current = paused;
  const routeKey = JSON.stringify([owner, devices?.node_id, (devices?.routes || []).map(route => [route.node, route.url, route.allow_private_http, route.paths])]);
  const scope = JSON.stringify([routeKey, generation, language]);
  const views = state.scope === scope ? state.views : {};
  const sources: Source[] = [{ owner, local: true }, ...Array.from(new Set((devices?.routes || []).map(route => route.node))).filter(node => node !== owner).sort().map(owner => ({ owner, local: false }))];
  useEffect(() => {
    const changed = () => setGeneration(value => value + 1);
    window.addEventListener('cranibox:peers-changed', changed);
    return () => window.removeEventListener('cranibox:peers-changed', changed);
  }, []);
  useEffect(() => {
    let disposed = false, running = 0;
    const queue: Source[] = [], queued = new Set<string>(), active = new Set<string>();
    setState({ scope, views: Object.fromEntries(sources.map(source => [source.owner, { reading: true }])) });
    const publish = (node: string, view: View) => {
      if (!disposed) setState(previous => previous.scope === scope ? { scope, views: { ...previous.views, [node]: view } } : previous);
    };
    const pump = () => {
      if (disposed || document.hidden) return;
      while (running < 4 && queue.length) {
        const source = queue.shift()!;
        queued.delete(source.owner); active.add(source.owner); running++;
        setState(previous => previous.scope === scope ? { scope, views: { ...previous.views, [source.owner]: { ...previous.views[source.owner], reading: true } } } : previous);
        void (async () => {
          try {
            const response = await client({ operation: 'agent_sessions', node: source.local ? null : source.owner, command: { operation: 'list' } });
            const catalog = agentCatalog(response, source.owner, t('会话目录身份或格式无效，请检查设备服务。', 'Invalid session catalog or device identity. Check the device service.'));
            publish(source.owner, { reading: false, snapshot: { catalog, received: performance.now() } });
          } catch (error) {
            // Revocation and read failure remove this source's previous public metadata.
            publish(source.owner, { reading: false, error: error instanceof Error ? error.message : String(error) });
          } finally { active.delete(source.owner); running--; pump(); }
        })();
      }
    };
    const enqueue = (node?: string) => {
      if (disposed || document.hidden) return;
      for (const source of sources) if ((!node || source.owner === node) && !queued.has(source.owner) && !active.has(source.owner)) { queue.push(source); queued.add(source.owner); }
      pump();
    };
    read.current = enqueue; enqueue();
    const timer = setInterval(() => { if (!pausedRef.current) enqueue(); }, 5000);
    const resume = () => { if (!document.hidden) { pump(); if (!pausedRef.current) enqueue(); } };
    document.addEventListener('visibilitychange', resume);
    return () => { disposed = true; clearInterval(timer); document.removeEventListener('visibilitychange', resume); };
    // The serialized route key prevents unrelated task polling from restarting these reads.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, scope]);
  useEffect(() => { const timer = setInterval(() => setClock(performance.now()), 1000); return () => clearInterval(timer); }, []);
  useEffect(() => { if (!paused) read.current(); }, [paused]);
  const verified = sources.filter(source => views[source.owner]?.snapshot).length;
  const current = sources.reduce((count, source) => { const snapshot = views[source.owner]?.snapshot; return count + (snapshot?.catalog.sessions.filter(report => currentAgent(report, snapshot, clock)).length || 0); }, 0);
  const reading = sources.some(source => views[source.owner]?.reading ?? true);
  const query = search.trim().toLocaleLowerCase();
  const status = (report: AgentReport, snapshot: AgentSnapshot) => report.status === 'ended' ? t('已结束登记', 'Registration ended') : !currentAgent(report, snapshot, clock) ? t('登记已过期', 'Report expired') : ({ ready: t('报告空闲', 'Reports ready'), busy: t('报告忙碌', 'Reports busy'), waiting: t('报告等待中', 'Reports waiting') }[report.status]);
  async function copyInstructions() {
    try { await navigator.clipboard.writeText(agentRegistrationInstructions(language)); setNotice(t('已复制本机 Agent 登记说明。', 'Local Agent registration instructions copied.')); }
    catch { setNotice(t('复制失败。请让本机 Agent 使用 device_agent_sessions 的 register 登记，并每两分钟 heartbeat。', 'Copy failed. Ask the local Agent to register through device_agent_sessions and heartbeat every two minutes.')); }
  }
  return <section className="agent-federation" aria-label={t('所有设备的 Agent', 'Agents across devices')}>
    <div className="agent-federation-heading"><div><h3><Bot size={20} aria-hidden="true" />{t('所有设备的 Agent', 'Agents across devices')}</h3><p role="status" aria-atomic="true">{t('有效登记', 'Current registrations')}: {current} · {t('已确认设备', 'Verified devices')}: {verified} / {sources.length}{paused ? t(' · 已暂停自动刷新', ' · Updates paused') : reading ? t(' · 正在刷新', ' · Refreshing') : ''}</p></div><div className="direct-actions"><button type="button" onClick={() => void copyInstructions()}><Copy size={16} aria-hidden="true" />{t('登记本机 Agent', 'Register a local Agent')}</button><button type="button" onClick={() => read.current()}><RefreshCw size={16} aria-hidden="true" />{t('刷新所有 Agent', 'Refresh all Agents')}</button></div></div>
    <p className="direct-note">{t('汇总本机与已连接设备的公开登记；目标需授予消息权限。状态由 Agent 报告，5 分钟未续期即过期。选中会话仅填写委托，不会启动程序。', 'Public registrations from this device and saved peers; peers must grant message access. Agents report their own state and expire after 5 minutes without renewal. Selecting a session fills the delegation form and never starts a process.')}</p>
    {notice && <p role="status">{notice}</p>}
    <div className="agent-federation-filters"><label htmlFor={filter}>{t('查找 Agent 或设备', 'Find an Agent or device')}<span><Search size={17} aria-hidden="true" /><input id={filter} type="search" value={search} onChange={event => setSearch(event.target.value)} placeholder={t('名称、运行工具或设备 ID', 'Name, harness or device ID')} /></span></label><label className="agent-federation-inactive"><input type="checkbox" checked={includeInactive} onChange={event => setIncludeInactive(event.target.checked)} />{t('显示过期与结束的登记', 'Include expired and ended registrations')}</label></div>
    <div className="agent-federation-sources">{sources.map(source => {
      const view = views[source.owner], snapshot = view?.snapshot;
      const reports = snapshot?.catalog.sessions.filter(report => (includeInactive || currentAgent(report, snapshot, clock)) && (!query || [source.owner, report.name, report.harness].some(value => value.toLocaleLowerCase().includes(query)))) || [];
      return <article className="agent-federation-source" key={source.owner} aria-label={t('设备 ', 'Device ') + source.owner}>
        <div className="agent-federation-device"><div><Laptop size={17} aria-hidden="true" /><strong>{source.local ? t('本机', 'This device') : t('已连接设备', 'Connected device')}</strong><code>{source.owner}</code></div><span>{view?.error ? t('无法读取', 'Unavailable') : !snapshot ? t('正在读取…', 'Reading…') : reports.length + ' ' + t('条登记', 'registrations')}</span></div>
        {view?.error ? <div className="agent-federation-unavailable"><p className="direct-error">{view.error}</p><button type="button" disabled={view.reading} onClick={() => read.current(source.owner)}>{t('重试读取', 'Retry directory')}</button></div> : snapshot && !reports.length ? <p className="direct-note">{query ? t('没有匹配的登记。', 'No matching registrations.') : includeInactive ? t('暂无登记会话。', 'No registered sessions.') : t('暂无有效登记。已运行的 Agent 可通过 MCP 登记；登记不等同于进程状态。', 'No current registrations. A running Agent can register through MCP; registration is separate from process status.')}</p> : <div className="agent-session-list">{reports.map(report => <article className="agent-session-card" key={report.id}>
          <div><strong>{report.name}</strong><span className={currentAgent(report, snapshot!, clock) ? 'agent-session-current' : 'direct-note'}>{status(report, snapshot!)}</span></div><p>{report.harness}</p><details><summary>{t('会话与最近报告', 'Session and last report')}</summary><code>{report.id}</code><small>{new Date(report.updated_at * 1000).toLocaleString(language)}</small></details>
          {!source.local && <button type="button" disabled={disabled || !currentAgent(report, snapshot!, clock)} onClick={() => { if (snapshot && currentAgent(report, snapshot, performance.now())) onDelegate(source.owner, report.id); }}>{t('委托给此 Agent', 'Delegate to this Agent')}<ArrowUpRight size={16} aria-hidden="true" /></button>}
        </article>)}</div>}
      </article>;
    })}</div>
  </section>;
}
