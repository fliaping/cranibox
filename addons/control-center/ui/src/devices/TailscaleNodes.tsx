import { useEffect, useRef, useState, type FormEvent } from 'react';
import { Check, X } from 'lucide-react';
import type { NetworkClient } from './NetworkProvidersView';
import { TailscaleDiscovery, type TailscaleDiscoveryState, type TailscaleIdentity } from './TailscaleDiscovery';

export type TailscaleDevice = { name: string; addresses: string[]; online: boolean; path: string };
type Found = TailscaleIdentity;

/** Look up only the selected peer; capability credentials are a separate explicit step. */
export function TailscaleNodes({ devices, discovery, own, client, disabled, t, onConnected, onDiscoveryChanged }: { devices: TailscaleDevice[]; discovery?: TailscaleDiscoveryState; own: string; client: NetworkClient; disabled: boolean; t: (cn: string, en: string) => string; onConnected: () => void; onDiscoveryChanged: (value: unknown) => void }) {
  const [address, setAddress] = useState(''), [port, setPort] = useState(''), [token, setToken] = useState('');
  const [found, setFound] = useState<Found>(), [error, setError] = useState(''), [busy, setBusy] = useState(false), [notice, setNotice] = useState('');
  const mounted = useRef(true), mutation = useRef(false), portField = useRef<HTMLInputElement>(null), tokenField = useRef<HTMLInputElement>(null), trigger = useRef<HTMLButtonElement | null>(null);
  const device = devices.find(device => device.addresses.includes(address));
  const online = device?.online === true;
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => { if (address && !found) portField.current?.focus(); }, [address, found]);
  useEffect(() => { if (found) tokenField.current?.focus(); }, [found]);
  function reset() { setFound(undefined); setToken(''); setError(''); setNotice(''); }
  function close() { setAddress(''); setPort(''); reset(); trigger.current?.focus(); }
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (mutation.current || disabled || !online) return;
    mutation.current = true; setBusy(true); setError(''); setNotice('');
    try {
      if (!found) {
        const result = await client({ operation: 'tailscale_discover', address, port: Number(port) }) as Found;
        if (typeof result.node_id !== 'string' || result.address !== address || result.port !== Number(port) || typeof result.url !== 'string') throw Error(t('节点返回了无效身份，请更新后端后重试。', 'Invalid node identity. Upgrade the backend and retry.'));
        if (mounted.current) setFound(result);
      } else {
        await client({ operation: 'tailscale_connect', address, port: Number(port), node: found.node_id, token });
        if (mounted.current) {
          close(); setNotice(t('已保存设备连接，可在直连设备中检查权限、打开终端或桌面。', 'Device connection saved. Check its permissions or open a terminal or desktop in Direct devices.')); onConnected();
        }
      }
    } catch (e) { if (mounted.current) setError(e instanceof Error ? e.message : String(e)); }
    finally { mutation.current = false; if (mounted.current) setBusy(false); }
  }
  return <>
    {discovery && <TailscaleDiscovery status={discovery} client={client} disabled={disabled || busy} t={t} onChanged={onDiscoveryChanged} onSelect={(identity, button) => { trigger.current = button; reset(); setAddress(identity.address); setPort(String(identity.port)); setFound(identity); }} />}
    <p className="direct-note">{t('选择在线设备，填写目标 Cranibox 的监听端口以核对 NodeID。网络成员身份不会自动授予能力。', 'Choose an online device and enter its Cranibox listener port to check its NodeID. Network membership does not grant capabilities.')}</p>
    {!devices.length && <p className="direct-note">{t('当前没有可见的 Tailscale 设备。', 'No visible Tailscale devices.')}</p>}
    <ul className="network-ts-devices">{devices.map((device, index) => <li key={device.name + index}><div><strong>{device.name}</strong><small>{device.addresses.join(' · ')}</small><small>{device.online ? t('在线', 'Online') : t('离线', 'Offline')}</small></div><button disabled={disabled || busy || !device.online || !device.addresses.length} onClick={event => { trigger.current = event.currentTarget; reset(); setPort(''); setAddress(device.addresses.find(ip => !ip.includes(':')) ?? device.addresses[0]); }}>{t('查找节点', 'Find node')}</button></li>)}</ul>
    {notice && <p role="status" className="direct-note"><Check size={16} aria-hidden="true" />{notice}</p>}
    {!!address && <form className="network-target" onSubmit={submit} aria-label={t('接入 Tailscale 节点', 'Connect Tailscale node')} aria-busy={busy}>
      <div className="direct-heading"><h4>{device?.name || address}</h4><button type="button" disabled={busy} aria-label={t('关闭 Tailscale 节点接入', 'Close Tailscale node connection')} onClick={close}><X size={16} aria-hidden="true" /></button></div>
      <p className="direct-note">{t('先在目标的“入站授权”开启可由 Tailscale 访问的节点监听，并为以下 ID 授予所需权限：', 'First enable a node listener reachable over Tailscale in the target’s Incoming access, and grant the required capabilities to this ID: ')}<code>{own}</code></p>
      <label>{t('目标 Tailscale 地址', 'Target Tailscale address')}<select value={address} disabled={busy} onChange={event => { setAddress(event.target.value); reset(); }}>{(device?.addresses ?? [address]).map(ip => <option key={ip} value={ip}>{ip}</option>)}</select></label>
      <label>{t('目标节点监听端口', 'Target node listener port')}<input ref={portField} type="number" required min={1} max={65535} inputMode="numeric" disabled={busy} value={port} onChange={event => { setPort(event.target.value); reset(); }} /><small className="direct-note">{t('填写 Cranibox 的 HTTP 端口；需要满足 tailnet 访问策略。', 'Use the Cranibox HTTP port. Tailnet access policy must allow the connection.')}</small></label>
      {!online && <p className="direct-error" role="alert">{t('该设备已离线或不再可见，刷新后再试。', 'This device is offline or no longer visible. Refresh and retry.')}</p>}
      {found && <><div className="network-saved direct-note"><strong>{t('目标节点身份', 'Target node identity')}</strong><code>{found.node_id}</code><code>{found.url}</code></div><label>{t('目标访问 Token', 'Target access token')}<input ref={tokenField} required type="password" autoComplete="off" minLength={64} maxLength={64} pattern="[A-Fa-f0-9]{64}" disabled={busy} value={token} onChange={event => setToken(event.target.value)} /></label><p className="direct-note">{t('已有此设备时，将使用相同目标授权添加备用路径。Token 仅在本机 Rust 后端保存。', 'For an existing device, adds an alternative path using the same target grant. Only the local Rust backend stores the token.')}</p></>}
      {error && <p className="direct-error" role="alert">{error}</p>}
      <div className="direct-actions"><button className="direct-primary" type="submit" disabled={disabled || busy || !online}>{busy ? t('正在处理…', 'Working…') : found ? t('保存设备连接', 'Save device connection') : t('核对节点身份', 'Check node identity')}</button>{found && <button type="button" disabled={busy} onClick={() => { reset(); portField.current?.focus(); }}>{t('重新查找', 'Find again')}</button>}</div>
    </form>}
  </>;
}
