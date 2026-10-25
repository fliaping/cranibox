/** Both HTTP adapters use this flow. Retrying keeps the exact URL, body and session key. */
export type AccessWait = {
  id: number; target: string; route: string; session?: string; capability: string;
  state: 'pending' | 'denied' | 'expired' | 'off' | 'cooldown' | 'error'; hint?: string;
  retry: () => void; cancel: () => void;
};
let serial = 0;
let waits: AccessWait[] = [];
const listeners = new Set<() => void>();
export const accessWaits = () => waits;
export const subscribeAccessWaits = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };
function publish() { for (const listener of listeners) listener(); }
export type AccessFetchOptions = { signal?: AbortSignal; waiting?: (waiting: boolean) => void; timeoutMs?: number };
function targetOf(route: string, body?: BodyInit | null) {
  const url = new URL(route, location.href);
  let data: Record<string, unknown> = {};
  try { if (typeof body === 'string') data = JSON.parse(body); } catch { /* Not a JSON body. */ }
  const target = url.pathname.match(/\/(?:nodes|devices)\/([^/]+)/)?.[1] || url.searchParams.get('node') || data.node || data.target || '';
  return { target: decodeURIComponent(String(target)).replace(/^(direct|local):/, ''), session: typeof data.session_id === 'string' ? data.session_id : undefined };
}
export async function accessFetch(route: string, init: RequestInit, options: AccessFetchOptions = {}): Promise<Response> {
  const controller = new AbortController();
  const externalSignal = options.signal || init.signal;
  const signal = controller.signal;
  let item: AccessWait | undefined;
  let wake: (() => void) | undefined;
  let cancelled = false;
  const cancel = () => { cancelled = true; controller.abort(); wake?.(); };
  externalSignal?.addEventListener('abort', cancel, { once: true });
  if (externalSignal?.aborted) cancel();
  const attempt = (url: string, request: RequestInit) => fetch(url, { ...request, signal: options.timeoutMs ? AbortSignal.any([signal, AbortSignal.timeout(options.timeoutMs)]) : signal });
  const check = () => { if (cancelled || signal?.aborted) throw new DOMException('Request cancelled', 'AbortError'); };
  const pause = (ms?: number) => new Promise<void>(resolve => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    wake = () => { clearTimeout(timer); wake = undefined; resolve(); };
    if (ms !== undefined) timer = setTimeout(wake, ms);
    if (cancelled) wake();
  });
  try {
    for (;;) {
      check();
      const response = await attempt(route, init);
      if (response.status !== 403) return response;
      const value = await response.clone().json().catch(() => null);
      if (!['approval_required', 'capability_off', 'approval_cooldown'].includes(value?.error)) return response;
      const update = (state: AccessWait['state'], hint?: string) => {
        if (!item) item = { id: ++serial, ...targetOf(route, init.body), route, capability: value.capability, state, retry: () => wake?.(), cancel };
        item = { ...item, state, hint };
        waits = [...waits.filter(wait => wait.id !== item!.id), item]; publish(); options.waiting?.(true);
      };
      if (value.error !== 'approval_required') {
        update(value.error === 'capability_off' ? 'off' : 'cooldown', value.hint);
        await pause(); check(); continue;
      }
      update('pending');
      let approved = false;
      while (!approved) {
        await pause(2000); check();
        try {
          const stateResponse = await attempt('/api/v1/app/access/requests/' + encodeURIComponent(value.request_id) + '?node=' + encodeURIComponent(item!.target), { credentials: 'same-origin' });
          if (!stateResponse.ok) throw Error('Unable to check approval');
          const request = await stateResponse.json();
          if (request.state === 'approved') { approved = true; options.waiting?.(false); }
          else if (request.state === 'denied' || request.state === 'expired') {
            update(request.state); await pause(); check(); break;
          } else if (request.state !== 'pending') throw Error('Invalid approval state');
          else if (Date.now() >= value.expires_at * 1000) { update('expired'); await pause(); check(); break; }
        } catch (problem) {
          check(); update('error', problem instanceof Error ? problem.message : String(problem)); await pause(); check(); update('pending');
        }
      }
    }
  } finally {
    externalSignal?.removeEventListener('abort', cancel);
    if (item) { waits = waits.filter(wait => wait.id !== item!.id); publish(); options.waiting?.(false); }
  }
}
