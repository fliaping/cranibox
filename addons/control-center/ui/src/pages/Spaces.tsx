import { useCallback } from 'react';
import { useNavigate } from 'react-router-dom';
import type { DirectClient } from '../devices/DirectDevicesView';
import { SpacesView } from '../spaces/SpacesView';
import { inShell, shellCall } from '../app/shell';
import { request } from '../workspace';
import { directTarget } from '../terminal/TerminalEnvironment';
import { useTerminalWindow } from '../terminal/TerminalWindowContext';

export function Spaces() {
  const navigate = useNavigate(), { openTerminal } = useTerminalWindow();
  const client = useCallback((command: Record<string, unknown>) => request('/api/spaces', command), []);
  const direct = useCallback<DirectClient>(command => request('/api/peers', command, { background: true }), []);
  const openApplication = useCallback(async (id: string, application: 'desktop' | 'code') => {
    // The desktop app's webview opens no new windows; its browser takes the one-time link.
    if (inShell()) { const value = await request('/api/spaces/application', { id, application }); await shellCall('open_external', { url: value.url }); return; }
    const tab = window.open('about:blank', '_blank');
    if (!tab) throw new Error(document.documentElement.lang.startsWith('zh') ? '请允许此页面打开新窗口，然后重试。' : 'Allow pop-ups for this page and retry.');
    tab.opener = null;
    try { const value = await request('/api/spaces/application', { id, application }); tab.location.replace(value.url); }
    catch (error) { tab.close(); throw error; }
  }, []);
  return <SpacesView direct={direct} onDevices={() => navigate('/nodes')} client={client} language={document.documentElement.lang} onOpenApplication={openApplication} onOpenTerminal={node => openTerminal({ node: directTarget(node), create: true })} onProjects={() => navigate('/projects')} />;
}
