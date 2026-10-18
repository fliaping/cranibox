import { useEffect, useRef, useState } from 'react';
import type { DirectClient } from './DirectDevicesView';

export function BrowserTargetPanel({ client, node, language }: { client: DirectClient; node: string; language: string }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const [busy, setBusy] = useState(false), [stopping, setStopping] = useState(false), [error, setError] = useState('');
  const [status, setStatus] = useState<{ paused: boolean; backend_available?: boolean; control_busy?: boolean }>();
  const serial = useRef(0), mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; ++serial.current; }; }, []);
  async function call(operation: 'status' | 'emergency_stop') {
    const current = ++serial.current; setError('');
    if (operation === 'status') setBusy(true); else setStopping(true);
    try {
      const value = await client({ operation: 'browser', node, command: { operation } }) as { node: string; result: typeof status };
      if (value.node !== node) throw Error(t('响应设备身份不匹配。', 'Response device identity does not match.'));
      if (mounted.current && current === serial.current) setStatus(value.result);
    } catch (error) { if (mounted.current && current === serial.current) setError(error instanceof Error ? error.message : String(error)); }
    finally { if (mounted.current) { if (operation === 'status') setBusy(false); else setStopping(false); } }
  }
  return <section className="direct-grant-form" aria-label={t('目标浏览器', 'Target browser')}><h4>{t('目标浏览器', 'Target browser')}</h4>
    <p className="direct-note">{t('Agent 在调用设备运行以下 MCP 命令，使用已保存的直连凭据操作目标浏览器。网页只提供状态和停止控制。', 'Run this MCP command on the calling device to control the target browser using its saved direct credential. This page provides status and emergency stop.')}</p>
    <code className="browser-command">cranibox-node peer call --node {node} browser-mcp</code>
    <p role="status">{status ? status.paused ? t('已停止，请在目标设备恢复。', 'Stopped. Resume on the target device.') : !status.backend_available ? t('目标浏览器不可用。', 'Target browser unavailable.') : status.control_busy ? t('浏览器正在被远程控制。', 'Browser has a remote controller.') : t('目标浏览器就绪。', 'Target browser ready.') : t('尚未检查目标浏览器。', 'Target browser not checked.')}</p>
    <div className="direct-actions"><button disabled={busy || stopping} onClick={() => void call('status')}>{busy ? t('检查中…', 'Checking…') : t('检查目标浏览器', 'Check target browser')}</button><button disabled={stopping} onClick={() => void call('emergency_stop')}>{stopping ? t('正在停止…', 'Stopping…') : t('停止目标浏览器操作', 'Stop target browser control')}</button></div>
    {error && <p role="alert" className="direct-error">{error}</p>}
  </section>;
}
