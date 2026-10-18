import { useEffect, useRef, useState, type MouseEvent } from 'react';
import { Download } from 'lucide-react';

export type TailscaleSetupState = { phase: 'idle' | 'running' | 'succeeded' | 'failed' | 'interrupted'; error?: string | null; action?: string | null };
export type TailscaleSetupPlan = { available: boolean; reason?: string; id?: string; platform: string; action?: string; requires_admin?: boolean; expires_in_seconds?: number; commands?: { program: string; args: string[] }[]; script?: string | null; script_sha256?: string | null };
type Props = {
  setup?: TailscaleSetupState; systemState: string; busy: boolean;
  t: (cn: string, en: string) => string;
  review: (done: (value: TailscaleSetupPlan) => void) => Promise<void>;
  execute: (id: string, done: () => void) => Promise<void>;
  openLink: (event: MouseEvent<HTMLAnchorElement>, url: string) => void;
};

/** Shared by the browser and native client; only the node runs the reviewed plan. */
export function TailscaleSetup({ setup, systemState, busy, t, review, execute, openLink }: Props) {
  const [plan, setPlan] = useState<TailscaleSetupPlan>(), [confirmed, setConfirmed] = useState(false);
  const [expires, setExpires] = useState(0), [now, setNow] = useState(Date.now());
  const heading = useRef<HTMLHeadingElement>(null), reviewButton = useRef<HTMLButtonElement>(null);
  const running = setup?.phase === 'running', expired = !!plan?.available && now >= expires;
  useEffect(() => { if (plan) heading.current?.focus(); }, [plan]);
  useEffect(() => { if (running) { setPlan(undefined); setConfirmed(false); } }, [running]);
  useEffect(() => { if (!plan?.available) return; const timer = setInterval(() => setNow(Date.now()), 1000); return () => clearInterval(timer); }, [plan]);
  if (!['NotInstalled', 'Unavailable'].includes(systemState) && (!setup || setup.phase === 'idle') && !plan) return null;
  const clear = () => { setPlan(undefined); setConfirmed(false); requestAnimationFrame(() => reviewButton.current?.focus()); };
  const action = plan?.action === 'start_service' ? t('启动系统 Tailscale 服务', 'Start the system Tailscale service') : plan?.action === 'open_app' ? t('打开 Tailscale 应用', 'Open the Tailscale app') : t('安装系统 Tailscale', 'Install system Tailscale');
  return <div className="network-setup tailscale-setup" aria-label={t('Tailscale 安装引导', 'Tailscale setup')} aria-busy={running || busy}>
    {running && <p className="direct-note" role="status">{t('正在执行安装方案，请在这台设备上完成系统授权。关闭此页面不会中断安装；请勿重复运行安装器。', 'Setup is running. Complete the system permission prompt on this device. You can close this page; do not run a second installer.')}</p>}
    {setup?.phase === 'succeeded' && <p className="direct-note" role="status">{t('安装命令已完成，正在检查系统服务。若仍无法连接，请打开 Tailscale 完成系统权限设置。登录需在下方单独完成。', 'Setup commands finished. Checking the system service. If it stays unavailable, open Tailscale to complete system permissions. Sign in separately below.')}</p>}
    {(setup?.phase === 'failed' || setup?.phase === 'interrupted' || setup?.error) && <p className="direct-error" role="alert">{setup.error || t('安装尚未完成，请检查系统安装器后重试。', 'Setup is incomplete. Check the system installer before retrying.')}</p>}
    {!running && ['NotInstalled', 'Unavailable'].includes(systemState) && <>
      <p className="direct-note">{t('先检查当前节点的安装或服务启动方案，再确认执行。系统可能要求管理员授权；安装不会自动登录账号。', 'Review installation or service startup on this node, then confirm. The system may request administrator permission. Setup does not sign in automatically.')}</p>
      {!plan ? <button ref={reviewButton} disabled={busy} onClick={() => void review(value => { setPlan(value); setConfirmed(false); const timestamp = Date.now(); setNow(timestamp); setExpires(timestamp + (value.expires_in_seconds || 0) * 1000); })}><Download size={16} aria-hidden="true" />{busy ? t('正在检查…', 'Checking…') : t('检查 Tailscale 安装方案', 'Review Tailscale setup')}</button> : <div className="network-plan">
        <h4 tabIndex={-1} ref={heading}>{plan.available ? action : t('需要手动配置', 'Manual setup required')}</h4>
        {plan.available ? <>
          <p className="direct-note">{plan.platform} · {plan.requires_admin ? t('需要系统管理员授权', 'System administrator permission required') : t('无需管理员权限', 'No administrator permission required')}</p>
          <p className="direct-note">{t('使用官方软件源或系统包管理器，接受 Tailscale 的软件许可；可能修改软件源并启用后台服务。已有网络配置保持不变。', 'Uses official packages or the system package manager and accepts Tailscale software terms. May add a package source and enable its service. Existing network settings are preserved.')}</p>
          {plan.platform === 'macos' && <p className="direct-note">{t('安装后请在 macOS 系统设置中允许 Tailscale 的网络扩展。', 'After installation, allow the Tailscale network extension in macOS System Settings.')}</p>}
          <details><summary>{t('查看实际命令', 'View commands')}</summary><div className="tailscale-setup-commands">{plan.commands?.map((command, index) => <pre key={index}><code>{[command.program, ...command.args.map(arg => JSON.stringify(arg))].join(' ')}</code></pre>)}{plan.script && <><p className="direct-note">SHA-256 <code>{plan.script_sha256}</code></p><pre><code>{plan.script}</code></pre></>}</div></details>
          <label className="direct-check"><input type="checkbox" checked={confirmed} disabled={busy || expired} onChange={e => setConfirmed(e.target.checked)} />{t('我已审阅，允许在当前节点执行以上操作', 'I reviewed and allow these actions on this node')}</label>
          {expired && <p className="direct-error" role="alert">{t('方案已过期，请取消后重新检查。', 'This plan expired. Cancel and review a new plan.')}</p>}
          <div className="direct-actions"><button className="direct-primary" disabled={busy || !confirmed || expired || !plan.id} onClick={() => { if (plan.id) void execute(plan.id, clear); }}>{busy ? t('正在提交…', 'Submitting…') : t('确认并执行', 'Confirm and run')}</button><button disabled={busy} onClick={clear}>{t('取消', 'Cancel')}</button></div>
        </> : <><p className="direct-note" role="status">{plan.reason}</p><button disabled={busy} onClick={clear}>{t('返回', 'Back')}</button></>}
      </div>}
      <a className="network-link" href="https://tailscale.com/download" onClick={e => openLink(e, 'https://tailscale.com/download')} target="_blank" rel="noreferrer">{t('打开官方下载页面', 'Open official downloads')}</a>
    </>}
  </div>;
}
