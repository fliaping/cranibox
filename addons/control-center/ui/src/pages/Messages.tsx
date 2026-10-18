import { useMemo } from 'react';
import { useNavigate } from 'react-router-dom';
import { DeviceMessagesView } from '../messages/DeviceMessagesView';
import type { DirectClient } from '../devices/DirectDevicesView';
import { request } from '../workspace';

export function Messages() {
  const navigate = useNavigate();
  const client = useMemo<DirectClient>(() => command => request('/api/peers', command), []);
  return <DeviceMessagesView client={client} language={window.workspaceWeb.language} onDevices={() => navigate('/nodes')} />;
}
