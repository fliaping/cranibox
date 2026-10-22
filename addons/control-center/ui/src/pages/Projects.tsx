import { useMemo } from 'react';
import type { ProjectClient } from '../projects/ProjectsView';
import { FederatedProjectsView } from '../projects/FederatedProjectsView';
import type { DirectClient } from '../devices/DirectDevicesView';
import { request, t, useWorkspace } from '../workspace';
import { useTerminalWindow } from '../terminal/TerminalWindowContext';

export function Projects() {
  const { snapshot } = useWorkspace();
  const { openTerminal } = useTerminalWindow();
  const client = useMemo<ProjectClient>(() => ({
    projects: command => request('/api/projects', command),
    terminals: (node, command) => request('/api/terminal/sessions', { ...command, node }),
    open: (node, session) => openTerminal({ node: node || '', session: session.id }),
  }), [openTerminal]);
  const direct = useMemo<DirectClient>(() => command => request('/api/peers', command), []);
  return <FederatedProjectsView client={client} direct={direct} language={window.workspaceWeb.language} devices={[
    { id: snapshot?.local_node?.id || '', name: t('Workspace 主机', 'Workspace host') + (snapshot?.access?.hostname ? ' · ' + snapshot.access.hostname : ''), online: true, terminal_sessions: true, can_copy: !!snapshot?.local_node?.id },
  ]} />;
}
