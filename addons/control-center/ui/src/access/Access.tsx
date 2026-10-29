import { useEffect, useId, useRef, useState, useSyncExternalStore } from 'react';
import { Link } from 'react-router-dom';
import { useApp } from '../app/store';
import { useTerminalDevices } from '../app/targets';
import { Alert, capabilityName, Card, formatDeviceLabel, isThis, Pill } from '../app/ui';
import type { Device } from '../personal/api';
import { accessWaits, subscribeAccessWaits, type AccessWait } from './request';
import { capabilities, grantScopes, groups, labelName, operationName, reviewSource, riskName, riskTone, ruleName, scopeName, verdictName, verdictTone, type AccessRequest, type AuditEntry, type AuditView, type Capability, type GrantScope, type Mode, type Modes, type OperationDetail, type Review, type ReviewBackend, type ReviewState } from './types';
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
    {!isThis(device) && <Grants node={device.node_id} />}
    <AutoReview />
    <AccessLog key={device.node_id} initialNode={isThis(device) ? '' : device.node_id} />
  </div>;
}

const ownerOnly: Capability[] = ['update', 'llm', 'vault'];
function until(at: number | null | undefined, t: (zh: string, en: string) => string) {
  if (!at) return t('直到撤销', 'Until revoked');
  const date = new Date(at * 1000), today = new Date();
  const sameDay = date.toDateString() === today.toDateString();
  const text = sameDay ? date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : date.toLocaleString([], { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' });
  return t('有效至 ' + text, 'Until ' + text);
}
/** Approvals this device has given the peer that are still in force, each revocable, and
 * a way to grant access for a while without waiting for a request. */
function Grants({ node }: { node: string }) {
  const { t, api, access, refresh } = useApp();
  const [busy, setBusy] = useState(''), [error, setError] = useState(''), [adding, setAdding] = useState(false);
  const approvals = (access?.approvals || []).filter(approval => approval.from === node);
  const sessions = (access?.sessions || []).filter(session => session.from === node);
  const allowed = access?.peers.find(peer => peer.node === node);
  const always = allowed ? capabilities.filter(cap => allowed.capabilities[cap] === 'allow' && !ownerOnly.includes(cap)) : [];
  async function revoke(id: string) {
    setBusy(id); setError('');
    try { await api.call('POST', '/access/revoke', { id }); await refresh(); }
    catch (problem) { setError(problem instanceof Error ? problem.message : String(problem)); }
    finally { setBusy(''); }
  }
  return <Card label={t('当前授权', 'Current approvals')}>
    <div className="cx-between"><h2>{t('当前授权', 'Current approvals')}</h2>{allowed && <button className="cx-btn small" onClick={() => setAdding(true)}>{t('按时间授权…', 'Grant for a while…')}</button>}</div>
    {!approvals.length && <p className="cx-muted">{t('没有临时授权。需要时对方会再次请求；你也可以按时间预先授权，期间不再逐条打扰你。', 'No temporary approvals. The device asks again when it needs one, or grant access for a while so it does not ask each time.')}</p>}
    {!!approvals.length && <ul className="cx-access-grants">{approvals.map(approval => {
      const using = sessions.some(session => session.basis === approval.id);
      return <li key={approval.id} className="cx-between">
        <span><strong>{capabilityName(approval.capability, t)}</strong> <span className="cx-muted">· {approval.scope === 'session' ? t('仅本次', 'This time only') : until(approval.expires_at, t)}</span>{using && <> <Pill tone="info">{t('正在使用', 'In use')}</Pill></>}</span>
        <button className="cx-btn small danger" disabled={busy === approval.id} onClick={() => void revoke(approval.id)}>{t('撤销', 'Revoke')}</button>
      </li>;
    })}</ul>}
    {!!always.length && <p className="cx-muted">{t('始终允许：', 'Always allowed: ')}{always.map(cap => capabilityName(cap, t)).join('、')}{t('。要收回，请把它们改为「询问」或「关闭」。危险命令仍会每次询问。', '. To take these back, set them to Ask or Off. Dangerous commands still ask every time.')}</p>}
    {error && <Alert>{error}</Alert>}
    {adding && allowed && <GrantDialog node={node} modes={allowed.capabilities} close={() => setAdding(false)} />}
  </Card>;
}
function GrantDialog({ node, modes, close }: { node: string; modes: Modes; close: () => void }) {
  const { t, api, refresh } = useApp(), { displayName } = useTerminalDevices(), title = useId(), dialog = useRef<HTMLDialogElement>(null);
  const choices = groups.filter(group => !group.always && group.id !== 'messages');
  const [chosen, setChosen] = useState<string[]>([]), [scope, setScope] = useState<GrantScope>('1h'), [busy, setBusy] = useState(false), [error, setError] = useState('');
  useEffect(() => { const element = dialog.current; element?.showModal(); return () => element?.close(); }, []);
  const caps = choices.filter(group => chosen.includes(group.id)).flatMap(group => group.caps).filter(cap => modes[cap] !== 'off' && !ownerOnly.includes(cap));
  async function grant() {
    setBusy(true); setError('');
    try { await api.call('POST', '/access/grants', { node, capabilities: caps, scope }); await refresh(); close(); }
    catch (problem) { setError(problem instanceof Error ? problem.message : String(problem)); }
    finally { setBusy(false); }
  }
  return <dialog ref={dialog} className="cx-sheet cx-access-dialog" aria-labelledby={title} onCancel={event => { event.preventDefault(); close(); }}>
    <div className="cx-sheet-head"><h2 id={title}>{t('允许 ' + displayName(node) + ' 一段时间', 'Allow ' + displayName(node) + ' for a while')}</h2></div>
    <div className="cx-sheet-body">
      <p className="cx-muted">{t('在这段时间内，对方使用所选功能时不再逐条询问你。删除数据、提权等危险操作，以及自动审核认为可疑的操作，仍会每次请你确认。', 'For this long the device can use what you choose without asking each time. Dangerous operations, and any the automatic review flags, still ask every time.')}</p>
      <fieldset className="cx-access-choices"><legend>{t('允许做什么', 'What it can do')}</legend>
        {choices.map(group => {
          const off = group.caps.every(cap => modes[cap] === 'off');
          return <label key={group.id} className="cx-check"><input type="checkbox" disabled={off} checked={chosen.includes(group.id)} onChange={event => setChosen(list => event.target.checked ? [...list, group.id] : list.filter(id => id !== group.id))} /> {t(group.zh, group.en)}{off && <small className="cx-muted"> · {t('已关闭', 'Off')}</small>}</label>;
        })}
      </fieldset>
      <fieldset className="cx-access-choices"><legend>{t('多长时间', 'For how long')}</legend>
        <div className="cx-segment" role="group" aria-label={t('多长时间', 'For how long')}>{grantScopes.map(item => <button key={item} aria-pressed={scope === item} onClick={() => setScope(item)}>{scopeName(item, t)}</button>)}</div>
        {scope === 'always' && <small className="cx-muted">{t('直到你在这里撤销为止。', 'Until you revoke it here.')}</small>}
      </fieldset>
      {error && <Alert>{error}</Alert>}
      <div className="cx-actions cx-approval-actions">
        <button className="cx-btn" onClick={close}>{t('取消', 'Cancel')}</button>
        <button className="cx-btn primary" disabled={busy || !caps.length} onClick={() => void grant()}>{busy ? t('授权中…', 'Granting…') : t('授权', 'Grant')}</button>
      </div>
    </div>
  </dialog>;
}

/** Which second opinion this device asks before a covered operation runs. */
function AutoReview() {
  const { t, api } = useApp();
  const [state, setState] = useState<ReviewState | null>(null), [draft, setDraft] = useState<{ backend: ReviewBackend; model: string; url: string; key: string } | null>(null);
  const [models, setModels] = useState<string[]>([]), [busy, setBusy] = useState(''), [error, setError] = useState(''), [result, setResult] = useState<{ ok: boolean; review?: Review; error?: string } | null>(null), [saved, setSaved] = useState(false);
  useEffect(() => {
    api.call<ReviewState>('GET', '/access/review').then(setState).catch(problem => setError(problem instanceof Error ? problem.message : String(problem)));
    api.call<{ id: string; availability?: string }[]>('GET', '/models').then(list => setModels(Array.isArray(list) ? list.map(item => item.id).filter(Boolean) : [])).catch(() => setModels([]));
  }, [api]);
  if (!state) return error ? <Card label={t('自动审核', 'Automatic review')}><h2>{t('自动审核', 'Automatic review')}</h2><Alert>{error}</Alert></Card> : null;
  const current = draft || { backend: state.settings.backend, model: state.settings.model || '', url: state.settings.url || '', key: '' };
  const edit = (change: Partial<typeof current>) => { setDraft({ ...current, ...change }); setSaved(false); setResult(null); };
  const options: [ReviewBackend, string, string][] = [
    ['rules', t('仅内置规则', 'Built-in rules only'), t('不请模型判断。危险操作每次确认，其余按你的授权放行。', 'No model. Dangerous operations always ask; the rest follow your approvals.')],
    ['model', t('用我的模型', 'Use my model'), t('通过本机的 Cranibox 模型网关判断，内容不出你的账号。', 'Judged through this device’s Cranibox model gateway.')],
    ['jev', 'Jev（TypeSafe）', t('专门的判断模型，速度快、成本低，需要 API 密钥。', 'A fast, low-cost judgment model. Needs an API key.')],
    ['http', t('自定义决策服务', 'Custom decision service'), t('发送到你自己的决策服务（Cranibox 决策 API v1）。', 'Sent to your own service (Cranibox decision API v1).')],
  ];
  async function save() {
    setBusy('save'); setError('');
    const settings = { backend: current.backend, model: current.backend === 'model' ? current.model || null : current.backend === 'jev' && current.model ? current.model : null, url: current.backend === 'http' || current.backend === 'jev' ? current.url.trim() || null : null };
    try { setState(await api.call<ReviewState>('PUT', '/access/review', { settings, ...(current.key ? { key: current.key } : {}) })); setDraft(null); setSaved(true); }
    catch (problem) { setError(problem instanceof Error ? problem.message : String(problem)); }
    finally { setBusy(''); }
  }
  async function check() {
    setBusy('test'); setResult(null);
    try { setResult(await api.call('POST', '/access/review/test', {})); }
    catch (problem) { setResult({ ok: false, error: problem instanceof Error ? problem.message : String(problem) }); }
    finally { setBusy(''); }
  }
  return <Card label={t('自动审核', 'Automatic review')}>
    <h2>{t('自动审核', 'Automatic review')}</h2>
    <p className="cx-muted">{t('对方在已授权的范围内操作时，先让模型看一眼：正常的直接放行，可疑的再请你确认。模型只能多加一道确认，不能放行你没授权的操作；它不可用时，高风险操作改为询问你。', 'Before a covered operation runs, a model takes a look: routine ones go through, suspicious ones come to you. It can only add a confirmation, never allow what you have not; when it is unavailable, high-risk operations ask you.')}</p>
    <div className="cx-access-review-options" role="radiogroup" aria-label={t('审核方式', 'Review method')}>{options.map(([value, label, detail]) => <label key={value} className={'cx-access-option' + (current.backend === value ? ' active' : '')}><input type="radio" name="review-backend" checked={current.backend === value} onChange={() => edit({ backend: value })} /><span><strong>{label}</strong><small className="cx-muted">{detail}</small></span></label>)}</div>
    {current.backend === 'model' && <label className="cx-field">{t('模型', 'Model')}{models.length ? <select value={current.model} onChange={event => edit({ model: event.target.value })}><option value="">{t('请选择', 'Choose')}</option>{models.map(id => <option key={id} value={id}>{id}</option>)}</select> : <input value={current.model} placeholder="provider/model" onChange={event => edit({ model: event.target.value })} />}{!models.length && <small className="cx-muted">{t('本机还没有可用的模型，请先在「模型」里添加。', 'No models on this device yet; add one under Models first.')}</small>}</label>}
    {(current.backend === 'http' || current.backend === 'jev') && <label className="cx-field">{current.backend === 'jev' ? t('服务地址（可选）', 'Service address (optional)') : t('服务地址', 'Service address')}<input value={current.url} placeholder={current.backend === 'jev' ? 'https://api.typesafe.ai' : 'https://decide.example.com/v1/decide'} onChange={event => edit({ url: event.target.value })} /></label>}
    {(current.backend === 'jev' || current.backend === 'http') && <label className="cx-field">{current.backend === 'jev' ? t('Jev API 密钥', 'Jev API key') : t('访问密钥（可选）', 'Access key (optional)')}<input type="password" autoComplete="off" value={current.key} placeholder={state.has_key ? t('已保存，留空则不变', 'Saved; leave empty to keep') : ''} onChange={event => edit({ key: event.target.value })} /></label>}
    {error && <Alert>{error}</Alert>}
    {result && (result.ok && result.review ? <Alert tone="info">{t('测试成功：', 'Test passed: ')}{verdictName(result.review.verdict, t)} · {result.review.reason} · {result.review.latency_ms ?? 0} ms</Alert> : <Alert>{t('测试失败：', 'Test failed: ')}{result.error}</Alert>)}
    <div className="cx-actions"><button className="cx-btn primary" disabled={!draft || !!busy} onClick={() => void save()}>{busy === 'save' ? t('保存中…', 'Saving…') : t('保存审核方式', 'Save review method')}</button>{state.settings.backend !== 'rules' && !draft && <button className="cx-btn" disabled={!!busy} onClick={() => void check()}>{busy === 'test' ? t('测试中…', 'Testing…') : t('测试', 'Test')}</button>}{saved && <span role="status">{t('已保存', 'Saved')}</span>}</div>
  </Card>;
}

export function AccessLog({ initialNode = '' }: { initialNode?: string }) {
  const { t, api, access } = useApp(), { displayName } = useTerminalDevices();
  const [node, setNode] = useState(initialNode), [capability, setCapability] = useState(''), [kind, setKind] = useState(''), [risk, setRisk] = useState(''), [result, setResult] = useState(''), [search, setSearch] = useState(''), [q, setQ] = useState('');
  const [data, setData] = useState<AuditView | null>(null), [error, setError] = useState(''), [busy, setBusy] = useState(false), [more, setMore] = useState(false), [open, setOpen] = useState<AuditEntry | null>(null);
  const generation = useRef(0);
  async function load(before?: number) {
    const current = ++generation.current; setBusy(true); setError('');
    const query = new URLSearchParams({ limit: '50' });
    if (node) query.set('node', node);
    if (capability) query.set('capability', capability);
    if (kind) query.set('event', kind);
    if (risk) query.set('risk', risk);
    if (result) query.set('result', result);
    if (q) query.set('q', q);
    if (before !== undefined) query.set('before', String(before));
    try {
      const value = await api.call<AuditView>('GET', '/access/audit?' + query);
      if (current === generation.current) { setData(previous => ({ ...value, entries: before === undefined ? value.entries : [...(previous?.entries || []), ...value.entries] })); setMore(value.next_before !== undefined ? value.next_before !== null : value.entries.length === 50); }
    }
    catch (problem) { if (current === generation.current) setError(problem instanceof Error ? problem.message : String(problem)); }
    finally { if (current === generation.current) setBusy(false); }
  }
  useEffect(() => { setData(null); void load(); return () => { ++generation.current; }; }, [node, capability, kind, risk, result, q, api]);
  const shown = data?.entries.filter(entry => (!kind || (kind === 'operations') === (entry.event === 'operation')) && (!risk || entry.operation?.risk === risk)) || [];
  const who = (entry: AuditEntry) => entry.name && entry.name !== entry.from ? entry.name : entry.from ? displayName(entry.from) : t('本机', 'This device');
  return <Card label={t('操作记录', 'Operation log')}>
    <div className="cx-between"><h2>{t('操作记录', 'Operation log')}</h2><button className="cx-btn small" disabled={busy} onClick={() => void load()}>{t('刷新', 'Refresh')}</button></div>
    <form className="cx-access-search" role="search" onSubmit={event => { event.preventDefault(); setQ(search.trim()); }}>
      <input type="search" aria-label={t('搜索记录', 'Search the log')} placeholder={t('搜索命令、文件、设备或原因', 'Search commands, files, devices or reasons')} value={search} onChange={event => { setSearch(event.target.value); if (!event.target.value) setQ(''); }} />
      <button className="cx-btn small" type="submit">{t('搜索', 'Search')}</button>
    </form>
    <div className="cx-access-filters"><label className="cx-field">{t('设备', 'Device')}<select value={node} onChange={event => setNode(event.target.value)}><option value="">{t('全部设备', 'All devices')}</option>{access?.peers.map(peer => <option key={peer.node} value={peer.node}>{displayName(peer.node)}</option>)}</select></label>
      <label className="cx-field">{t('能力', 'Capability')}<select value={capability} onChange={event => setCapability(event.target.value)}><option value="">{t('全部能力', 'All capabilities')}</option>{capabilities.map(cap => <option key={cap} value={cap}>{capabilityName(cap, t)}</option>)}</select></label>
      <label className="cx-field">{t('类型', 'Type')}<select value={kind} onChange={event => setKind(event.target.value)}><option value="">{t('全部记录', 'Everything')}</option><option value="operations">{t('远程操作', 'Remote operations')}</option><option value="access">{t('批准与设置', 'Approvals & settings')}</option></select></label>
      <label className="cx-field">{t('风险', 'Risk')}<select value={risk} onChange={event => setRisk(event.target.value)}><option value="">{t('全部风险', 'Any risk')}</option><option value="danger">{t('高风险', 'High risk')}</option><option value="change">{t('修改', 'Changes')}</option><option value="read">{t('只读', 'Read only')}</option></select></label>
      <label className="cx-field">{t('结果', 'Result')}<select value={result} onChange={event => setResult(event.target.value)}><option value="">{t('全部结果', 'Any result')}</option><option value="ok">{t('已完成', 'Done')}</option><option value="failed">{t('失败', 'Failed')}</option><option value="blocked">{t('被阻止或拒绝', 'Blocked or denied')}</option><option value="approved">{t('已批准', 'Approved')}</option></select></label></div>
    <p className="cx-muted">{t('其他设备在本机做的每个操作都记在这里，由本机保存并逐条校验，无法被对方修改。命令中的密码、令牌等敏感内容会被隐藏。点「详情」查看完整参数、结果和审核依据。', 'Every operation another device performs here is recorded and chained on this device; the other device cannot change it. Passwords, tokens and similar values are hidden. Open Details for the full arguments, result and why it was allowed.')}</p>
    {error && <Alert>{error}</Alert>}
    {data && <><p role="status"><Pill tone={data.verified ? 'good' : 'bad'}>{data.verified ? t('记录完整', 'Log verified') : t('记录可能被篡改（第 ' + data.broken_index + ' 条）', 'Log may have been tampered with (entry ' + data.broken_index + ')')}</Pill></p>
      <ul className="cx-access-log">{shown.map(entry => <LogEntry key={entry.index} entry={entry} who={who(entry)} open={() => setOpen(entry)} />)}</ul>
      {!shown.length && <p className="cx-muted">{t('暂无记录', 'No entries')}</p>}
      {more && <button className="cx-btn" disabled={busy} onClick={() => void load(data.next_before ?? data.entries.at(-1)?.index)}>{t('更早记录', 'Older entries')}</button>}
    </>}
    {open && <EntryDetail entry={open} who={who(open)} close={() => setOpen(null)} />}
  </Card>;
}
function outcomeName(entry: AuditEntry, t: (zh: string, en: string) => string) {
  if (entry.operation?.kind === 'exec.finished') return entry.result === 'ok' ? t('成功结束', 'Succeeded') : entry.detail?.outcome === 'timed_out' ? t('超时', 'Timed out') : entry.detail?.outcome === 'cancelled' ? t('已停止', 'Stopped') : t('失败', 'Failed') + (entry.detail?.exit_code != null ? t('（退出码 ' + entry.detail.exit_code + '）', ' (exit ' + entry.detail.exit_code + ')') : '');
  return entry.result === 'ok' ? t('已完成', 'Done') : entry.result === 'refused' ? t('被阻止', 'Blocked') : entry.result === 'rate_limited' ? t('过于频繁', 'Too frequent') : t('失败', 'Failed');
}
function basisName(entry: AuditEntry, t: (zh: string, en: string) => string) {
  if (!entry.basis) return '';
  if (entry.basis === 'allow') return t('「允许」模式', 'Allow mode');
  return t('经你批准', 'You approved');
}
function LogEntry({ entry, who, open }: { entry: AuditEntry; who: string; open: () => void }) {
  const { t } = useApp(), operation = entry.operation;
  const time = <time dateTime={new Date(entry.at * 1000).toISOString()}>{new Date(entry.at * 1000).toLocaleString()}</time>;
  const details = <button className="cx-btn small cx-access-more" onClick={open}>{t('详情', 'Details')}</button>;
  if (entry.event === 'operation' && operation) {
    const ok = entry.result === 'ok', review = entry.review, repeats = entry.detail?.repeats;
    return <li className="cx-access-op-entry">
      <div className="cx-between"><span><strong>{who}</strong> · {operationName(operation.kind, t)}</span>{details}</div>
      {operation.summary && <code className="cx-access-op">{operation.summary}</code>}
      <div style={{ fontVariantNumeric: 'tabular-nums' }}><Pill tone={ok ? 'good' : 'bad'}>{outcomeName(entry, t)}</Pill> <Pill tone={riskTone(operation.risk)}>{riskName(operation.risk, t)}</Pill>{entry.basis && entry.basis !== 'allow' && <> <Pill>{t('经你批准', 'You approved')}</Pill></>}{review && <> <Pill tone={review.fallback ? 'warn' : verdictTone(review.verdict)}>{review.fallback ? t('审核不可用', 'Review unavailable') : reviewSource(review.source, t) + '：' + verdictName(review.verdict, t)}</Pill></>}{!!repeats && <> <Pill>{t('另 ' + repeats + ' 次相同读取', repeats + ' more identical reads')}</Pill></>} · {time}</div>
      <small className="cx-muted cx-sr">#{entry.index} · {operation.kind} · {entry.status ?? ''} · {entry.transport === 'e2e' ? t('端到端加密', 'End-to-end encrypted') : entry.transport === 'job' ? t('本机记录', 'Recorded here') : t('旧版令牌', 'Legacy token')}</small>
    </li>;
  }
  const revoked = entry.event === 'denied' && entry.result === 'revoked';
  const reason = entry.event === 'refused' ? refusalReason(entry.result, t) : '';
  const grant = entry.event === 'approved' && entry.result.includes(':') ? scopeName(entry.result.split(':')[1], t) : '';
  return <li>
    <div className="cx-between"><span><strong>{who}</strong> · {entry.capability ? capabilityName(entry.capability, t) : revoked ? t('收回授权', 'Withdrew an approval') : entry.event === 'policy_changed' && entry.basis === 'review' ? t('自动审核', 'Automatic review') : '—'}{operation && <> · {operationName(operation.kind, t)}</>}</span>{details}</div>
    {operation?.summary && <code className="cx-access-op">{operation.summary}</code>}
    <div style={{ fontVariantNumeric: 'tabular-nums' }}><Pill tone={revoked ? '' : entry.result === 'allow' || entry.event === 'approved' ? 'good' : entry.result === 'deny' || entry.event === 'denied' || entry.event === 'refused' ? 'bad' : ''}>{revoked ? t('已撤销', 'Revoked') : eventName(entry.event, t)}</Pill>{grant && <> {grant}</>}{entry.review && entry.event === 'request_created' && <> <Pill tone={verdictTone(entry.review.verdict)}>{entry.review.fallback ? t('审核不可用', 'Review unavailable') : verdictName(entry.review.verdict, t)}</Pill></>}{reason && <> {reason}</>} · {time}</div>
    <small className="cx-muted cx-sr">#{entry.index} · {entry.result}{entry.basis && ' · ' + entry.basis}</small>
  </li>;
}
function bytes(value: number | undefined) {
  if (value === undefined) return '';
  return value < 1024 ? value + ' B' : value < 1048576 ? (value / 1024).toFixed(1) + ' KB' : (value / 1048576).toFixed(1) + ' MB';
}
function DetailRows({ detail }: { detail: OperationDetail }) {
  const { t } = useApp();
  const rows: [string, React.ReactNode][] = [];
  if (detail.argv?.length) rows.push([t('命令与参数', 'Command and arguments'), <code className="cx-access-op">{detail.argv.map(arg => /\s/.test(arg) ? JSON.stringify(arg) : arg).join(' ')}</code>]);
  if (detail.cwd) rows.push([t('工作目录', 'Working directory'), <code>{detail.cwd}</code>]);
  if (detail.env_keys?.length) rows.push([t('环境变量（只记名称）', 'Environment (names only)'), detail.env_keys.join(', ')]);
  if (detail.timeout_secs) rows.push([t('超时', 'Timeout'), detail.timeout_secs + ' s']);
  if (detail.paths?.length) rows.push([t('文件', 'Files'), <code>{detail.paths.join('\n')}</code>]);
  if (detail.bytes !== undefined) rows.push([t('大小', 'Size'), bytes(detail.bytes)]);
  if (detail.space) rows.push([t('虚拟空间', 'Virtual space'), detail.space]);
  if (detail.url) rows.push([t('网址', 'URL'), <code>{detail.url}</code>]);
  if (detail.target) rows.push([t('对象', 'Target'), detail.target]);
  if (detail.findings?.length) rows.push([t('命中的安全规则', 'Safety rules matched'), detail.findings.map(code => ruleName(code, t)).join('、')]);
  if (detail.exit_code !== undefined) rows.push([t('退出码', 'Exit code'), String(detail.exit_code)]);
  if (detail.duration_ms !== undefined) rows.push([t('耗时', 'Duration'), detail.duration_ms < 1000 ? detail.duration_ms + ' ms' : (detail.duration_ms / 1000).toFixed(1) + ' s']);
  if (detail.stdout_bytes !== undefined || detail.stderr_bytes !== undefined) rows.push([t('输出大小', 'Output size'), t('标准输出 ', 'stdout ') + bytes(detail.stdout_bytes ?? 0) + ' · ' + t('错误输出 ', 'stderr ') + bytes(detail.stderr_bytes ?? 0)]);
  if (detail.stdout_tail) rows.push([t('输出末尾', 'End of output'), <code className="cx-access-op">{detail.stdout_tail}</code>]);
  if (detail.stderr_tail) rows.push([t('错误输出末尾', 'End of errors'), <code className="cx-access-op">{detail.stderr_tail}</code>]);
  if (detail.error) rows.push([t('错误', 'Error'), detail.error]);
  if (detail.repeats) rows.push([t('合并的相同读取', 'Identical reads folded in'), String(detail.repeats)]);
  if (detail.job) rows.push([t('任务编号', 'Job'), <code>{detail.job}</code>]);
  return <>{rows.map(([label, value]) => <div className="cx-access-row" key={label}><dt>{label}</dt><dd>{value}</dd></div>)}</>;
}
function ReviewBox({ review }: { review: Review }) {
  const { t } = useApp();
  return <div className={'cx-access-review ' + (review.fallback ? 'warn' : verdictTone(review.verdict))}>
    <div className="cx-between"><strong>{review.fallback ? t('自动审核不可用', 'Automatic review unavailable') : reviewSource(review.source, t) + ' · ' + verdictName(review.verdict, t)}</strong>{!review.fallback && <span className="cx-muted">{t('风险 ', 'Risk ') + review.risk + '/100'}</span>}</div>
    <p>{review.reason}</p>
    {!!review.labels?.length && <div>{review.labels.map(label => <Pill key={label}>{labelName(label, t)}</Pill>)}</div>}
    {(review.model || review.latency_ms !== undefined) && <small className="cx-muted">{[review.model, review.latency_ms !== undefined ? review.latency_ms + ' ms' : ''].filter(Boolean).join(' · ')}</small>}
  </div>;
}
function EntryDetail({ entry, who, close }: { entry: AuditEntry; who: string; close: () => void }) {
  const { t } = useApp(), title = useId(), dialog = useRef<HTMLDialogElement>(null), operation = entry.operation;
  useEffect(() => { const element = dialog.current, restore = document.activeElement as HTMLElement | null; element?.showModal(); return () => { element?.close(); if (restore?.isConnected) restore.focus(); }; }, []);
  return <dialog ref={dialog} className="cx-sheet cx-access-dialog cx-access-drawer" aria-labelledby={title} onCancel={event => { event.preventDefault(); close(); }} onClick={event => { if (event.target === dialog.current) close(); }}>
    <div className="cx-sheet-head"><h2 id={title}>{operation ? operationName(operation.kind, t) : eventName(entry.event, t)}</h2><button className="cx-btn small" onClick={close} aria-label={t('关闭', 'Close')}>{t('关闭', 'Close')}</button></div>
    <div className="cx-sheet-body">
      {operation?.summary && <code className="cx-access-op">{operation.summary}</code>}
      <dl className="cx-access-rows">
        <div className="cx-access-row"><dt>{t('时间', 'Time')}</dt><dd>{new Date(entry.at * 1000).toLocaleString()}</dd></div>
        <div className="cx-access-row"><dt>{t('来自', 'From')}</dt><dd>{who}{entry.from && entry.from !== who && <small className="cx-muted cx-mono"> {entry.from}</small>}</dd></div>
        {entry.capability && <div className="cx-access-row"><dt>{t('能力', 'Capability')}</dt><dd>{capabilityName(entry.capability, t)}</dd></div>}
        {operation && <div className="cx-access-row"><dt>{t('风险', 'Risk')}</dt><dd><Pill tone={riskTone(operation.risk)}>{riskName(operation.risk, t)}</Pill>{operation.confirm && <> {t('每次都需确认', 'Always confirmed')}</>}</dd></div>}
        <div className="cx-access-row"><dt>{t('结果', 'Result')}</dt><dd>{entry.event === 'operation' ? outcomeName(entry, t) : eventName(entry.event, t)}{entry.status ? ' · HTTP ' + entry.status : ''}</dd></div>
        {entry.basis && <div className="cx-access-row"><dt>{t('放行依据', 'Allowed by')}</dt><dd>{basisName(entry, t)}{entry.basis !== 'allow' && <small className="cx-muted cx-mono"> {entry.basis}</small>}</dd></div>}
        {entry.detail && <DetailRows detail={entry.detail} />}
        {entry.transport && <div className="cx-access-row"><dt>{t('通道', 'Channel')}</dt><dd>{entry.transport === 'e2e' ? t('端到端加密', 'End-to-end encrypted') : entry.transport === 'job' ? t('本机记录', 'Recorded here') : t('旧版令牌', 'Legacy token')}</dd></div>}
        <div className="cx-access-row"><dt>{t('记录编号', 'Entry')}</dt><dd className="cx-mono">#{entry.index}{entry.prev && <small className="cx-muted"> · {t('上一条哈希 ', 'prev ') + entry.prev.slice(0, 12)}…</small>}</dd></div>
      </dl>
      {entry.review && <ReviewBox review={entry.review} />}
    </div>
  </dialog>;
}
function refusalReason(result: string, t: (zh: string, en: string) => string) {
  return ({ approval_pending: t('已有请求在等你批准', 'a request is already waiting for you'), cooldown: t('刚被拒绝，稍后才能再请求', 'recently denied, cooling down'), off: t('此能力已关闭', 'this capability is off') } as Record<string, string>)[result] || '';
}
function eventName(event: string, t: (zh: string, en: string) => string) {
  return ({ session_started: t('开始使用', 'Session started'), session_ended: t('结束使用', 'Session ended'), request_created: t('请求批准', 'Approval requested'), approved: t('已批准', 'Approved'), denied: t('已拒绝', 'Denied'), expired: t('已过期', 'Expired'), policy_changed: t('访问设置已更改', 'Policy changed'), refused: t('访问被阻止', 'Access refused'), emergency_disconnect: t('全部断开', 'Disconnected'), checkpoint: t('归档校验点', 'Archive checkpoint'), operation: t('远程操作', 'Remote operation') } as Record<string, string>)[event] || event;
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
  async function decideAll(scope?: string) {
    setBusy(true); setError('');
    const done: string[] = [];
    try { for (const request of pending) { await api.call('POST', '/access/requests/' + encodeURIComponent(request.id), scope ? { decision: 'approve', scope } : { decision: 'deny' }); done.push(request.id); } }
    catch (problem) { setError(problem instanceof Error ? problem.message : String(problem)); }
    finally { setDecided(ids => [...ids, ...done]); await refresh(); setBusy(false); }
  }
  const sessions = new Map<string, NonNullable<typeof access>['sessions']>();
  for (const session of access?.sessions || []) sessions.set(session.from, [...(sessions.get(session.from) || []), session]);
  return <div className="cx-access-surfaces">
    {accessError && <Alert action={<button className="cx-btn small" onClick={() => void refresh()}>{t('重试', 'Retry')}</button>}>{t('无法读取访问状态：', 'Unable to read access status: ')}{accessError}</Alert>}
    {pending.length > 0 && <div className="cx-access-banner" role="status">{t(pending.length + ' 个访问请求等待你批准', pending.length + ' access request(s) need your approval')}</div>}
    {!!sessions.size && <div className="cx-access-banner" role="status"><span>{[...sessions].map(([node, items]) => t(displayName(node) + ' 正在使用 ', displayName(node) + ' is using ') + [...new Set(items.map(item => capabilityName(item.capability, t)))].join(' · ')).join('；')}</span><div className="cx-actions"><button className="cx-btn small danger" disabled={busy} onClick={async () => { setBusy(true); try { await api.call('POST', '/access/revoke', {}); await refresh(); } catch (problem) { setError(String(problem)); } finally { setBusy(false); } }}>{t('全部断开', 'Disconnect all')}</button><Link className="cx-btn small" to="/app/access-log">{t('操作记录', 'Operation log')}</Link></div></div>}
    {error && !pending.length && <Alert>{error}</Alert>}
    <RequesterStatus />
    {pending[0] && <ApprovalDialog key={pending[0].id} request={pending[0]} name={displayName(pending[0].from) || pending[0].name} left={Math.max(0, Math.ceil(pending[0].expires_at - now / 1000))} busy={busy} error={error} decide={scope => void decide(pending[0], scope)} others={pending.slice(1)} decideAll={scope => void decideAll(scope)} />}
  </div>;
}
function ApprovalDialog({ request, name, left, busy, error, decide, others, decideAll }: { request: AccessRequest; name: string; left: number; busy: boolean; error: string; decide: (scope?: string) => void; others: AccessRequest[]; decideAll: (scope?: string) => void }) {
  const { t } = useApp(), title = useId(), dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => { const element = dialog.current, restore = document.activeElement as HTMLElement | null; element?.showModal(); return () => { element?.close(); if (restore?.isConnected) restore.focus(); }; }, []);
  const peerLabel = formatDeviceLabel(name, request.from, t), operation = request.operation, review = request.review, detail = request.detail;
  const flagged = !!review && !review.fallback && review.verdict !== 'allow';
  const unchecked = !!review?.fallback;
  const once = !!operation?.confirm || flagged || unchecked && operation?.risk === 'danger';
  const heading = operation ? t(peerLabel.label + ' 想在本设备上' + operationName(operation.kind, t), peerLabel.label + ' wants to ' + operationName(operation.kind, t).toLowerCase() + ' on this device') : t(peerLabel.label + ' 请求使用 ' + capabilityName(request.capability, t), peerLabel.label + ' requests ' + capabilityName(request.capability, t));
  const scopes = once ? [['session', t('允许这一次', 'Allow this once')]] as const : [['session', t('允许本次', 'Allow once')], ['1h', t('允许 1 小时', 'Allow 1 hour')], ['today', t('今天内允许', 'Allow today')], ['7d', t('允许 7 天', 'Allow 7 days')]] as const;
  const batchable = others.length > 0 && !once && others.every(item => !item.operation?.confirm && !(item.review && item.review.verdict !== 'allow' && !item.review.fallback));
  const extra = detail && (detail.argv?.length || detail.cwd || detail.paths?.length || detail.env_keys?.length || detail.url || detail.findings?.length);
  return <dialog ref={dialog} className="cx-sheet cx-access-dialog" aria-labelledby={title} onCancel={event => event.preventDefault()} onKeyDown={event => {
    if (event.key !== 'Tab') return;
    const buttons = dialog.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)');
    if (!buttons?.length) { event.preventDefault(); return; }
    const first = buttons[0], last = buttons[buttons.length - 1];
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
  }}>
    <div className="cx-sheet-head">
      <div className="cx-access-head-title">
        <h2 id={title}>{heading}</h2>
        <div className="cx-access-peer-meta">
          {peerLabel.shortId && <span className="cx-id-pill">{peerLabel.shortId}</span>}
          <span className="cx-sr">{request.from}</span>
        </div>
      </div>
      <span className="cx-timer-pill" role="timer">{t('剩余 ' + left + ' 秒', left + ' seconds left')}</span>
    </div>
    <div className="cx-sheet-body">
      {operation && <div className="cx-access-op-box">
        <div className="cx-between"><span className="cx-muted">{capabilityName(request.capability, t)}</span><Pill tone={riskTone(operation.risk)}>{riskName(operation.risk, t)}</Pill></div>
        {operation.summary && <code className="cx-access-op">{operation.summary}</code>}
        {extra && <details className="cx-access-more-detail"><summary>{t('查看完整内容', 'Show everything')}</summary><dl className="cx-access-rows"><DetailRows detail={detail!} /></dl></details>}
      </div>}
      {review && <ReviewBox review={review} />}
      {flagged ? <Alert tone="warn">{t('自动审核认为这个操作需要你亲自确认，所以批准只对这一次有效，之前的授权不会扩大到它。不认识就拒绝。', 'The automatic review wants you to confirm this one yourself, so an approval covers only this call and earlier approvals do not extend to it. If you do not recognise it, deny it.')}</Alert>
        : once ? <Alert tone="warn">{t('这个操作可能删除数据、获得管理员权限或长期留下访问入口，所以每次都要你确认，批准只对这一次有效。不认识就拒绝。', 'This could delete data, gain admin rights or leave lasting access, so it asks every time and an approval covers only this one call. If you do not recognise it, deny it.')}</Alert>
        : <p className="cx-muted">{operation && request.capability !== 'messages' ? t('「允许本次」只放行这一个操作；选择 1 小时、今天或 7 天后，对方在这段时间内可以继续使用「' + capabilityName(request.capability, t) + '」而不再询问（危险操作仍会询问）。', '“Allow once” lets only this operation through. With 1 hour, today or 7 days, the device can keep using ' + capabilityName(request.capability, t) + ' without asking until then (dangerous operations still ask).') : t('仅在你同意后，对方才能在本设备上使用此功能。', 'This device can use this function here only after you approve.')}</p>}
      {error && <Alert>{error}</Alert>}
      <div className="cx-actions cx-approval-actions">
        <button autoFocus className="cx-btn cx-approval-deny" disabled={busy} onClick={() => decide()}>{t('拒绝', 'Deny')}</button>
        {scopes.map(([scope, label]) => (
          <button className={scope === 'session' ? 'cx-btn primary' : 'cx-btn'} key={scope} disabled={busy} onClick={() => decide(scope)}>{label}</button>
        ))}
      </div>
      {others.length > 0 && <div className="cx-access-batch">
        <span className="cx-muted">{t('另有 ' + others.length + ' 个请求等待处理', others.length + ' more request(s) waiting')}</span>
        <div className="cx-actions">
          <button className="cx-btn small danger" disabled={busy} onClick={() => decideAll()}>{t('全部拒绝', 'Deny all')}</button>
          {batchable && <button className="cx-btn small" disabled={busy} onClick={() => decideAll('1h')}>{t('全部允许 1 小时', 'Allow all for 1 hour')}</button>}
        </div>
      </div>}
    </div>
  </dialog>;
}
