import { Terminal } from 'lucide-react';
import { Button,Card,Empty,Icon,PageHead } from '../components';
import { t,useWorkspace } from '../workspace';

import { AgentCard } from './shared';

export function Agents() { const { snapshot } = useWorkspace(); return <><PageHead title="Agents" description={t('安装和启动 Agent，在你的持久化工作区中协作。', 'Install and launch Agents in your persistent workspace.')} /><div className="agent-grid">{snapshot?.agents?.map(agent => <AgentCard key={agent.id} agent={agent} />)}</div>{!snapshot?.agents?.length && <Card><Empty title={t('尚未发现 Agent', 'No Agents found')} detail={t('刷新状态，检查安装情况。', 'Refresh the status to check installations.')} action={<Button action="refresh">{t('刷新', 'Refresh')}</Button>} /></Card>}<p className="page-note"><Icon icon={Terminal} />{t('交互式 Agent 会在工作区终端运行。首次使用请按 Agent 提示完成登录。', 'Interactive Agents run in the workspace terminal. Follow the Agent prompts to sign in on first use.')}</p></>; }
