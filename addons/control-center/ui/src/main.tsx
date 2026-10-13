import { Activity,ArrowUpRight,Bot,Boxes,Globe,LayoutDashboard,Menu,Monitor,Moon,Network,RefreshCw,Server,ShieldCheck,Sun,Terminal,Workflow,X } from 'lucide-react';
import { useEffect,useRef,useState } from 'react';
import { createRoot } from 'react-dom/client';
import { HashRouter,Navigate,NavLink,Route,Routes,useLocation,useNavigate } from 'react-router-dom';
import { Button,Empty,Icon } from './components';
import { Overlays } from './overlays';
import { Agents,Browser,Desktop,Diagnostics,NetworkPage,Onboarding,Overview,Resources,Services } from './pages';
import { History } from './pages/History';
import { TerminalWindow,TerminalWindowToggle } from './terminal/TerminalWindow';
import { TerminalWindowProvider,useTerminalWindow } from './terminal/TerminalWindowContext';
import { Nodes } from './pages/Nodes';
import { Performance } from './pages/Performance';
import './style.css';
import './terminal/terminal.css';
import './terminal/window.css';
import { openUrl,t,useWorkspace,WorkspaceProvider } from './workspace';

const nav = [
  { id: 'overview', label: t('总览', 'Overview'), icon: LayoutDashboard },
  { id: 'terminal', label: t('终端', 'Terminal'), icon: Terminal },
  { id: 'nodes', label: t('远程节点', 'Remote nodes'), icon: Server },
  { id: 'history', label: t('操作历史', 'Operation history'), icon: Activity },
  { id: 'performance', label: t('资源与性能', 'Resources & performance'), icon: Activity },
  { id: 'agents', label: 'Agents', icon: Bot },
  { id: 'browser', label: t('浏览器', 'Browser'), icon: Globe },
  { id: 'resources', label: 'MCP & Skills', icon: Workflow },
  { id: 'services', label: t('服务', 'Services'), icon: Server },
  { id: 'desktop', label: t('桌面', 'Desktop'), icon: Monitor },
  { id: 'network', label: t('网络', 'Network'), icon: Network },
  { id: 'diagnostics', label: t('诊断', 'Diagnostics'), icon: ShieldCheck },
];
function App() {
  const workspace = useWorkspace();
  const { snapshot, state, error, notice } = workspace;
  const location = useLocation(), navigate = useNavigate();
  const terminal = useTerminalWindow();
  useEffect(() => {
    if (location.pathname !== '/terminal') return;
    const query = new URLSearchParams(location.search);
    terminal.openTerminal({ node: query.get('node') || '', session: query.get('session') || undefined });
    navigate('/overview', { replace: true });
  }, [location.pathname, location.search, terminal.openTerminal, navigate]);
  const [theme, setTheme] = useState(() => { try { return localStorage.getItem('workspace.theme') === 'light' ? 'light' : 'dark'; } catch { return 'dark'; } });
  const [menu, setMenu] = useState(false);
  const main = useRef<HTMLElement>(null), menuTrigger = useRef<HTMLDivElement>(null), sidebar = useRef<HTMLElement>(null);
  const current = nav.find(item => location.pathname === '/' + item.id) || nav[0];
  useEffect(() => { document.documentElement.dataset.theme = theme; try { localStorage.setItem('workspace.theme', theme); } catch {} }, [theme]);
  useEffect(() => { setMenu(false); main.current?.scrollTo(0, 0); document.title = current.label + ' · Cranibox'; }, [location.pathname, current.label]);
  useEffect(() => {
    if (!menu) return;
    const controls = () => Array.from(sidebar.current?.querySelectorAll<HTMLElement>('a[href], button:not(:disabled)') || []).filter(item => item.offsetParent !== null);
    controls()[0]?.focus();
    const handleKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') { setMenu(false); requestAnimationFrame(() => menuTrigger.current?.querySelector('button')?.focus()); }
      if (event.key === 'Tab') {
        const items = controls(), first = items[0], last = items[items.length - 1];
        if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
        else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
      }
    };
    const desktop = window.matchMedia('(min-width: 901px)');
    const resized = () => { if (desktop.matches) setMenu(false); };
    document.addEventListener('keydown', handleKey); desktop.addEventListener('change', resized);
    return () => { document.removeEventListener('keydown', handleKey); desktop.removeEventListener('change', resized); };
  }, [menu]);
  const updated = state.snapshotAt ? new Intl.DateTimeFormat(window.workspaceWeb.language === 'zh-cn' ? 'zh-CN' : 'en-US', { hour: '2-digit', minute: '2-digit', second: '2-digit' }).format(new Date(state.snapshotAt)) : '—';
  return <div className="app"><a href="#main-content" className="skip-link" onClick={event => { event.preventDefault(); main.current?.focus(); }}>{t('跳至主要内容', 'Skip to main content')}</a><aside ref={sidebar} role={menu ? 'dialog' : undefined} aria-modal={menu || undefined} aria-label={menu ? t('工作区导航', 'Workspace navigation') : undefined} className={'sidebar ' + (menu ? 'menu-open' : '')} id="workspace-navigation"><div className="sidebar-brand"><LinkBrand /><button className="button secondary icon-button mobile-menu-close" aria-label={t('关闭导航', 'Close navigation')} onClick={() => { setMenu(false); requestAnimationFrame(() => menuTrigger.current?.querySelector('button')?.focus()); }}><Icon icon={X} /></button></div><div className="nav-section-label">{t('工作区管理', 'WORKSPACE')}</div><nav aria-label={t('工作区导航', 'Workspace navigation')}>{nav.map(item => item.id === 'terminal' ? <button key={item.id} className={'nav-link terminal-nav-button' + (terminal.mode === 'open' ? ' active' : '')} aria-label={item.label} title={item.label} aria-expanded={terminal.mode === 'open'} onClick={() => { setMenu(false); terminal.openTerminal(); }}><Icon icon={Terminal} /><span>{item.label}</span></button> : <NavLink key={item.id} aria-label={item.label} title={item.label} to={'/' + item.id} onClick={() => { if (menu) { setMenu(false); requestAnimationFrame(() => main.current?.focus()); } }} className={({ isActive }) => 'nav-link' + (isActive ? ' active' : '')}><Icon icon={item.icon} /><span>{item.label}</span>{item.id === 'agents' && snapshot && <span className="nav-count">{snapshot.agents?.filter(a => a.installed).length || 0}</span>}</NavLink>)}</nav><div className="sidebar-bottom"><div className="workspace-label"><span className={'connection-dot ' + (error ? 'warning' : '')} /><strong>{snapshot?.access?.hostname || 'Cranibox'}</strong></div><code>{snapshot?.workspace?.root || '/config/Workspace'}</code><span className="sidebar-revision">{t('运行版本', 'REVISION')} {snapshot?.revision || '—'}</span></div></aside>{menu && <button className="menu-scrim" aria-label={t('关闭导航', 'Close navigation')} onClick={() => { setMenu(false); requestAnimationFrame(() => menuTrigger.current?.querySelector('button')?.focus()); }} />}
    <div className="workspace-shell" inert={menu}><header className="topbar"><div className="breadcrumb"><div className="mobile-menu" ref={menuTrigger}><button className="button secondary icon-button" aria-label={menu ? t('关闭导航', 'Close navigation') : t('打开导航', 'Open navigation')} aria-expanded={menu} aria-controls="workspace-navigation" onClick={() => setMenu(!menu)}><Icon icon={menu ? X : Menu} /></button></div><span className="breadcrumb-workspace">Workspace</span><span className="breadcrumb-separator">/</span><strong>{current.label}</strong></div><div className="topbar-actions"><TerminalWindowToggle /><Button local icon={Activity} onClick={() => workspace.setOutputOpen(true)} label={t('操作日志', 'Activity log')} /><Button local icon={RefreshCw} onClick={workspace.refresh} label={t('刷新状态', 'Refresh status')} disabled={state.refreshing} /><Button local onClick={() => workspace.action('switchLanguage')} label={t('切换为英语', 'Switch to Chinese')}>{t('EN', '中文')}</Button><Button local icon={theme === 'dark' ? Sun : Moon} onClick={() => setTheme(theme === 'dark' ? 'light' : 'dark')} label={theme === 'dark' ? t('切换浅色主题', 'Switch to light theme') : t('切换深色主题', 'Switch to dark theme')} /></div></header>
    <div className="sync-bar"><div className="sync-state"><span className={'connection-dot ' + (error ? 'warning' : '')} /><span>{error ? t('状态更新失败', 'Status update failed') : state.refreshing ? t('正在更新状态', 'Updating status') : snapshot ? t('状态已同步', 'Status synchronized') : t('正在连接工作区', 'Connecting to workspace')}</span></div><div className="sync-controls"><span>{t('更新于 ', 'Updated at ')}<time>{updated}</time></span>{state.busy && <><button className="text-link" onClick={() => workspace.setOutputOpen(true)}><Icon icon={Activity} />{t('正在执行操作', 'Operation running')}</button><button className="text-link danger-text" onClick={workspace.cancel}>{t('取消', 'Cancel')}</button></>}</div></div>
    {error && <div className="banner error-banner" role="alert"><div><strong>{t('无法获取最新状态', 'Unable to get the latest status')}</strong><p>{error}{snapshot && ' ' + t('下方保留上次成功更新的数据。', 'The last successful snapshot is shown below.')}</p></div><Button local onClick={workspace.refresh}>{t('重试', 'Retry')}</Button></div>}
    {notice && <div className="banner" role="status"><span>{notice.message}</span><div className="actions">{notice.url && <Button local icon={ArrowUpRight} onClick={() => openUrl(notice.url!)}>{t('打开', 'Open')}</Button>}<Button local icon={X} label={t('关闭通知', 'Dismiss notice')} onClick={workspace.dismissNotice} /></div></div>}
    <main id="main-content" ref={main} tabIndex={-1}><div className="page-content">{snapshot ? <Routes><Route path="/terminal" element={<Overview />} /><Route path="/nodes" element={<Nodes />} /><Route path="/history" element={<History />} /><Route path="/performance" element={<Performance />} /><Route path="/overview" element={<><Onboarding /><Overview /></>} /><Route path="/agents" element={<Agents />} /><Route path="/browser" element={<Browser />} /><Route path="/resources" element={<Resources />} /><Route path="/services" element={<Services />} /><Route path="/desktop" element={<Desktop />} /><Route path="/network" element={<NetworkPage />} /><Route path="/diagnostics" element={<Diagnostics />} /><Route path="*" element={<Navigate to="/overview" replace />} /></Routes> : error ? <Empty title={t('暂时无法连接工作区', 'Workspace is currently unavailable')} detail={t('检查网络连接后重试。', 'Check your connection and try again.')} action={<Button action="refresh" tone="primary">{t('重新连接', 'Reconnect')}</Button>} /> : <div className="loading-state" role="status"><div className="loading-header" /><div className="loading-hero" /><div className="loading-cards"><span /><span /><span /><span /></div><p>{t('正在读取工作区状态…', 'Reading workspace status…')}</p></div>}<footer className="page-footer"><span>Cranibox</span><span>{t('持久化工作区 · 浏览器管理', 'Persistent workspace · Browser control')}</span></footer></div></main></div><TerminalWindow suspended={menu} /><Overlays /></div>;
}
function LinkBrand() { return <a className="brand" href="#/overview"><span className="brand-mark"><Boxes size={25} strokeWidth={1.7} aria-hidden="true" /></span><span><strong>Cranibox</strong><small>AGENT WORKSPACE</small></span></a>; }
// Keep bookmarks from the previous control center working.
const legacy = location.hash.slice(1);
if (nav.some(item => item.id === legacy)) history.replaceState(null, '', '#/' + legacy);
createRoot(document.getElementById('root')!).render(<HashRouter><TerminalWindowProvider><WorkspaceProvider><App /></WorkspaceProvider></TerminalWindowProvider></HashRouter>);
