import { useEffect, useRef, useState } from 'react';
import { ArrowRight, Check, Monitor, Search } from 'lucide-react';
import type { DesktopServiceClient } from './DesktopServicePanel';
import type { DirectCatalog } from '../devices/DirectDevicesView';

export type ExistingDesktopService = { address: string; protocol: 'vnc' | 'selkies'; configured: boolean };

export function ExistingDesktopServices({ client, language, disabled, current, onSelect }: { client: DesktopServiceClient; language: string; disabled: boolean; current?: DirectCatalog['desktop']; onSelect: (service: ExistingDesktopService) => void }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const [services, setServices] = useState<ExistingDesktopService[]>(), [busy, setBusy] = useState(false), [error, setError] = useState('');
  const mounted = useRef(true), pending = useRef(false), serial = useRef(0);
  useEffect(() => {
    mounted.current = true; pending.current = false; setBusy(false); setServices(undefined); setError('');
    return () => { mounted.current = false; ++serial.current; };
  }, [client]);
  async function discover() {
    if (pending.current || disabled) return;
    pending.current = true; const current = ++serial.current;
    setBusy(true); setError(''); setServices(undefined);
    try {
      const result = await client({ operation: 'discover' }) as { services?: ExistingDesktopService[] } | null;
      if (!mounted.current || current !== serial.current) return;
      if (!Array.isArray(result?.services) || result.services.some(service => !service || typeof service.address !== 'string' || !['vnc', 'selkies'].includes(service.protocol) || typeof service.configured !== 'boolean')) throw Error(t('无法读取检测结果。请更新节点程序，或使用“配置共享”手动填写地址。', 'Cannot read discovery results. Update this node or enter its address with Configure sharing.'));
      setServices(result.services);
    } catch (error) { if (mounted.current && current === serial.current) setError(error instanceof Error ? error.message : String(error)); }
    finally { if (mounted.current && current === serial.current) { pending.current = false; setBusy(false); } }
  }
  return <section className="desktop-discovery" aria-label={t('已有桌面服务', 'Existing desktop services')}>
    <div className="direct-heading"><div><h4>{t('接入已有桌面服务', 'Use an existing desktop service')}</h4><p>{t('已有 VNC 或 Selkies？检测本机服务，选择后核对配置，再开启共享。', 'Already running VNC or Selkies? Find a local service, review its settings, then enable sharing.')}</p></div><button disabled={disabled || busy} onClick={() => void discover()}><Search size={16} aria-hidden="true" />{busy ? t('正在检测…', 'Detecting…') : t('检测已有服务', 'Detect existing services')}</button></div>
    <p className="direct-note">{t('检查本机 VNC 5900、Selkies 8082 和已保存的地址；其他端口可使用“配置共享”手动填写。检测不会开启共享或连接桌面。', 'Checks local VNC 5900, Selkies 8082 and saved addresses. Use Configure sharing for other ports. Detection does not enable sharing or open a viewer.')}</p>
    {error && <p className="direct-error" role="alert">{error}</p>}
    {(busy || services) && <p className="direct-note" role="status">{busy ? t('正在检查桌面协议，请稍候…', 'Checking desktop protocols…') : services!.length ? t('找到 ' + services!.length + ' 个服务，选择一个以核对配置。', 'Found ' + services!.length + (services!.length === 1 ? ' service. Choose it to review.' : ' services. Choose one to review.')) : t('未找到就绪的服务。请先启动 VNC 或 Selkies，再重试检测；使用其他端口时可手动配置。', 'No ready services found. Start VNC or Selkies and retry, or configure a different port manually.')}</p>}
    {!!services?.length && <ul className="desktop-discovery-results">{services.map(service => <li key={service.protocol + service.address}><Monitor size={20} aria-hidden="true" /><div><strong>{service.protocol === 'selkies' ? 'Selkies' : 'VNC'}</strong><code>{service.address}</code>{current?.address === service.address && (current.protocol || 'vnc') === service.protocol && <span><Check size={14} aria-hidden="true" />{t('当前共享配置', 'Current sharing configuration')}</span>}</div><button disabled={disabled || busy} aria-label={t('核对 ', 'Review ') + (service.protocol === 'selkies' ? 'Selkies' : 'VNC') + ' · ' + service.address} onClick={() => onSelect(service)}>{t('核对配置', 'Review settings')}<ArrowRight size={16} aria-hidden="true" /></button></li>)}</ul>}
  </section>;
}
