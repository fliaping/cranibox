import { createContext, useContext, type ReactNode } from 'react';

export type TerminalInput = { type: 'input'; data: string } | { type: 'resize'; cols: number; rows: number };
export type TerminalConnection = { send: (input: TerminalInput) => void; close: () => void };
export type TerminalEvents = { ready: () => void; output: (data: Uint8Array) => void; error: (message: string, fatal: boolean) => void; ended: () => void; closed: () => void; waiting?: (waiting: boolean) => void };
export type TerminalOptions = { node: string; session_id: string; cols: number; rows: number; takeover: boolean; signal?: AbortSignal };
export type TerminalTarget = { id: string; aliases?: string[]; name: string; available: boolean };
export type TerminalEnvironment = {
  t: (cn: string, en: string) => string;
  available: boolean;
  includeLocal: boolean;
  localNode?: string;
  targets: TerminalTarget[];
  manage: (node: string, command: Record<string, unknown>) => Promise<any>;
  connect: (options: TerminalOptions, events: TerminalEvents) => Promise<TerminalConnection>;
};
const Context = createContext<TerminalEnvironment | null>(null);
export function TerminalEnvironmentProvider({ value, children }: { value: TerminalEnvironment; children: ReactNode }) { return <Context.Provider value={value}>{children}</Context.Provider>; }
export function useTerminalEnvironment() { const value = useContext(Context); if (!value) throw Error('Terminal environment is missing'); return value; }
export const directTarget = (node: string) => 'direct:' + node;
export const localTarget = (node: string) => 'local:' + node;
export function terminalTarget(node: string): { node: string; direct: true } | { node: string; local: true } | { node: string | null } {
  return node.startsWith('direct:') ? { node: node.slice(7), direct: true }
    : node.startsWith('local:') ? { node: node.slice(6), local: true } : { node: node || null };
}
