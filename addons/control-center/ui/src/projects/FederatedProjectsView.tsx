import { useEffect, useMemo, useRef, useState } from 'react';
import { Cable, Database } from 'lucide-react';
import type { DirectCatalog, DirectClient } from '../devices/DirectDevicesView';
import { directTarget, localTarget, terminalTarget } from '../terminal/TerminalEnvironment';
import { ProjectsView, type Catalog, type Device, type ProjectClient } from './ProjectsView';
import { ProjectSync } from './ProjectSync';

type RemoteCatalog = Catalog & { owner: string; permissions: { terminals: boolean; projects: boolean; exec?: boolean }; sync?: { state: 'online' | 'offline'; synced_at?: number; warning?: string } };
type Source = { id: string; checked: number; revision?: number; error?: string; terminals?: boolean; exec?: boolean; offline?: boolean; syncedAt?: number; warning?: string };
const problem = (error: unknown) => error instanceof Error ? error.message : String(error);

/** Each write goes only to its authoritative source. Rust supplies explicitly
 * stale, read-only snapshots for outages; denied sources contribute no paths. */
export function FederatedProjectsView({ client: gateway, direct, devices, language = 'en', local, gatewayEnabled = true }: {
  client: ProjectClient; direct: DirectClient; devices: Device[]; language?: string;
  local?: { id: string; name: string; client: ProjectClient }; gatewayEnabled?: boolean;
}) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const [sources, setSources] = useState<Source[]>([]);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const client = useMemo<ProjectClient>(() => {
    const catalogs = new Map<string, Catalog>();
    let current: Source[] = [], revision = 0, mutationVersion = 0;
    const publish = () => { if (mounted.current) setSources([...current]); };
    const merged = (): Catalog => {
      const projects = new Map<string, Catalog['projects'][number]>();
      for (const [source, catalog] of catalogs) for (const project of catalog.projects) {
        const route = source === ':local' && local ? localTarget(local.id) : directTarget(source);
        const state = current.find(s => s.id === source);
        const replicas = project.replicas.map(replica => source ? { ...replica,
          // Route information comes from local configuration, never remote metadata.
          id: route + '/' + replica.id, node: route,
          ...(state?.offline ? { offline_synced_at: state.syncedAt } : {}),
        } : replica);
        const existing = projects.get(project.id);
        if (existing) existing.replicas.push(...replicas);
        else projects.set(project.id, { ...project, replicas });
      }
      return { revision: ++revision, projects: [...projects.values()] };
    };
    const remote = async (node: string, command: Record<string, unknown>) => {
      const value = await direct({ operation: 'projects', node, command }) as RemoteCatalog;
      if (value.owner !== node || !value.permissions?.projects || !Array.isArray(value.projects) ||
        value.projects.some(p => !Array.isArray(p.replicas) || p.replicas.some(r => r.node !== node))) throw Error('Unexpected project catalog owner');
      if (value.sync?.state === 'offline' && (command.operation !== 'list' ||
        !Number.isSafeInteger(value.sync.synced_at) || value.sync.synced_at! <= 0 || value.sync.synced_at! > 253402300799000)) throw Error('Invalid offline project catalog');
      return value;
    };
    return {
      copy: command => direct(command),
      projects: async command => {
        if (command.operation === 'list') {
          const version = mutationVersion;
          // Resolve the gateway and direct catalogs independently, so an unavailable
          // gateway cannot delay starting the requests to healthy peers.
          const [base, connections, own] = await Promise.allSettled([gatewayEnabled ? gateway.projects(command) : Promise.resolve(undefined), (async () => {
            const config = await direct({ operation: 'list' }) as DirectCatalog;
            if (!Array.isArray(config?.routes)) throw Error('Direct connections unavailable');
            const routes = [...config.routes].sort((a, b) => a.node.localeCompare(b.node));
            const results = await Promise.allSettled(routes.map(route => remote(route.node, { operation: 'list' })));
            return { routes, results };
          })(), local?.client.projects(command)]);
          // A list that started before a write must not replace that write's result.
          if (version !== mutationVersion) return merged();
          catalogs.clear(); current = [];
          if (local) {
            if (own.status === 'fulfilled' && own.value) { catalogs.set(':local', own.value); current.push({ id: ':local', checked: Date.now(), revision: own.value.revision, terminals: true, exec: true }); }
            else current.push({ id: ':local', checked: Date.now(), error: own.status === 'rejected' ? problem(own.reason) : 'Local catalog unavailable' });
          }
          if (gatewayEnabled) {
            if (base.status === 'fulfilled' && base.value) { catalogs.set('', base.value); current.push({ id: '', checked: Date.now(), revision: base.value.revision }); }
            else current.push({ id: '', checked: Date.now(), error: base.status === 'rejected' ? problem(base.reason) : 'Workspace catalog unavailable' });
          }
          if (connections.status === 'fulfilled') {
            const { routes, results } = connections.value;
            results.forEach((result, i) => {
              const id = routes[i].node;
              if (result.status === 'fulfilled') {
                const value = result.value;
                catalogs.set(id, value); current.push({ id, checked: Date.now(), revision: value.revision, terminals: value.permissions.terminals, exec: value.permissions.exec,
                  offline: value.sync?.state === 'offline', syncedAt: value.sync?.synced_at, warning: value.sync?.warning });
              } else current.push({ id, checked: Date.now(), error: problem(result.reason) });
            });
          } else current.push({ id: ':routes', checked: Date.now(), error: problem(connections.reason) });
          publish(); return merged();
        }
        let source = '', request = { ...command };
        if (command.operation === 'register' && typeof command.node === 'string') {
          const target = terminalTarget(command.node);
          if ('direct' in target) { source = target.node; request.node = source; }
          else if ('local' in target && local && target.node === local.id) { source = ':local'; request.node = local.id; }
        } else if (typeof command.replica_id === 'string' && /^(direct|local):/.test(command.replica_id)) {
          const separator = command.replica_id.lastIndexOf('/');
          const target = terminalTarget(command.replica_id.slice(0, separator));
          const replica = command.replica_id.slice(separator + 1);
          source = 'direct' in target ? target.node : 'local' in target && local && target.node === local.id ? ':local' : '';
          if (!source || !catalogs.get(source)?.projects.some(p => p.replicas.some(r => r.id === replica))) throw Error('Project replica unavailable');
          request.replica_id = replica;
        }
        ++mutationVersion;
        if (current.some(s => s.id === source && s.offline)) throw Error('Device is offline. Refresh its catalog after reconnecting.');
        if (!source && !gatewayEnabled) throw Error('Workspace catalog unavailable');
        const value = source === ':local' && local ? await local.client.projects(request) : source ? await remote(source, request) : await gateway.projects(request);
        catalogs.set(source, value);
        current = current.map(s => s.id === source ? { ...s, error: undefined, offline: false, syncedAt: undefined, revision: value.revision, checked: Date.now() } : s);
        publish(); return merged();
      },
      terminals: (node, command) => {
        const target = terminalTarget(node || '');
        return 'local' in target && local && target.node === local.id ? local.client.terminals(local.id, command) : 'direct' in target
          ? direct({ operation: 'sessions', node: target.node, command }) as ReturnType<ProjectClient['terminals']>
          : gateway.terminals(node, command);
      },
      open: gateway.open,
    };
  }, [gateway, direct, local, gatewayEnabled]);
  const gatewayFailed = sources.some(s => !s.id && s.error);
  const targets = [
    ...devices.map(device => ({ ...device, projects: gatewayEnabled && !gatewayFailed, online: gatewayEnabled && !gatewayFailed && device.online })),
    ...sources.filter(s => s.id && s.id !== ':routes' && !s.error).map(s => ({ id: s.id === ':local' && local ? localTarget(local.id) : directTarget(s.id), name: s.id === ':local' && local ? local.name : s.id + t(' · 直连', ' · Direct'), online: !s.offline, projects: !s.offline, terminal_sessions: !s.offline && s.terminals === true, can_copy: !s.offline && s.exec === true })),
  ];
  const failed = sources.filter(s => s.error).length;
  const offline = sources.filter(s => s.offline).length;
  const sourceDetails = sources.length > 0 && <details className="projects-sources">
    <summary><Database size={16} aria-hidden="true" /><strong>{t('目录来源', 'Catalog sources')}</strong><span role="status" aria-atomic="true">{sources.length - failed - offline}/{sources.length} {t('已读取', 'available')}{offline > 0 && t(' · ', ' · ') + offline + t(' 个离线快照', ' offline snapshots')}{failed > 0 && t(' · 部分目录不可用', ' · Some catalogs unavailable')}</span></summary>
    <p className="projects-note">{t('汇总各节点授权的项目。离线快照只供查看，重连后刷新；已确认撤权的来源会被移除。读取失败不代表项目或会话已删除。', 'Projects are read from their owning nodes. Offline snapshots are read-only; refresh after reconnecting. Confirmed revocations remove the cached source. An unavailable catalog does not mean its projects or sessions were deleted.')}</p>
    <ul>{sources.map(source => <li key={source.id}>{source.id ? <Cable size={16} aria-hidden="true" /> : <Database size={16} aria-hidden="true" />}<div><strong>{source.id === ':local' && local ? local.name : source.id === ':routes' ? t('直连配置', 'Direct configuration') : source.id || t('Workspace 目录', 'Workspace catalog')}</strong><small>{source.error || (source.offline ? t('离线快照 · 版本 ', 'Offline snapshot · revision ') : t('已读取 · 版本 ', 'Available · revision ')) + source.revision}</small>{source.offline && source.syncedAt && <small>{t('上次同步：', 'Last synced: ')}<time dateTime={new Date(source.syncedAt).toISOString()}>{new Date(source.syncedAt).toLocaleString()}</time></small>}{source.warning && <small>{t('本机无法保存离线目录。', 'Offline catalog could not be saved on this device.')}</small>}</div><time dateTime={new Date(source.checked).toISOString()}>{new Date(source.checked).toLocaleTimeString()}</time></li>)}</ul>
  </details>;
  return <ProjectsView client={client} devices={targets} language={language} sources={<>{sourceDetails}<ProjectSync client={direct} owner={local?.id || devices[0]?.id} language={language} /></>} />;
}
