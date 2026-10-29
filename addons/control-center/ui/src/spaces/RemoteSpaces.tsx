import { useEffect, useRef, useState } from 'react';
import { Box, Cable, ChevronDown, LoaderCircle, Play, RefreshCw, Square } from 'lucide-react';
import { ask } from '../confirm';
import { useOptionalApp } from '../app/store';
import { DeviceIcon, formatDeviceLabel, shortNodeId } from '../app/ui';
import type { DirectCatalog, DirectClient } from '../devices/DirectDevicesView';
import { useTerminalDevices } from '../app/targets';

type Summary = { id: string; name: string; node: string; provider: string; phase: string };
type Catalog = { owner: string; spaces: Summary[] };
type Source = { node: string; name?: string; kind?: string; phase: 'loading' | 'ready' | 'denied' | 'error'; checked?: number; error?: string; catalog?: Catalog };
const problem = (error: unknown) => error instanceof Error ? error.message : String(error);
const providers: Record<string, string> = { docker: 'Docker', colima: 'Colima', orbstack: 'OrbStack', wsl: 'WSL2 · Ubuntu' };

/** Routes come from this device's configuration. A directory entry can never
 * choose an endpoint, obtain credentials or create another connection. */
export function RemoteSpaces({ client, language, phaseName, onDevices }: {
  client: DirectClient; language: string; phaseName: (phase: string) => string; onDevices: () => void;
}) {
  const t = (zh: string, en: string) => language.startsWith('zh') ? zh : en;
  const app = useOptionalApp();
  const { nodes: terminalNodes } = useTerminalDevices();
  const [sources, setSources] = useState<Source[]>([]), [error, setError] = useState(''), [loading, setLoading] = useState(true);
  const [showDetails, setShowDetails] = useState(false);
  const [controlling, setControlling] = useState(''), [controlError, setControlError] = useState<{ id: string; message: string } | null>(null);
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
              const identity = await client({ operation: 'probe', node }) as { nodes: { id: string; name?: string; kind?: string; spaces?: boolean }[] };
              if (identity?.nodes?.length !== 1 || identity.nodes[0].id !== node) throw Error('Unexpected device identity');
              const peerInfo = identity.nodes[0];
              const peerName = peerInfo.name && peerInfo.name !== node ? peerInfo.name : undefined;
              const peerKind = peerInfo.kind;
              if (peerInfo.spaces !== true) result = { node, name: peerName, kind: peerKind, phase: 'denied', checked: Date.now() };
              else {
                const catalog = await client({ operation: 'space_catalog', node }) as Catalog;
                if (catalog?.owner !== node || !Array.isArray(catalog.spaces) || catalog.spaces.length > 32) throw Error('Unexpected space catalog owner or size');
                result = { node, name: peerName, kind: peerKind, phase: 'ready', checked: Date.now(), catalog };
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
      ++generation; setSources([]); setLoading(true); void load();
    }
    refresh.current = () => { void load(); };
    const visible = () => { if (!document.hidden) void load(); };
    void load(); const timer = setInterval(() => { if (!active) void load(); }, 15000);
    window.addEventListener('cranibox:peers-changed', changed);
    document.addEventListener('visibilitychange', visible);
    return () => { disposed = true; ++generation; clearInterval(timer); window.removeEventListener('cranibox:peers-changed', changed); document.removeEventListener('visibilitychange', visible); };
  }, [client]);

  function resolveSourceDevice(source: Source) {
    const appDevice = app?.devices.find(d => d.node_id === source.node);
    const termNode = terminalNodes.find(n => n.id === source.node);
    const rawName = appDevice?.name || source.name || (termNode?.name && termNode.name !== source.node ? termNode.name : source.node);
    const kind = appDevice?.kind || source.kind || 'server';
    return { ...formatDeviceLabel(rawName, source.node, t), kind };
  }

  /** Plain-language reason a remote device refused or could not be reached. */
  function controlProblem(message: string) {
    if (/does not allow|\b403\b.*(forbidden|capability|not granted)|API 403|capability not granted|capability_off/i.test(message)) return t('这台设备不允许你停止或启动它的空间。请在那台设备的“入站权限”里为本设备同时开启“空间”和“命令执行”。', 'This device doesn’t allow you to stop or start its spaces. On that device, turn on both Spaces and Commands in Incoming access for this device.');
    if (/space not found|API 404/i.test(message)) return t('那台设备上找不到这个空间，可能已被删除。请刷新后再试。', 'This space was not found on that device. It may have been deleted. Refresh and try again.');
    if (/timed out|timeout|connect|unreachable|refused|offline|dns|network|reset/i.test(message)) return t('无法连接这台设备，它可能已离线或已休眠。请稍后重试。', 'Couldn’t reach this device. It may be offline or asleep. Try again later.');
    return t('那台设备未能完成操作：', 'That device couldn’t do this: ') + message;
  }
  async function control(source: Source, space: Summary, action: 'stop' | 'resume') {
    if (action === 'stop' && !await ask(t(`停止“${space.name}”？\n\n它在另一台设备上运行，其中的 Agent 和终端会被中断。数据会保留，之后可以再次启动。`, `Stop “${space.name}”?\n\nIt runs on another device; its Agents and terminals will be interrupted. Data is kept, and you can start it again later.`))) return;
    setControlling(space.id); setControlError(null);
    try {
      const catalog = await client({ operation: 'space_control', node: source.node, id: space.id, action }) as Catalog;
      if (catalog?.owner === source.node && Array.isArray(catalog.spaces)) setSources(previous => previous.map(item => item.node === source.node ? { ...item, catalog, checked: Date.now() } : item));
      else refresh.current();
    } catch (error) { setControlError({ id: space.id, message: controlProblem(problem(error)) }); }
    finally { setControlling(''); }
  }
  const remoteBusy = ['preparing', 'pulling', 'creating', 'starting', 'enrolling', 'stopping', 'removing', 'deleting'];

  const ready = sources.filter(source => source.phase === 'ready');
  const unavailable = sources.filter(source => source.phase === 'denied' || source.phase === 'error');
  const checking = sources.filter(source => source.phase === 'loading').length;
  const denied = unavailable.filter(source => source.phase === 'denied').length;
  const failed = unavailable.length - denied;
  const count = ready.reduce((sum, source) => sum + (source.catalog?.spaces.length || 0), 0);
  const checkedAt = (source: Source) => source.checked && <time dateTime={new Date(source.checked).toISOString()}>{t('检查于 ', 'Checked ')}{new Date(source.checked).toLocaleTimeString()}</time>;
  return <section className="remote-spaces" aria-label={t('其他设备的虚拟空间', 'Spaces on other devices')}>
    <div className="direct-heading"><div><h3><Cable size={18} aria-hidden="true" />{t('其他设备', 'Other devices')}</h3><p>{t('查看已连接设备共享的虚拟空间。', 'Spaces shared by your connected devices.')}</p></div><div className="direct-actions"><button disabled={loading} aria-label={t('刷新远端空间', 'Refresh remote spaces')} onClick={() => refresh.current()}>{loading ? <LoaderCircle size={16} className="spaces-spinner" aria-hidden="true" /> : <RefreshCw size={16} aria-hidden="true" />}</button><button onClick={onDevices}>{t('设备连接', 'Device connections')}</button></div></div>
    {error && <p className="direct-error" role="alert">{error}</p>}
    {!sources.length && !error && <p className="spaces-muted" role="status">{loading ? t('正在读取设备连接…', 'Reading device connections…') : t('连接其他设备并开启空间共享后，即可在这里查看。', 'Connect another device and enable space sharing to see its spaces here.')}</p>}
    {sources.length > 0 && <p className="spaces-muted" role="status">{count} {t('个空间', count === 1 ? 'space' : 'spaces')} · {ready.length}/{sources.length} {t('个设备目录可用', 'device directories available')}{loading && t(' · 正在检查', ' · Checking')}</p>}
    {ready.length > 0 && <div className="remote-space-sources">{ready.map(source => {
      const dev = resolveSourceDevice(source);
      return <article key={source.node} className="remote-space-source" aria-label={source.node}>
        <div className="remote-space-source-head">
          <div className="remote-device-identity">
            <span className="remote-device-icon"><DeviceIcon kind={dev.kind} size={16} /></span>
            <strong>{dev.label}{dev.rawHidden && <span className="cx-sr"> {dev.rawHidden}</span>}</strong>
            {dev.shortId && <span className="cx-id-pill">{dev.shortId}</span>}
          </div>
          {checkedAt(source)}
        </div>
        {!source.catalog?.spaces.length ? <p className="remote-space-empty-row spaces-muted">{t('这台设备上还没有空间', 'No spaces on this device yet')}<span className="cx-sr"> {t('此设备的空间目录为空。', 'This device’s space directory is empty.')}</span></p> : <ul className="remote-space-list">{source.catalog.spaces.map(space => {
          const shortNode = shortNodeId(space.node);
          return <li key={space.id}><Box size={18} aria-hidden="true" /><div><strong>{space.name}</strong><small>{providers[space.provider] || space.provider} · {shortNode.full ? <span className="cx-id-pill">{shortNode.display}<span className="cx-sr"> {shortNode.full}</span></span> : space.node}</small></div><span className={'remote-space-phase ' + space.phase}>{remoteBusy.includes(space.phase) && <LoaderCircle size={13} className="spaces-spinner" aria-hidden="true" />}{phaseName(space.phase)}</span>
            <span className="remote-space-actions">{space.phase === 'ready' && <button disabled={!!controlling} aria-busy={controlling === space.id} aria-label={t(`停止 ${space.name}`, `Stop ${space.name}`)} onClick={() => void control(source, space, 'stop')}>{controlling === space.id ? <LoaderCircle size={14} className="spaces-spinner" aria-hidden="true" /> : <Square size={13} aria-hidden="true" />}{t('停止', 'Stop')}</button>}{['stopped', 'failed', 'interrupted'].includes(space.phase) && <button disabled={!!controlling} aria-busy={controlling === space.id} aria-label={t(`启动 ${space.name}`, `Start ${space.name}`)} onClick={() => void control(source, space, 'resume')}>{controlling === space.id ? <LoaderCircle size={14} className="spaces-spinner" aria-hidden="true" /> : <Play size={13} aria-hidden="true" />}{t('启动', 'Start')}</button>}</span>
            {controlError?.id === space.id && <p className="direct-error remote-space-error" role="alert">{controlError.message}</p>}</li>;
        })}</ul>}
        <span className="cx-sr">{t('以上为管理目录记录的状态，未检测容器。终端、项目和桌面需要单独连接并取得空间节点授权。', 'Status comes from the management directory; containers were not inspected. Connect to and obtain access from each space node to use its terminals, projects or desktop.')}</span>
      </article>;
    })}</div>}
    {(unavailable.length > 0 || checking > 0) && <details className="remote-space-details" open={showDetails} onToggle={event => setShowDetails(event.currentTarget.open)}>
      <summary><span>{t('设备目录状态', 'Device directory status')}</span><span className="remote-space-counts">{[denied > 0 && t(`${denied} 个未共享`, `${denied} not shared`), failed > 0 && t(`${failed} 个不可用`, `${failed} unavailable`), checking > 0 && t(`${checking} 个正在检查`, `${checking} checking`)].filter(Boolean).join(' · ')}</span><ChevronDown size={16} aria-hidden="true" /></summary>
      <div className="remote-space-details-body">
        {denied > 0 && <p className="spaces-muted">{t('未共享的设备尚未开启空间目录权限。可在对应设备的入站权限中开启。', 'Space catalog access is missing or unsupported on devices marked Not shared. Enable it in the target’s Incoming access settings.')}</p>}
        {failed > 0 && <p className="spaces-muted">{t('暂时无法连接这些设备，可刷新重试或检查设备连接。', 'Could not reach these devices right now. Refresh to retry, or check Device connections.')}</p>}
        {checking > 0 && <p className="spaces-muted">{t('正在检查其余设备的目录权限…', 'Checking directory access on the remaining devices…')}</p>}
        <div className="remote-space-issues">{unavailable.map(source => {
          const dev = resolveSourceDevice(source);
          return <article key={source.node} className="remote-space-issue" aria-label={source.node}>
            <div className="remote-space-issue-head">
              <div className="remote-device-identity">
                <span className="remote-device-icon"><DeviceIcon kind={dev.kind} size={15} /></span>
                <strong>{dev.label}{dev.rawHidden && <span className="cx-sr"> {dev.rawHidden}</span>}</strong>
                {dev.shortId && <span className="cx-id-pill">{dev.shortId}</span>}
              </div>
              <span className="remote-issue-badge">{source.phase === 'denied' ? t('未共享', 'Not shared') : t('不可用', 'Unavailable')}</span>
            </div>
            {source.error && <p className="remote-space-issue-error">{source.error}</p>}
            {checkedAt(source)}
          </article>;
        })}</div>
      </div>
    </details>}
  </section>;
}
