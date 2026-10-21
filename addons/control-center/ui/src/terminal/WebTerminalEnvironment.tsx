import { useCallback, useEffect, useState, type ReactNode } from 'react';
import { request, t, useWorkspace } from '../workspace';
import type { DirectCatalog } from '../devices/DirectDevicesView';
import { directTarget, terminalTarget, TerminalEnvironmentProvider, type TerminalEnvironment } from './TerminalEnvironment';

const connect: TerminalEnvironment['connect'] = async (options, events) => {
  const { ticket } = await request('/api/terminal/ticket', { ...options, ...terminalTarget(options.node) });
  const endpoint = new URL('/api/terminal/ws', location.href);
  endpoint.protocol = location.protocol === 'https:' ? 'wss:' : 'ws:'; endpoint.searchParams.set('ticket', ticket);
  const socket = new WebSocket(endpoint); socket.binaryType = 'arraybuffer';
  let ready = false;
  const markReady = () => { if (!ready) { ready = true; events.ready(); } };
  socket.onmessage = event => {
    if (event.data instanceof ArrayBuffer) { markReady(); events.output(new Uint8Array(event.data)); return; }
    try {
      const value = JSON.parse(event.data);
      if (value.type === 'ready') markReady();
      else if (value.type === 'error') { events.error(value.message, true); socket.close(); }
      else if (value.type === 'attachment_closed') { events.ended(); socket.close(); }
      else throw Error('Invalid terminal response');
    } catch { events.error(t('终端响应异常，请重新接回。', 'Invalid terminal response. Reattach to continue.'), true); socket.close(); }
  };
  socket.onerror = () => events.error(t('无法连接终端，请检查节点后重试。', 'Unable to connect. Check the node and retry.'), false);
  socket.onclose = () => events.closed();
  return { close: () => socket.close(), send: input => { if (socket.readyState === WebSocket.OPEN) socket.send(JSON.stringify(input)); } };
};
export function WebTerminalEnvironment({ children }: { children: ReactNode }) {
  const { snapshot, available } = useWorkspace();
  // Without the container workspace the node serves terminals itself.
  const ready = !!snapshot || available === false;
  const [routes, setRoutes] = useState<DirectCatalog['routes']>([]);
  useEffect(() => {
    let alive = true, serial = 0;
    const refresh = async () => { const current = ++serial; try { const value = await request('/api/peers', { operation: 'list' }) as DirectCatalog; if (alive && serial === current) setRoutes(value.routes || []); } catch { /* Workspace displays session/bootstrap errors. */ } };
    if (ready) void refresh();
    window.addEventListener('cranibox:peers-changed', refresh); window.addEventListener('focus', refresh);
    return () => { alive = false; ++serial; window.removeEventListener('cranibox:peers-changed', refresh); window.removeEventListener('focus', refresh); };
  }, [ready]);
  const manage = useCallback<TerminalEnvironment['manage']>((node, command) => request('/api/terminal/sessions', { ...command, ...terminalTarget(node) }), []);
  const targets = [
    ...(snapshot?.local_node ? [{ id: snapshot.local_node.id, name: t('Workspace 主机', 'Workspace host') + (snapshot.access?.hostname ? ' · ' + snapshot.access.hostname : ''), available: snapshot.local_node.online }] : []),
    ...(snapshot?.remote_nodes?.nodes || []).map(node => ({ id: node.id, aliases: node.aliases, name: node.name || node.id, available: !!(node.online && node.terminal_sessions) })),
    ...routes.map(route => ({ id: directTarget(route.node), name: route.node + t(' · 直连', ' · Direct'), available: true })),
  ];
  return <TerminalEnvironmentProvider value={{ t, available: ready, includeLocal: !snapshot?.local_node, localNode: snapshot?.local_node?.id, targets, manage, connect }}>{children}</TerminalEnvironmentProvider>;
}
