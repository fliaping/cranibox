import { ArrowUpRight,Check,Circle,Search,TriangleAlert,type LucideIcon } from 'lucide-react';
import type { ButtonHTMLAttributes,ReactNode } from 'react';
import { Link } from 'react-router-dom';
import { t,useWorkspace } from './workspace';

export function Icon({ icon: Glyph, className = '' }: { icon: LucideIcon; className?: string }) { return <Glyph size={18} strokeWidth={1.8} aria-hidden="true" className={className} />; }
export function Button({ children, icon, action, extra, onClick, tone = 'secondary', disabled = false, local = false, label, type = 'button', ...attributes }: { children?: ReactNode; icon?: LucideIcon; action?: string; extra?: Record<string, unknown>; onClick?: () => void; tone?: string; disabled?: boolean; local?: boolean; label?: string } & Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'onClick'>) {
  const workspace = useWorkspace();
  const isLocal = local || ['openWorkspace', 'openDesktop', 'openUrl', 'openPath', 'terminal', 'showOutput', 'refresh', 'copyTunnel', 'copyDesktopTunnel', 'connectTailscale'].includes(action || '');
  return <button {...attributes} type={type} className={'button ' + tone + (!children ? ' icon-button' : '')} aria-label={label} title={label} disabled={disabled || (!isLocal && action !== 'desktopEmergencyStop' && workspace.locked)} onClick={onClick || (() => action && workspace.action(action, extra))}>{icon && <Icon icon={icon} />}{children}</button>;
}
export function Status({ state, label }: { state?: string; label?: string }) {
  const good = ['ready', 'running', 'active', 'connected', 'exited'].includes(state || '');
  const bad = ['failed', 'missing', 'unsupported', 'exposed', 'timed_out'].includes(state || '');
  const warning = ['needs-attention', 'needs-restart', 'needs-setup', 'paused', 'needs-login', 'incomplete', 'interrupted', 'disconnected', 'cancelled'].includes(state || '');
  const names: Record<string, string> = { exited: t('完成', 'Completed'), interrupted: t('已中断', 'Interrupted'), disconnected: t('连接中断', 'Disconnected'), timed_out: t('超时', 'Timed out'), cancelled: t('已取消', 'Cancelled'), ready: t('已就绪', 'Ready'), running: t('运行中', 'Running'), active: t('运行中', 'Running'), connected: t('已连接', 'Connected'), inactive: t('已停止', 'Stopped'), stopped: t('已停止', 'Stopped'), missing: t('未安装', 'Not installed'), optional: t('未配置', 'Not configured'), failed: t('失败', 'Failed'), 'needs-attention': t('需处理', 'Needs attention'), 'needs-restart': t('需重启', 'Restart required'), 'needs-setup': t('需配置', 'Setup required'), 'needs-login': t('需登录', 'Sign-in required'), paused: t('已暂停', 'Paused'), unsupported: t('不支持', 'Unsupported'), unknown: t('未知', 'Unknown') };
  return <span className={'status ' + (good ? 'good' : bad ? 'bad' : warning ? 'warning' : 'neutral')}><Icon icon={good ? Check : bad || warning ? TriangleAlert : Circle} />{label || names[state || 'unknown'] || state}</span>;
}
export function Card({ title, children, action, className = '', subtitle }: { title?: ReactNode; children: ReactNode; action?: ReactNode; className?: string; subtitle?: ReactNode }) {
  return <section className={'card ' + className}>{title && <div className="card-head"><div><h2>{title}</h2>{subtitle && <p className="muted">{subtitle}</p>}</div>{action}</div>}{children}</section>;
}
export function PageHead({ title, description, action }: { title: string; description: string; action?: ReactNode }) { return <div className="page-head"><div><span className="eyebrow">CRANIBOX / WORKSPACE</span><h1>{title}</h1><p>{description}</p></div>{action && <div className="actions">{action}</div>}</div>; }
export function Metric({ icon, label, value, detail, to }: { icon: LucideIcon; label: string; value: ReactNode; detail?: string; to?: string }) { return <Link to={to || '/overview'} className="metric-card"><div className="metric-top"><span className="metric-icon"><Icon icon={icon} /></span><Icon icon={ArrowUpRight} /></div><span className="metric-label">{label}</span><strong className="metric-value">{value}</strong><span className="muted">{detail}</span></Link>; }
export function Empty({ title, detail, action }: { title: string; detail?: string; action?: ReactNode }) { return <div className="empty"><span className="empty-mark"><Circle size={24} aria-hidden="true" /></span><h3>{title}</h3>{detail && <p>{detail}</p>}{action}</div>; }
export function Filter({ label, value, onChange }: { label: string; value: string; onChange: (value: string) => void }) { return <label className="search"><Icon icon={Search} /><span className="sr-only">{label}</span><input type="search" placeholder={label} value={value} onChange={event => onChange(event.target.value)} /></label>; }
export function Row({ icon, title, detail, children }: { icon?: LucideIcon; title: ReactNode; detail?: ReactNode; children?: ReactNode }) { return <div className="list-row">{icon && <span className="row-icon"><Icon icon={icon} /></span>}<div className="row-main"><strong>{title}</strong>{detail && <div className="muted">{detail}</div>}</div><div className="row-end">{children}</div></div>; }
export function Facts({ items }: { items: [string, ReactNode][] }) { return <dl className="facts">{items.map(([label, value]) => <div key={label}><dt>{label}</dt><dd>{value ?? '—'}</dd></div>)}</dl>; }
