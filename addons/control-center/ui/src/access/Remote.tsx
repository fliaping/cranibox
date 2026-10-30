import { useEffect, useId, useRef, useState, useSyncExternalStore } from 'react';
import { Link } from 'react-router-dom';
import { useApp } from '../app/store';
import { Alert, capabilityName, Card, Pill } from '../app/ui';
import { ApiError, type Api } from '../personal/api';
import { operationName, riskName, riskTone, type AccessRequest, type Approver, type RemoteDevice, type RemoteView } from './types';

type T = (zh: string, en: string) => string;

/** Requests waiting on the owner's other devices, shared by the banner and the page. */
let current: RemoteView | null = null, failure = '', timer: ReturnType<typeof setInterval> | undefined, source: Api | null = null;
const listeners = new Set<() => void>();
async function load() {
  if (!source || document.hidden) return;
  try { current = await source.call<RemoteView>('GET', '/access/remote'); failure = ''; }
  catch (problem) { failure = problem instanceof Error ? problem.message : String(problem); }
  listeners.forEach(listener => listener());
}
function subscribe(listener: () => void) {
  listeners.add(listener);
  if (!timer) { void load(); timer = setInterval(() => void load(), 8000); }
  return () => { listeners.delete(listener); if (!listeners.size && timer) { clearInterval(timer); timer = undefined; } };
}
export function useRemote() {
  const { api } = useApp();
  source = api;
  const view = useSyncExternalStore(subscribe, () => current);
  return { view, error: useSyncExternalStore(subscribe, () => failure), reload: load };
}
const waiting = (view: RemoteView | null) => (view?.devices || []).reduce((sum, device) => sum + device.requests.length, 0);
function secretName(approver: Approver | undefined, t: T) {
  return approver?.recovery_kind === 'code' ? t('恢复码', 'recovery code') : t('恢复口令', 'recovery passphrase');
}
function requestTitle(request: AccessRequest, who: string, where: string, t: T) {
  return request.operation
    ? t(who + ' 想在「' + where + '」上' + operationName(request.operation.kind, t), who + ' wants to ' + operationName(request.operation.kind, t).toLowerCase() + ' on ' + where)
    : t(who + ' 请求在「' + where + '」上使用' + capabilityName(request.capability, t), who + ' requests ' + capabilityName(request.capability, t) + ' on ' + where);
}

/** A line on every page when another of your devices has requests waiting. */
export function RemoteBanner() {
  const { t } = useApp(), { view } = useRemote(), count = waiting(view);
  if (!count) return null;
  return <div className="cx-access-banner" role="status"><span>{t('你的其他设备上有 ' + count + ' 个请求等你批准', count + ' request(s) on your other devices need your approval')}</span>
    <div className="cx-actions"><Link className="cx-btn small primary" to="/app/approvals">{t('去处理', 'Review')}</Link></div></div>;
}

