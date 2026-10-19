import { useCallback, useEffect, useRef, useState, type FormEvent } from 'react';
import { Cable, Check, ChevronDown, Monitor, RefreshCw, Settings2, ShieldCheck } from 'lucide-react';
import type { DirectCatalog, DirectClient } from '../devices/DirectDevicesView';
import { VncViewer } from './VncViewer';
import type { DesktopConnector, DesktopTarget, DesktopNode } from './transport';
import '../devices/devices.css';
import './desktop.css';
import { DesktopServicePanel, type DesktopServiceClient } from './DesktopServicePanel';
import { ExistingDesktopServices, type ExistingDesktopService } from './ExistingDesktopServices';
import type { OpenDesktopGuide } from './DesktopPlatformGuide';

export function RemoteDesktopsView({ client, connect, language = 'en', local, serviceClient, relayNodes = [], openGuide, onAccess }: { client: DirectClient; connect: DesktopConnector; language?: string; local?: { id: string; name: string }; serviceClient?: DesktopServiceClient; relayNodes?: DesktopNode[]; openGuide?: OpenDesktopGuide; onAccess?: () => void }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const [catalog, setCatalog] = useState<DirectCatalog>(), [selected, setSelected] = useState(''), [active, setActive] = useState<DesktopTarget | null>(null);
  const [loading, setLoading] = useState(true), [busy, setBusy] = useState(false), [error, setError] = useState(''), [notice, setNotice] = useState('');
  const [setup, setSetup] = useState(false), [address, setAddress] = useState('127.0.0.1:5900');
  const [protocol, setProtocol] = useState<'vnc' | 'selkies'>('vnc');
  const [route, setRoute] = useState<'direct' | 'relay'>('direct');
  const mounted = useRef(true), serial = useRef(0), mutation = useRef(false);
  const addressInput = useRef<HTMLInputElement>(null), configureButton = useRef<HTMLButtonElement>(null), focusReview = useRef(false);
  useEffect(() => { if (setup && focusReview.current) { focusReview.current = false; addressInput.current?.focus(); } }, [setup, address, protocol]);
  function review(service: ExistingDesktopService) {
    if (mutation.current) return;
    setError(''); setNotice(''); focusReview.current = true; setProtocol(service.protocol); setAddress(service.address); setSetup(true);
    if (setup && protocol === service.protocol && address === service.address) { focusReview.current = false; addressInput.current?.focus(); }
  }
  const refresh = useCallback(async () => {
    const current = ++serial.current;
    try { const value = await client({ operation: 'list' }) as DirectCatalog; if (mounted.current && current === serial.current) { setCatalog(value); setError(''); } }
    catch (error) { if (mounted.current && current === serial.current) setError(error instanceof Error ? error.message : String(error)); }
    finally { if (mounted.current && current === serial.current) setLoading(false); }
  }, [client]);
  useEffect(() => {
    mounted.current = true; void refresh();
    window.addEventListener('cranibox:peers-changed', refresh); window.addEventListener('focus', refresh);
    return () => { mounted.current = false; ++serial.current; window.removeEventListener('cranibox:peers-changed', refresh); window.removeEventListener('focus', refresh); };
  }, [refresh]);
  const targets: { id: string; name: string; local: boolean; detail: string; direct?: boolean; relay?: DesktopNode }[] = local ? [{ id: local.id, name: local.name, local: true, detail: t('当前接入设备', 'Current access device') }] : [];
  for (const saved of catalog?.routes || []) { if (!targets.some(target => target.id === saved.node)) targets.push({ id: saved.node, name: saved.node, local: false, detail: saved.url, direct: true }); }
  for (const node of relayNodes) {
    const existing = targets.find(target => target.id === node.id);
    if (existing) { if (!existing.local) { existing.relay = node; existing.name = node.name || node.id; } }
    else targets.push({ id: node.id, name: node.name || node.id, local: false, detail: t('通过 Workspace 中继', 'Via Workspace relay'), relay: node });
  }
  const target = targets.find(target => target.id === selected) || targets[0];
  const relay = !!target && !target.local && (!target.direct || route === 'relay');
  const unavailable = relay && (!target?.relay?.online ? t('设备离线，等待重新连接。', 'Device offline. Waiting for it to reconnect.') : !target.relay.desktop_relay ? t('请升级目标设备客户端以启用桌面中继。', 'Update the target client to enable desktop relay.') : '');
  useEffect(() => { if (active && !targets.some(target => target.id === active.node && target.local === active.local && (!active.relay || target.relay))) setActive(null); }, [catalog, local?.id, active, relayNodes]);
  async function run(action: () => Promise<void>) {
    if (mutation.current) return; mutation.current = true; setBusy(true); setError(''); setNotice('');
    try { await action(); } catch (error) { if (mounted.current) setError(error instanceof Error ? error.message : String(error)); }
    finally { mutation.current = false; if (mounted.current) setBusy(false); }
  }
  async function share(event: FormEvent) {
    event.preventDefault(); await run(async () => {
      if (serviceClient) {
        const check = await serviceClient({ operation: 'check', address: address.trim(), ...(protocol === 'selkies' ? { protocol } : {}) }) as { ready: boolean; error?: string };
        if (!check.ready) throw Error(check.error || t('此端口尚未提供所选桌面服务，请启动服务并重试。', 'This port is not serving the selected desktop protocol. Start the server and retry.'));
      }
      const value = await client({ operation: 'desktop', desktop: { address: address.trim(), ...(protocol === 'selkies' ? { protocol } : {}) } }) as DirectCatalog;
      if (!mounted.current) return;
      ++serial.current; setCatalog(value); setSetup(false); setNotice(t('本机桌面共享已开启。请在设备网络的“入站授权”中授予桌面权限。', 'Local sharing enabled. Grant desktop access under Incoming access in Device network.'));
      window.dispatchEvent(new Event('cranibox:peers-changed'));
    });
  }
  async function open() {
    if (!target) return;
    const chosen = target;
    await run(async () => {
      if (chosen.local) { if (!catalog?.desktop) throw Error(t('请先配置本机桌面共享。', 'Configure local desktop sharing first.')); }
      else if (!relay) {
        const value = await client({ operation: 'probe', node: chosen.id }) as { nodes: { desktop?: boolean }[] };
        if (!value.nodes[0]?.desktop) throw Error(t('目标设备尚未开启桌面共享，或未授予当前设备桌面权限。', 'The target has not enabled desktop sharing or granted this device desktop access.'));
      }
      if (unavailable) throw Error(unavailable);
      if (mounted.current) setActive({ node: chosen.id, local: chosen.local, ...(relay ? { relay: true } : {}) });
    });
  }
  return <section className="direct-devices remote-desktops" aria-label={t('跨设备桌面', 'Cross-device desktops')}>
    <div className="direct-heading"><div><h2>{t('你的设备桌面', 'Your device desktops')}</h2><p>{t('从一个入口接入所有授权设备。桌面始终运行在目标设备上。', 'One place for your authorized devices. Each desktop runs on its own device.')}</p></div><button disabled={loading || busy} onClick={() => void refresh()} aria-label={t('刷新桌面设备', 'Refresh desktop devices')}><RefreshCw size={16} aria-hidden="true" /></button></div>
    {error && <p className="direct-error" role="alert">{error}</p>}{notice && <p className="direct-note" role="status"><Check size={16} aria-hidden="true" />{notice}</p>}
    <div className="desktop-layout"><div className="desktop-sidebar"><div className="direct-list" aria-label={t('桌面设备列表', 'Desktop device list')}>
      {targets.map(item => <button key={item.id} disabled={busy} aria-pressed={target?.id === item.id} onClick={() => { if (target?.id !== item.id) { setSelected(item.id); setRoute('direct'); setActive(null); setError(''); } }}><span className="desktop-device-icon">{item.local ? <Monitor size={20} aria-hidden="true" /> : <Cable size={20} aria-hidden="true" />}</span><span><strong>{item.name}</strong><small>{item.direct || item.local ? item.detail : item.relay?.online ? t('中继在线', 'Relay online') : t('中继离线', 'Relay offline')}</small></span></button>)}
    </div><div className="desktop-access-note"><ShieldCheck size={18} aria-hidden="true" /><p>{t('每台设备独立授权，一次只允许一个查看器控制桌面。', 'Each device grants access independently and accepts one desktop controller at a time.')}</p></div></div>
    <div className="desktop-workspace">{loading ? <div className="direct-empty" role="status">{t('正在读取桌面设备…', 'Loading desktop devices…')}</div> : target ? <><div className="direct-heading desktop-target"><div><h3>{target.name}</h3><p>{target.local ? t('本机桌面', 'Local desktop') : target.detail}</p></div>{!active && <button className="direct-primary" disabled={busy || !!unavailable || (target.local && !catalog?.desktop)} onClick={() => void open()}><Monitor size={16} aria-hidden="true" />{busy ? t('正在检查…', 'Checking…') : t('连接桌面', 'Connect desktop')}</button>}</div>{!target.local && <div className="desktop-route-options">{target.direct && target.relay && <label>{t('连接方式', 'Connection route')}<select aria-label={t('连接方式', 'Connection route')} value={relay ? 'relay' : 'direct'} onChange={event => { setRoute(event.target.value as 'direct' | 'relay'); setActive(null); }}><option value="direct">{t('设备直连', 'Direct connection')}</option><option value="relay">{t('Workspace 中继', 'Workspace relay')}</option></select></label>}<p className="direct-note" role="status">{unavailable || (relay ? t('通过中继连接，无需开放目标设备端口。', 'Connect through the relay without opening ports on the target.') : t('通过已保存的地址直接连接设备。', 'Connect directly using the saved device address.'))}</p>{relay && local && <p className="direct-note">{t('请在目标设备的入站授权中，为此 NodeID 授予桌面权限：', 'Grant desktop access on the target to this NodeID: ')}<code>{local.id}</code></p>}</div>}{active ? <VncViewer key={active.node} target={active} connect={connect} language={language} /> : <div className="desktop-welcome"><div className="desktop-welcome-icon"><Monitor size={40} aria-hidden="true" /></div><h3>{t('让设备近在眼前', 'Your desktop, within reach')}</h3><p>{target.local && !catalog?.desktop ? t('先为本机接入一个 VNC 或 Selkies 服务，即可让授权设备访问这台电脑。', 'Connect a local VNC or Selkies server below to make this desktop available to authorized devices.') : t('连接后可查看屏幕、使用键盘和鼠标。断开连接不会关闭桌面。', 'Connect to view the screen and use keyboard and mouse. Disconnecting keeps the desktop running.')}</p><span className="desktop-route-tag"><ShieldCheck size={14} aria-hidden="true" />{relay ? t('目标设备授权 · Workspace 中继', 'Target-owned access · Workspace relay') : t('目标设备授权 · 私网直连', 'Target-owned access · Private direct route')}</span></div>}</> : <div className="direct-empty"><Monitor size={32} aria-hidden="true" /><h3>{t('还没有可连接的桌面', 'No desktops configured')}</h3><p>{t('连接中继或添加直连设备，并在目标设备上开启桌面共享、授予桌面权限。', 'Connect devices to the relay or add a direct route, then enable desktop sharing and grant desktop access on the target.')}</p></div>}</div></div>
    {local && catalog && <section className="desktop-sharing" aria-label={t('本机桌面共享', 'Local desktop sharing')}><div className="direct-heading"><div><h3><Settings2 size={18} aria-hidden="true" />{t('共享这台设备的桌面', 'Share this device’s desktop')}</h3><p>{catalog.desktop ? t('已开启 · ', 'Enabled · ') + (catalog.desktop.protocol === 'selkies' ? 'Selkies · ' : 'VNC · ') + catalog.desktop.address : t('尚未开启。接入本机 VNC 或 Selkies 服务后，授权设备即可访问。', 'Not enabled. Connect a local VNC or Selkies server to share with authorized devices.')}</p></div><div className="direct-actions"><button ref={configureButton} disabled={busy} aria-expanded={setup} onClick={() => { setSetup(!setup); setProtocol(catalog.desktop?.protocol || 'vnc'); setAddress(catalog.desktop?.address || '127.0.0.1:5900'); }}><ChevronDown size={16} aria-hidden="true" />{t('配置共享', 'Configure sharing')}</button>{catalog.desktop && <button disabled={busy} onClick={() => void run(async () => { const value = await client({ operation: 'desktop', desktop: null }) as DirectCatalog; if (mounted.current) { ++serial.current; setCatalog(value); setActive(null); setNotice(t('共享已关闭，现有查看器将断开。', 'Sharing stopped. Existing viewers will disconnect.')); window.dispatchEvent(new Event('cranibox:peers-changed')); } })}>{t('停止共享', 'Stop sharing')}</button>}</div></div>{serviceClient && <ExistingDesktopServices client={serviceClient} language={language} disabled={busy} current={catalog.desktop} onSelect={review} />}{setup && <form aria-label={t('核对桌面共享配置', 'Review desktop sharing')} onSubmit={share}><h4>{t('核对桌面共享配置', 'Review desktop sharing')}</h4>
      <label>{t('桌面服务', 'Desktop protocol')}<select disabled={busy} aria-label={t('桌面服务', 'Desktop protocol')} value={protocol} onChange={event => { const next = event.target.value as 'vnc' | 'selkies'; setProtocol(next); setAddress(next === 'selkies' ? '127.0.0.1:8082' : '127.0.0.1:5900'); }}><option value="vnc">VNC</option><option value="selkies">Selkies · Linux</option></select></label>
      <p className="direct-note">{protocol === 'selkies' ? t('接入已运行的 Selkies WebSocket 服务（JPEG 编码）。连接将接管主显示器；已有 Selkies 查看器会断开。支持纯文本剪贴板，暂不支持音频和文件传输。', 'Connect a running Selkies WebSocket service with JPEG support. Connecting takes over the primary display and disconnects its existing Selkies viewer. Plain-text clipboard is supported; audio and file transfer are unavailable.') : t('填写此设备上已安装的 VNC 服务地址，Cranibox 会通过授权通道转发画面和输入。', 'Enter the address of a VNC server on this device. Cranibox forwards its display and input through an authorized connection.')}</p>
      <label>{protocol === 'selkies' ? t('本机 Selkies 地址', 'Local Selkies address') : t('本机 VNC 地址', 'Local VNC address')}<input ref={addressInput} disabled={busy} required aria-label={protocol === 'selkies' ? t('本机 Selkies 地址', 'Local Selkies address') : t('本机 VNC 地址', 'Local VNC address')} aria-describedby="desktop-vnc-address-help" value={address} onChange={event => setAddress(event.target.value)} placeholder={protocol === 'selkies' ? '127.0.0.1:8082' : '127.0.0.1:5900'} /><span id="desktop-vnc-address-help" className="direct-note">{t('使用 127.0.0.1:端口 或 [::1]:端口；访问其他设备请使用左侧设备列表。', 'Use 127.0.0.1:port or [::1]:port. Choose other devices from the list above.')}{protocol === 'selkies' && t(' 使用内部 WebSocket 端口，不是网页端口；服务需以非 token 模式运行，仅在可信容器或回环网络内开放。', ' Use the internal WebSocket port, not its web page port. The service must run without token authentication and remain inside a trusted container or loopback network.')}</span></label>
      <div className="direct-actions"><button disabled={busy} className="direct-primary" type="submit">{busy ? t('正在检查并保存…', 'Checking and saving…') : t('保存并开启共享', 'Save and enable sharing')}</button><button type="button" disabled={busy} onClick={() => { setSetup(false); configureButton.current?.focus(); }}>{t('取消', 'Cancel')}</button></div>
    </form>}{serviceClient && <DesktopServicePanel client={serviceClient} language={language} shared={catalog.desktop} onReview={review} openGuide={openGuide} onAccess={onAccess} onChanged={() => { setActive(null); void refresh(); }} />}</section>}
  </section>;
}
