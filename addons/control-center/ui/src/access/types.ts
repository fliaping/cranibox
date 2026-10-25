export const capabilities = ['messages', 'terminals', 'exec', 'projects', 'desktop', 'computer', 'browser', 'update', 'spaces', 'activity', 'llm', 'vault'] as const;
export type Capability = typeof capabilities[number];
export type Mode = 'off' | 'ask' | 'allow';
export type Modes = Record<Capability, Mode>;
export type AccessRequest = { id: string; from: string; name: string; capability: Capability; created_at: number; expires_at: number; state: string; path: unknown };
export type AccessView = { peers: { node: string; name: string; capabilities: Modes }[]; pending: AccessRequest[]; approvals: { id: string; from: string; capability: Capability; expires_at: number | null }[]; sessions: { id: string; from: string; name: string; capability: Capability }[] };
export type AuditEntry = { index: number; at: number; event: string; from: string | null; name: string | null; capability: Capability | null; basis: string | null; result: string };
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
