import { useEffect, useId, useRef, useState, type FormEvent, type MouseEvent } from 'react';
import { ArrowLeft, ArrowRight, Check, ExternalLink, ShieldCheck } from 'lucide-react';
import type { DesktopServiceClient } from './DesktopServicePanel';
import type { ExistingDesktopService } from './ExistingDesktopServices';

export type DesktopGuideLink = 'windows_download' | 'windows_settings' | 'mac_sharing';
export type OpenDesktopGuide = (guide: DesktopGuideLink) => Promise<unknown>;
const links: Record<DesktopGuideLink, string> = {
  windows_download: 'https://uvnc.com/downloads/ultravnc.html',
  windows_settings: 'https://uvnc.com/docs/ultravnc-server/49-ultravnc-server-configuration.html',
  mac_sharing: 'https://support.apple.com/guide/mac-help/turn-screen-sharing-on-or-off-mh11848/mac',
};

export function DesktopPlatformGuide({ platform, client, language, disabled, shared, onReview, openGuide, onAccess }: {
  platform: 'windows' | 'macos'; client: DesktopServiceClient; language: string; disabled: boolean;
  shared?: { address: string; protocol?: 'vnc' | 'selkies' } | null;
  onReview: (service: ExistingDesktopService) => void; openGuide?: OpenDesktopGuide; onAccess?: () => void;
}) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const [step, setStep] = useState(1), [address, setAddress] = useState('127.0.0.1:5900'), [checked, setChecked] = useState('');
  const [busy, setBusy] = useState(false), [error, setError] = useState('');
  const mounted = useRef(true), request = useRef(0), pending = useRef(false), focusStep = useRef(false);
  const heading = useRef<HTMLHeadingElement>(null), field = useRef<HTMLInputElement>(null), id = useId();
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; ++request.current; };
  }, []);
  useEffect(() => {
    if (focusStep.current) { focusStep.current = false; if (step === 2) field.current?.focus(); else heading.current?.focus(); }
  }, [step]);
  function move(next: number) { setError(''); focusStep.current = true; setStep(next); }
  async function open(event: MouseEvent<HTMLAnchorElement>, guide: DesktopGuideLink) {
    if (!openGuide) return;
    event.preventDefault(); setError('');
    try { await openGuide(guide); }
    catch (error) { if (mounted.current) setError(error instanceof Error ? error.message : String(error)); }
  }
  function link(guide: DesktopGuideLink, label: string) {
    return <a className="desktop-guide-link" href={links[guide]} target="_blank" rel="noopener noreferrer" onClick={event => void open(event, guide)}>{label}<ExternalLink size={15} aria-hidden="true" /><span className="sr-only">{t('（在浏览器新窗口打开）', ' (opens in a browser window)')}</span></a>;
  }
  async function check(event: FormEvent) {
    event.preventDefault();
    if (pending.current || disabled) return;
    const value = address.trim(), current = ++request.current;
    setError(''); setChecked('');
    if (!/^(127\.0\.0\.1|\[::1\]):[0-9]{1,5}$/.test(value) || Number(value.slice(value.lastIndexOf(':') + 1)) < 1 || Number(value.slice(value.lastIndexOf(':') + 1)) > 65535) {
      setError(t('填写 127.0.0.1:端口 或 [::1]:端口，端口范围为 1–65535。', 'Enter 127.0.0.1:port or [::1]:port, with a port from 1 to 65535.')); field.current?.focus(); return;
    }
    pending.current = true; setBusy(true);
    try {
      const result = await client({ operation: 'check', address: value }) as { ready?: boolean; error?: string } | null;
      if (!mounted.current || current !== request.current) return;
      if (!result?.ready) throw Error(result?.error || t('未收到 VNC 握手。请先启动服务、检查端口，再重试。', 'No VNC greeting received. Start the server, check its port and retry.'));
      setChecked(value); move(3);
    } catch (error) {
      if (mounted.current && current === request.current) setError(error instanceof Error ? error.message : String(error));
    } finally { pending.current = false; if (mounted.current && current === request.current) setBusy(false); }
  }
  const enabled = !!checked && shared?.address === checked && (shared.protocol || 'vnc') === 'vnc';
  return <section className="desktop-platform-guide" aria-label={t('桌面共享准备向导', 'Desktop sharing setup guide')}>
    <p className="direct-note">{t('请在运行节点的这台设备上操作；浏览器所在电脑可能是另一台设备。', 'Follow these steps on the device running this node; your browser may be on another computer.')}</p>
    <ol className="desktop-setup-steps" aria-label={t('准备进度', 'Setup progress')}>{[t('准备服务', 'Prepare server'), t('检查端口', 'Check port'), t('开启与授权', 'Enable and grant access')].map((label, index) => <li key={label} aria-current={step === index + 1 ? 'step' : undefined} className={step > index + 1 ? 'complete' : ''}><span>{step > index + 1 ? <Check size={14} aria-hidden="true" /> : index + 1}</span>{label}</li>)}</ol>
    <h5 ref={heading} tabIndex={-1}>{step === 1 ? platform === 'windows' ? t('在 Windows 准备 UltraVNC Server', 'Prepare UltraVNC Server on Windows') : t('开启 macOS 屏幕共享', 'Enable macOS Screen Sharing') : step === 2 ? t('检查这台设备的 VNC 服务', 'Check this device’s VNC server') : t('核对后开启，再授权设备', 'Review sharing, then grant device access')}</h5>
    {step === 1 ? <>
      {platform === 'windows' ? <><p>{t('从官网安装 UltraVNC 的 Server 组件，可选择注册为系统服务以便开机后运行。在服务器设置中配置独立 VNC 密码，启用 Allow Loopback Connections 和 Loopback Only。Cranibox 不需要其 Viewer。', 'Install the Server component from UltraVNC’s official site. Registering it as a system service lets it run after startup. Set a separate VNC password and enable Allow Loopback Connections and Loopback Only in its server settings. Cranibox does not need its Viewer.')}</p><p className="direct-note">{t('WSL2 中的节点只访问 WSL2 的本机服务。共享 Windows 桌面时，请在 Windows 原生客户端中完成这些步骤。', 'A node inside WSL2 accesses WSL2’s local services. To share the Windows desktop, use the native Windows client.')}</p><div className="direct-actions">{link('windows_download', t('UltraVNC 官方下载', 'Official UltraVNC download'))}{link('windows_settings', t('服务器设置说明', 'Server settings documentation'))}</div></> : <><p>{t('打开“系统设置 → 通用 → 共享 → 屏幕共享”，开启共享。在设置中启用“VNC 显示程序可以使用密码控制屏幕”，设置独立 VNC 密码。系统版本不同，选项名称可能略有变化。', 'Open System Settings → General → Sharing → Screen Sharing and turn it on. Enable “VNC viewers may control screen with password” and set a separate VNC password. Labels may vary by macOS version.')}</p><p className="direct-note">{t('系统屏幕共享也可能接受局域网连接；请用主机网络访问控制限制访问范围。Cranibox 使用回环地址不会更改系统服务的监听或防火墙设置。', 'System Screen Sharing may also accept LAN connections. Restrict access with host network controls. Cranibox’s loopback connection does not change the system service’s listeners or firewall.')}</p>{link('mac_sharing', t('Apple 屏幕共享说明', 'Apple Screen Sharing documentation'))}</>}
      <p className="direct-note">{t('按 VNC 提供者或系统的实际提示授予录屏、辅助功能等权限。安装与系统权限由你在此设备完成；Cranibox 尚未自动安装或托管这两个平台的 VNC 服务。', 'Grant screen recording, accessibility or other permissions requested by the provider or system. Complete installation and system permissions on this device; Cranibox does not yet install or manage VNC on these platforms.')}</p>
      <div className="direct-actions"><button disabled={disabled} onClick={() => move(2)}>{t('服务已准备，检查端口', 'Server ready, check port')}<ArrowRight size={16} aria-hidden="true" /></button></div>
    </> : step === 2 ? <form aria-label={t('检查本机 VNC 服务', 'Check local VNC service')} aria-busy={busy} onSubmit={event => void check(event)}>
      <label htmlFor={id}>{t('待检查的 VNC 地址', 'VNC address to check')}</label><input id={id} ref={field} value={address} required maxLength={64} spellCheck={false} autoComplete="off" disabled={busy || disabled} aria-invalid={!!error} aria-describedby={id + '-help' + (error ? ' ' + id + '-error' : '')} onChange={event => { setAddress(event.target.value); setChecked(''); setError(''); }} />
      <p id={id + '-help'} className="direct-note">{t('通常为 127.0.0.1:5900，也支持 [::1]:端口。这里只读取握手，不发送密码、不获取控制权，也不开启共享。', 'Usually 127.0.0.1:5900; [::1]:port also works. This reads only the greeting, without sending a password, taking control or enabling sharing.')}</p>
      <div className="direct-actions"><button type="button" disabled={busy || disabled} onClick={() => move(1)}><ArrowLeft size={16} aria-hidden="true" />{t('上一步', 'Back')}</button><button className="direct-primary" type="submit" disabled={busy || disabled}>{busy ? t('正在检查…', 'Checking…') : t('检查 VNC 服务', 'Check VNC server')}</button></div>
    </form> : <>
      <p className="desktop-guide-ready" role="status"><Check size={18} aria-hidden="true" />{t('已收到 VNC 握手：', 'VNC greeting received: ')}<code>{checked}</code></p>
      <p>{enabled ? t('此地址已开启共享。在“设备网络 → 入站授权”为调用方勾选桌面权限，然后从另一台设备连接。', 'Sharing is enabled for this address. Grant the caller desktop permission under Device network → Incoming access, then connect from that device.') : t('下一步将填写共享核对表单，确认保存后才会开启共享。随后在“设备网络 → 入站授权”为调用方授予桌面权限。', 'Next, review the sharing form. Sharing starts only after you save it. Then grant the caller desktop permission under Device network → Incoming access.')}</p>
      <p className="direct-note">{t('握手成功尚不代表认证、画面或输入正常。请在连接查看器时填写 VNC 凭据，并验证屏幕与键鼠。', 'A greeting does not verify authentication, display or input. Enter the VNC credentials in the viewer and verify the screen, keyboard and mouse.')}</p>
      <div className="direct-actions"><button disabled={disabled} onClick={() => move(2)}><ArrowLeft size={16} aria-hidden="true" />{t('重新检查', 'Check again')}</button>{enabled && onAccess ? <button className="direct-primary" disabled={disabled} onClick={onAccess}><ShieldCheck size={16} aria-hidden="true" />{t('前往设备授权', 'Open device access')}</button> : <button className="direct-primary" disabled={disabled} onClick={() => onReview({ address: checked, protocol: 'vnc', configured: enabled })}>{t('核对并开启共享', 'Review and enable sharing')}<ArrowRight size={16} aria-hidden="true" /></button>}</div>
    </>}
    {error && <p id={id + '-error'} className="direct-error" role="alert">{error}</p>}
  </section>;
}
