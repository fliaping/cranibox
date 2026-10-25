import { useEffect, useId, useRef, useState, useSyncExternalStore } from 'react';
import { Link } from 'react-router-dom';
import { useApp } from '../app/store';
import { useTerminalDevices } from '../app/targets';
import { Alert, capabilityName, Card, isThis, Pill } from '../app/ui';
import type { Device } from '../personal/api';
import { accessWaits, subscribeAccessWaits, type AccessWait } from './request';
import { capabilities, groups, type AccessRequest, type AuditView, type Capability, type Mode, type Modes } from './types';
import './access.css';

function ModeControl({ label, value, disabled, change }: { label: string; value?: Mode; disabled: boolean; change: (mode: Mode) => void }) {
  const { t } = useApp();
  return <div className="cx-segment cx-access-modes" role="group" aria-label={label}>{(['off', 'ask', 'allow'] as const).map(mode => <button key={mode} aria-pressed={mode === value} disabled={disabled} onClick={() => change(mode)}>{mode === 'off' ? t('关闭', 'Off') : mode === 'ask' ? t('询问', 'Ask') : t('允许', 'Allow')}</button>)}</div>;
}
export function DeviceAccess({ device }: { device: Device }) {
  const { t, api, access, accessError, refresh } = useApp(), { displayName } = useTerminalDevices();
  const peer = access?.peers.find(peer => peer.node === device.node_id);
  const [draft, setDraft] = useState<Modes | null>(null), [saving, setSaving] = useState(false), [error, setError] = useState(''), [saved, setSaved] = useState(false);
  useEffect(() => { setDraft(null); setSaved(false); setError(''); }, [device.node_id]);
  const modes = draft || peer?.capabilities;
  function change(caps: Capability[], mode: Mode) { setDraft(current => ({ ...(current || modes), ...Object.fromEntries(caps.map(cap => [cap, mode])) }) as Modes); setSaved(false); }
  async function save() {
    if (!draft) return;
    setSaving(true); setError('');
    try { await api.call('PUT', '/access/' + encodeURIComponent(device.node_id), draft); await refresh(); setDraft(null); setSaved(true); }
    catch (problem) { setError(problem instanceof Error ? problem.message : String(problem)); }
    finally { setSaving(false); }
  }
  return <div className="cx-access-stack">
    <Card label={t('访问', 'Access')}>
      <h2>{isThis(device) ? t('本设备的访问设置', 'Access to this device') : t(displayName(device.node_id) + ' 可以在本设备上…', displayName(device.node_id) + ' can use this device to…')}</h2>
      <p className="cx-muted">{t('这些设置保存在本机。要使用对方的功能，请在对方设备上批准或调整访问设置。', 'These settings are stored here. To use another device, approve or change access on that device.')}</p>
      {isThis(device) ? <p>{t('请打开其他已配对设备的「访问」分区，选择它们能在本机做什么。', 'Open a paired device’s Access tab to choose what it can do here.')}</p> : !modes ? <Alert action={<button className="cx-btn small" onClick={() => void refresh()}>{t('重试', 'Retry')}</button>}>{accessError || t('正在读取访问设置…', 'Loading access settings…')}</Alert> : <>
        {groups.map(group => <section className="cx-access-group" key={group.id} aria-label={t(group.zh, group.en)}>
          <div className="cx-between"><strong>{t(group.zh, group.en)}</strong><ModeControl label={t(group.zh, group.en)} value={group.caps.every(cap => modes[cap] === modes[group.caps[0]]) ? modes[group.caps[0]] : undefined} disabled={saving} change={mode => change(group.caps, mode)} /></div>
          {!group.caps.every(cap => modes[cap] === modes[group.caps[0]]) && <small className="cx-muted">{t('已逐项设置', 'Custom modes')}</small>}
          {group.risk && group.caps.some(cap => modes[cap] === 'allow') && <Alert tone="warn">{group.always ? t('即使选择允许，远程更新、模型与账号操作仍需逐次批准。', 'Remote updates, model and account access still require approval each time, even with Allow.') : t('允许后，对方无需询问即可使用这些功能。无本地界面的设备也一样，请谨慎选择。', 'Allow lets this device use these functions without asking, including on headless devices. Choose carefully.')}</Alert>}
          <details><summary>{t('高级', 'Advanced')}</summary>{group.caps.map(cap => <div className="cx-between" key={cap}><span>{capabilityName(cap, t)}</span><ModeControl label={capabilityName(cap, t)} value={modes[cap]} disabled={saving} change={mode => change([cap], mode)} /></div>)}</details>
        </section>)}
        {error && <Alert>{error}</Alert>}
        <div className="cx-actions"><button className="cx-btn primary" disabled={!draft || saving} onClick={() => void save()}>{saving ? t('保存中…', 'Saving…') : t('保存', 'Save')}</button>{saved && <span role="status">{t('已保存', 'Saved')}</span>}</div>
      </>}
    </Card>
    <AccessLog key={device.node_id} initialNode={isThis(device) ? '' : device.node_id} />
  </div>;
}

