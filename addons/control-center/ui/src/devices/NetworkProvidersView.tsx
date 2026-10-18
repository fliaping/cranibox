import { useCallback, useEffect, useId, useRef, useState, type FormEvent, type MouseEvent } from 'react';
import { Check, Download, Eye, EyeOff, Network, RefreshCw, ShieldCheck, X } from 'lucide-react';
import './devices.css';
import './networks.css';
import { InvitationEnrollment, NetworkInvitations, type Invitation } from './NetworkInvitations';
import { TailscaleNodes, type TailscaleDevice } from './TailscaleNodes';
import type { TailscaleDiscoveryState } from './TailscaleDiscovery';
import { TailscaleSetup, type TailscaleSetupState, type TailscaleSetupPlan } from './TailscaleSetup';

export type NetworkCommand = { operation: 'status' | 'plan' | 'install' | 'easy_tier_stop' | 'tailscale_login' | 'tailscale_cancel_login' | 'tailscale_setup_plan' }
  | { operation: 'tailscale_setup'; plan_id: string }
  | { operation: 'easy_tier_join'; network: string; token: string; peers: string[]; listen_port: number | null }
  | { operation: 'easy_tier_enroll'; url: string; token: string; listen_port: number | null }
  | { operation: 'easy_tier_invite'; peers: string[] }
  | { operation: 'easy_tier_revoke_invitation'; id: string }
  | { operation: 'easy_tier_connect'; node: string; token: string }
  | { operation: 'tailscale_discover'; address: string; port: number }
  | { operation: 'tailscale_discovery'; ports: number[] }
  | { operation: 'tailscale_connect'; address: string; port: number; node: string; token: string };
export type NetworkClient = (command: NetworkCommand) => Promise<unknown>;
type Device = { node: string; address: string; path: string; latency_ms: number | null };
type State = { node_id: string; easytier: { installed: boolean; enabled: boolean; version: string; phase: string; network: string | null; peer: string | null; peers?: string[]; listen_port: number | null; error: string | null; devices: Device[]; connections: string[]; invitations?: Invitation[]; invitation_error?: string | null }; tailscale: { installed?: boolean; state: string; login_pending?: boolean; login_url?: string | null; error?: string; devices: TailscaleDevice[]; discovery?: TailscaleDiscoveryState; setup?: TailscaleSetupState } };
type Plan = { version: string; platform: string; directory: string; sha256: string };
const message = (error: unknown) => error instanceof Error ? error.message : String(error);
function state(value: unknown): State {
  const result = value as State;
  if (!result?.easytier || !Array.isArray(result.easytier.devices) || !result.tailscale || !Array.isArray(result.tailscale.devices)) throw Error('Network management is unavailable. Upgrade the node backend and retry.');
  return result;
}
function loginUrl(value?: string | null) {
  try { const url = new URL(value || ''); return url.protocol === 'https:' && url.hostname === 'login.tailscale.com' && !url.port && !url.username && !url.password && !url.hash ? url.href : undefined; } catch { return undefined; }
}

