import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from 'react';
import { inShell, shellCall } from '../app/shell';

/** The desktop app's separate terminal window (opened with `?window=terminal`). */
export const isTerminalWindow = () => new URLSearchParams(location.search).get('window') === 'terminal';

export type TerminalSelection = { node: string; id: string };
export type TerminalLaunch = { node?: string; session?: string; command?: string; cwd?: string; name?: string; create?: boolean };
type LaunchRequest = TerminalLaunch & { serial: number };
type WindowState = { mode: 'closed' | 'open' | 'minimized'; selection: TerminalSelection };
const storageKey = 'cranibox.terminal.window';
function savedWindow(): WindowState {
  try {
    const value = JSON.parse(sessionStorage.getItem(storageKey) || 'null');
    if (['closed', 'open', 'minimized'].includes(value?.mode) && typeof value.selection?.node === 'string' && typeof value.selection?.id === 'string') return value;
  } catch { /* Browser storage is optional. */ }
  return { mode: 'closed', selection: { node: '', id: '' } };
}
type TerminalWindow = WindowState & {
  launch: LaunchRequest | null; count: number; title: string;
  openTerminal: (options?: TerminalLaunch) => void; minimize: () => void; close: () => void;
  select: (selection: TerminalSelection) => void; consume: (serial: number) => void;
  report: (count: number, title: string) => void;
};
const Context = createContext<TerminalWindow | null>(null);
export function useTerminalWindow() {
  const value = useContext(Context);
  if (!value) throw Error('Terminal window provider missing');
  return value;
}
export function TerminalWindowProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState(savedWindow), [launch, setLaunch] = useState<LaunchRequest | null>(null);
  const [summary, setSummary] = useState({ count: 0, title: '' });
  const serial = useRef(0);
  useEffect(() => { try { sessionStorage.setItem(storageKey, JSON.stringify(state)); } catch { /* Storage is optional. */ } }, [state]);
  const openTerminal = useCallback((options: TerminalLaunch = {}) => {
    // In the desktop app, terminals open in their own native window with tabs.
    if (inShell() && !isTerminalWindow()) { void shellCall('open_terminal', { ...options }).catch(() => undefined); return; }
    setState(previous => ({ mode: 'open', selection: options.node !== undefined || options.session ? { node: options.node || '', id: options.session || '' } : previous.selection }));
    if (options.create || options.command || options.session) setLaunch({ ...options, serial: ++serial.current });
  }, []);
  const select = useCallback((selection: TerminalSelection) => setState(previous => ({ ...previous, selection })), []);
  const consume = useCallback((value: number) => setLaunch(previous => previous?.serial === value ? null : previous), []);
  const report = useCallback((count: number, title: string) => setSummary(previous => previous.count === count && previous.title === title ? previous : { count, title }), []);
  const minimize = useCallback(() => {
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
    setState(previous => ({ ...previous, mode: 'minimized' }));
    requestAnimationFrame(() => document.getElementById('terminal-window-toggle')?.focus());
  }, []);
  const close = useCallback(() => {
    setLaunch(null); setState(previous => ({ ...previous, mode: 'closed' }));
    requestAnimationFrame(() => document.getElementById('terminal-window-toggle')?.focus());
  }, []);
  return <Context.Provider value={{ ...state, ...summary, launch, openTerminal, minimize, close, select, consume, report }}>{children}</Context.Provider>;
}
