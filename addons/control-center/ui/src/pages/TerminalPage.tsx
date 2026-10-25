// Workspace navigation adapted from devswha/herdr-web-ui (MIT).
// See docs/licenses/herdr-web-ui.txt for the upstream notice.
import { ChevronDown, Columns2, Folder, PanelLeft, Plus, RefreshCw, Search, Server, ShieldCheck, Terminal, X } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { Button, Modal } from '../terminal/ui';
import { useTerminalEnvironment } from '../terminal/TerminalEnvironment';
import { TerminalPane } from '../terminal/TerminalPane';
import { TerminalMenu } from '../terminal/TerminalMenu';
import { terminalKey, type Catalog, type OpenTerminal, type Session } from '../terminal/types';
import { useTerminalWindow } from '../terminal/TerminalWindowContext';
import { useTerminalDevices } from '../app/targets';

const tabsStorage = 'cranibox.terminal.tabs', maxTabs = 4;
function savedTabs(): OpenTerminal[] {
  try {
    const value: unknown = JSON.parse(sessionStorage.getItem(tabsStorage) || '[]');
    return Array.isArray(value) ? value.filter((item): item is OpenTerminal => item && ['node', 'id', 'name', 'cwd'].every(key => typeof item[key] === 'string')).slice(0, maxTabs) : [];
  } catch { return []; }
}
export function TerminalPage({ visible = true }: { visible?: boolean }) {
  const { t, includeLocal, localNode, manage } = useTerminalEnvironment();
  const { nodes, displayName } = useTerminalDevices();
  const terminalWindow = useTerminalWindow();
  const { select, consume, report, launch } = terminalWindow;
  const target = terminalWindow.selection.node || localNode || '', selected = terminalWindow.selection.id;
  const activeKey = terminalKey({ node: target, id: selected });
  const [tabs, setTabs] = useState(savedTabs), [split, setSplit] = useState('');
  const [catalog, setCatalog] = useState<Catalog | null>(null), [loading, setLoading] = useState(true);
  const [error, setError] = useState(''), [filter, setFilter] = useState(''), [pending, setPending] = useState(false);
  const [drawer, setDrawer] = useState(false), [name, setName] = useState(''), [cwd, setCwd] = useState('');
  const [command, setCommand] = useState('');
  const [dialog, setDialog] = useState<{ operation: 'create' } | { operation: 'rename' | 'end'; session: Session; node: string } | null>(null);
  const [rename, setRename] = useState('');
  const mounted = useRef(true), mutation = useRef(false), listing = useRef(0), restored = useRef(false), closingSelection = useRef('');
  const catalogOwner = useRef<string | null>(null);
  const findNode = (id: string) => nodes.find(node => node.id === id) || nodes.find(node => node.aliases?.includes(id));
  const targetNode = findNode(target);
  const online = !target ? includeLocal : !!targetNode?.available;
  const machineName = (node: string) => displayName(node) || (includeLocal ? t('当前工作区', 'This workspace') : t('选择设备', 'Choose a device'));
  const machineOnline = (node: string) => !node ? includeLocal : !!findNode(node)?.available;
  const sessions = catalog?.sessions || [];
  const filtered = sessions.filter(item => (item.name + ' ' + item.cwd).toLowerCase().includes(filter.toLowerCase()));
  const active = tabs.find(item => terminalKey(item) === activeKey);
  const openTabs = useRef(tabs), availableNodes = useRef(machineOnline);
  openTabs.current = tabs; availableNodes.current = machineOnline;
  useEffect(() => {
    const canonical = (id: string) => (nodes.find(node => node.id === id) || nodes.find(node => node.aliases?.includes(id)))?.id || id;
    setTabs(previous => {
      if (!previous.some(item => canonical(item.node) !== item.node)) return previous;
      return previous.map(item => ({ ...item, node: canonical(item.node) }))
        .filter((item, index, values) => values.findIndex(value => terminalKey(value) === terminalKey(item)) === index);
    });
    const beside = tabs.find(item => terminalKey(item) === split);
    if (beside && canonical(beside.node) !== beside.node) setSplit(terminalKey({ ...beside, node: canonical(beside.node) }));
    const node = canonical(terminalWindow.selection.node);
    if (node !== terminalWindow.selection.node) select({ node, id: selected });
  }, [nodes, tabs, split, terminalWindow.selection.node, selected, select]);
  useEffect(() => {
    if (localNode) setTabs(previous => previous.some(item => !item.node) ? previous
      .map(item => item.node ? item : { ...item, node: localNode })
      .filter((item, index, values) => values.findIndex(value => terminalKey(value) === terminalKey(item)) === index) : previous);
  }, [localNode]);

  const refresh = useCallback(async (showLoading = false) => {
    const current = ++listing.current;
    if (showLoading) setLoading(true);
    try {
      const owners = [...new Set([target, ...openTabs.current.map(item => item.node)])].filter(node => availableNodes.current(node));
      const results = await Promise.allSettled(owners.map(async node => ({ node, value: await manage(node, { operation: 'list' }) as Catalog })));
      if (!mounted.current || current !== listing.current) return;
      const catalogs = new Map<string, Catalog>();
      results.forEach((result, index) => {
        if (result.status === 'fulfilled') catalogs.set(result.value.node, result.value.value);
        else if (owners[index] === target) setError(result.reason instanceof Error ? result.reason.message : String(result.reason));
      });
      const value = catalogs.get(target);
      if (value) { catalogOwner.current = target; setCatalog(value); }
      else if (!online) setCatalog(null);
      setTabs(previous => previous.map(item => {
        const catalog = catalogs.get(item.node);
        if (!catalog) return item;
        const session = catalog.sessions.find(candidate => candidate.id === item.id);
        return session ? { ...item, name: session.name, cwd: session.cwd, ended: item.ended || session.status === 'ended' } : { ...item, ended: true };
      }));
    } catch (problem) { if (mounted.current && current === listing.current) setError(problem instanceof Error ? problem.message : String(problem)); }
    finally { if (mounted.current && current === listing.current) setLoading(false); }
  }, [target, online, manage]);
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
    try { sessionStorage.setItem(tabsStorage, JSON.stringify(tabs.map(({ node, id, name, cwd, ended }) => ({ node, id, name, cwd, ended })))); } catch { /* Storage is optional. */ }
    if (split && !tabs.some(item => terminalKey(item) === split)) setSplit('');
  }, [tabs, split]);
  useEffect(() => {
    if (restored.current) return;
    restored.current = true;
    if (!selected && !terminalWindow.selection.node && tabs.length && !launch) { const item = tabs[0]; select({ node: item.node, id: item.id }); }
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
    if (!launch || !visible || loading || mutation.current || target !== (launch.node !== undefined ? launch.node || localNode || '' : target) || online && catalogOwner.current !== target) return;
    consume(launch.serial);
    if (launch.session) return;
    if (!online || !catalog?.installed) { setError(target && target !== localNode ? t('节点终端暂不可用，请检查客户端连接。', 'Terminals are unavailable. Check the node connection.') : t('工作区终端暂不可用，请检查 herdr。', 'Workspace terminals are unavailable. Check herdr.')); return; }
    if (!launch.command) {
      const existing = catalog.sessions.find(item => item.id === selected && item.status === 'running') || catalog.sessions.find(item => item.status === 'running');
      if (existing) { openSession(existing); return; }
    }
    if (tabs.length >= maxTabs) { setError(t('最多同时打开 4 个标签。先关闭一个标签，后台会话会保留。', 'Up to 4 tabs can be open. Close a tab to keep its session running.')); return; }
    mutation.current = true; setPending(true); setError('');
    void manage(target, { operation: 'create', name: launch.name || automaticName(target, launch.cwd), cwd: launch.cwd || null }).then(value => {
      if (!mounted.current) return;
      setTabs(previous => [...previous, { node: target, id: value.session.id, name: value.session.name, cwd: value.session.cwd, initialCommand: launch.command }]);
      navigate({ node: target, id: value.session.id });
      void refresh();
    }).catch(problem => { if (mounted.current) setError(problem instanceof Error ? problem.message : String(problem)); }).finally(() => { mutation.current = false; if (mounted.current) setPending(false); });
  }, [launch, visible, loading, target, localNode, catalog, online, selected, tabs.length, consume]);
  function createDialog() { setName(''); setCwd(''); setCommand(''); setError(''); setDrawer(false); setDialog({ operation: 'create' }); }
  function sessionDialog(operation: 'rename' | 'end', item: Session, node = target) { setRename(item.name); setError(''); setDrawer(false); setDialog({ operation, session: item, node }); }
  function automaticName(node: string, directory = '') {
    const base = (directory.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || machineName(node)).slice(0, 65);
    const names = new Set([...tabs.filter(item => item.node === node).map(item => item.name), ...(node === target ? sessions.map(item => item.name) : [])]);
    let number = 1;
    while (names.has(base + ' ' + number)) number++;
    return base + ' ' + number;
  }
  async function createSession(node = target, directory = '', sessionName = '', startCommand = '', replace?: OpenTerminal) {
    if (mutation.current) return false;
    mutation.current = true; listing.current++; setPending(true); setError('');
    try {
      if (!machineOnline(node)) throw Error(t('设备离线，请恢复连接后再试。', 'Device offline. Try again when it reconnects.'));
      if (!replace && tabs.length >= maxTabs) throw Error(t('先关闭一个标签再创建；后台会话不会停止。', 'Close a tab before creating another. Its session keeps running.'));
      const value = await manage(node, { operation: 'create', name: sessionName.trim() || automaticName(node, directory), cwd: directory.trim() || null });
      if (!mounted.current) return false;
      const entry: OpenTerminal = { node, id: value.session.id, name: value.session.name, cwd: value.session.cwd, startCommand: startCommand.trim() || undefined };
      setTabs(previous => replace ? previous.map(item => terminalKey(item) === terminalKey(replace) ? entry : item) : [...previous, entry]);
      if (replace && split === terminalKey(replace)) setSplit(terminalKey(entry));
      navigate(entry);
      return true;
    } catch (problem) { if (mounted.current) setError(problem instanceof Error ? problem.message : String(problem)); return false; }
    finally { mutation.current = false; if (mounted.current) { setPending(false); void refresh(); } }
  }
  function markEnded(key: string) {
    setTabs(previous => previous.map(item => terminalKey(item) === key && !item.ended ? { ...item, ended: true } : item));
  }
  async function submit() {
    if (!dialog || mutation.current) return;
    if (dialog.operation === 'create') {
      if (await createSession(target, cwd, name, command)) setDialog(null);
      return;
    }
    const owner = dialog.node;
    mutation.current = true; setPending(true); setError('');
    try {
      await manage(owner, { operation: dialog.operation, id: dialog.session.id, ...(dialog.operation === 'rename' ? { name: rename.trim() } : {}) });
      if (!mounted.current) return;
      const key = terminalKey({ node: owner, id: dialog.session.id });
      if (dialog.operation === 'end') closeTab(key);
      else setTabs(previous => previous.map(item => terminalKey(item) === key ? { ...item, name: rename.trim() } : item));
      setDialog(null); await refresh();
    } catch (problem) { if (mounted.current) setError(problem instanceof Error ? problem.message : String(problem)); }
    finally { mutation.current = false; if (mounted.current) setPending(false); }
  }
  const tabSession = (item: OpenTerminal): Session => ({ ...item, status: item.ended ? 'ended' : 'running', created_at: 0, last_attached_at: null });
  const devicePicker = <div className="terminal-target" title={machineName(target)}><Server size={18} aria-hidden="true" /><label><span className="sr-only">{t('目标', 'Target')}</span><select value={target} disabled={pending} onChange={event => navigate(tabs.find(item => item.node === event.target.value) || { node: event.target.value, id: '' })}><option value="" disabled={!includeLocal}>{includeLocal ? machineName('') + t(' · 在线', ' · Online') : t('选择设备', 'Choose a device')}</option>{target && !targetNode && <option value={target} disabled>{machineName(target)}{t(' · 离线', ' · Offline')}</option>}{nodes.map(node => <option key={node.id} value={node.id} disabled={!node.available}>{machineName(node.id)}{node.available ? t(' · 在线', ' · Online') : t(' · 离线', ' · Offline')}</option>)}</select></label></div>;
  const sessionList = <>
    <div className="terminal-list-heading"><h2>{t('会话', 'Sessions')} <span>{sessions.length}</span></h2><div className="actions"><Button local icon={RefreshCw} label={t('刷新会话', 'Refresh sessions')} disabled={loading || pending} onClick={() => void refresh(true)} /><Button local icon={Plus} label={t('新建会话', 'New session')} disabled={!online || !catalog?.installed || pending || tabs.length >= maxTabs} onClick={() => void createSession()} /></div></div>
    <label className="terminal-search"><Search size={16} aria-hidden="true" /><span className="sr-only">{t('搜索会话', 'Search sessions')}</span><input type="search" placeholder={t('搜索会话…', 'Search sessions…')} value={filter} onChange={event => setFilter(event.target.value)} /></label>
    {!online && <p className="terminal-list-note">{!target ? t('选择已授权的直连设备以查看会话。', 'Choose an authorized direct device to view its sessions.') : t('此设备的终端暂不可用，请检查连接与授权。', 'Terminals are unavailable. Check the device connection and grants.')}</p>}
    {online && catalog && !catalog.installed && <p className="terminal-list-note">{target && target !== localNode ? t('节点终端暂不可用，请更新客户端后刷新。', 'Terminals are unavailable. Update the node client, then refresh.') : t('工作区未安装 herdr，请安装后刷新。', 'Install herdr in this workspace, then refresh.')}</p>}
    <div className="terminal-session-list" aria-label={t('会话列表', 'Session list')}>
      {filtered.map(item => <div key={item.id} className={'terminal-session-row' + (item.id === selected ? ' selected' : '')}>
        <button className="terminal-session-select" aria-label={t('打开 ', 'Open ') + item.name} aria-pressed={item.id === selected} disabled={pending || item.status !== 'running'} onClick={() => openSession(item)}><Terminal size={17} aria-hidden="true" /><span><strong title={item.name}>{item.name}</strong><small title={item.cwd}>{item.cwd}</small></span><span className={'terminal-session-dot ' + item.status} /><span className="sr-only">{item.status === 'running' ? t('运行中', 'Running') : t('已结束', 'Ended')}</span></button>
        <TerminalMenu label={t('会话操作 ', 'Session actions ') + item.name}><button role="menuitem" disabled={item.status !== 'running'} onClick={() => openSession(item, true)}>{t('并排打开', 'Open beside')}</button><button role="menuitem" aria-label={t('重命名 ', 'Rename ') + item.name} onClick={() => sessionDialog('rename', item)}>{t('重命名', 'Rename')}</button><button role="menuitem" className="danger-text" aria-label={t('结束 ', 'End ') + item.name} onClick={() => sessionDialog('end', item)}>{t('结束会话…', 'End session…')}</button></TerminalMenu>
      </div>)}
      {!filtered.length && <div className="terminal-list-empty"><Folder size={22} aria-hidden="true" /><p>{loading ? t('正在加载…', 'Loading…') : filter ? t('没有匹配的会话', 'No matching sessions') : t('还没有会话', 'No sessions yet')}</p>{!filter && <span>{t('新建一个终端开始工作', 'Create a terminal to get started')}</span>}</div>}
    </div>
    <div className="terminal-sidebar-foot"><ShieldCheck size={15} aria-hidden="true" /><span>{t('关闭窗口后，会话继续运行', 'Sessions survive closing this window')}</span></div>
  </>;
  return <div className="terminal-workbench">
    <h1 className="sr-only">{t('终端', 'Terminal')}</h1>
    <aside className="terminal-sidebar" aria-label={t('终端会话', 'Terminal sessions')}>{sessionList}</aside>
    <div className="terminal-stage">
      <div className="terminal-toolbar">{devicePicker}<div className="terminal-create-split"><Button local icon={Plus} label={t('新建会话', 'New session')} disabled={!online || !catalog?.installed || pending || tabs.length >= maxTabs} onClick={() => void createSession()} /><TerminalMenu icon={ChevronDown} label={t('新建会话选项', 'New session options')}><button role="menuitem" disabled={!online || !catalog?.installed || pending || tabs.length >= maxTabs} onClick={createDialog}>{t('新建（更多选项）…', 'New with options…')}</button></TerminalMenu></div></div>
      <div className="terminal-tabs-bar">
        <button className="terminal-drawer-toggle" type="button" aria-label={t('打开会话列表', 'Open session list')} aria-expanded={drawer} onClick={() => setDrawer(true)}><PanelLeft size={19} aria-hidden="true" /></button>
        <div className="terminal-tabs" role="tablist" aria-label={t('打开的终端', 'Open terminals')}>
          {tabs.map(item => { const key = terminalKey(item); return <div key={key} className={'terminal-tab' + (key === activeKey ? ' active' : '') + (key === split ? ' beside' : '')}><button role="tab" tabIndex={key === activeKey || !active && tabs[0] === item ? 0 : -1} aria-selected={key === activeKey} aria-controls={'pane-' + item.id} id={'tab-' + item.id} title={machineName(item.node) + ' · ' + item.name} onClick={event => { if (event.detail) event.currentTarget.blur(); selectTab(item); }} onDoubleClick={() => { if (!item.ended) sessionDialog('rename', tabSession(item), item.node); }} onKeyDown={event => {
            const index = tabs.indexOf(item), next = event.key === 'ArrowRight' ? (index + 1) % tabs.length : event.key === 'ArrowLeft' ? (index - 1 + tabs.length) % tabs.length : event.key === 'Home' ? 0 : event.key === 'End' ? tabs.length - 1 : -1;
            if (next >= 0) { event.preventDefault(); selectTab(tabs[next]); document.getElementById('tab-' + tabs[next].id)?.focus(); }
          }}><Terminal size={15} aria-hidden="true" /><span>{item.name}</span>{item.node && <Server size={12} aria-hidden="true" />}</button><button className="terminal-tab-close" aria-label={t('关闭标签 ', 'Close tab ') + item.name} title={item.ended ? t('关闭标签', 'Close tab') : t('关闭标签，保留后台会话', 'Close tab, keep session running')} onClick={() => closeTab(key)}><X size={14} aria-hidden="true" /></button></div>; })}
        </div>
        {split && <Button local icon={Columns2} label={t('退出并排查看', 'Exit side by side')} onClick={() => setSplit('')} />}
      </div>
      {error && !dialog && <div className="terminal-alert" role="alert"><span>{error}</span><Button local icon={X} label={t('关闭错误提示', 'Dismiss error')} onClick={() => setError('')} /></div>}
      <div className={'terminal-panes' + (active && split && split !== activeKey ? ' is-split' : '')}>
        {tabs.map(item => { const key = terminalKey(item); return <div key={key} id={'pane-' + item.id} role="tabpanel" aria-labelledby={'tab-' + item.id} className={'terminal-panel' + (key === activeKey ? ' active' : key === split && active ? ' beside' : '')}>
          <TerminalPane pending={pending} onCheckSession={() => void refresh()} onEnded={() => markEnded(key)} onRecreate={() => void createSession(item.node, item.cwd, '', '', item)} entry={item} visible={visible && (key === activeKey || !!active && key === split)} online={machineOnline(item.node)} machine={machineName(item.node)} onClose={() => closeTab(key)} onRename={() => sessionDialog('rename', tabSession(item), item.node)} onEnd={() => sessionDialog('end', tabSession(item), item.node)} />
        </div>; })}
        {!active && <div className="terminal-welcome"><div className="terminal-welcome-mark"><Terminal size={32} strokeWidth={1.5} aria-hidden="true" /></div><h2>{t('你的终端工作台', 'Your terminal workspace')}</h2><p>{t('选择一个会话继续工作，或新建一个终端。', 'Pick a session to continue, or create a new terminal.')}</p><Button local tone="primary" icon={Plus} disabled={!online || !catalog?.installed || pending || tabs.length >= maxTabs} onClick={() => void createSession()}>{t('新建会话', 'New session')}</Button><span><ShieldCheck size={15} aria-hidden="true" />{t('关闭窗口，任务继续运行', 'Close this window. Your tasks keep running.')}</span></div>}
      </div>
      <footer className="terminal-statusbar"><span><span className={'connection-dot ' + (!online ? 'warning' : '')} />{machineName(target)}</span><span>{tabs.length}/{maxTabs} {t('标签', 'tabs')}<span className="terminal-status-hint"> · {t('关闭标签保留会话', 'Closing a tab keeps its session')}</span></span></footer>
    </div>
    {drawer && <Modal title={t('会话', 'Sessions')} className="terminal-session-drawer" onCancel={() => setDrawer(false)}>{sessionList}</Modal>}
    {dialog && <Modal title={dialog.operation === 'create' ? t('新建会话', 'New session') : dialog.operation === 'rename' ? t('重命名会话', 'Rename session') : t('结束会话', 'End session')} onCancel={() => { if (!pending) { setDialog(null); setError(''); } }} className="terminal-session-dialog"><form onSubmit={event => { event.preventDefault(); void submit(); }}>
      <p className="terminal-dialog-machine"><Server size={16} aria-hidden="true" />{machineName(dialog.operation === 'create' ? target : dialog.node)}</p>
      {dialog.operation === 'create' ? <><label className="field"><span>{t('会话名称', 'Session name')}</span><input autoFocus maxLength={80} value={name} disabled={pending} onChange={event => setName(event.target.value)} placeholder={t('例如：构建任务', 'e.g. Build task')} /></label><details className="terminal-directory"><summary>{t('指定工作目录', 'Choose a working directory')}</summary><label className="field"><span>{t('工作目录（可选）', 'Working directory (optional)')}</span><input value={cwd} disabled={pending} onChange={event => setCwd(event.target.value)} placeholder={t('使用默认工作目录', 'Use the default working directory')} /></label></details><label className="field"><span>{t('启动命令（可选）', 'Start command (optional)')}</span><input value={command} disabled={pending} onChange={event => setCommand(event.target.value)} placeholder={t('例如：npm run dev', 'e.g. npm run dev')} /></label><p className="muted">{t('创建后自动打开，关闭窗口也不会中断。', 'Opens automatically. Closing this window keeps it running.')}</p></> : dialog.operation === 'rename' ? <label className="field"><span>{t('会话名称', 'Session name')}</span><input autoFocus required maxLength={80} value={rename} disabled={pending} onChange={event => setRename(event.target.value)} /></label> : <p>{t('结束“', 'End “')}{dialog.session.name}{t('”？此会话的命令与后台任务将停止。关闭标签可以保留它们。', '”? Commands and background tasks will stop. Close its tab to keep them running.')}</p>}
      {error && <p className="form-error" role="alert">{error}</p>}<div className="modal-actions"><Button local disabled={pending} onClick={() => { setDialog(null); setError(''); }}>{t('取消', 'Cancel')}</Button><Button local type="submit" tone={dialog.operation === 'end' ? 'danger' : 'primary'} disabled={pending}>{pending ? t('正在处理…', 'Working…') : dialog.operation === 'create' ? t('创建并打开', 'Create and open') : dialog.operation === 'rename' ? t('保存', 'Save') : t('结束会话', 'End session')}</Button></div>
    </form></Modal>}
  </div>;
}