/** Browser and Tauri control the same node-owned provider service. Never persist form secrets. */
export function NetworkProvidersView({ client, language = 'en', host = 'local', openExternal }: { client: NetworkClient; language?: string; host?: 'local' | 'server'; openExternal?: (url: string) => Promise<unknown> }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const [status, setStatus] = useState<State>(), [error, setError] = useState(''), [notice, setNotice] = useState('');
  const [readError, setReadError] = useState('');
  const [busy, setBusy] = useState(false), [loading, setLoading] = useState(true), [plan, setPlan] = useState<Plan>();
  const [peer, setPeer] = useState(''), [network, setNetwork] = useState('cranibox'), [token, setToken] = useState(''), [showToken, setShowToken] = useState(false);
  const [incoming, setIncoming] = useState(false), [port, setPort] = useState('11010');
  const [joinMode, setJoinMode] = useState<'invitation' | 'manual'>('invitation');
  const [entryError, setEntryError] = useState('');
  const entryId = useId(), entryField = useRef<HTMLTextAreaElement>(null);
  const [target, setTarget] = useState(''), [accessToken, setAccessToken] = useState('');
  const mounted = useRef(true), mutation = useRef(false), reading = useRef(false), serial = useRef(0);
  const tokenField = useRef<HTMLInputElement>(null), accessField = useRef<HTMLInputElement>(null);
  const read = useCallback(async () => {
    if (reading.current || mutation.current || document.hidden) return;
    reading.current = true; const current = ++serial.current;
    try { const result = state(await client({ operation: 'status' })); if (mounted.current && current === serial.current) { setStatus(result); setReadError(''); } }
    catch (e) { if (mounted.current && current === serial.current) setReadError(message(e)); }
    finally { reading.current = false; if (mounted.current && current === serial.current) setLoading(false); }
  }, [client]);
  useEffect(() => {
    mounted.current = true; void read();
    const timer = setInterval(() => void read(), 5000);
    const resume = () => { if (!document.hidden) void read(); }; document.addEventListener('visibilitychange', resume);
    return () => { mounted.current = false; ++serial.current; clearInterval(timer); document.removeEventListener('visibilitychange', resume); };
  }, [read]);
  useEffect(() => { if (target) accessField.current?.focus(); }, [target]);
  async function run(command: NetworkCommand, done?: (value: unknown) => void) {
    if (mutation.current) return;
    mutation.current = true; ++serial.current; setBusy(true); setError(''); setNotice('');
    try { const value = await client(command); if (mounted.current) { if (!['plan', 'tailscale_setup_plan', 'easy_tier_connect', 'easy_tier_invite'].includes(command.operation)) setStatus(state(value)); done?.(value); } }
    catch (e) { if (mounted.current) setError(message(e)); }
    finally { mutation.current = false; if (mounted.current) { setBusy(false); setLoading(false); } }
  }
  async function join(event: FormEvent) {
    event.preventDefault();
    const peers = peer.split(/\r?\n/).map(value => value.trim()).filter(Boolean);
    let problem = peers.length > 8 ? t('最多填写 8 个网络入口。', 'Use at most 8 network entries.') : !peers.length && !incoming ? t('填写至少一个网络入口，或在下方启用入站连接。', 'Enter at least one network entry or enable incoming access below.') : '';
    const seen = new Set<string>();
    for (const [index, address] of peers.entries()) {
      if (problem) break;
      try {
        const url = new URL(address), tcp = ['tcp:', 'udp:'].includes(url.protocol);
        if (!['tcp:', 'udp:', 'ws:', 'wss:'].includes(url.protocol) || !url.hostname || (tcp && !url.port) || url.port === '0' || url.username || url.password || url.search || url.hash || new TextEncoder().encode(address).length > 2048 || (tcp && url.pathname && url.pathname !== '/')) throw Error();
        if (tcp && url.pathname === '/') url.pathname = '';
        if (seen.has(url.href)) { problem = t('存在重复的网络入口，请删除重复地址。', 'Remove duplicate network entries.'); break; }
        seen.add(url.href);
      } catch { problem = t('第 ' + (index + 1) + ' 个地址无效。使用 tcp://、udp://、ws:// 或 wss:// 地址；不要包含凭据或查询参数。', 'Entry ' + (index + 1) + ' is invalid. Use tcp://, udp://, ws:// or wss:// without credentials or query parameters.'); }
    }
    setEntryError(problem);
    if (problem) { entryField.current?.focus(); return; }
    await run({ operation: 'easy_tier_join', network: network.trim(), token, peers, listen_port: incoming ? Number(port) : null }, () => { setToken(''); setShowToken(false); setNotice(t('连接配置已保存，后台正在建立网络。', 'Connection saved. The background service is joining the network.')); });
  }
  async function connect(event: FormEvent) {
    event.preventDefault();
    await run({ operation: 'easy_tier_connect', node: target, token: accessToken }, value => {
      setStatus(state((value as { network: unknown }).network)); setAccessToken(''); setTarget('');
      setNotice(t('已保存设备连接，可在下方检查能力、打开终端或到远程桌面页面访问。', 'Device connection saved. Check its capabilities below, open a terminal or visit Remote desktops.'));
      window.dispatchEvent(new Event('cranibox:peers-changed'));
      window.dispatchEvent(new Event('cranibox:network-routes-changed'));
    });
  }
  const et = status?.easytier, ts = status?.tailscale;
  const phase = et?.phase === 'running' ? t('后台运行中', 'Running in background') : et?.phase === 'installing' ? t('正在安装…', 'Installing…') : et?.phase === 'restarting' ? t('正在连接…', 'Connecting…') : et?.phase === 'failed' ? t('需要处理', 'Needs attention') : t('未连接', 'Disconnected');
  const auth = loginUrl(ts?.login_url);
  function openLink(event: MouseEvent<HTMLAnchorElement>, url: string) {
    if (!openExternal) return;
    event.preventDefault();
    void openExternal(url).catch(e => { if (mounted.current) setError(message(e)); });
  }
  return <section className="direct-devices network-providers" aria-label={t('设备组网', 'Device networks')}>
    <div className="direct-heading"><div><h2>{t('设备组网', 'Device networks')}</h2><p>{host === 'server' ? t('在当前 Workspace 主机上管理网络，浏览器所在设备不会自动加入。', 'Manage networking on this Workspace host. Your browser’s device does not join automatically.') : t('让这台设备加入可信设备网络，网页关闭后后台继续连接。', 'Join a network of trusted devices. The background connection continues when this view closes.')}</p></div><button disabled={busy || loading} onClick={() => void read()} aria-label={t('刷新设备网络', 'Refresh device networks')}><RefreshCw size={16} aria-hidden="true" /></button></div>
    {loading && <p role="status">{t('正在读取网络状态…', 'Loading network status…')}</p>}
    {(error || readError) && <p className="direct-error" role="alert">{error || readError}</p>}
    {notice && <p className="direct-note" role="status"><Check size={16} aria-hidden="true" />{notice}</p>}
    {status && <div className="network-grid">
      <article className="network-provider" aria-label="EasyTier">
        <div className="direct-heading"><div><h3><Network size={18} aria-hidden="true" />EasyTier</h3><p>{t('自部署 · 对等连接 · 无需虚拟网卡', 'Self hosted · Peer connections · No virtual adapter')}</p></div><span className={'network-phase ' + (et?.phase === 'running' ? 'active' : '')} role="status">{phase}</span></div>
        {et?.error && <p className="direct-error" role="alert">{et.error}</p>}
        {!et?.installed && et?.phase !== 'installing' && <div className="network-setup"><p className="direct-note">{t('先安装经过校验的 EasyTier 程序，安装到当前节点的私有目录，无需管理员权限。', 'Install verified EasyTier binaries in this node’s private directory. Administrator access is not required.')}</p>
          {!plan ? <button disabled={busy} onClick={() => void run({ operation: 'plan' }, value => setPlan(value as Plan))}><Download size={16} aria-hidden="true" />{t('检查安装方案', 'Review installation')}</button> : <div className="network-plan"><strong>EasyTier {plan.version} · {plan.platform}</strong><p className="direct-note">{t('安装位置', 'Install location')}<br /><code>{plan.directory}</code></p><p className="direct-note">SHA-256<br /><code>{plan.sha256}</code></p><p className="direct-note">{t('从官方 GitHub Release 下载；不创建 TUN、不修改系统路由或 DNS。', 'Downloads the official GitHub release. No TUN adapter, system route or DNS changes.')}</p><div className="direct-actions"><button disabled={busy} className="direct-primary" onClick={() => void run({ operation: 'install' }, () => setPlan(undefined))}>{busy ? t('正在提交…', 'Submitting…') : t('下载并安装', 'Download and install')}</button><button disabled={busy} onClick={() => setPlan(undefined)}>{t('取消', 'Cancel')}</button></div></div>}
        </div>}
        {et?.installed && !et.enabled && <><div className="network-method direct-actions" role="group" aria-label={t('连接方式', 'Connection method')}><button type="button" aria-pressed={joinMode === 'invitation'} disabled={busy} onClick={() => setJoinMode('invitation')}>{t('使用邀请', 'Use invitation')}</button><button type="button" aria-pressed={joinMode === 'manual'} disabled={busy} onClick={() => setJoinMode('manual')}>{t('手动填写地址', 'Enter addresses manually')}</button></div>
          {joinMode === 'invitation' ? <InvitationEnrollment busy={busy} t={t} submit={(url, token, done) => run({ operation: 'easy_tier_enroll', url, token, listen_port: null }, () => { done(); setNotice(t('已加入网络，后台正在连接设备。', 'Network joined. The background service is connecting to devices.')); })} /> : <form onSubmit={join}>
          <div className="network-entry"><label htmlFor={entryId}>{t('网络入口', 'Network entry addresses')}</label><textarea id={entryId} ref={entryField} rows={3} required={!incoming} value={peer} maxLength={17000} spellCheck={false} autoCapitalize="off" autoCorrect="off" aria-invalid={!!entryError} aria-describedby={entryId + '-help' + (entryError ? ' ' + entryId + '-error' : '')} placeholder={'tcp://device.example.com:11010\nwss://backup.example.com/mesh'} onChange={e => { setPeer(e.target.value); setEntryError(''); }} /></div>
          <small id={entryId + '-help'} className="direct-note network-entry-help">{t('每行一个地址，最多 8 个。使用多台在线成员作为入口，单个入口离线后仍可通过其他成员重新接入。首台设备可在下方开启入站端口。', 'One address per line, up to 8. Add several online members so another can reconnect you if an entry goes offline. The first device can enable an incoming port below.')}</small>
          {entryError && <p id={entryId + '-error'} className="direct-error" role="alert">{entryError}</p>}
          <label>{t('网络 Token', 'Network token')}<span className="network-secret"><input ref={tokenField} type={showToken ? 'text' : 'password'} required minLength={32} maxLength={256} autoComplete="off" value={token} onChange={e => setToken(e.target.value)} /><button type="button" disabled={busy} aria-label={showToken ? t('隐藏网络 Token', 'Hide network token') : t('显示网络 Token', 'Show network token')} onClick={() => setShowToken(v => !v)}>{showToken ? <EyeOff size={16} aria-hidden="true" /> : <Eye size={16} aria-hidden="true" />}</button></span><small className="direct-note">{t('同一网络的设备使用相同 Token，只分享给可信设备；访问能力由目标设备单独授权。', 'Members share this token. Share it with trusted devices; each target grants its own capabilities.')}</small></label>
          <details><summary>{t('网络名称与入站设置', 'Network name and incoming access')}</summary><div className="network-advanced"><label>{t('网络名称', 'Network name')}<input required value={network} pattern="[A-Za-z0-9_-]{1,64}" maxLength={64} onChange={e => setNetwork(e.target.value)} /></label><label className="direct-check"><input type="checkbox" checked={incoming} onChange={e => setIncoming(e.target.checked)} />{t('允许其他设备连接此节点（TCP / UDP）', 'Accept incoming peers on this node (TCP / UDP)')}</label>{incoming && <label>{t('入站端口', 'Incoming port')}<input type="number" required min={1024} max={65535} value={port} onChange={e => setPort(e.target.value)} /><small className="direct-note">{t('在所有网卡监听此端口。公网接入还需配置主机防火墙或路由器。', 'Listens on every interface at this port. Public access also needs host firewall or router configuration.')}</small></label>}<button type="button" disabled={busy} onClick={() => { const bytes = crypto.getRandomValues(new Uint8Array(32)); setToken(Array.from(bytes, b => b.toString(16).padStart(2, '0')).join('')); setShowToken(true); tokenField.current?.focus(); }}>{t('生成新网络 Token', 'Generate new network token')}</button></div></details>
          <button className="direct-primary" type="submit" disabled={busy}>{busy ? t('正在保存…', 'Saving…') : t('连接 EasyTier', 'Connect EasyTier')}</button>
        </form>}</>}
        {et?.enabled && <><div className="network-saved direct-note"><p>{t('网络：', 'Network: ')}<strong>{et.network}</strong></p>{(et.peers ?? (et.peer ? [et.peer] : [])).map(address => <code key={address}>{address}</code>)}{et.listen_port && <p>{t('入站端口：', 'Incoming port: ')}{et.listen_port} TCP / UDP</p>}</div><div className="direct-actions"><button disabled={busy} onClick={() => { if (confirm(t('断开 EasyTier 网络？已授权设备的终端和桌面连接将断开，目标 Shell 会继续运行。', 'Disconnect EasyTier? Connections to authorized devices will close; target shells continue running.'))) void run({ operation: 'easy_tier_stop' }); }}>{t('断开 EasyTier', 'Disconnect EasyTier')}</button></div>
          <div className="network-discovery"><h4>{t('发现的节点', 'Discovered nodes')} <span>{et.devices.length}</span></h4>{!et.devices.length ? <p className="direct-note">{t('正在等待其他 Cranibox 节点加入相同网络。', 'Waiting for other Cranibox nodes on the same network.')}</p> : <ul>{et.devices.map(device => <li key={device.node}><div><strong>{device.node}</strong><small>{device.path === 'direct' ? t('点对点', 'Peer to peer') : t('经网络中继', 'Via network relay')}{device.latency_ms != null ? ' · ' + device.latency_ms.toFixed(1) + ' ms' : ''}{et.connections.includes(device.node) ? t(' · 已保存连接', ' · Connection saved') : ''}</small></div><button disabled={busy} onClick={() => { setAccessToken(''); setTarget(device.node); }}>{t('接入', 'Connect')}</button></li>)}</ul>}</div>
          {et.invitation_error && <p className="direct-error" role="alert">{et.invitation_error}</p>}
          {host === 'server' && <NetworkInvitations entries={et.peers ?? (et.peer ? [et.peer] : [])} listenPort={et.listen_port} invitations={et.invitations} busy={busy} t={t} create={(peers, done) => run({ operation: 'easy_tier_invite', peers }, value => { const result = value as { network: State; invitation: Parameters<typeof done>[0] }; setStatus(state(result.network)); done(result.invitation); })} revoke={id => run({ operation: 'easy_tier_revoke_invitation', id })} />}
        </>}
        {!!target && <form className="network-target" onSubmit={connect}><div className="direct-heading"><h4>{t('接入节点', 'Connect node')} {target}</h4><button type="button" disabled={busy} aria-label={t('关闭节点接入', 'Close node connection')} onClick={() => { setTarget(''); setAccessToken(''); }}><X size={16} aria-hidden="true" /></button></div><p className="direct-note">{t('请让目标节点在“入站授权”中为以下 ID 授权：', 'Ask the target to grant this ID access in Incoming access: ')}<code>{status.node_id}</code></p><label>{t('目标访问 Token', 'Target access token')}<input ref={accessField} required pattern="[A-Fa-f0-9]{64}" maxLength={64} type="password" autoComplete="off" value={accessToken} onChange={e => setAccessToken(e.target.value)} /></label><button className="direct-primary" type="submit" disabled={busy}>{busy ? t('正在保存…', 'Saving…') : t('保存设备连接', 'Save device connection')}</button></form>}
      </article>
      <article className="network-provider" aria-label="Tailscale"><div className="direct-heading"><div><h3><ShieldCheck size={18} aria-hidden="true" />Tailscale</h3><p>{t('使用系统服务 · 可与 EasyTier 同时运行', 'Uses the system service · Works alongside EasyTier')}</p></div><span className={'network-phase ' + (ts?.state === 'Running' ? 'active' : '')} role="status">{ts?.state === 'Running' ? t('已连接', 'Connected') : ts?.state === 'NeedsLogin' ? t('等待登录', 'Login required') : ts?.state === 'NotInstalled' ? t('尚未安装', 'Not installed') : ts?.state === 'Checking' ? t('检查中…', 'Checking…') : t('检查系统服务', 'Check system service')}</span></div>
        {ts?.error && <p className="direct-error" role="alert">{ts.error}</p>}
        <TailscaleSetup setup={ts?.setup} systemState={ts?.state || 'Checking'} busy={busy || !!ts?.login_pending} t={t} openLink={openLink} review={done => run({ operation: 'tailscale_setup_plan' }, value => done(value as TailscaleSetupPlan))} execute={(plan_id, done) => run({ operation: 'tailscale_setup', plan_id }, done)} />
        {ts?.state === 'NeedsLogin' && !ts.login_pending && <button disabled={busy || ts.setup?.phase === 'running'} onClick={() => void run({ operation: 'tailscale_login' })}>{t('登录 Tailscale', 'Sign in to Tailscale')}</button>}
        {ts?.login_pending && <div className="network-setup"><p className="direct-note">{auth ? t('在 Tailscale 页面完成登录，此处会自动刷新。', 'Complete sign in on Tailscale. Status refreshes automatically.') : t('正在等待 Tailscale 返回登录链接…', 'Waiting for a Tailscale sign in link…')}</p><div className="direct-actions">{auth && <a className="network-link" href={auth} onClick={e => openLink(e, auth)} target="_blank" rel="noreferrer">{t('打开 Tailscale 登录', 'Open Tailscale sign in')}</a>}<button disabled={busy} onClick={() => void run({ operation: 'tailscale_cancel_login' })}>{t('取消登录', 'Cancel sign in')}</button></div></div>}
        {(ts?.state === 'Running' || ts?.discovery?.enabled) && <TailscaleNodes devices={ts.devices} discovery={ts.discovery} onDiscoveryChanged={value => { ++serial.current; setStatus(state(value)); setReadError(''); }} own={status.node_id} client={client} disabled={busy} t={t} onConnected={() => { window.dispatchEvent(new Event('cranibox:peers-changed')); window.dispatchEvent(new Event('cranibox:network-routes-changed')); void read(); }} />}
      </article>
    </div>}
  </section>;
}
