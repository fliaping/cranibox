import { Server, Terminal } from 'lucide-react';
import { useCallback } from 'react';
import { Link } from 'react-router-dom';
import { DirectDevicesView, type DirectCommand } from '../devices/DirectDevicesView';
import { NetworkProvidersView, type NetworkCommand } from '../devices/NetworkProvidersView';
import { PairingCard, PeerTransport } from '../app/Connections';
import { Button, Card, PageHead, Row, Status } from '../components';
import { request, t, useWorkspace } from '../workspace';
import { useTerminalWindow } from '../terminal/TerminalWindowContext';
import { directTarget } from '../terminal/TerminalEnvironment';

/** Peer transport details and optional networks; `embedded` inside the Network page. */
export function Nodes({ embedded = false }: { embedded?: boolean }) {
  const { snapshot } = useWorkspace();
  const { openTerminal } = useTerminalWindow();
  const directClient = useCallback((command: DirectCommand) => request('/api/peers', command), []);
  const networkClient = useCallback((command: NetworkCommand) => request('/api/network', command), []);
  return <>{!embedded && <><PageHead title={t('设备与网络', 'Devices & network')} description={t('配对新设备；设备之间经 iroh 对等直连，也可以加入 EasyTier 或 Tailscale 网络。', 'Pair new devices. Devices connect peer to peer over iroh, and can also join an EasyTier or Tailscale network.')} action={snapshot ? <Link className="button secondary" to="/history">{t('任务历史', 'Operation history')}</Link> : undefined} />
    <PairingCard /></>}
    {snapshot?.local_node && <Card className="node-host" title={t('当前接入节点', 'Connected node')} action={<Status state="connected" label={t('在线', 'Online')} />}><Row icon={Server} title={snapshot.access?.hostname || t('Workspace 主机', 'Workspace host')} detail={snapshot.local_node.id}><Button local icon={Terminal} onClick={() => openTerminal({ node: snapshot.local_node!.id, create: true })}>{t('打开节点终端', 'Open node terminal')}</Button></Row><p className="muted">{t('浏览器通过这台设备访问工作空间。命令在这台设备执行；其他设备需获得授权后接入。', 'Your browser accesses the workspace through this device. Commands run here; other devices need permission to connect.')}</p></Card>}
    <PeerTransport />
    <NetworkProvidersView client={networkClient} language={window.workspaceWeb.language} host="server" />
    <DirectDevicesView client={directClient} language={window.workspaceWeb.language} onOpenTerminal={node => openTerminal({ node: directTarget(node), create: true })} /></>;
}
