import { Code2,Monitor,MonitorOff,Play,Power,RefreshCw,Shield,Square } from 'lucide-react';
import { Button,Card,Empty,Facts,Icon,PageHead,Row,Status } from '../components';
import { t,useWorkspace } from '../workspace';

import { labelAgent } from './shared';
import type { DesktopPower } from '../types';
import { WebRemoteDesktops } from '../desktop/WebRemoteDesktops';

const megabytes = (value?: number | null) => !value ? '' : value >= 1024 ? (value / 1024).toFixed(1).replace(/\.0$/, '') + ' GB' : Math.round(value) + ' MB';

/** 桌面开关：关闭后停止图形桌面、桌面浏览器和画面串流，Agent 与终端照常运行。 */
function DesktopPowerCard({ power }: { power: DesktopPower }) {
  const on = power.state === 'on';
  const busy = power.phase === 'starting' || power.phase === 'stopping';
  const label = { running: t('已开启', 'On'), starting: t('正在开启…', 'Starting…'), stopping: t('正在关闭…', 'Stopping…'), stopped: t('已关闭', 'Off') }[power.phase] || power.phase;
  const memory = on
    ? (power.desktop_memory_mb ? t('桌面当前约占用 ' + megabytes(power.desktop_memory_mb) + ' 内存，关闭后会释放。', 'The desktop uses about ' + megabytes(power.desktop_memory_mb) + ' of memory; turning it off frees it.') : t('关闭桌面可以释放它占用的内存。', 'Turning the desktop off frees its memory.'))
    : t('已为你节省约 ' + megabytes(power.memory_hint_mb) + ' 内存。Agent、终端和其他服务照常运行。', 'Saving about ' + megabytes(power.memory_hint_mb) + ' of memory. Agents, terminals and other services keep running.');
  return <Card className="desktop-power" title={<span className="desktop-power-title"><Icon icon={on ? Monitor : MonitorOff} />{t('桌面', 'Desktop')}</span>} action={<Status state={power.phase === 'running' ? 'running' : power.phase === 'stopped' ? 'stopped' : 'needs-attention'} label={label} />}>
    <p className="muted">{memory}</p>
    {power.container_memory_mb ? <Facts items={[[t('容器内存', 'Container memory'), megabytes(power.container_memory_mb)], [t('启动时', 'At startup'), power.boot_state === 'off' ? t('不开启桌面', 'Desktop off') : t('开启桌面', 'Desktop on')]]} /> : null}
    {power.error && <p className="callout warning-text">{power.error}</p>}
    <div className="actions">{on
      ? <Button action="desktopPower" extra={{ action: 'off' }} icon={Power} disabled={busy}>{t('关闭桌面', 'Turn off desktop')}</Button>
      : <Button action="desktopPower" extra={{ action: 'on' }} icon={Power} tone="primary" disabled={busy}>{t('开启桌面', 'Turn on desktop')}</Button>}</div>
    {on && <p className="muted">{t('关闭时会关掉桌面里打开的窗口和桌面浏览器，未保存的内容会丢失。', 'Turning off closes open desktop windows and the desktop browser; unsaved work is lost.')}</p>}
  </Card>;
}