/** 待处理的审批: what waits on your other devices, and whether they may decide for this one. */
export function RemoteApprovals() {
  const { t } = useApp(), { view, error, reload } = useRemote();
  const [open, setOpen] = useState<{ device: RemoteDevice; index: number } | null>(null), [done, setDone] = useState('');
  const [now, setNow] = useState(Date.now());
  useEffect(() => { const id = setInterval(() => setNow(Date.now()), 1000); return () => clearInterval(id); }, []);
  const approver = view?.approver, secret = secretName(approver, t);
  const devices = view?.devices || [];
  const item = open && devices.find(device => device.node === open.device.node)?.requests[open.index];
  return <div className="cx-stack loose">
    <Card label={t('待处理的审批', 'Pending approvals')}>
      <div className="cx-between"><h2>{t('待处理的审批', 'Pending approvals')}</h2><button className="cx-btn small" onClick={() => void reload()}>{t('刷新', 'Refresh')}</button></div>
      <p className="cx-muted">{t('这里是你其他设备上等待批准的请求。在另一台设备上批准，需要输入你的 Cranibox ' + secret + '：它只在本机用来签名这一次决定，不会发送出去，也不会保存。', 'Requests waiting on your other devices. Approving from another device needs your Cranibox ' + secret + ': it is used on this device only to sign this one decision, and is never sent or saved.')}</p>
      {error && <Alert>{error}</Alert>}
      {!view && !error && <p className="cx-muted">{t('正在查看你的其他设备…', 'Checking your other devices…')}</p>}
      {approver && !approver.owner_device && <p className="cx-muted">{t('同一身份下还没有其他设备。配对后，它们的请求会出现在这里。', 'No other devices share your identity yet. Once they do, their requests appear here.')}</p>}
      {approver && approver.owner_device && !approver.recovery && <Alert tone="warn" action={<Link className="cx-btn small" to="/app/identity">{t('去设置', 'Set it up')}</Link>}>{t('还没有设置恢复口令。设置后，才能在一台设备上批准另一台设备的请求。', 'No recovery passphrase yet. Set one to approve one device’s requests from another.')}</Alert>}
      {done && <div className="cx-notice" role="status"><span>{done}</span></div>}
      {devices.map(device => <section key={device.node} className="cx-access-group" aria-label={device.name}>
        <div className="cx-between"><strong>{device.name}</strong>{!device.online ? <Pill>{t('连不上', 'Unreachable')}</Pill> : device.error === 'disabled' ? <Pill>{t('不接受其他设备审批', 'Not accepting remote approval')}</Pill> : <Pill tone={device.requests.length ? 'warn' : 'good'}>{device.requests.length ? t(device.requests.length + ' 个等待中', device.requests.length + ' waiting') : t('没有等待的请求', 'Nothing waiting')}</Pill>}</div>
        {device.requests.length > 0 && <ul className="cx-access-grants">{device.requests.map((entry, index) => {
          const left = Math.max(0, Math.ceil(Math.min(entry.request.expires_at, entry.challenge_expires_at) - now / 1000));
          return <li key={entry.request.id} className="cx-between">
            <span><strong>{requestTitle(entry.request, entry.request.name || entry.request.from, device.name, t)}</strong>{entry.request.operation && <> <Pill tone={riskTone(entry.request.operation.risk)}>{riskName(entry.request.operation.risk, t)}</Pill></>}{entry.once_only && <> <Pill tone="warn">{t('只能允许一次', 'Once only')}</Pill></>} <span className="cx-muted">· {t('剩余 ' + left + ' 秒', left + ' s left')}</span>
              {entry.request.operation?.summary && <code className="cx-access-op">{entry.request.operation.summary}</code>}</span>
            <button className="cx-btn small primary" disabled={!approver?.recovery || !left} onClick={() => { setDone(''); setOpen({ device, index }); }}>{t('查看并决定', 'Review')}</button>
          </li>;
        })}</ul>}
      </section>)}
    </Card>
    {approver && <RemoteSetting approver={approver} />}
    {open && item && <RemoteApprovalDialog device={open.device} item={item} approver={approver!} close={() => setOpen(null)} decided={text => { setOpen(null); setDone(text); void reload(); }} />}
  </div>;
}

/** Whether the owner's other devices may decide requests waiting on this one. */
function RemoteSetting({ approver }: { approver: Approver }) {
  const { t, api } = useApp(), { reload } = useRemote();
  const [busy, setBusy] = useState(false), [error, setError] = useState(''), [on, setOn] = useState(approver.remote_enabled);
  useEffect(() => setOn(approver.remote_enabled), [approver.remote_enabled]);
  async function toggle(enabled: boolean) {
    setBusy(true); setError(''); setOn(enabled);
    try { await api.call('PUT', '/access/approver', { remote_enabled: enabled }); await reload(); }
    catch (problem) { setOn(!enabled); setError(problem instanceof Error ? problem.message : String(problem)); }
    finally { setBusy(false); }
  }
  return <Card label={t('从其他设备批准本机', 'Approval from other devices')}>
    <h2>{t('从其他设备批准本机', 'Approval from other devices')}</h2>
    <label className="cx-check"><input type="checkbox" checked={on} disabled={busy} onChange={event => void toggle(event.target.checked)} /> {t('允许我的其他设备批准本机上的请求', 'Let my other devices approve requests waiting here')}</label>
    <p className="cx-muted cx-small">{t('每次都要输入恢复口令，并且只能用一次。连续输错 ' + approver.attempts + ' 次后，本机会暂停接受其他设备的批准（先 5 分钟，再错时间加倍）。在本机上直接批准不受影响，也不需要输入恢复口令。', 'Each approval needs your recovery passphrase and works once. After ' + approver.attempts + ' wrong attempts this device stops accepting approvals from other devices for a while (5 minutes, doubling after that). Approving here directly is not affected and needs no passphrase.')}</p>
    {error && <Alert>{error}</Alert>}
  </Card>;
}

