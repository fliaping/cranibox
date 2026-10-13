import { ArrowRight,Code2,Monitor } from 'lucide-react';
import { useState } from 'react';
import { Link } from 'react-router-dom';
import { Button,Card,Icon,Status } from '../components';
import { t,useWorkspace } from '../workspace';

import { AgentCard } from './shared';

export function Onboarding() {
  const { snapshot, state } = useWorkspace();
  const [step, setStep] = useState(0);
  if (!state.forceWelcome && snapshot?.bootstrap?.onboarding?.complete !== false) return null;
  const titles = [t('欢迎来到 Cranibox', 'Welcome to Cranibox'), t('选择你的 Agent', 'Choose your Agent'), t('打开工作区', 'Open your workspace'), t('图形桌面', 'Graphical desktop'), t('按需扩展', 'Extend when needed')];
  const descriptions = [t('用几个简单步骤了解工作环境。', 'Get to know your environment in a few simple steps.'), t('安装 Agent，并在终端完成登录。', 'Install an Agent and sign in from the terminal.'), t('项目存储在持久化目录中，容器重建后仍会保留。', 'Projects live in persistent storage and survive container rebuilds.'), t('通过浏览器访问完整的图形工作区。', 'Access the full graphical workspace in your browser.'), t('需要时再配置工具、技能与远程访问。', 'Configure tools, Skills, and remote access when you need them.')];
  return <Card className="onboarding"><div className="wizard-progress" aria-label={t('入门进度', 'Getting started progress')}>{titles.map((_, i) => <span className={i <= step ? 'active' : ''} key={i} aria-current={i === step ? 'step' : undefined}>{i + 1}</span>)}</div><span className="eyebrow">{t('快速开始', 'GETTING STARTED')} · {step + 1} / 5</span><h2>{titles[step]}</h2><p className="muted">{descriptions[step]}</p><div className="wizard-body">{step === 0 && <Status state={snapshot?.bootstrap?.foundation_ready ? 'ready' : 'needs-attention'} label={snapshot?.bootstrap?.foundation_ready ? t('基础环境已就绪', 'Foundation ready') : t('基础环境仍在准备', 'Foundation preparing')} />}{step === 1 && <div className="agent-grid">{snapshot?.agents?.map(agent => <AgentCard key={agent.id} agent={agent} />)}</div>}{step === 2 && <><code className="code-block">{snapshot?.workspace?.root}</code><Button action="openWorkspace" icon={Code2}>{t('打开工作区', 'Open workspace')}</Button></>}{step === 3 && <Button action={snapshot?.desktop?.installed ? 'openDesktop' : 'installDesktop'} icon={Monitor}>{t('查看桌面', 'Explore desktop')}</Button>}{step === 4 && <div className="actions"><Link to="/resources" className="button secondary">MCP & Skills</Link><Link to="/network" className="button secondary">{t('网络访问', 'Network access')}</Link></div>}</div><div className="wizard-actions"><Button action="completeOnboarding">{t('跳过向导', 'Skip guide')}</Button><div className="actions">{step > 0 && <Button onClick={() => setStep(step - 1)} local>{t('上一步', 'Back')}</Button>}{step < 4 ? <Button onClick={() => setStep(step + 1)} local tone="primary">{t('继续', 'Continue')}<Icon icon={ArrowRight} /></Button> : <Button action="completeOnboarding" tone="primary">{t('完成向导', 'Finish guide')}</Button>}</div></div></Card>;
}
