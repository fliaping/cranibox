import { useEffect, useId, useRef, useState, type FormEvent } from 'react';
import { RefreshCw } from 'lucide-react';
import type { NetworkClient } from './NetworkProvidersView';

type Endpoint = { address: string; port: number; url: string };
export type TailscaleIdentity = Endpoint & { node_id: string; name?: string; paths?: Endpoint[] };
export type TailscaleDiscoveryState = { enabled: boolean; port: number | null; ports?: number[]; phase: string; observed_at: number | null; checked: number; skipped: number; ambiguous: number; error: string | null; devices: TailscaleIdentity[] };
type Translate = (cn: string, en: string) => string;

function DiscoveredNode({ device, disabled, t, onSelect }: { device: TailscaleIdentity; disabled: boolean; t: Translate; onSelect: (device: TailscaleIdentity, trigger: HTMLButtonElement) => void }) {
  const id = useId();
  const [selected, setSelected] = useState('');
  const paths = device.paths?.length ? device.paths : [device];
  const path = paths.find(path => path.url === selected) ?? paths[0];
  return <li><div><strong>{device.name || device.address}</strong><small>{device.node_id}</small>
    {paths.length > 1 ? <div className="network-discovered-path"><label htmlFor={id}>{t('连接地址', 'Connection address')}</label><select id={id} value={path.url} disabled={disabled} onChange={event => setSelected(event.target.value)}>{paths.map(path => <option key={path.url} value={path.url}>{path.url}</option>)}</select></div> : <small>{path.url}</small>}
  </div><button type="button" disabled={disabled} onClick={event => onSelect({ ...device, ...path }, event.currentTarget)}>{t('接入节点', 'Connect node')}</button></li>;
}