export function AccessLog({ initialNode = '' }: { initialNode?: string }) {
  const { t, api, access } = useApp(), { displayName } = useTerminalDevices();
  const [node, setNode] = useState(initialNode), [capability, setCapability] = useState(''), [data, setData] = useState<AuditView | null>(null), [error, setError] = useState(''), [busy, setBusy] = useState(false), [more, setMore] = useState(false);
  const generation = useRef(0);
  async function load(before?: number) {
    const current = ++generation.current; setBusy(true); setError('');
    const query = new URLSearchParams({ limit: '50' });
    if (node) query.set('node', node);
    if (capability) query.set('capability', capability);
    if (before !== undefined) query.set('before', String(before));
    try { const value = await api.call<AuditView>('GET', '/access/audit?' + query); if (current === generation.current) { setData(previous => ({ ...value, entries: before === undefined ? value.entries : [...(previous?.entries || []), ...value.entries] })); setMore(value.entries.length === 50); } }
    catch (problem) { if (current === generation.current) setError(problem instanceof Error ? problem.message : String(problem)); }
    finally { if (current === generation.current) setBusy(false); }
  }
  useEffect(() => { setData(null); void load(); return () => { ++generation.current; }; }, [node, capability, api]);
  return <Card label={t('访问记录', 'Access log')}>
    <div className="cx-between"><h2>{t('访问记录', 'Access log')}</h2><button className="cx-btn small" disabled={busy} onClick={() => void load()}>{t('刷新', 'Refresh')}</button></div>
    <div className="cx-access-filters"><label className="cx-field">{t('设备', 'Device')}<select value={node} onChange={event => setNode(event.target.value)}><option value="">{t('全部设备', 'All devices')}</option>{access?.peers.map(peer => <option key={peer.node} value={peer.node}>{displayName(peer.node)}</option>)}</select></label>
      <label className="cx-field">{t('能力', 'Capability')}<select value={capability} onChange={event => setCapability(event.target.value)}><option value="">{t('全部能力', 'All capabilities')}</option>{capabilities.map(cap => <option key={cap} value={cap}>{capabilityName(cap, t)}</option>)}</select></label></div>
    {error && <Alert>{error}</Alert>}
    {data && <><p role="status"><Pill tone={data.verified ? 'good' : 'bad'}>{data.verified ? t('记录完整', 'Log verified') : t('记录可能被篡改（第 ' + data.broken_index + ' 条）', 'Log may have been tampered with (entry ' + data.broken_index + ')')}</Pill></p>
      <ul className="cx-access-log">{data.entries.map(entry => <li key={entry.index}><div><strong>{entry.name && entry.name !== entry.from ? entry.name : entry.from ? displayName(entry.from) : t('本机', 'This device')}</strong> · {entry.capability ? capabilityName(entry.capability, t) : '—'}</div><div>{eventName(entry.event, t)} · <time dateTime={new Date(entry.at * 1000).toISOString()}>{new Date(entry.at * 1000).toLocaleString()}</time></div><small className="cx-muted">#{entry.index} · {entry.result}{entry.basis && ' · ' + entry.basis}</small></li>)}</ul>
      {!data.entries.length && <p className="cx-muted">{t('暂无记录', 'No entries')}</p>}
      {more && <button className="cx-btn" disabled={busy} onClick={() => void load(data.entries.at(-1)?.index)}>{t('更早记录', 'Older entries')}</button>}
    </>}
  </Card>;
}
function eventName(event: string, t: (zh: string, en: string) => string) {
  return ({ session_started: t('开始使用', 'Session started'), session_ended: t('结束使用', 'Session ended'), request_created: t('请求批准', 'Approval requested'), approved: t('已批准', 'Approved'), denied: t('已拒绝', 'Denied'), expired: t('已过期', 'Expired'), policy_changed: t('访问设置已更改', 'Policy changed'), refused: t('访问被阻止', 'Access refused'), emergency_disconnect: t('全部断开', 'Disconnected'), checkpoint: t('归档校验点', 'Archive checkpoint') } as Record<string, string>)[event] || event;
}

export function RequesterStatus({ target, session }: { target?: string; session?: string }) {
  const { t } = useApp(), { displayName } = useTerminalDevices();
  const waits = useSyncExternalStore(subscribeAccessWaits, accessWaits);
  return <>{waits.filter(wait => (!target || wait.target === target.replace(/^(direct|local):/, '')) && (!session || wait.session === session)).map(wait => <div key={wait.id} className="cx-access-banner" role="status"><span>{waitMessage(wait, displayName(wait.target) || t('目标设备', 'the target device'), t)}{wait.hint && <small>{wait.hint}</small>}</span><div className="cx-actions">{wait.state !== 'pending' && <button className="cx-btn small" onClick={wait.retry}>{t('重试', 'Retry')}</button>}<button className="cx-btn small" onClick={wait.cancel}>{t('取消', 'Cancel')}</button></div></div>)}</>;
}
function waitMessage(wait: AccessWait, name: string, t: (zh: string, en: string) => string) {
  if (wait.state === 'pending') return t('等待 ' + name + ' 上的批准…', 'Waiting for approval on ' + name + '…');
  if (wait.state === 'denied') return t('已拒绝', 'Denied');
  if (wait.state === 'expired') return t('已过期', 'Expired');
  if (wait.state === 'off') return t('此能力已关闭，请在目标设备的访问设置中开启。', 'This capability is off. Enable it in the target device’s access settings.');
  return wait.state === 'cooldown' ? t('刚才的请求被拒绝，请稍后重试。', 'The last request was denied. Please retry shortly.') : t('无法查询批准状态，请重试。', 'Unable to check approval. Retry to continue.');
}

export function AccessSurfaces() {
  const { t, api, access, accessError, refresh } = useApp(), { displayName } = useTerminalDevices();
  const [now, setNow] = useState(Date.now()), [error, setError] = useState(''), [busy, setBusy] = useState(false), [decided, setDecided] = useState<string[]>([]);
  useEffect(() => { const timer = setInterval(() => setNow(Date.now()), 1000); return () => clearInterval(timer); }, []);
  const pending = (access?.pending || []).filter(request => request.expires_at * 1000 > now && !decided.includes(request.id)).sort((a, b) => a.created_at - b.created_at || a.id.localeCompare(b.id));
  async function decide(request: AccessRequest, scope?: string) {
    setBusy(true); setError('');
    try { await api.call('POST', '/access/requests/' + encodeURIComponent(request.id), scope ? { decision: 'approve', scope } : { decision: 'deny' }); setDecided(ids => [...ids, request.id]); await refresh(); }
    catch (problem) { setError(problem instanceof Error ? problem.message : String(problem)); await refresh(); }
    finally { setBusy(false); }
  }
  const sessions = new Map<string, NonNullable<typeof access>['sessions']>();
  for (const session of access?.sessions || []) sessions.set(session.from, [...(sessions.get(session.from) || []), session]);
  return <div className="cx-access-surfaces">
    {accessError && <Alert action={<button className="cx-btn small" onClick={() => void refresh()}>{t('重试', 'Retry')}</button>}>{t('无法读取访问状态：', 'Unable to read access status: ')}{accessError}</Alert>}
    {pending.length > 0 && <div className="cx-access-banner" role="status">{t(pending.length + ' 个访问请求等待你批准', pending.length + ' access request(s) need your approval')}</div>}
    {!!sessions.size && <div className="cx-access-banner" role="status"><span>{[...sessions].map(([node, items]) => t(displayName(node) + ' 正在使用 ', displayName(node) + ' is using ') + [...new Set(items.map(item => capabilityName(item.capability, t)))].join(' · ')).join('；')}</span><div className="cx-actions"><button className="cx-btn small danger" disabled={busy} onClick={async () => { setBusy(true); try { await api.call('POST', '/access/revoke', {}); await refresh(); } catch (problem) { setError(String(problem)); } finally { setBusy(false); } }}>{t('全部断开', 'Disconnect all')}</button><Link className="cx-btn small" to="/app/access-log">{t('访问记录', 'Access log')}</Link></div></div>}
    {error && !pending.length && <Alert>{error}</Alert>}
    <RequesterStatus />
    {pending[0] && <ApprovalDialog key={pending[0].id} request={pending[0]} name={displayName(pending[0].from) || pending[0].name} left={Math.max(0, Math.ceil(pending[0].expires_at - now / 1000))} busy={busy} error={error} decide={scope => void decide(pending[0], scope)} />}
  </div>;
}
function ApprovalDialog({ request, name, left, busy, error, decide }: { request: AccessRequest; name: string; left: number; busy: boolean; error: string; decide: (scope?: string) => void }) {
  const { t } = useApp(), title = useId(), dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => { const element = dialog.current, restore = document.activeElement as HTMLElement | null; element?.showModal(); return () => { element?.close(); if (restore?.isConnected) restore.focus(); }; }, []);
  return <dialog ref={dialog} className="cx-sheet cx-access-dialog" aria-labelledby={title} onCancel={event => event.preventDefault()} onKeyDown={event => {
    if (event.key !== 'Tab') return;
    const buttons = dialog.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)');
    if (!buttons?.length) { event.preventDefault(); return; }
    const first = buttons[0], last = buttons[buttons.length - 1];
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
  }}>
    <div className="cx-sheet-head"><h2 id={title}>{t(name + ' 请求使用 ' + capabilityName(request.capability, t), name + ' requests ' + capabilityName(request.capability, t))}</h2></div>
    <div className="cx-sheet-body"><p className="cx-mono">{request.from}</p><p>{t('仅在你同意后，对方才能在本设备上使用此功能。', 'This device can use this function here only after you approve.')}</p><p role="timer">{t('剩余 ' + left + ' 秒', left + ' seconds left')}</p>{error && <Alert>{error}</Alert>}<div className="cx-actions"><button autoFocus className="cx-btn" disabled={busy} onClick={() => decide()}>{t('拒绝', 'Deny')}</button>{[['session', t('允许本次', 'Allow once')], ['30m', t('允许 30 分钟', 'Allow 30 minutes')], ['8h', t('允许 8 小时', 'Allow 8 hours')]].map(([scope, label]) => <button className="cx-btn primary" key={scope} disabled={busy} onClick={() => decide(scope)}>{label}</button>)}</div></div>
  </dialog>;
}
