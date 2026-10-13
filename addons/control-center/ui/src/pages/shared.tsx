import { ArrowUpRight,Bot,Plus,RefreshCw,Terminal } from 'lucide-react';
import { Button,Card,Icon,Row,Status } from '../components';
import type { Agent,Check as CheckType } from '../types';
import { t } from '../workspace';

export const labelAgent = (id: string) => ({ codex: 'Codex', 'claude-code': 'Claude Code', hermes: 'Hermes', 'deepseek-harness': 'DeepSeek', openclaw: 'OpenClaw' }[id] || id);
const agentDescriptions: Record<string, [string, string]> = {
  codex: ['在终端中完成编程与项目维护。', 'Code and maintain projects from your terminal.'],
  'claude-code': ['在项目上下文中协作编程。', 'Collaborate on code in your project context.'],
  hermes: ['管理 Agent 会话、工具与工作流。', 'Manage Agent sessions, tools, and workflows.'],
  'deepseek-harness': ['使用 DeepSeek 模型执行编程任务。', 'Run coding tasks with DeepSeek models.'],
  openclaw: ['通过 Gateway 运行持续的 Agent 服务。', 'Run persistent Agents through the Gateway.'],
};
export function AgentCard({ agent }: { agent: Agent }) {
  const description = agentDescriptions[agent.id];
  const web = agent.web_url || agent.web_path;
  return <Card className="agent-card"><div className="agent-top"><span className={'agent-avatar agent-' + agent.id}><Icon icon={Bot} /></span><div className="row-main"><h2>{agent.label}</h2><span className="muted">{agent.id}</span></div>{agent.selected && <span className="tag">{t('首选', 'Preferred')}</span>}</div><p className="agent-description">{description ? t(...description) : agent.label}</p><div className="agent-status"><Status state={agent.installed ? 'ready' : 'optional'} label={agent.installed ? t('已安装', 'Installed') : t('可安装', 'Available to install')} /><span className="muted">{agent.version || t('安装后显示版本', 'Version available after install')}</span></div><code className="code-block">{web || agent.launch || agent.id}</code><div className="actions">{agent.installed ? <Button icon={web ? ArrowUpRight : Terminal} action={agent.web_url ? 'openUrl' : agent.web_path ? 'openPath' : 'terminal'} extra={{ url: agent.web_url, path: agent.web_path, command: agent.launch, name: agent.label }} tone="primary">{web ? agent.web_label || t('打开 Web UI', 'Open Web UI') : t('启动 Agent', 'Launch Agent')}</Button> : <Button icon={Plus} action="installAgent" extra={{ id: agent.id }} tone="primary">{t('安装 Agent', 'Install Agent')}</Button>}{agent.installed && web && <Button icon={Terminal} action="terminal" extra={{ command: agent.launch, name: agent.label }} label={t('在终端中启动', 'Launch in a terminal')} />}{agent.id === 'openclaw' && agent.installed && <Button icon={RefreshCw} action="updateOpenClaw">{t('更新 Gateway', 'Update Gateway')}</Button>}</div></Card>;
}
export function CheckList({ items }: { items: CheckType[] }) { return <div className="list">{items.map(check => <Row key={check.id} title={check.label || check.id} detail={check.detail}><Status state={check.state} /></Row>)}</div>; }