export function TailscaleDiscovery({ status, client, disabled, t, onChanged, onSelect }: { status: TailscaleDiscoveryState; client: NetworkClient; disabled: boolean; t: (cn: string, en: string) => string; onChanged: (value: unknown) => void; onSelect: (device: TailscaleIdentity, trigger: HTMLButtonElement) => void }) {
  const savedPorts = (status.ports ?? (status.port ? [status.port] : [])).join(', ');
  const [ports, setPorts] = useState(savedPorts), [busy, setBusy] = useState(false), [error, setError] = useState(''), [fieldError, setFieldError] = useState('');
  const mounted = useRef(true), mutation = useRef(false), dirty = useRef(false);
  const field = useRef<HTMLInputElement>(null), id = useId();
  const primary = useRef<HTMLButtonElement>(null), wasEnabled = useRef(status.enabled);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => { if (!dirty.current) setPorts(savedPorts); }, [savedPorts]);
  useEffect(() => { if (wasEnabled.current && !status.enabled) primary.current?.focus(); wasEnabled.current = status.enabled; }, [status.enabled]);
  function validate() {
    const values = ports.trim().split(/[,，、\s]+/).filter(Boolean);
    const parsed = values.map(Number);
    const problem = !values.length || values.length > 8 || values.some((value, index) => !/^\d{1,5}$/.test(value) || parsed[index] < 1 || parsed[index] > 65535) ? t('填写 1–8 个端口，用逗号或空格分隔；每个端口为 1–65535。', 'Enter 1–8 ports separated by commas or spaces, each between 1 and 65535.') : new Set(parsed).size !== parsed.length ? t('存在重复端口，请删除后再保存。', 'Remove duplicate ports before saving.') : '';
    setFieldError(problem);
    return problem ? undefined : parsed;
  }
  async function save(next: number[]) {
    if (mutation.current || disabled) return;
    mutation.current = true; setBusy(true); setError('');
    try {
      const value = await client({ operation: 'tailscale_discovery', ports: next });
      if (mounted.current) { dirty.current = false; setPorts(next.join(', ')); setFieldError(''); onChanged(value); }
    } catch (e) { if (mounted.current) setError(e instanceof Error ? e.message : String(e)); }
    finally { mutation.current = false; if (mounted.current) setBusy(false); }
  }
  function submit(event: FormEvent) { event.preventDefault(); const next = validate(); if (next) void save(next); else field.current?.focus(); }
  const label = status.phase === 'scanning' ? t('正在发现节点…', 'Discovering nodes…') : status.phase === 'waiting' ? t('等待系统 Tailscale 连接', 'Waiting for system Tailscale') : status.phase === 'failed' ? t('发现失败，将自动重试', 'Discovery failed; retrying automatically') : status.enabled ? t('持续发现已启用', 'Continuous discovery enabled') : t('持续发现未启用', 'Continuous discovery is off');
  return <section className="network-discovery network-ts-discovery" aria-label={t('Tailscale 持续发现', 'Continuous Tailscale discovery')}>
    <h4>{t('自动发现 Cranibox 节点', 'Discover Cranibox nodes automatically')}</h4>
    <p className="direct-note">{t('保存各设备使用的节点端口，每轮检查最多 128 台在线设备，也能发现同一设备上的多个节点。关闭页面后仍会继续；接入时需单独授权。', 'Save the listener ports used by your devices to check up to 128 online devices each round, including multiple nodes on one device. Discovery continues after closing this view; connecting requires a separate grant.')}</p>
    <form className="network-discovery-controls" onSubmit={submit} aria-busy={busy}>
      <label htmlFor={id}>{t('节点监听端口', 'Node listener ports')}<input id={id} ref={field} type="text" required maxLength={64} inputMode="text" autoComplete="off" spellCheck={false} placeholder="1234, 1235" value={ports} disabled={disabled || busy} aria-invalid={!!fieldError} aria-describedby={id + '-help' + (fieldError ? ' ' + id + '-error' : '')} onBlur={() => { validate(); }} onChange={e => { dirty.current = true; setPorts(e.target.value); setFieldError(''); }} /></label>
      <small id={id + '-help'} className="direct-note">{t('最多 8 个，用逗号或空格分隔。例如 1234, 1235。按填写顺序优先显示连接地址。', 'Up to 8, separated by commas or spaces. For example: 1234, 1235. Addresses follow this order.')}</small>
      <div className="direct-actions"><button ref={primary} className="direct-primary" disabled={disabled || busy} type="submit">{busy ? t('正在保存…', 'Saving…') : status.enabled ? t('保存并重新发现', 'Save and rediscover') : t('启用持续发现', 'Enable continuous discovery')}</button>{status.enabled && <button type="button" disabled={disabled || busy} onClick={() => void save([])}>{t('停止发现', 'Stop discovery')}</button>}</div>
      {fieldError && <p id={id + '-error'} className="direct-error" role="alert">{fieldError}</p>}
    </form>
    <p className="direct-note" role="status">{status.phase === 'scanning' && <RefreshCw size={14} aria-hidden="true" />}{label}{status.observed_at && <><br />{t('最近完成', 'Last completed')}: <time dateTime={new Date(status.observed_at * 1000).toISOString()}>{new Date(status.observed_at * 1000).toLocaleString()}</time> · {t('已检查', 'Checked')} {status.checked}</>}</p>
    {(error || status.error) && <p className="direct-error" role="alert">{error || status.error}</p>}
    {status.skipped > 0 && <p className="direct-note">{t('达到单轮上限，尚未检查设备', 'Round limit reached; devices not checked')}: {status.skipped}</p>}
    {status.ambiguous > 0 && <p className="direct-error" role="alert">{t('多个设备返回相同 NodeID，已隐藏冲突身份。请检查重复的节点配置。', 'Several devices returned the same NodeID. Conflicting identities are hidden; check for copied node configurations.')} ({status.ambiguous})</p>}
    {status.enabled && status.phase === 'ready' && !status.devices.length && <p className="direct-note">{t('未发现兼容节点。检查端口、目标监听和 tailnet 访问策略，也可在下方手动查找。', 'No compatible nodes found. Check the ports, target listeners and tailnet access policy, or use manual lookup below.')}</p>}
    {!!status.devices.length && <ul aria-label={t('自动发现的节点', 'Automatically discovered nodes')}>{status.devices.map(device => <DiscoveredNode key={device.node_id} device={device} disabled={disabled || busy} t={t} onSelect={onSelect} />)}</ul>}
  </section>;
}
