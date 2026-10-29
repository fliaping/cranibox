export const capabilities = ['messages', 'terminals', 'exec', 'projects', 'desktop', 'computer', 'browser', 'update', 'spaces', 'activity', 'llm', 'vault'] as const;
export type Capability = typeof capabilities[number];
export type Mode = 'off' | 'ask' | 'allow';
export type Modes = Record<Capability, Mode>;
export type Risk = 'read' | 'change' | 'danger';
/** What a remote device asked this device to do, described and redacted by this device. */
export type Operation = { kind: string; risk: Risk; summary: string; confirm?: boolean };
/** Full detail of one operation, as recorded by the target device (secrets removed). */
export type OperationDetail = { argv?: string[]; cwd?: string; env_keys?: string[]; timeout_secs?: number; paths?: string[]; bytes?: number; space?: string; url?: string; target?: string; job?: string; outcome?: string; exit_code?: number; duration_ms?: number; stdout_bytes?: number; stderr_bytes?: number; stdout_tail?: string; stderr_tail?: string; error?: string; repeats?: number; findings?: string[] };
export type Verdict = 'allow' | 'ask' | 'deny';
/** The automatic reviewer's verdict on an operation. */
export type Review = { source: string; verdict: Verdict; risk: number; reason: string; labels?: string[]; model?: string | null; fallback?: boolean; latency_ms?: number };
export type ReviewBackend = 'rules' | 'model' | 'jev' | 'http';
export type ReviewSettings = { backend: ReviewBackend; model?: string | null; url?: string | null; timeout_ms?: number | null };
export type ReviewState = { settings: ReviewSettings; has_key: boolean };
export type AccessRequest = { id: string; from: string; name: string; capability: Capability; created_at: number; expires_at: number; state: string; path: unknown; operation?: Operation; detail?: OperationDetail; review?: Review };
export type Approval = { id: string; from: string; capability: Capability; scope?: 'session' | 'until'; expires_at: number | null; approved_at?: number };
export type AccessView = { peers: { node: string; name: string; capabilities: Modes }[]; pending: AccessRequest[]; approvals: Approval[]; sessions: { id: string; from: string; name: string; capability: Capability; basis?: string }[] };
export type AuditEntry = { index: number; at: number; event: string; from: string | null; name: string | null; capability: Capability | null; basis: string | null; result: string; prev?: string; operation?: Operation; status?: number; transport?: string; detail?: OperationDetail; review?: Review };
export type AuditView = { entries: AuditEntry[]; verified: boolean; broken_index: number | null; next_before?: number | null };
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
  'exec.run': ['运行命令', 'Run a command'], 'exec.finished': ['命令结束', 'Command finished'], 'exec.status': ['查看命令结果', 'Check a command'], 'exec.cancel': ['停止命令', 'Stop a command'],
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

/** Built-in rule findings (stable codes from the target device). */
const ruleNames: Record<string, [string, string]> = {
  recursive_delete: ['递归删除文件', 'Deletes files recursively'], disk: ['改写磁盘或分区', 'Writes disks or partitions'], power: ['关机或重启', 'Shuts down or restarts'],
  privilege: ['获取管理员权限', 'Gains admin rights'], account: ['修改系统账号', 'Changes system accounts'], persistence: ['开机自启或留下长期访问', 'Runs at login or leaves lasting access'],
  service: ['停止或更改系统服务', 'Stops or changes system services'], registry: ['修改注册表', 'Changes the registry'], permissions: ['批量修改权限', 'Changes permissions recursively'],
  container_delete: ['删除容器或镜像', 'Deletes containers or images'], kill: ['批量结束进程', 'Kills processes'], eval: ['动态执行代码', 'Evaluates code'],
  remote_code: ['下载并运行网络上的代码', 'Downloads and runs code'], obfuscated: ['内容经过编码隐藏', 'Hides what it runs'], inline_code: ['内联脚本含危险操作', 'Inline script does something dangerous'],
  credential_access: ['读取密钥或凭据文件', 'Reads keys or credentials'], login_files: ['修改登录或密钥文件', 'Changes login or key files'],
};
export function ruleName(code: string, t: T) {
  const known = ruleNames[code];
  return known ? t(known[0], known[1]) : code;
}
const labelNames: Record<string, [string, string]> = {
  destructive: ['破坏性', 'Destructive'], irreversible: ['不可撤销', 'Irreversible'], exfiltration: ['外传数据', 'Sends data out'], credentials: ['涉及凭据', 'Credentials'],
  persistence: ['长期驻留', 'Persistence'], privilege: ['提权', 'Privilege'], network: ['网络访问', 'Network'], out_of_scope: ['超出项目范围', 'Outside the project'],
  injection: ['疑似提示注入', 'Possible prompt injection'], obfuscated: ['内容隐藏', 'Obfuscated'],
};
export function labelName(label: string, t: T) {
  const known = labelNames[label];
  return known ? t(known[0], known[1]) : label;
}
export function verdictName(verdict: Verdict, t: T) {
  return verdict === 'allow' ? t('可放行', 'Looks fine') : verdict === 'ask' ? t('需要你确认', 'Needs you') : t('建议拒绝', 'Recommends denying');
}
export const verdictTone = (verdict: Verdict) => verdict === 'allow' ? 'good' : verdict === 'ask' ? 'warn' : 'bad';
export function reviewSource(source: string, t: T) {
  return source === 'model' ? t('模型审核', 'Model review') : source === 'jev' ? 'Jev' : source === 'http' ? t('决策服务', 'Decision service') : source;
}
/** Durations a time grant can last. */
export const grantScopes = ['1h', 'today', '7d', 'always'] as const;
export type GrantScope = typeof grantScopes[number];
export function scopeName(scope: string, t: T) {
  return ({ session: t('本次', 'Once'), '30m': t('30 分钟', '30 minutes'), '1h': t('1 小时', '1 hour'), '8h': t('8 小时', '8 hours'), today: t('今天', 'Today'), '7d': t('7 天', '7 days'), always: t('直到撤销', 'Until revoked') } as Record<string, string>)[scope] || scope;
}
