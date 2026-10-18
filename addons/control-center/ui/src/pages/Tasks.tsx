import { useMemo } from 'react';
import { useNavigate } from 'react-router-dom';
import { DeviceTasksView } from '../tasks/DeviceTasksView';
import type { DirectClient } from '../devices/DirectDevicesView';
import { request } from '../workspace';
export function Tasks() {
  const navigate = useNavigate();
  const client = useMemo<DirectClient>(() => command => request('/api/peers', command), []);
  return <DeviceTasksView client={client} language={window.workspaceWeb.language} onDevices={() => navigate('/nodes')} onMessages={() => navigate('/messages')} />;
}
