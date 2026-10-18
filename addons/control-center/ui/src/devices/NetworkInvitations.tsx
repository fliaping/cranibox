import { useEffect, useId, useRef, useState, type FormEvent } from 'react';
import { Check, Copy, Link } from 'lucide-react';

type Translate = (cn: string, en: string) => string;
export type Invitation = { id: string; expires_at: number; redeemed_at?: number | null };
export type IssuedInvitation = Invitation & { token: string };

export function InvitationEnrollment({ busy, t, submit }: { busy: boolean; t: Translate; submit: (url: string, token: string, done: () => void) => Promise<void> }) {
  const id = useId();
  const [url, setUrl] = useState(''), [token, setToken] = useState('');
  async function join(event: FormEvent) {
    event.preventDefault();
    await submit(url.trim(), token.trim(), () => setToken(''));
  }
  return <form onSubmit={join} className="network-enrollment">
    <p className="direct-note">{t('在已联网的 Workspace 网页中创建邀请，然后填写它的地址和邀请 Token。无需手动复制网络配置。', 'Create an invitation in a connected Workspace, then enter its address and invitation token here. Network settings are filled in for you.')}</p>
    <div className="network-entry"><label htmlFor={id + '-url'}>{t('Workspace 地址', 'Workspace address')}</label><input id={id + '-url'} type="url" required maxLength={2048} placeholder="https://workspace.example.com" value={url} disabled={busy} autoCapitalize="off" autoCorrect="off" spellCheck={false} onChange={event => setUrl(event.target.value)} /></div>
    <div className="network-entry"><label htmlFor={id + '-token'}>{t('邀请 Token', 'Invitation token')}</label><input id={id + '-token'} type="password" required pattern="cbi_[A-Fa-f0-9]{64}" maxLength={68} autoComplete="off" placeholder="cbi_…" value={token} disabled={busy} onChange={event => setToken(event.target.value)} /></div>
    <p className="direct-note">{t('邀请 15 分钟内有效，只能接入一台设备。若连接中断，在原设备使用相同地址和邀请重试。终端和桌面仍需目标设备单独授权。', 'Invitations last 15 minutes and admit one device. If the connection drops, retry on the original device with the same address and invitation. Terminal and desktop access require separate permission.')}</p>
    <button className="direct-primary" disabled={busy} type="submit"><Link size={16} aria-hidden="true" />{busy ? t('正在加入…', 'Joining…') : t('通过邀请加入', 'Join with invitation')}</button>
  </form>;
}

export function NetworkInvitations({ entries, listenPort, invitations, busy, t, create, revoke }: { entries: string[]; listenPort: number | null; invitations?: Invitation[]; busy: boolean; t: Translate; create: (peers: string[], done: (value: IssuedInvitation) => void) => Promise<void>; revoke: (id: string) => Promise<void> }) {
  const id = useId(), tokenField = useRef<HTMLInputElement>(null);
  const [peers, setPeers] = useState(() => {
    if (entries.length) return entries.join('\n');
    const host = window.location.hostname;
    return listenPort && !['localhost', '127.0.0.1', '[::1]'].includes(host) ? 'tcp://' + host + ':' + listenPort : '';
  });
  const [issued, setIssued] = useState<IssuedInvitation>(), [copied, setCopied] = useState(false);
  useEffect(() => { if (issued && invitations && !invitations.some(invitation => invitation.id === issued.id && !invitation.redeemed_at)) setIssued(undefined); }, [invitations, issued]);
  async function invite(event: FormEvent) {
    event.preventDefault();
    await create(peers.split(/\r?\n/).map(value => value.trim()).filter(Boolean), invitation => { setIssued(invitation); setCopied(false); });
  }
  async function copy() {
    if (!issued) return;
    try { await navigator.clipboard.writeText(issued.token); setCopied(true); }
    catch { tokenField.current?.focus(); tokenField.current?.select(); }
  }
  return <details className="network-invitations"><summary>{t('邀请其他设备', 'Invite another device')}</summary><div className="network-advanced">
    <p className="direct-note">{t('新设备使用当前 Workspace 地址领取网络配置。确认下面的入口能从新设备访问；HTTPS 网页地址本身不是 EasyTier 中继端口。', 'New devices fetch their settings from this Workspace. Confirm they can reach the network entries below; the HTTPS page address is separate from the EasyTier relay port.')}</p>
    <form onSubmit={invite}>
      <div className="network-entry"><label htmlFor={id + '-entries'}>{t('邀请使用的网络入口', 'Invitation entry addresses')}</label><textarea id={id + '-entries'} required rows={2} maxLength={17000} spellCheck={false} autoCapitalize="off" autoCorrect="off" placeholder="tcp://device.example.com:11010" value={peers} onChange={event => setPeers(event.target.value)} /></div>
      <small className="direct-note">{t('每行一个，最多 8 个。只有已开启入站监听、且端口可达的成员才能作为入口。', 'One per line, up to 8. Use members with an enabled, reachable incoming listener.')}</small>
      <button type="submit" disabled={busy}>{t('创建 15 分钟邀请', 'Create 15 minute invitation')}</button>
    </form>
    {issued && <div className="network-invitation-result" role="region" aria-label={t('新设备邀请', 'New device invitation')}>
      <div className="network-entry"><label htmlFor={id + '-address'}>{t('分享 Workspace 地址', 'Share Workspace address')}</label><input id={id + '-address'} value={window.location.origin} readOnly onFocus={event => event.target.select()} /></div>
      <div className="network-entry"><label htmlFor={id + '-issued'}>{t('分享邀请 Token', 'Share invitation token')}</label><input id={id + '-issued'} ref={tokenField} value={issued.token} autoComplete="off" spellCheck={false} readOnly onFocus={event => event.target.select()} /></div>
      <div className="direct-actions"><button type="button" onClick={() => void copy()}>{copied ? <Check size={16} aria-hidden="true" /> : <Copy size={16} aria-hidden="true" />}{copied ? t('已复制', 'Copied') : t('复制邀请 Token', 'Copy invitation token')}</button><button type="button" onClick={() => setIssued(undefined)}>{t('隐藏邀请', 'Hide invitation')}</button></div>
      <p className="direct-note">{t('仅此处展示 Token。刷新后仍可撤销邀请，但无法再次查看 Token。', 'The token is shown only here. After reloading you can revoke the invitation, but cannot view its token again.')}</p>
    </div>}
    {!!invitations?.length && <ul aria-label={t('有效邀请', 'Active invitations')}>{invitations.map(invitation => <li key={invitation.id}><div><strong>{t('邀请 ', 'Invitation ') + invitation.id.slice(0, 8)}</strong><small>{invitation.redeemed_at ? t('已领取 · 仅原设备可重试', 'Claimed · Only the original device can retry') : t('等待设备领取', 'Waiting for a device')}</small><small>{t('有效至 ', 'Expires at ') + new Date(invitation.expires_at * 1000).toLocaleTimeString()}</small></div><button type="button" disabled={busy} aria-label={t('撤销邀请 ', 'Revoke invitation ') + invitation.id.slice(0, 8)} onClick={() => void revoke(invitation.id)}>{t('撤销', 'Revoke')}</button></li>)}</ul>}
  </div></details>;
}
