export const capabilities = ['messages', 'terminals', 'exec', 'projects', 'desktop', 'computer', 'browser', 'update', 'spaces', 'activity', 'llm', 'vault'] as const;
export type Capability = typeof capabilities[number];
export type Mode = 'off' | 'ask' | 'allow';
export type Modes = Record<Capability, Mode>;
export type Risk = 'read' | 'change' | 'danger';
/** What a remote device asked this device to do, described and redacted by this device. */
export type Operation = { kind: string; risk: Risk; summary: string; confirm?: boolean };
export type AccessRequest = { id: string; from: string; name: string; capability: Capability; created_at: number; expires_at: number; state: string; path: unknown; operation?: Operation };
export type Approval = { id: string; from: string; capability: Capability; scope?: 'session' | 'until'; expires_at: number | null; approved_at?: number };
export type AccessView = { peers: { node: string; name: string; capabilities: Modes }[]; pending: AccessRequest[]; approvals: Approval[]; sessions: { id: string; from: string; name: string; capability: Capability; basis?: string }[] };
export type AuditEntry = { index: number; at: number; event: string; from: string | null; name: string | null; capability: Capability | null; basis: string | null; result: string; operation?: Operation; status?: number; transport?: string };
export type AuditView = { entries: AuditEntry[]; verified: boolean; broken_index: number | null };
export const groups: { id: string; zh: string; en: string; caps: Capability[]; risk?: boolean; always?: boolean }[] = [
  { id: 'messages', zh: '消息与任务', en: 'Messages & tasks', caps: ['messages'] },
  { id: 'terminal', zh: '终端与命令', en: 'Terminal & commands', caps: ['terminals', 'exec', 'projects'], risk: true },
  { id: 'screen', zh: '屏幕', en: 'Screen', caps: ['desktop', 'computer'], risk: true },
  { id: 'browser', zh: '浏览器', en: 'Browser', caps: ['browser'], risk: true },
  { id: 'update', zh: '远程更新', en: 'Remote updates', caps: ['update'], risk: true, always: true },
  { id: 'spaces', zh: '虚拟空间', en: 'Virtual spaces', caps: ['spaces'], risk: true },
  { id: 'activity', zh: '活动与同步', en: 'Activity & sync', caps: ['activity'] },
  { id: 'models', zh: '模型与账号', en: 'Models & accounts', caps: ['llm', 'vault'], risk: true, always: true },
];

type T = (zh: string, en: string) => string;
const operationNames: Record<string, [string, string]> = {
  'exec.run': ['运行命令', 'Run a command'], 'exec.status': ['查看命令结果', 'Check a command'], 'exec.cancel': ['停止命令', 'Stop a command'],
  'files.write': ['写入文件', 'Write a file'], 'files.read': ['读取文件', 'Read a file'],
  'terminal.list': ['查看终端列表', 'List terminals'], 'terminal.create': ['新建终端', 'Open a terminal'], 'terminal.rename': ['重命名终端', 'Rename a terminal'], 'terminal.end': ['结束终端', 'End a terminal'], 'terminal.remove': ['删除终端', 'Remove a terminal'], 'terminal.attach': ['连接终端', 'Connect to a terminal'],
  'desktop.view': ['查看并控制屏幕', 'View and control the screen'],
  'projects.list': ['查看项目', 'List projects'], 'projects.sync': ['同步项目', 'Sync projects'], 'projects.copy_status': ['查看复制进度', 'Check a copy'], 'projects.export': ['导出项目文件', 'Export project files'], 'projects.import': ['导入项目文件', 'Import project files'], 'projects.transfer': ['传输项目', 'Transfer a project'], 'projects.bind': ['在项目中打开终端', 'Open a terminal in a project'],
  'spaces.list': ['查看虚拟空间', 'List virtual spaces'], 'spaces.stop': ['停止虚拟空间', 'Stop a virtual space'], 'spaces.resume': ['启动虚拟空间', 'Start a virtual space'],
  'update.check': ['检查更新', 'Check for updates'], 'update.install': ['安装更新', 'Install an update'], 'update.rollback': ['回滚更新', 'Roll back an update'],
  'activity.read': ['读取活动', 'Read activity'], 'messages.send': ['发送消息', 'Send a message'], 'messages.read': ['读取消息', 'Read messages'], 'tasks.send': ['发送任务', 'Send a task'], 'tasks.read': ['查看任务', 'Check tasks'], 'agent_sessions.read': ['查看智能体会话', 'List agent sessions'],
  'llm.use': ['使用模型', 'Use a model'],
};
export function operationName(kind: string, t: T) {
  const known = operationNames[kind];
  if (known) return t(known[0], known[1]);
  const area = kind.split('.')[0];
  if (area === 'computer') return t('控制电脑', 'Control the computer');
  if (area === 'browser') return t('控制浏览器', 'Control the browser');
  if (area === 'agent') return t('运行智能体', 'Run an agent');
  if (area === 'vault') return t('使用账号凭据', 'Use saved accounts');
  if (area === 'projects') return t('修改项目', 'Change a project');
  if (area === 'terminal') return t('管理终端', 'Manage terminals');
  return kind;
}
export function riskName(risk: Risk, t: T) {
  return risk === 'read' ? t('只读', 'Read only') : risk === 'change' ? t('修改', 'Changes') : t('高风险', 'High risk');
}
export const riskTone = (risk: Risk) => risk === 'danger' ? 'bad' : risk === 'change' ? 'warn' : '';
