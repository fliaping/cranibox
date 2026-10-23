import { useCallback } from 'react';
import { request, useWorkspace } from '../workspace';
import type { DirectCommand } from '../devices/DirectDevicesView';
import { RemoteDesktopsView } from './RemoteDesktopsView';
import type { DesktopServiceCommand } from './DesktopServicePanel';

export function WebRemoteDesktops() {
  const { snapshot } = useWorkspace();
  const client = useCallback((command: DirectCommand) => request('/api/peers', command), []);
  const serviceClient = useCallback((command: DesktopServiceCommand) => request('/api/desktop/service', command), []);
  return <RemoteDesktopsView client={client} serviceClient={serviceClient} language={document.documentElement.lang} onAccess={() => { location.hash = '/nodes'; }} local={snapshot?.local_node ? { id: snapshot.local_node.id, name: snapshot.access?.hostname || 'Workspace' } : undefined} />;
}
