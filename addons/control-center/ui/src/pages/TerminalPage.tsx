// Workspace navigation adapted from devswha/herdr-web-ui (MIT).
// See docs/licenses/herdr-web-ui.txt for the upstream notice.
import { Columns2, Folder, PanelLeft, Plus, RefreshCw, Search, Server, ShieldCheck, Terminal, X } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { Button } from '../components';
import { Modal } from '../overlays';
import { request, t, useWorkspace } from '../workspace';
import { TerminalPane } from '../terminal/TerminalPane';
import { TerminalMenu } from '../terminal/TerminalMenu';
import { terminalKey, type Catalog, type OpenTerminal, type Session } from '../terminal/types';
import { useTerminalWindow } from '../terminal/TerminalWindowContext';

const tabsStorage = 'cranibox.terminal.tabs', maxTabs = 4;
function savedTabs(): OpenTerminal[] {
  try {
    const value: unknown = JSON.parse(sessionStorage.getItem(tabsStorage) || '[]');
    return Array.isArray(value) ? value.filter((item): item is OpenTerminal => item && ['node', 'id', 'name', 'cwd'].every(key => typeof item[key] === 'string')).slice(0, maxTabs) : [];
  } catch { return []; }
}
export function TerminalPage({ visible = true }: { visible?: boolean }) {
  const { snapshot } = useWorkspace();
  const terminalWindow = useTerminalWindow();
  const { select, consume, report, launch } = terminalWindow;
  const target = terminalWindow.selection.node, selected = terminalWindow.selection.id;
  const activeKey = terminalKey({ node: target, id: selected });
  const [tabs, setTabs] = useState(savedTabs), [split, setSplit] = useState('');
  const [catalog, setCatalog] = useState<Catalog | null>(null), [loading, setLoading] = useState(true);
  const [error, setError] = useState(''), [filter, setFilter] = useState(''), [pending, setPending] = useState(false);
  const [drawer, setDrawer] = useState(false), [name, setName] = useState(''), [cwd, setCwd] = useState('');
  const [dialog, setDialog] = useState<{ operation: 'create' } | { operation: 'rename' | 'end'; session: Session; node: string } | null>(null);
  const [rename, setRename] = useState('');
  const mounted = useRef(true), mutation = useRef(false), listing = useRef(0), restored = useRef(false), closingSelection = useRef('');
  const catalogOwner = useRef<string | null>(null);
  const nodes = snapshot?.remote_nodes?.nodes || [];
  const targetNode = nodes.find(node => node.id === target);
  const online = !target || !!(targetNode?.online && targetNode.terminal_sessions);
  const machineName = (node: string) => node ? nodes.find(item => item.id === node)?.name || node : t('当前工作区', 'This workspace');
  const machineOnline = (node: string) => !node || nodes.some(item => item.id === node && item.online && item.terminal_sessions);
  const sessions = catalog?.sessions || [];
  const filtered = sessions.filter(item => (item.name + ' ' + item.cwd).toLowerCase().includes(filter.toLowerCase()));
  const active = tabs.find(item => terminalKey(item) === activeKey);

  const refresh = useCallback(async (showLoading = false) => {
    const current = ++listing.current;
    if (!online) { setCatalog(null); setLoading(false); return; }
    if (showLoading) setLoading(true);
    try {
      const value: Catalog = await request('/api/terminal/sessions', { node: target || null, operation: 'list' });
      if (!mounted.current || current !== listing.current) return;
      catalogOwner.current = target;
      setCatalog(value);
      setTabs(previous => previous.flatMap(item => {
        if (item.node !== target) return [item];
        const session = value.sessions.find(candidate => candidate.id === item.id);
        return session?.status === 'running' ? [{ ...item, name: session.name, cwd: session.cwd }] : session?.status === 'unknown' ? [item] : [];
      }));
    } catch (problem) { if (mounted.current && current === listing.current) setError(problem instanceof Error ? problem.message : String(problem)); }
    finally { if (mounted.current && current === listing.current) setLoading(false); }
  }, [target, online]);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; listing.current++; };
  }, []);
  useEffect(() => {
    catalogOwner.current = null; setCatalog(null); setFilter(''); setError(''); void refresh(true);
    const timer = globalThis.setInterval(() => { if (!document.hidden && !mutation.current) void refresh(); }, 15000);
    return () => { listing.current++; globalThis.clearInterval(timer); };
  }, [refresh]);
  useEffect(() => {
    try { sessionStorage.setItem(tabsStorage, JSON.stringify(tabs.map(({ node, id, name, cwd }) => ({ node, id, name, cwd })))); } catch { /* Storage is optional. */ }
    if (split && !tabs.some(item => terminalKey(item) === split)) setSplit('');
  }, [tabs, split]);
  useEffect(() => {
    if (restored.current) return;
    restored.current = true;
    if (!selected && !target && tabs.length && !launch) { const item = tabs[0]; select({ node: item.node, id: item.id }); }
  }, [tabs, target, selected, select, launch]);
  useEffect(() => {
    if (closingSelection.current !== activeKey) closingSelection.current = '';
    if (loading || !selected || active || !catalog || closingSelection.current === activeKey) return;
    const item = catalog.sessions.find(value => value.id === selected && value.status === 'running');
    if (item && tabs.length < maxTabs) setTabs(previous => previous.some(value => terminalKey(value) === activeKey) ? previous : [...previous, { node: target, id: item.id, name: item.name, cwd: item.cwd }]);
  }, [catalog, selected, target, active, activeKey, tabs.length, loading]);
  useEffect(() => {
    report(tabs.length, active?.name || '');
  }, [tabs.length, active?.name, report]);
  function navigate(item?: { node: string; id: string }) {
    select(item || { node: '', id: '' });
    setDrawer(false); setError('');
  }
  function openSession(item: Session, beside = false) {
    const value: OpenTerminal = { node: target, id: item.id, name: item.name, cwd: item.cwd }, key = terminalKey(value);
    if (!tabs.some(tab => terminalKey(tab) === key)) {
      if (tabs.length >= maxTabs) { setError(t('最多同时打开 4 个标签。关闭标签可保留后台会话。', 'Open up to 4 tabs. Close a tab to keep its session running.')); return; }
      setTabs(previous => [...previous, value]);
    }
    if (beside && active && activeKey !== key) { setSplit(key); setDrawer(false); }
    else { if (split === key) setSplit(activeKey); navigate(value); }
  }
  function closeTab(key: string) {
    // Selection can update after tab state. Avoid immediately reopening a closed tab.
    if (key === activeKey) closingSelection.current = key;
    const next = tabs.filter(item => terminalKey(item) !== key); setTabs(next);
    if (key === split) setSplit('');
    if (key === activeKey) navigate(next.find(item => item.node === target) || next[0] || { node: target, id: '' });
  }
  function selectTab(item: OpenTerminal) { if (split === terminalKey(item)) setSplit(activeKey); navigate(item); }
  useEffect(() => {
    if (visible) document.querySelector<HTMLElement>('.terminal-tab.active')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }, [activeKey, visible]);
  useEffect(() => {
    if (!launch || !visible || loading || mutation.current || target !== (launch.node ?? target) || online && catalogOwner.current !== target) return;
    consume(launch.serial);
    if (launch.session) return;
    if (!online || !catalog?.installed) { setError(t('此节点的持久终端暂不可用，请检查节点或 herdr。', 'Persistent terminals are unavailable. Check this node and herdr.')); return; }
    if (!launch.command) {
      const existing = catalog.sessions.find(item => item.id === selected && item.status === 'running') || catalog.sessions.find(item => item.status === 'running');
      if (existing) { openSession(existing); return; }
    }
    if (tabs.length >= maxTabs) { setError(t('最多同时打开 4 个标签。先关闭一个标签，后台会话会保留。', 'Up to 4 tabs can be open. Close a tab to keep its session running.')); return; }
    mutation.current = true; setPending(true); setError('');
    void request('/api/terminal/sessions', { node: target || null, operation: 'create', name: launch.name || t('终端 ', 'Terminal ') + (sessions.length + 1), cwd: launch.cwd || null }).then(value => {
      if (!mounted.current) return;
      setTabs(previous => [...previous, { node: target, id: value.session.id, name: value.session.name, cwd: value.session.cwd, initialCommand: launch.command }]);
      navigate({ node: target, id: value.session.id });
      void refresh();
    }).catch(problem => { if (mounted.current) setError(problem instanceof Error ? problem.message : String(problem)); }).finally(() => { mutation.current = false; if (mounted.current) setPending(false); });
  }, [launch, visible, loading, target, catalog, online, selected, tabs.length, consume]);
  function createDialog() { setName(''); setCwd(''); setError(''); setDrawer(false); setDialog({ operation: 'create' }); }
  function sessionDialog(operation: 'rename' | 'end', item: Session, node = target) { setRename(item.name); setError(''); setDrawer(false); setDialog({ operation, session: item, node }); }
  async function submit() {
    if (!dialog || mutation.current) return;
    const owner = dialog.operation === 'create' ? target : dialog.node;
    mutation.current = true; setPending(true); setError('');
    try {
      if (dialog.operation === 'create') {
        if (tabs.length >= maxTabs) throw Error(t('先关闭一个标签再创建；后台会话不会停止。', 'Close a tab before creating another. Its session keeps running.'));
        const value = await request('/api/terminal/sessions', { node: owner || null, operation: 'create', name: name.trim() || t('终端 ', 'Terminal ') + (sessions.length + 1), cwd: cwd.trim() || null });
        if (!mounted.current) return;
        setTabs(previous => [...previous, { node: owner, id: value.session.id, name: value.session.name, cwd: value.session.cwd }]);
        navigate({ node: owner, id: value.session.id });
      } else {
        await request('/api/terminal/sessions', { node: owner || null, operation: dialog.operation, id: dialog.session.id, ...(dialog.operation === 'rename' ? { name: rename.trim() } : {}) });
        if (!mounted.current) return;
        const key = terminalKey({ node: owner, id: dialog.session.id });
        if (dialog.operation === 'end') closeTab(key);
        else setTabs(previous => previous.map(item => terminalKey(item) === key ? { ...item, name: rename.trim() } : item));
      }
      setDialog(null); await refresh();
    } catch (problem) { if (mounted.current) setError(problem instanceof Error ? problem.message : String(problem)); }
    finally { mutation.current = false; if (mounted.current) setPending(false); }
  }
  const tabSession = (item: OpenTerminal): Session => ({ ...item, status: 'running', created_at: 0, last_attached_at: null });
  const sessionList = <>
    <div className="terminal-target"><Server size={18} aria-hidden="true" /><label><span className="sr-only">{t('目标', 'Target')}</span><select value={target} disabled={pending} onChange={event => navigate(tabs.find(item => item.node === event.target.value) || { node: event.target.value, id: '' })}><option value="">{t('当前工作区', 'This workspace')}</option>{nodes.map(node => <option key={node.id} value={node.id}>{node.name || node.id}{!node.online ? t(' · 离线', ' · Offline') : ''}</option>)}</select></label></div>
    <div className="terminal-list-heading"><h2>{t('会话', 'Sessions')} <span>{sessions.length}</span></h2><div className="actions"><Button local icon={RefreshCw} label={t('刷新会话', 'Refresh sessions')} disabled={loading || pending} onClick={() => void refresh(true)} /><Button local icon={Plus} label={t('新建会话', 'New session')} disabled={!online || !catalog?.installed || pending} onClick={createDialog} /></div></div>
    <label className="terminal-search"><Search size={16} aria-hidden="true" /><span className="sr-only">{t('搜索会话', 'Search sessions')}</span><input type="search" placeholder={t('搜索会话…', 'Search sessions…')} value={filter} onChange={event => setFilter(event.target.value)} /></label>
    {!online && <p className="terminal-list-note">{targetNode?.online ? t('此节点暂不支持持久终端，远程命令仍可用。', 'This node does not support persistent terminals. Remote commands remain available.') : t('节点离线，恢复后可接回会话。', 'Node offline. Reattach when it reconnects.')}</p>}
    {online && catalog && !catalog.installed && <p className="terminal-list-note">{t('此节点未安装 herdr，请安装后刷新。', 'Install herdr on this machine, then refresh.')}</p>}
    <div className="terminal-session-list" aria-label={t('会话列表', 'Session list')}>
      {filtered.map(item => <div key={item.id} className={'terminal-session-row' + (item.id === selected ? ' selected' : '')}>
        <button className="terminal-session-select" aria-label={t('打开 ', 'Open ') + item.name} aria-pressed={item.id === selected} disabled={pending || item.status !== 'running'} onClick={() => openSession(item)}><Terminal size={17} aria-hidden="true" /><span><strong title={item.name}>{item.name}</strong><small title={item.cwd}>{item.cwd}</small></span><span className={'terminal-session-dot ' + item.status} /><span className="sr-only">{item.status === 'running' ? t('运行中', 'Running') : t('已结束', 'Ended')}</span></button>
        <TerminalMenu label={t('会话操作 ', 'Session actions ') + item.name}><button role="menuitem" disabled={item.status !== 'running'} onClick={() => openSession(item, true)}>{t('并排打开', 'Open beside')}</button><button role="menuitem" aria-label={t('重命名 ', 'Rename ') + item.name} onClick={() => sessionDialog('rename', item)}>{t('重命名', 'Rename')}</button><button role="menuitem" className="danger-text" aria-label={t('结束 ', 'End ') + item.name} onClick={() => sessionDialog('end', item)}>{t('结束会话…', 'End session…')}</button></TerminalMenu>
      </div>)}
      {!filtered.length && <div className="terminal-list-empty"><Folder size={22} aria-hidden="true" /><p>{loading ? t('正在加载…', 'Loading…') : filter ? t('没有匹配的会话', 'No matching sessions') : t('还没有会话', 'No sessions yet')}</p>{!filter && <span>{t('新建一个终端开始工作', 'Create a terminal to get started')}</span>}</div>}
    </div>
    <div className="terminal-sidebar-foot"><ShieldCheck size={15} aria-hidden="true" /><span>{t('关闭网页后，会话继续运行', 'Sessions survive closing the browser')}</span></div>
  </>;
  return <div className="terminal-workbench">
    <h1 className="sr-only">{t('终端', 'Terminal')}</h1>
    <aside className="terminal-sidebar" aria-label={t('终端会话', 'Terminal sessions')}>{sessionList}</aside>
    <div className="terminal-stage">
      <div className="terminal-tabs-bar">
        <button className="terminal-drawer-toggle" type="button" aria-label={t('打开会话列表', 'Open session list')} aria-expanded={drawer} onClick={() => setDrawer(true)}><PanelLeft size={19} aria-hidden="true" /></button>
        <div className="terminal-tabs" role="tablist" aria-label={t('打开的终端', 'Open terminals')}>
          {tabs.map(item => { const key = terminalKey(item); return <div key={key} className={'terminal-tab' + (key === activeKey ? ' active' : '') + (key === split ? ' beside' : '')}><button role="tab" tabIndex={key === activeKey || !active && tabs[0] === item ? 0 : -1} aria-selected={key === activeKey} aria-controls={'pane-' + item.id} id={'tab-' + item.id} title={machineName(item.node) + ' · ' + item.name} onClick={event => { if (event.detail) event.currentTarget.blur(); selectTab(item); }} onKeyDown={event => {
            const index = tabs.indexOf(item), next = event.key === 'ArrowRight' ? (index + 1) % tabs.length : event.key === 'ArrowLeft' ? (index - 1 + tabs.length) % tabs.length : event.key === 'Home' ? 0 : event.key === 'End' ? tabs.length - 1 : -1;
            if (next >= 0) { event.preventDefault(); selectTab(tabs[next]); document.getElementById('tab-' + tabs[next].id)?.focus(); }
          }}><Terminal size={15} aria-hidden="true" /><span>{item.name}</span>{item.node && <Server size={12} aria-hidden="true" />}</button><button className="terminal-tab-close" aria-label={t('关闭标签 ', 'Close tab ') + item.name} title={t('关闭标签，保留后台会话', 'Close tab, keep session running')} onClick={() => closeTab(key)}><X size={14} aria-hidden="true" /></button></div>; })}
        </div>
        <Button local icon={Plus} label={t('新建会话', 'New session')} disabled={!online || !catalog?.installed || pending || tabs.length >= maxTabs} onClick={createDialog} />
        {split && <Button local icon={Columns2} label={t('退出并排查看', 'Exit side by side')} onClick={() => setSplit('')} />}
      </div>
      {error && !dialog && <div className="terminal-alert" role="alert"><span>{error}</span><Button local icon={X} label={t('关闭错误提示', 'Dismiss error')} onClick={() => setError('')} /></div>}
      <div className={'terminal-panes' + (active && split && split !== activeKey ? ' is-split' : '')}>
        {tabs.map(item => { const key = terminalKey(item); return <div key={key} id={'pane-' + item.id} role="tabpanel" aria-labelledby={'tab-' + item.id} className={'terminal-panel' + (key === activeKey ? ' active' : key === split && active ? ' beside' : '')}>
          <TerminalPane entry={item} visible={visible && (key === activeKey || !!active && key === split)} online={machineOnline(item.node)} machine={machineName(item.node)} onClose={() => closeTab(key)} onRename={() => sessionDialog('rename', tabSession(item), item.node)} onEnd={() => sessionDialog('end', tabSession(item), item.node)} />
        </div>; })}
        {!active && <div className="terminal-welcome"><div className="terminal-welcome-mark"><Terminal size={32} strokeWidth={1.5} aria-hidden="true" /></div><h2>{t('你的终端工作台', 'Your terminal workspace')}</h2><p>{t('选择一个会话继续工作，或新建一个终端。', 'Pick a session to continue, or create a new terminal.')}</p><Button local tone="primary" icon={Plus} disabled={!online || !catalog?.installed || pending} onClick={createDialog}>{t('新建会话', 'New session')}</Button><span><ShieldCheck size={15} aria-hidden="true" />{t('关闭网页，任务继续运行', 'Close the browser. Your tasks keep running.')}</span></div>}
      </div>
      <footer className="terminal-statusbar"><span><span className={'connection-dot ' + (!online ? 'warning' : '')} />{machineName(target)}</span><span>{tabs.length}/{maxTabs} {t('标签', 'tabs')}<span className="terminal-status-hint"> · {t('关闭标签保留会话', 'Closing a tab keeps its session')}</span></span></footer>
    </div>
    {drawer && <Modal title={t('会话', 'Sessions')} className="terminal-session-drawer" onCancel={() => setDrawer(false)}>{sessionList}</Modal>}
    {dialog && <Modal title={dialog.operation === 'create' ? t('新建会话', 'New session') : dialog.operation === 'rename' ? t('重命名会话', 'Rename session') : t('结束会话', 'End session')} onCancel={() => { if (!pending) { setDialog(null); setError(''); } }} className="terminal-session-dialog"><form onSubmit={event => { event.preventDefault(); void submit(); }}>
      {dialog.operation === 'create' ? <><p className="terminal-dialog-machine"><Server size={16} aria-hidden="true" />{machineName(target)}</p><label className="field"><span>{t('会话名称', 'Session name')}</span><input autoFocus maxLength={80} value={name} disabled={pending} onChange={event => setName(event.target.value)} placeholder={t('例如：构建任务', 'e.g. Build task')} /></label><details className="terminal-directory"><summary>{t('指定工作目录', 'Choose a working directory')}</summary><label className="field"><span>{t('工作目录（可选）', 'Working directory (optional)')}</span><input value={cwd} disabled={pending} onChange={event => setCwd(event.target.value)} placeholder={t('使用默认工作目录', 'Use the default working directory')} /></label></details><p className="muted">{t('创建后自动打开，关闭网页也不会中断。', 'Opens automatically. Closing the browser keeps it running.')}</p></> : dialog.operation === 'rename' ? <label className="field"><span>{t('会话名称', 'Session name')}</span><input autoFocus required maxLength={80} value={rename} disabled={pending} onChange={event => setRename(event.target.value)} /></label> : <p>{t('结束“', 'End “')}{dialog.session.name}{t('”？此会话的命令与后台任务将停止。关闭标签可以保留它们。', '”? Commands and background tasks will stop. Close its tab to keep them running.')}</p>}
      {error && <p className="form-error" role="alert">{error}</p>}<div className="modal-actions"><Button local disabled={pending} onClick={() => { setDialog(null); setError(''); }}>{t('取消', 'Cancel')}</Button><Button local type="submit" tone={dialog.operation === 'end' ? 'danger' : 'primary'} disabled={pending}>{pending ? t('正在处理…', 'Working…') : dialog.operation === 'create' ? t('创建并打开', 'Create and open') : dialog.operation === 'rename' ? t('保存', 'Save') : t('结束会话', 'End session')}</Button></div>
    </form></Modal>}
  </div>;
}
