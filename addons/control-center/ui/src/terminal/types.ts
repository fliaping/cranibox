export type Session = { id: string; name: string; cwd: string; status: 'running' | 'ended' | 'unknown'; created_at: number; last_attached_at: number | null };
export type Catalog = { installed: boolean; version: string | null; sessions: Session[] };
export type OpenTerminal = { node: string; id: string; name: string; cwd: string; initialCommand?: string };
export const terminalKey = (value: { node: string; id: string }) => value.node + ':' + value.id;
