import { useEffect, useRef, useState } from 'react';
import { Bot, Copy, RefreshCw } from 'lucide-react';
import type { DirectClient } from '../devices/DirectDevicesView';
import { agentCatalog, agentRegistrationInstructions, currentAgent, type AgentReport as Agent, type AgentCatalog as Catalog, type AgentSnapshot as Snapshot } from './agent-session-catalog';
import './agent-sessions.css';

export type AgentCommand = { operation: 'list' } | { operation: 'register'; id: string; name: string; harness: string } | { operation: 'heartbeat'; id: string; revision: number; status: 'ready' | 'busy' | 'waiting' } | { operation: 'end'; id: string; revision: number };

/** Self-reported leases, deliberately separate from process or task execution status. */
export function AgentSessions({ client, node, language, paused, selected = '', onSelect, onValidChange, disabled = false, purpose = 'task' }: { client: DirectClient; node?: string; language: string; paused: boolean; selected?: string; onSelect?: (id: string) => void; onValidChange?: (valid: boolean) => void; disabled?: boolean; purpose?: 'task' | 'message' }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const [snapshot, setSnapshot] = useState<Snapshot>(), [error, setError] = useState(''), [reading, setReading] = useState(false), [notice, setNotice] = useState('');
  const [clock, setClock] = useState(() => performance.now()), [refresh, setRefresh] = useState(0);
  const read = useRef<() => void>(() => undefined);
  useEffect(() => {
    let disposed = false, active = false;
    const load = async () => {
      if (active || document.hidden) return;
      active = true; setReading(true);
      try {
        const value = await client({ operation: 'agent_sessions', node: node || null, command: { operation: 'list' } }) as Catalog;
        const catalog = agentCatalog(value, node || value?.owner, t('会话目录响应无效，请更新设备服务。', 'Invalid session catalog. Update the device service.'));
        if (!disposed) { const at = performance.now(); setSnapshot({ catalog, received: at }); setClock(at); setError(''); }
      } catch (error) { if (!disposed) { setSnapshot(undefined); setError(error instanceof Error ? error.message : String(error)); } }
      finally { active = false; if (!disposed) setReading(false); }
    };
    read.current = () => void load(); void load();
    const timer = paused ? undefined : setInterval(() => void load(), 5000);
    const resume = () => { if (!paused) void load(); };
    document.addEventListener('visibilitychange', resume);
    return () => { disposed = true; clearInterval(timer); document.removeEventListener('visibilitychange', resume); };
  }, [client, node, paused, refresh, language]);
  useEffect(() => { const timer = setInterval(() => setClock(performance.now()), 1000); return () => clearInterval(timer); }, []);
  const sessions = snapshot?.catalog.sessions || [];
  const active = (agent: Agent) => !!snapshot && currentAgent(agent, snapshot, clock);
  const valid = !selected || !error && sessions.some(agent => agent.id === selected && active(agent));
  useEffect(() => { onValidChange?.(valid); }, [valid, selected, node, onValidChange]);
  const status = (agent: Agent) => agent.status === 'ended' ? t('已结束登记', 'Registration ended') : !active(agent) ? t('登记已过期', 'Report expired') : ({ ready: t('报告空闲', 'Reports ready'), busy: t('报告忙碌', 'Reports busy'), waiting: t('报告等待中', 'Reports waiting') } as Record<string, string>)[agent.status];
  async function copyInstructions() {
    try {
      await navigator.clipboard.writeText(agentRegistrationInstructions(language));
      setNotice(t('已复制本机 Agent 登记说明。', 'Local Agent registration instructions copied.'));
    } catch { setNotice(t('复制失败。请让本机 Agent 使用 device_agent_sessions 的 register 登记，并每两分钟 heartbeat。', 'Copy failed. Ask the local Agent to register through device_agent_sessions and heartbeat every two minutes.')); }
  }
  if (onSelect) return <div className="agent-session-picker">
    <label>{purpose === 'message' ? t('收件 Agent', 'Recipient Agent') : t('执行 Agent', 'Target Agent')}<select disabled={disabled} value={selected} onChange={event => onSelect(event.target.value)}>
      <option value="">{purpose === 'message' ? t('设备收件箱 · 不指定 Agent', 'Device inbox · no Agent selected') : t('设备级委托 · 任一 Agent 领取', 'Device delegation · any Agent may claim')}</option>
      {selected && !sessions.some(agent => agent.id === selected) && <option value={selected} disabled>{t('未能确认所选会话', 'Selected session is unconfirmed')}</option>}
      {sessions.map(agent => <option key={agent.id} value={agent.id} disabled={!!error || !active(agent)}>{agent.name} · {agent.harness} · {status(agent)}</option>)}
    </select></label>
    <div className="direct-actions"><button type="button" disabled={reading} onClick={() => read.current()}><RefreshCw size={15} aria-hidden="true" />{t('刷新 Agent', 'Refresh Agents')}</button><small>{error ? t('当前目录未确认', 'Current directory unconfirmed') : reading && !snapshot ? t('正在读取会话…', 'Reading sessions…') : sessions.filter(active).length + ' ' + t('条有效登记', 'current registrations')}</small></div>
    {error && <p className="direct-error" role="alert">{error}</p>}
    {!valid && <p className="direct-error">{purpose === 'message' ? t('所选会话未确认或已过期。请刷新或等待续期；取消回复后可另发设备消息。', 'The selected session is unconfirmed or expired. Refresh or wait for renewal. Cancel a reply to compose a separate device message.') : t('所选会话未确认或已过期。请刷新、等待其续期，或明确改选设备级委托。', 'The selected session is unconfirmed or expired. Refresh, wait for renewal, or explicitly choose device delegation.')}</p>}
    <p className="direct-note">{purpose === 'message' ? t('消息保留所选会话地址，不会自动唤醒 Agent。会话 UUID 用于筛选收件，不是访问权限；本机仍能读取完整收件箱。', 'The message keeps its session address and does not wake an Agent. Session UUIDs filter messages; they do not restrict access to the local mailbox.') : t('状态由 Agent 主动报告，5 分钟未续期即过期。定向委托仅允许该会话领取，不会自动启动 Agent。', 'Agents report their own state; reports expire after 5 minutes. A targeted task can only be claimed by that session and does not launch an Agent.')}</p>
  </div>;
  return <details className="agent-session-directory">
    <summary><Bot size={18} aria-hidden="true" />{t('本机 Agent 会话', 'Agent sessions on this device')}<span>{error ? t('当前目录未确认', 'Current directory unconfirmed') : reading && !snapshot ? t('正在读取会话…', 'Reading sessions…') : sessions.filter(active).length + ' ' + t('条有效登记', 'current registrations')}</span></summary>
    <div className="direct-actions"><button type="button" onClick={() => void copyInstructions()}><Copy size={16} aria-hidden="true" />{t('复制 Agent 登记说明', 'Copy Agent registration instructions')}</button><button type="button" disabled={reading} onClick={() => setRefresh(value => value + 1)}><RefreshCw size={16} aria-hidden="true" />{t('刷新会话', 'Refresh sessions')}</button></div>
    {notice && <p role="status">{notice}</p>}{error && <p className="direct-error" role="alert">{error}</p>}
    <p className="direct-note">{t('让本机 Agent 通过 MCP 登记后，授权设备即可按会话委托。登记状态不证明进程仍存活；关闭或过期的登记不会停止已经领取的工作。', 'After a local Agent registers through MCP, authorized devices can delegate to its session. A report does not prove process liveness. Ended or expired registrations do not stop claimed work.')}</p>
    {reading && !snapshot && <p role="status">{t('正在读取 Agent 会话…', 'Reading Agent sessions…')}</p>}
    {!reading && !error && !sessions.length && <p>{t('暂无登记会话。将登记说明交给已配置 MCP 的本机 Agent。', 'No registered sessions. Give the instructions to a local Agent with MCP configured.')}</p>}
    <div className="agent-session-list">{sessions.map(agent => <article className="agent-session-card" key={agent.id}><div><strong>{agent.name}</strong><span className={active(agent) && !error ? 'agent-session-current' : 'direct-note'}>{error ? t('当前状态未确认', 'Current report unconfirmed') : status(agent)}</span></div><p>{agent.harness}</p><code>{agent.id}</code><small>{t('最近报告：', 'Last report: ')}{new Date(agent.updated_at * 1000).toLocaleString(language)}</small></article>)}</div>
  </details>;
}
