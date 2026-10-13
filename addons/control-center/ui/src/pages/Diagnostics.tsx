import { RefreshCw,Terminal } from 'lucide-react';
import { Button,Card,Facts,PageHead,Status } from '../components';
import { t,useWorkspace } from '../workspace';

import { CheckList } from './shared';

export function Diagnostics() { const { snapshot } = useWorkspace(); return <><PageHead title={t('环境诊断', 'Diagnostics')} description={t('检查工作区基础环境和可选能力，查看操作输出。', 'Check the workspace foundation and optional capabilities, and review operation output.')} action={<Button action="showOutput" icon={Terminal}>{t('操作日志', 'Activity log')}</Button>} /><div className="grid two"><Card title={t('功能组件', 'Capabilities')}><CheckList items={snapshot?.capabilities || []} /></Card><Card title={t('基础环境检查', 'Environment checks')} action={<Status state={snapshot?.doctor?.healthy ? 'ready' : 'needs-attention'} />}><CheckList items={snapshot?.doctor?.checks || []} /></Card></div><Card title={t('应用维护', 'Application maintenance')} subtitle={t('更新应用源码并在日志中查看结果。', 'Update application source and review the result in the activity log.')}><Facts items={[[t('运行版本', 'Running revision'), <code>{snapshot?.revision}</code>], [t('工作区', 'Workspace'), <code>{snapshot?.workspace?.root}</code>]]} /><Button action="updateSource" icon={RefreshCw}>{t('更新应用源码', 'Update application source')}</Button></Card></>; }