function failureText(problem: unknown, secret: string, t: T) {
  if (!(problem instanceof ApiError)) return problem instanceof Error ? problem.message : String(problem);
  const body = problem.body || {};
  const until = typeof body.until === 'number' ? new Date(body.until * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : '';
  switch (problem.code) {
    case 'wrong_code': return t(secret + '不正确。' + (typeof body.left === 'number' ? '还可以再试 ' + body.left + ' 次。' : ''), 'That ' + secret + ' is not right.' + (typeof body.left === 'number' ? ' ' + body.left + ' attempt(s) left.' : ''));
    case 'locked': return t('输错次数太多，那台设备暂停接受其他设备的批准' + (until ? '，到 ' + until + ' 恢复' : '') + '。你仍可以在那台设备上直接处理。', 'Too many wrong attempts; that device is not accepting approvals from other devices' + (until ? ' until ' + until : '') + '. You can still decide on that device itself.');
    case 'not_a_code': return t('这不像是你的' + secret + '，请检查是否输错。', 'That does not look like your ' + secret + '; check for typos.');
    case 'no_recovery': return t('还没有设置恢复口令。', 'No recovery passphrase is set yet.');
    case 'request_unavailable': return t('这个请求已经被处理或已过期。', 'This request was already decided or has expired.');
    case 'operation_mismatch': return t('请求内容在你查看后发生了变化，已拒绝签名。请重新查看。', 'The request changed after you saw it, so nothing was signed. Review it again.');
    case 'once_only': return t('这个操作只能允许一次。', 'This operation can only be allowed once.');
    case 'disabled': return t('那台设备不接受其他设备的批准。', 'That device does not accept approvals from other devices.');
    default: return problem.message;
  }
}

function RemoteApprovalDialog({ device, item, approver, close, decided }: { device: RemoteDevice; item: RemoteDevice['requests'][number]; approver: Approver; close: () => void; decided: (text: string) => void }) {
  const { t, api } = useApp(), title = useId(), dialog = useRef<HTMLDialogElement>(null);
  const [code, setCode] = useState(''), [busy, setBusy] = useState(false), [error, setError] = useState('');
  const request = item.request, operation = request.operation, secret = secretName(approver, t);
  useEffect(() => { const element = dialog.current; element?.showModal(); return () => element?.close(); }, []);
  const scopes = item.once_only ? [['session', t('允许这一次', 'Allow this once')]] as const : [['session', t('允许本次', 'Allow once')], ['1h', t('允许 1 小时', 'Allow 1 hour')], ['today', t('今天内允许', 'Allow today')]] as const;
  async function decide(scope?: string) {
    setBusy(true); setError('');
    try {
      await api.call('POST', '/access/remote/decide', { node: device.node, request: request.id, op_hash: item.op_hash, decision: scope ? 'approve' : 'deny', scope: scope || null, auth: { code } });
      setCode('');
      decided(scope ? t('已在「' + device.name + '」上批准。', 'Approved on ' + device.name + '.') : t('已在「' + device.name + '」上拒绝。', 'Denied on ' + device.name + '.'));
    } catch (problem) { setCode(''); setError(failureText(problem, secret, t)); }
    finally { setBusy(false); }
  }
  return <dialog ref={dialog} className="cx-sheet cx-access-dialog" aria-labelledby={title} onCancel={event => { event.preventDefault(); close(); }}>
    <div className="cx-sheet-head"><h2 id={title}>{requestTitle(request, request.name || request.from, device.name, t)}</h2></div>
    <form className="cx-sheet-body" onSubmit={event => event.preventDefault()}>
      {operation && <div className="cx-access-op-box">
        <div className="cx-between"><span className="cx-muted">{capabilityName(request.capability, t)}</span><Pill tone={riskTone(operation.risk)}>{riskName(operation.risk, t)}</Pill></div>
        {operation.summary && <code className="cx-access-op">{operation.summary}</code>}
      </div>}
      {request.review && !request.review.fallback && <p className="cx-muted">{t('自动审核：', 'Automatic review: ')}{request.review.reason}</p>}
      {item.once_only ? <Alert tone="warn">{t('这个操作风险较高，从其他设备只能「允许这一次」。不认识就拒绝。', 'This is a high-risk operation, so from another device it can only be allowed once. If you do not recognise it, deny it.')}</Alert>
        : <p className="cx-muted">{t('授权到期或「' + device.name + '」重启后，需要重新批准。', 'Approvals end when they expire or when ' + device.name + ' restarts.')}</p>}
      <label className="cx-field">{t('在另一台设备上批准，需要输入你的 Cranibox ' + secret, 'Approving from another device needs your Cranibox ' + secret)}
        <input type="password" autoComplete="off" spellCheck={false} value={code} onChange={event => setCode(event.target.value)} autoFocus aria-describedby={title + '-hint'} /></label>
      <small id={title + '-hint'} className="cx-muted">{t('只在本机用来签名这一次决定，不会发送出去，也不会保存。如果担心这台设备不安全，请在可信的设备上处理，并更换恢复口令。', 'Used on this device only to sign this one decision; never sent or saved. If you do not trust this device, decide on a trusted one and replace your recovery passphrase.')}</small>
      {error && <Alert>{error}</Alert>}
      <div className="cx-actions cx-approval-actions">
        <button type="button" className="cx-btn" disabled={busy} onClick={close}>{t('取消', 'Cancel')}</button>
        <button type="button" className="cx-btn cx-approval-deny" disabled={busy || !code.trim()} onClick={() => void decide()}>{t('拒绝', 'Deny')}</button>
        {scopes.map(([scope, label]) => <button type="button" key={scope} className={scope === 'session' ? 'cx-btn primary' : 'cx-btn'} disabled={busy || !code.trim()} onClick={() => void decide(scope)}>{busy ? t('正在验证…', 'Verifying…') : label}</button>)}
      </div>
    </form>
  </dialog>;
}
