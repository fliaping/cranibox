import { useCallback } from 'react';
import { request, useWorkspace } from '../workspace';
import type { DirectCommand } from '../devices/DirectDevicesView';
import type { DesktopConnector } from './transport';
import { RemoteDesktopsView } from './RemoteDesktopsView';
import type { DesktopServiceCommand } from './DesktopServicePanel';

export const webDesktopConnector: DesktopConnector = async (target, signal) => {
  signal?.throwIfAborted();
  // Read the response even after navigation so a just-issued lease can be released.
  const { ticket } = await request('/api/desktop/ticket', target);
  const release = () => { void request('/api/desktop/cancel', { ticket }).catch(() => {}); };
  if (signal?.aborted) { release(); signal.throwIfAborted(); }
  signal?.addEventListener('abort', release, { once: true });
  const endpoint = new URL('/api/desktop/ws', location.href);
  endpoint.protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
  endpoint.searchParams.set('ticket', ticket);
  return endpoint.href;
};
export function WebRemoteDesktops() {
  const { snapshot } = useWorkspace();
  const client = useCallback((command: DirectCommand) => request('/api/peers', command), []);
  const serviceClient = useCallback((command: DesktopServiceCommand) => request('/api/desktop/service', command), []);
  return <RemoteDesktopsView client={client} serviceClient={serviceClient} connect={webDesktopConnector} language={document.documentElement.lang} onAccess={() => { location.hash = '/nodes'; }} local={snapshot?.local_node ? { id: snapshot.local_node.id, name: snapshot.access?.hostname || 'Workspace' } : undefined} />;
}
