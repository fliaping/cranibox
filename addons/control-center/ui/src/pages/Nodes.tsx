import { useCallback } from 'react';
import { DirectDevicesView, type DirectCommand } from '../devices/DirectDevicesView';
import { NetworkProvidersView, type NetworkCommand } from '../devices/NetworkProvidersView';
import { request } from '../workspace';
import { useTerminalWindow } from '../terminal/TerminalWindowContext';
import { directTarget } from '../terminal/TerminalEnvironment';

/** EasyTier and Tailscale, for the Network settings page. */
export function OtherNetworks() {
  const networkClient = useCallback((command: NetworkCommand) => request('/api/network', command), []);
  return <NetworkProvidersView client={networkClient} language={window.workspaceWeb.language} host="server" />;
}
/** Direct routes and the listener, for troubleshooting. */
export function DirectRoutes() {
  const { openTerminal } = useTerminalWindow();
  const directClient = useCallback((command: DirectCommand) => request('/api/peers', command), []);
  return <DirectDevicesView client={directClient} language={window.workspaceWeb.language} onOpenTerminal={node => openTerminal({ node: directTarget(node), create: true })} />;
}
