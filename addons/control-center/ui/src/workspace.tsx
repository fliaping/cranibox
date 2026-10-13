import { createContext,useCallback,useContext,useEffect,useRef,useState,type ReactNode } from 'react';
import type { PollState,Snapshot } from './types';
import { useTerminalWindow } from './terminal/TerminalWindowContext';

const config = window.workspaceWeb;
export const zh = config.language.startsWith('zh');
export const t = (cn: string, en: string) => zh ? cn : en;
export async function request(route: string, body: unknown) {
  const response = await fetch(route, { method: 'POST', credentials: 'same-origin', headers: { 'Content-Type': 'application/json', 'X-Workspace-CSRF': config.csrf }, body: JSON.stringify(body), signal: AbortSignal.timeout(12000) });
  if (response.status === 401) throw Error(t('会话已过期，请刷新页面。', 'Session expired. Reload this page.'));
  const value = await response.json();
  if (!response.ok) throw Error(value.error || String(response.status));
  return value;
}
export function safeUrl(value: string, base = config.workspaceUrl) {
  try { const url = new URL(value, base); return ['https:', 'http:'].includes(url.protocol) ? url.href : undefined; } catch { return undefined; }
}
export function openUrl(value: string) {
  const href = safeUrl(value);
  if (href) window.open(href, '_blank', 'noopener,noreferrer');
}
type LocalDialog = { title: string; command: string } | null;
interface Workspace {
  state: PollState; snapshot?: Snapshot; error: string; notice: { message: string; url?: string } | null;
  submitting: boolean; locked: boolean; outputOpen: boolean; setOutputOpen: (value: boolean) => void;
  localDialog: LocalDialog; setLocalDialog: (value: LocalDialog) => void;
  action: (type: string, extra?: Record<string, unknown>) => void;
  refresh: () => void; cancel: () => void; notify: (message: string) => void; dismissNotice: () => void;
}
const Context = createContext<Workspace | null>(null);
export function useWorkspace() { const value = useContext(Context); if (!value) throw Error('Workspace provider missing'); return value; }
export function WorkspaceProvider({ children }: { children: ReactNode }) {
  const { openTerminal } = useTerminalWindow();
  const [state, setState] = useState<PollState>({ cursor: config.cursor, busy: false });
  const [error, setError] = useState('');
  const [notice, setNotice] = useState<Workspace['notice']>(null);
  const [submitting, setSubmitting] = useState(false);
  const [outputOpen, setOutputOpen] = useState(false);
  const [localDialog, setLocalDialog] = useState<LocalDialog>(null);
  const pollRef = useRef<(force?: boolean) => Promise<void>>(async () => {});
  const actionPending = useRef(false);
  useEffect(() => {
    let active = true, polling = false, queued = false, cursor = config.cursor, live = false, connecting = false;
    let timer: ReturnType<typeof setTimeout>, reconnect: ReturnType<typeof setTimeout>;
    let controller: AbortController | undefined;
    function apply(next: PollState) {
      if (!active) return;
      cursor = next.cursor;
      if (next.snapshot?.locale?.current && next.snapshot.locale.current !== config.language) { location.reload(); return; }
      setState(previous => ({ ...previous, ...next, snapshot: next.snapshot || previous.snapshot, snapshotAt: next.snapshotAt || previous.snapshotAt, output: next.outputDelta !== undefined ? (next.outputReset ? next.outputDelta : (previous.output || '') + next.outputDelta).slice(-128 * 1024) : next.output ?? previous.output }));
      setError(next.error || '');
      for (const event of next.events || []) {
        if (event.type === 'output') setOutputOpen(true);
        if (event.type === 'notice') setNotice({ message: event.message || '' });
        if (event.type === 'openUrl') setNotice({ message: t('操作完成，可以打开桌面。', 'Operation complete. The desktop is ready to open.'), url: event.url });
      }
    }
    const poll = async (force = false) => {
      if (live && !force) return;
      if (polling) { queued ||= force; return; }
      clearTimeout(timer); polling = true;
      let delay = 5000;
      try {
        const next: PollState = await request('/api/poll', { cursor, refresh: force });
        if (!live) apply(next); delay = next.busy || next.refreshing || !next.snapshot ? 1000 : 15000;
      } catch (problem) { if (active) setError(problem instanceof Error ? problem.message : String(problem)); }
      finally {
        polling = false;
        if (active) { const forced = queued; queued = false; if (forced || !live) timer = setTimeout(() => void poll(forced), forced ? 0 : document.hidden ? 30000 : delay); }
      }
    };
    async function connect() {
      if (!active || document.hidden || connecting) return;
      connecting = true; controller = new AbortController();
      try {
        const response = await fetch('/api/events', { method: 'POST', credentials: 'same-origin', headers: { 'Content-Type': 'application/json', 'X-Workspace-CSRF': config.csrf }, body: '{}', signal: controller.signal });
        if (!response.ok || !response.body) throw Error(t('实时连接不可用，使用自动刷新。', 'Live connection unavailable; using automatic refresh.'));
        live = true; clearTimeout(timer);
        const reader = response.body.getReader(), decoder = new TextDecoder();
        let buffer = '';
        while (active) {
          const { value, done } = await reader.read(); if (done) break;
          buffer += decoder.decode(value, { stream: true });
          let boundary: number;
          while ((boundary = buffer.indexOf('\n\n')) >= 0) {
            const frame = buffer.slice(0, boundary); buffer = buffer.slice(boundary + 2);
            const data = frame.split('\n').filter(line => line.startsWith('data:')).map(line => line.slice(5).trimStart()).join('\n');
            if (data) apply(JSON.parse(data));
          }
        }
      } catch (problem) {
        if (active && !document.hidden) setError(problem instanceof Error ? problem.message : String(problem));
      } finally {
        live = false; connecting = false;
        if (active) { void poll(); clearTimeout(reconnect); reconnect = setTimeout(() => void connect(), document.hidden ? 5000 : controller?.signal.aborted ? 0 : 5000); }
      }
    }
    pollRef.current = poll;
    const visible = () => { if (document.hidden) controller?.abort(); else { clearTimeout(reconnect); void poll(); if (!live) void connect(); } };
    document.addEventListener('visibilitychange', visible);
    void poll(); void connect();
    return () => { active = false; controller?.abort(); clearTimeout(timer); clearTimeout(reconnect); document.removeEventListener('visibilitychange', visible); };
  }, []);
  const action = useCallback((type: string, extra: Record<string, unknown> = {}) => {
    if (type === 'refresh' || type === 'ready') { void pollRef.current(true); return; }
    if (type === 'showOutput') { setOutputOpen(true); return; }
    if (type === 'openWorkspace') { openUrl(config.workspaceUrl); return; }
    if (type === 'openDesktop') { openUrl(config.desktopUrl); return; }
    if (type === 'openUrl' && typeof extra.url === 'string') { openUrl(extra.url); return; }
    if (type === 'openPath' && typeof extra.path === 'string' && /^\/proxy\/\d+\/$/.test(extra.path)) { openUrl(extra.path); return; }
    if (type === 'terminal' || type === 'connectTailscale') {
      openTerminal({ node: typeof extra.node === 'string' ? extra.node : '', command: type === 'connectTailscale' ? 'workspacectl tailscale login' : typeof extra.command === 'string' ? extra.command : undefined, name: typeof extra.name === 'string' ? extra.name : undefined, cwd: typeof extra.cwd === 'string' ? extra.cwd : undefined, create: true }); return;
    }
    if (['copyTunnel', 'copyDesktopTunnel'].includes(type)) {
      const command = type === 'copyTunnel' ? 'ssh -N -L 9080:127.0.0.1:9080 -L 8443:127.0.0.1:8443 user@server' : 'ssh -N -L 3001:127.0.0.1:3001 user@server';
      setLocalDialog({ title: t('在终端中继续', 'Continue in a terminal'), command }); return;
    }
    if (actionPending.current && type !== 'desktopEmergencyStop') return;
    actionPending.current = true; setSubmitting(true); setNotice(null);
    void request('/api/action', { type, ...extra }).then(() => pollRef.current()).catch(problem => setNotice({ message: problem.message })).finally(() => { actionPending.current = false; setSubmitting(false); });
  }, [openTerminal]);
  const cancel = () => { void request('/api/cancel', {}).then(() => pollRef.current()).catch(problem => setNotice({ message: problem.message })); };
  return <Context.Provider value={{ state, snapshot: state.snapshot || undefined, error, notice, submitting, locked: state.busy || submitting, outputOpen, setOutputOpen, localDialog, setLocalDialog, action, refresh: () => void pollRef.current(true), cancel, notify: message => setNotice({ message }), dismissNotice: () => setNotice(null) }}>{children}</Context.Provider>;
}
