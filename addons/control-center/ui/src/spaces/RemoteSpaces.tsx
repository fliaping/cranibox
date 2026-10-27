import { useEffect, useRef, useState } from 'react';
import { Box, Cable, ChevronDown, LoaderCircle, RefreshCw } from 'lucide-react';
import type { DirectCatalog, DirectClient } from '../devices/DirectDevicesView';

type Summary = { id: string; name: string; node: string; provider: string; phase: string };
type Catalog = { owner: string; spaces: Summary[] };
type Source = { node: string; phase: 'loading' | 'ready' | 'denied' | 'error'; checked?: number; error?: string; catalog?: Catalog };
const problem = (error: unknown) => error instanceof Error ? error.message : String(error);
const providers: Record<string, string> = { docker: 'Docker', colima: 'Colima', orbstack: 'OrbStack', wsl: 'WSL2 · Ubuntu' };

/** Routes come from this device's configuration. A directory entry can never
 * choose an endpoint, obtain credentials or create another connection. */
export function RemoteSpaces({ client, language, phaseName, onDevices }: {
  client: DirectClient; language: string; phaseName: (phase: string) => string; onDevices: () => void;
}) {
  const t = (zh: string, en: string) => language.startsWith('zh') ? zh : en;
  const [sources, setSources] = useState<Source[]>([]), [error, setError] = useState(''), [loading, setLoading] = useState(true);
  const [showDetails, setShowDetails] = useState(false);
  const refresh = useRef<() => void>(() => {});
  useEffect(() => {
    let disposed = false, generation = 0, active = false, again = false;
    async function load() {
      if (document.hidden || disposed) return;
      if (active) { again = true; return; }
      active = true; const current = ++generation; setLoading(true);
      const valid = () => !disposed && current === generation;
      try {
        const config = await client({ operation: 'list' }) as DirectCatalog;
        if (!Array.isArray(config?.routes)) throw Error('Device connections unavailable');
        const nodes = [...new Set(config.routes.map(route => route.node))].sort();
        if (!valid()) return;
        setError(''); setSources(nodes.map(node => ({ node, phase: 'loading' })));
        let next = 0;
        const worker = async () => {
          while (next < nodes.length && valid()) {
            const node = nodes[next++]; let result: Source;
            try {
              const identity = await client({ operation: 'probe', node }) as { nodes: { id: string; spaces?: boolean }[] };
              if (identity?.nodes?.length !== 1 || identity.nodes[0].id !== node) throw Error('Unexpected device identity');
              if (identity.nodes[0].spaces !== true) result = { node, phase: 'denied', checked: Date.now() };
              else {
                const catalog = await client({ operation: 'space_catalog', node }) as Catalog;
                if (catalog?.owner !== node || !Array.isArray(catalog.spaces) || catalog.spaces.length > 32) throw Error('Unexpected space catalog owner or size');
                result = { node, phase: 'ready', checked: Date.now(), catalog };
              }
            } catch (error) { const msg = problem(error); result = msg === 'capability_off' ? { node, phase: 'denied', checked: Date.now() } : { node, phase: 'error', checked: Date.now(), error: msg }; }
            if (valid()) setSources(previous => previous.map(source => source.node === node ? result : source));
          }
        };
        await Promise.all(Array.from({ length: Math.min(8, nodes.length) }, worker));
      } catch (error) { if (valid()) { setError(problem(error)); setSources([]); } }
      finally {
        active = false;
        if (valid()) setLoading(false);
        if (again && !disposed) { again = false; void load(); }
      }
    }
    function changed() {
      // Hide former credentials' metadata immediately; an older response may
      // not restore it while a new connection is being checked.
      ++generation; setSources([]); setLoading(true); void load();
    }
    refresh.current = () => { void load(); };
    const visible = () => { if (!document.hidden) void load(); };
    void load(); const timer = setInterval(() => { if (!active) void load(); }, 15000);
    window.addEventListener('cranibox:peers-changed', changed);
    document.addEventListener('visibilitychange', visible);
    return () => { disposed = true; ++generation; clearInterval(timer); window.removeEventListener('cranibox:peers-changed', changed); document.removeEventListener('visibilitychange', visible); };
  }, [client]);
  const ready = sources.filter(source => source.phase === 'ready');
  const unavailable = sources.filter(source => source.phase === 'denied' || source.phase === 'error');
  const checking = sources.filter(source => source.phase === 'loading').length;
  const denied = unavailable.filter(source => source.phase === 'denied').length;
  const failed = unavailable.length - denied;
  const count = ready.reduce((sum, source) => sum + (source.catalog?.spaces.length || 0), 0);
  const checkedAt = (source: Source) => source.checked && <time dateTime={new Date(source.checked).toISOString()}>{t('检查于 ', 'Checked ')}{new Date(source.checked).toLocaleTimeString()}</time>;
  return <section className="remote-spaces" aria-label={t('其他设备的虚拟空间', 'Spaces on other devices')}>
    <div className="direct-heading"><div><h3><Cable size={18} aria-hidden="true" />{t('其他设备', 'Other devices')}</h3><p>{t('从已连接的设备读取授权空间目录。空间的管理和访问仍由归属设备控制。', 'Read shared space directories from connected devices. Each owner controls management and access.')}</p></div><div className="direct-actions"><button disabled={loading} aria-label={t('刷新远端空间', 'Refresh remote spaces')} onClick={() => refresh.current()}>{loading ? <LoaderCircle size={16} className="spaces-spinner" aria-hidden="true" /> : <RefreshCw size={16} aria-hidden="true" />}</button><button onClick={onDevices}>{t('设备连接', 'Device connections')}</button></div></div>
    {error && <p className="direct-error" role="alert">{error}</p>}
    {!sources.length && !error && <p className="spaces-muted" role="status">{loading ? t('正在读取设备连接…', 'Reading device connections…') : t('连接其他设备，并在对方授予“空间目录”权限后，即可在这里查看其空间。', 'Connect another device and ask its owner to grant Space catalog access to see its spaces here.')}</p>}
    {sources.length > 0 && <p className="spaces-muted" role="status">{count} {t('个空间', count === 1 ? 'space' : 'spaces')} · {ready.length}/{sources.length} {t('个设备目录可用', 'device directories available')}{loading && t(' · 正在检查', ' · Checking')}</p>}
    {ready.length > 0 && <div className="remote-space-sources">{ready.map(source => <article key={source.node} className="remote-space-source" aria-label={source.node}>
      <div className="remote-space-source-head"><strong>{source.node}</strong>{checkedAt(source)}</div>
      {!source.catalog?.spaces.length ? <p className="spaces-muted">{t('此设备的空间目录为空。', 'This device’s space directory is empty.')}</p> : <ul className="remote-space-list">{source.catalog.spaces.map(space => <li key={space.id}><Box size={18} aria-hidden="true" /><div><strong>{space.name}</strong><small>{providers[space.provider] || space.provider} · {space.node}</small></div><span>{phaseName(space.phase)}</span></li>)}</ul>}
      <p className="spaces-muted">{t('以上为管理目录记录的状态，未检测容器。终端、项目和桌面需要单独连接并取得空间节点授权。', 'Status comes from the management directory; containers were not inspected. Connect to and obtain access from each space node to use its terminals, projects or desktop.')}</p>
    </article>)}</div>}
    {(unavailable.length > 0 || checking > 0) && <details className="remote-space-details" open={showDetails} onToggle={event => setShowDetails(event.currentTarget.open)}>
      <summary><span>{t('设备目录状态', 'Device directory status')}</span><span className="remote-space-counts">{[denied > 0 && t(`${denied} 个未共享`, `${denied} not shared`), failed > 0 && t(`${failed} 个不可用`, `${failed} unavailable`), checking > 0 && t(`${checking} 个正在检查`, `${checking} checking`)].filter(Boolean).join(' · ')}</span><ChevronDown size={16} aria-hidden="true" /></summary>
      <div className="remote-space-details-body">
        {denied > 0 && <p className="spaces-muted">{t('未共享的设备没有授予空间目录权限，或版本尚不支持。请在对方的入站授权中开启。', 'Space catalog access is missing or unsupported on devices marked Not shared. Enable it in the target’s Incoming access settings.')}</p>}
        {failed > 0 && <p className="spaces-muted">{t('无法读取目录，不代表空间已删除。可刷新重试，或在设备连接中检查连接。', 'An unavailable directory does not mean its spaces were deleted. Refresh to retry, or check Device connections.')}</p>}
        {checking > 0 && <p className="spaces-muted">{t('正在检查其余设备的目录权限…', 'Checking directory access on the remaining devices…')}</p>}
        <div className="remote-space-issues">{unavailable.map(source => <article key={source.node} className="remote-space-issue" aria-label={source.node}>
          <div className="remote-space-issue-head"><strong>{source.node}</strong><span>{source.phase === 'denied' ? t('未共享', 'Not shared') : t('不可用', 'Unavailable')}</span></div>
          {source.error && <p className="remote-space-issue-error">{source.error}</p>}
          {checkedAt(source)}
        </article>)}</div>
      </div>
    </details>}
  </section>;
}