export function Desktop() {
  const { snapshot } = useWorkspace();
  const desktop = snapshot?.desktop || {}, cu = desktop.computer_use || {};
  const ready = desktop.runtime_available;
  // Without the container workspace only the cross-device viewer applies.
  if (!snapshot) return <><PageHead title={t('远程桌面', 'Remote desktops')} description={t('查看并操作你其他设备的屏幕，或共享这台设备的桌面。', 'View and control the screens of your other devices, or share this one.')} /><WebRemoteDesktops /></>;
  return <><PageHead title={t('远程桌面', 'Remote desktop')} description={t('在浏览器中访问图形工作区，管理 Agent 的桌面控制。', 'Access the graphical workspace in your browser and manage Agent desktop control.')} action={<Button icon={Monitor} action={ready ? 'openDesktop' : 'installDesktop'} tone="primary">{ready ? t('打开桌面', 'Open desktop') : t('安装桌面集成', 'Install desktop integration')}</Button>} /><WebRemoteDesktops />{desktop.power && <DesktopPowerCard power={desktop.power} />}<div className="grid two"><Card title="Selkies Desktop" action={<Status state={desktop.running ? 'running' : 'stopped'} />}><Facts items={[[t('镜像运行时', 'Image runtime'), <Status state={desktop.runtime_available ? 'ready' : 'missing'} />], [t('编辑器集成', 'Editor integration'), <Status state={desktop.installed ? 'ready' : 'missing'} />], [t('桌面服务', 'Desktop service'), <code>{desktop.service || 'svc-selkies'}</code>], [t('本地地址', 'Local address'), <code>{desktop.local_url || snapshot?.access?.desktop_local}</code>]]} /><div className="actions"><Button action="copyDesktopTunnel" icon={Code2}>{t('SSH 隧道', 'SSH tunnel')}</Button>{desktop.runtime_available && <Button action="serviceAction" extra={{ id: desktop.service || 'svc-selkies', kind: 's6', action: 'restart' }} icon={RefreshCw}>{t('重启桌面', 'Restart desktop')}</Button>}</div></Card><Card title="Computer Use" action={<Status state={cu.paused ? 'paused' : cu.state || 'unknown'} />}><p className="muted">{t('网页任务优先使用托管浏览器 CDP；原生应用使用桌面控制。', 'Use managed browser CDP for web tasks and desktop control for native applications.')}</p><Facts items={[[t('控制后端', 'Control backend'), cu.backend || 'PixelFlux'], [t('后端可用', 'Backend available'), <Status state={cu.backend_available ? 'ready' : 'needs-attention'} />], [t('屏幕分辨率', 'Resolution'), cu.resolution?.width ? cu.resolution.width + ' × ' + cu.resolution.height : t('未知', 'Unknown')]]} /><div className="actions">{!cu.installed ? <Button action="setupComputerUse">{t('配置 Computer Use', 'Set up Computer Use')}</Button> : !cu.running ? <Button action="desktopControl" extra={{ action: 'start' }} icon={Play}>{t('启动控制', 'Start control')}</Button> : cu.paused ? <Button action="desktopControl" extra={{ action: 'resume' }} icon={Play}>{t('恢复控制', 'Resume control')}</Button> : <Button action="desktopControl" extra={{ action: 'stop' }} icon={Square}>{t('停止服务', 'Stop service')}</Button>}<Button action="desktopEmergencyStop" tone="danger" icon={Square}>{t('紧急停止输入', 'Emergency stop input')}</Button></div>{cu.backend_diagnostic?.detail && <p className="callout warning-text">{cu.backend_diagnostic.detail}</p>}</Card></div><div className="grid two"><Card title={t('活动控制会话', 'Active control sessions')} action={<span className="tag">{cu.active_sessions?.length || 0}</span>}>{cu.active_sessions?.length ? cu.active_sessions.map(session => <Row key={session.id} icon={Monitor} title={session.id} detail={(session.last_action || '') + ' · ' + (session.idle_seconds || 0) + 's'}><Status state="active" /></Row>) : <Empty title={t('暂无活动会话', 'No active sessions')} detail={t('Agent 连接桌面控制后会显示在这里。', 'Agent desktop control sessions appear here once connected.')} />}</Card><Card title={t('控制端点与权限', 'Control endpoints & access')}><Facts items={[[t('Bridge', 'Bridge'), <code>{cu.bridge_url}</code>], ['PixelFlux', <code>{cu.native_bind || cu.native_url}</code>], [t('已连接 Agent', 'Configured Agents'), (cu.mcp_agents || []).map(labelAgent).join(' · ')]]} /><p className="callout"><Icon icon={Shield} />{t('桌面控制可读取完整屏幕并发送鼠标和键盘输入，仅供可信容器内 Agent 使用。', 'Desktop control can read the full screen and send mouse and keyboard input. Use it only with trusted container Agents.')}</p></Card></div></>;
}
