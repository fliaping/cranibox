import { Channel, invoke } from '@tauri-apps/api/core';
import { useCallback, useEffect, useState, type ReactNode } from 'react';
import { directTarget, localTarget, terminalTarget, TerminalEnvironmentProvider, type TerminalEnvironment } from '../../../addons/control-center/ui/src/terminal/TerminalEnvironment';
import type { DirectCatalog } from '../../../addons/control-center/ui/src/devices/DirectDevicesView';

const t = (cn: string, _en: string) => cn;
type Event = { type: 'ready' | 'output' | 'error' | 'transport_error' | 'attachment_closed' | 'closed'; data?: string; message?: string };
const connect: TerminalEnvironment['connect'] = async (options, events) => {
  const target = terminalTarget(options.node);
  if (!target.node) throw Error('请先选择设备');
  let attachment = '', closed = false, sequence = Promise.resolve(), queued = 0;
  const finish = () => { if (closed) return; closed = true; if (attachment) void invoke('terminal_close', { id: attachment }).catch(() => {}); events.closed(); };
  const channel = new Channel<Event>(event => {
    if (closed) return;
    if (event.type === 'ready') events.ready();
    else if (event.type === 'output') { try { events.output(Uint8Array.from(atob(event.data || ''), c => c.charCodeAt(0))); } catch { events.error('终端输出格式异常', true); finish(); } }
    else if (event.type === 'error' || event.type === 'transport_error') events.error(event.message || '终端连接中断', event.type === 'error');
    else if (event.type === 'attachment_closed') events.ended();
    else if (event.type === 'closed') finish();
  });
  attachment = await invoke<string>('terminal_open', { options: { ...options, node: target.node, direct: 'direct' in target, local: 'local' in target }, events: channel });
  if (closed) void invoke('terminal_close', { id: attachment }).catch(() => {});
  return {
    close: finish,
    send: input => {
      if (closed) return;
      if (++queued > 32) { queued--; events.error('输入过快，连接已脱离。请接回后检查命令状态。', true); finish(); return; }
      sequence = sequence.then(async () => { if (!closed) await invoke('terminal_input', { id: attachment, input }); }).catch(error => { if (!closed) { events.error(String(error), true); finish(); } }).finally(() => { queued--; });
    },
  };
};
export function DesktopTerminalEnvironment({ children, devices, local }: { children: ReactNode; devices: { id: string; aliases?: string[]; name?: string; online: boolean; terminal_sessions?: boolean }[]; local?: { id: string; online: boolean } }) {
  const [routes, setRoutes] = useState<DirectCatalog['routes']>([]);
  useEffect(() => {
    let alive = true, serial = 0;
    const refresh = async () => { const current = ++serial; try { const value = await invoke<DirectCatalog>('direct_peers', { command: { operation: 'list' } }); if (alive && serial === current) setRoutes(value?.routes || []); } catch { /* Device panel presents configuration errors. */ } };
    void refresh(); window.addEventListener('cranibox:peers-changed', refresh); window.addEventListener('focus', refresh);
    return () => { alive = false; ++serial; window.removeEventListener('cranibox:peers-changed', refresh); window.removeEventListener('focus', refresh); };
  }, []);
  const manage = useCallback<TerminalEnvironment['manage']>((target, command) => {
    const node = terminalTarget(target); if (!node.node) return Promise.reject(Error('请先选择设备'));
    return 'local' in node
      ? invoke('local_sessions', { command }) : 'direct' in node
      ? invoke('direct_peers', { command: { operation: 'sessions', node: node.node, command } })
      : invoke('terminal_sessions', { node: node.node, command });
  }, []);
  const targets = [
    ...(local ? [{ id: localTarget(local.id), name: '这台设备', available: local.online }] : []),
    ...devices.map(device => ({ id: device.id, aliases: device.aliases, name: (device.name || device.id) + ' · 网关', available: device.online && device.terminal_sessions === true })),
    ...routes.map(route => ({ id: directTarget(route.node), name: route.node + ' · 直连', available: true })),
  ];
  return <TerminalEnvironmentProvider value={{ t, available: true, includeLocal: false, targets, manage, connect }}>{children}</TerminalEnvironmentProvider>;
}
