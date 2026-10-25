import { AccessLog, AccessSurfaces } from "../access/Access";
import { Activity, Bell, Boxes, Container, FolderGit2, House, Inbox, Languages, Lock, Menu, Moon, Network, RefreshCw, Settings as SettingsIcon, Sparkles, Sun, X, type LucideIcon } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { Link, Navigate, Route, Routes, useLocation, useNavigate } from "react-router-dom";
import { Overlays } from "../overlays";
import { Agents, Browser, Diagnostics, NetworkPage as DomainsPage, Overview, Resources, Services } from "../pages";
import { Desktop } from "../pages/Desktop";
import { History } from "../pages/History";
import { Messages } from "../pages/Messages";
import { Performance } from "../pages/Performance";
import { Projects } from "../pages/Projects";
import { Spaces } from "../pages/Spaces";
import { Tasks } from "../pages/Tasks";
import { Onboarding as WorkspaceOnboarding } from "../pages/Onboarding";
import { TerminalWindow, TerminalWindowToggle } from "../terminal/TerminalWindow";
import { useTerminalWindow } from "../terminal/TerminalWindowContext";
import { useWorkspace } from "../workspace";
import { Capabilities } from "./Capabilities";
import { Settings } from "./Connections";
import { DeviceDetail } from "./DeviceDetail";
import { Devices, Home } from "./Home";
import { AgentAccess, IdentitySettings } from "./Identity";
import { NetworkPage } from "./Network";
import { Approvals } from "./Inbox";
import { Models } from "./Models";
import { PairingSheet } from "./Pairing";
import { SetupGuide } from "./Setup";
import { resolvedTheme, stored, useApp } from "./store";
import { Alert, Empty, Loading, PageHead, Sheet, StatusButton, SubTabs, Tabs, type TabItem, type Translate } from "./ui";
import "./app.css";

type SectionId = "home" | "devices" | "spaces" | "projects" | "inbox" | "models" | "settings";
type Page = { path: string; section: SectionId; title: (t: Translate) => string; description?: (t: Translate) => string; element: ReactNode; heading?: boolean; workspace?: boolean };
const workspacePaths = ["/overview", "/desktop", "/network", "/services", "/resources", "/browser", "/performance", "/diagnostics", "/history"];
function pages(models: ReactNode): Page[] {
  return [
    { path: "/app/home", section: "home", title: t => t("首页", "Home"), element: needsSetup() ? <Navigate to="/app/setup" replace /> : <Home /> },
    { path: "/app/setup", section: "home", title: t => t("设置向导", "Setup guide"), element: <SetupGuide /> },
    { path: "/app/access-log", section: "devices", title: t => t("访问记录", "Access log"), element: <AccessLog /> },
    { path: "/app/devices", section: "devices", title: t => t("设备", "Devices"), element: <Devices /> },
    { path: "/nodes", section: "devices", title: t => t("网络设置", "Network settings"), element: <NetworkPage /> },
    { path: "/spaces", section: "spaces", title: t => t("虚拟空间", "Virtual spaces"), heading: true, description: t => t("在设备上创建隔离的工作空间，带桌面和编辑器；也是 Agent 自己的电脑。", "Isolated workspaces with their own desktop and editor, created on your devices — your agents' own computers."), element: <Spaces /> },
    { path: "/projects", section: "projects", title: t => t("项目与会话", "Projects & sessions"), element: <Projects /> },
    { path: "/app/inbox", section: "inbox", title: t => t("审批", "Approvals"), heading: true, description: t => t("每一个重要决定都先经过你：配对、Agent 工具、屏幕控制。", "Every important decision comes to you first: pairing, agent tools, screen control."), element: <Approvals /> },
    { path: "/tasks", section: "inbox", title: t => t("任务", "Tasks"), heading: true, description: t => t("在设备之间委派和跟踪的协作任务。", "Collaboration tasks delegated and tracked across your devices."), element: <Tasks /> },
    { path: "/messages", section: "inbox", title: t => t("消息", "Messages"), heading: true, description: t => t("设备与 Agent 会话之间的消息。", "Messages between your devices and agent sessions."), element: <Messages /> },
    { path: "/app/models", section: "models", title: t => t("模型与 Agent", "Models & agents"), heading: true, description: t => t("连接订阅，为每个 Agent 选择模型。", "Connect subscriptions and choose a model for each agent."), element: models },
    { path: "/agents", section: "models", title: t => t("工作区 Agent", "Workspace agents"), element: <Agents />, workspace: true },
    { path: "/app/settings", section: "settings", title: t => t("通用", "General"), heading: true, description: t => t("外观、语言、自启动与隐私。", "Appearance, language, startup and privacy."), element: <Settings /> },
    { path: "/app/this", section: "settings", title: t => t("能力与权限", "Capabilities & permissions"), heading: true, description: t => t("这台设备提供的终端、桌面、Computer Use 与浏览器：安装、系统授权与共享。", "Terminal, desktop, Computer Use and browser on this device: install, system permissions and sharing."), element: <Capabilities /> },
    { path: "/app/identity", section: "settings", title: t => t("身份与安全", "Identity & security"), heading: true, description: t => t("身份码、恢复码，以及移除丢失的设备。", "Identity code, recovery code, and removing a lost device."), element: <IdentitySettings /> },
    { path: "/app/agent-access", section: "settings", title: t => t("Agent 接入", "Agent access"), heading: true, description: t => t("让 Codex、Claude Code 等 Agent 使用你的设备。", "Let Codex, Claude Code and other agents use your devices."), element: <AgentAccess /> },
    { path: "/overview", section: "settings", title: t => t("工作区", "Workspace"), element: <><WorkspaceOnboarding /><Overview /></>, workspace: true },
    { path: "/desktop", section: "settings", title: t => t("工作区桌面", "Workspace desktop"), element: <Desktop />, workspace: true },
    { path: "/network", section: "settings", title: t => t("域名与路由", "Domains & routes"), element: <DomainsPage />, workspace: true },
    { path: "/services", section: "settings", title: t => t("服务", "Services"), element: <Services />, workspace: true },
    { path: "/resources", section: "settings", title: () => "MCP & Skills", element: <Resources />, workspace: true },
    { path: "/browser", section: "settings", title: t => t("托管浏览器", "Managed browser"), element: <Browser />, workspace: true },
    { path: "/performance", section: "settings", title: t => t("资源与性能", "Resources & performance"), element: <Performance />, workspace: true },
    { path: "/diagnostics", section: "settings", title: t => t("诊断", "Diagnostics"), element: <Diagnostics />, workspace: true },
    { path: "/history", section: "settings", title: t => t("操作历史", "Operation history"), element: <History />, workspace: true },
  ];
}
const sections: { id: SectionId; to: string; icon: LucideIcon; label: (t: Translate) => string }[] = [
  { id: "home", to: "/app/home", icon: House, label: t => t("首页", "Home") },
  { id: "devices", to: "/app/devices", icon: Network, label: t => t("设备与网络", "Devices & network") },
  { id: "spaces", to: "/spaces", icon: Container, label: t => t("虚拟空间", "Virtual spaces") },
  { id: "projects", to: "/projects", icon: FolderGit2, label: t => t("项目与会话", "Projects & sessions") },
  { id: "inbox", to: "/app/inbox", icon: Inbox, label: t => t("待办", "Inbox") },
  { id: "models", to: "/app/models", icon: Sparkles, label: t => t("Agent 与模型", "Agents & models") },
  { id: "settings", to: "/app/settings", icon: SettingsIcon, label: t => t("本机设置", "Settings") },
];
function sectionTabs(section: SectionId, t: Translate, workspace: boolean, approvals: number): TabItem[] {
  if (section === "devices") return [{ to: "/app/devices", label: t("设备", "Devices") }, { to: "/nodes", label: t("网络设置", "Network settings") }, { to: "/app/access-log", label: t("访问记录", "Access log") }];
  if (section === "inbox") return [{ to: "/app/inbox", label: t("审批", "Approvals"), count: approvals }, { to: "/tasks", label: t("任务", "Tasks") }, { to: "/messages", label: t("消息", "Messages") }];
  if (section === "models") return workspace ? [{ to: "/app/models", label: t("模型与 Agent", "Models & agents") }, { to: "/agents", label: t("工作区 Agent", "Workspace agents") }] : [];
  if (section === "settings") return [{ to: "/app/settings", label: t("通用", "General") }, { to: "/app/this", label: t("能力与权限", "Capabilities") }, { to: "/app/identity", label: t("身份与安全", "Identity & security") }, { to: "/app/agent-access", label: t("Agent 接入", "Agent access") }, ...(workspace ? [{ to: "/overview", label: t("工作区", "Workspace") }] : [])];
  return [];
}
/** First launch on a node that remembers preferences (browser storage covers demo nodes). */
function needsSetup() {
  return window.workspaceWeb?.preferences?.onboarded === false && stored("cranibox.onboarded", "") !== "1";
}
/** How the other devices are reached right now, for the sidebar footer. */
function pathSummary(devices: ReturnType<typeof useApp>["devices"], t: Translate) {
  const online = devices.filter(device => !device.badges.includes("this") && device.online);
  if (!online.length) return t("暂无其他设备在线", "no other device online");
  const relayed = online.filter(device => device.path === "relay").length;
  if (!relayed) return t("全部直连", "all direct");
  return relayed === online.length ? t("经中继", "via relay") : t(`${online.length - relayed} 直连 · ${relayed} 中继`, `${online.length - relayed} direct · ${relayed} relayed`);
}
function TerminalRoute() {
  const location = useLocation(), navigate = useNavigate(), terminal = useTerminalWindow();
  useEffect(() => {
    const query = new URLSearchParams(location.search);
    terminal.openTerminal({ node: query.get("node") || "", session: query.get("session") || undefined });
    navigate("/app/home", { replace: true });
  }, []);
  return null;
}
export function Shell({ chatUrl = "/proxy/3080/" }: { chatUrl?: string }) {
  const app = useApp(), { t, approvals, status, loading, error, notice, setNotice, refresh, thisDevice, theme, setTheme, locale, setLocale } = app;
  const workspace = useWorkspace();
  const location = useLocation(), navigate = useNavigate();
  const [menu, setMenu] = useState(false), [statusOpen, setStatusOpen] = useState(false), [adding, setAdding] = useState(false);
  const main = useRef<HTMLElement>(null), sidebar = useRef<HTMLElement>(null), trigger = useRef<HTMLButtonElement>(null);
  const hasWorkspace = workspace.available === true;
  const all = pages(<Models chatUrl={chatUrl} adding={adding} setAdding={setAdding} />);
  const path = location.pathname;
  const device = path.startsWith("/app/device/");
  const page = all.find(item => item.path === path);
  const section: SectionId = device ? "devices" : page?.section || "home";
  useEffect(() => { setMenu(false); main.current?.scrollTo?.(0, 0); }, [path]);
  useEffect(() => { document.title = (device ? t("设备", "Device") : page ? page.title(t) : "Cranibox") + " · Cranibox"; }, [path, t]);
  useEffect(() => {
    if (!menu) return;
    const controls = () => Array.from(sidebar.current?.querySelectorAll<HTMLElement>("a[href], button:not(:disabled)") || []);
    controls()[0]?.focus();
    const close = () => { setMenu(false); requestAnimationFrame(() => trigger.current?.focus()); };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
      if (event.key === "Tab") {
        const items = controls(), first = items[0], last = items[items.length - 1];
        if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
        else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
      }
    };
    const wide = matchMedia("(min-width: 861px)"), resized = () => { if (wide.matches) setMenu(false); };
    document.addEventListener("keydown", key); wide.addEventListener("change", resized);
    return () => { document.removeEventListener("keydown", key); wide.removeEventListener("change", resized); };
  }, [menu]);
  const tabs = device ? [] : sectionTabs(section, t, hasWorkspace, approvals.length);
  const workspaceTabs: TabItem[] = [{ to: "/overview", label: t("概览", "Overview") }, { to: "/desktop", label: t("桌面", "Desktop") }, { to: "/services", label: t("服务", "Services") }, { to: "/network", label: t("域名与路由", "Domains & routes") }, { to: "/resources", label: "MCP & Skills" }, { to: "/browser", label: t("托管浏览器", "Managed browser") }, { to: "/performance", label: t("性能", "Performance") }, { to: "/diagnostics", label: t("诊断", "Diagnostics") }, { to: "/history", label: t("操作历史", "History") }];
  const inWorkspace = workspacePaths.includes(path);
  const currentSection = sections.find(item => item.id === section)!;
  const dark = resolvedTheme(theme) === "dark";
  function render(item: Page) {
    if (item.workspace && !hasWorkspace) return workspace.available === null ? <Loading label={t("正在连接工作区…", "Connecting to the workspace…")} />
      : <Empty icon={Boxes} title={t("这里没有容器工作区", "No container workspace here")} actions={<Link className="cx-btn" to="/app/settings">{t("返回本机设置", "Back to Settings")}</Link>}>{t("服务、MCP 与 Skills、性能和诊断面板只在 Cranibox 容器工作区中提供。", "Services, MCP & Skills, performance and diagnostics are only available in a Cranibox container workspace.")}</Empty>;
    return <>{item.heading && <PageHead title={item.title(t)} description={item.description?.(t)} />}{item.element}</>;
  }
  return <div className="cx-app">
    <a className="cx-skip" href="#main-content" onClick={event => { event.preventDefault(); main.current?.focus(); }}>{t("跳至主要内容", "Skip to main content")}</a>
    <aside ref={sidebar} id="app-navigation" className={"cx-sidebar " + (menu ? "open" : "")} role={menu ? "dialog" : undefined} aria-modal={menu || undefined} aria-label={menu ? t("导航", "Navigation") : undefined}>
      <div className="cx-between" style={{ flexWrap: "nowrap" }}>
        <Link className="cx-brand" to="/app/home"><span className="cx-brand-mark"><Boxes size={21} aria-hidden="true" /></span><span><strong>Cranibox</strong><small>{t("设备互联的 Agent 工作台", "The agent workspace across your devices")}</small></span></Link>
        {menu && <button className="cx-icon-btn" style={{ marginRight: 12 }} aria-label={t("关闭导航", "Close navigation")} onClick={() => { setMenu(false); requestAnimationFrame(() => trigger.current?.focus()); }}><X size={18} aria-hidden="true" /></button>}
      </div>
      <nav className="cx-nav" aria-label={t("主导航", "Main navigation")}>
        {sections.map(item => <Link key={item.id} to={item.to} className="cx-nav-link" onClick={() => { if (menu) { setMenu(false); requestAnimationFrame(() => main.current?.focus()); } }} aria-current={item.id === section ? "page" : undefined}>
          <item.icon size={19} aria-hidden="true" /><span>{item.label(t)}</span>{item.id === "inbox" && approvals.length > 0 && <span className="cx-count" aria-label={t(approvals.length + " 项待审批", approvals.length + " pending")}>{approvals.length}</span>}
        </Link>)}
      </nav>
      <div className="cx-sidebar-foot">
        <div className="cx-row"><span className={"cx-dot " + (thisDevice ? "online" : "")} aria-hidden="true" /><strong>{thisDevice?.name || workspace.snapshot?.access?.hostname || "Cranibox"}</strong></div>
        <span className="cx-row"><Lock size={13} aria-hidden="true" />{t("端到端加密", "End-to-end encrypted")} · {pathSummary(app.devices, t)}</span>
        {workspace.snapshot?.revision && <span className="cx-revision sidebar-revision">{t("版本 ", "Revision ")}{workspace.snapshot.revision}</span>}
      </div>
    </aside>
    {menu && <button className="cx-scrim" aria-label={t("关闭导航", "Close navigation")} onClick={() => { setMenu(false); requestAnimationFrame(() => trigger.current?.focus()); }} />}
    <div className="cx-main" inert={menu}>
      <header className="cx-topbar">
        <div className="cx-crumbs">
          <button ref={trigger} className="cx-icon-btn cx-menu-button" aria-label={t("打开导航", "Open navigation")} aria-expanded={menu} aria-controls="app-navigation" onClick={() => setMenu(!menu)}><Menu size={19} aria-hidden="true" /></button>
          <Link to={currentSection.to}>{currentSection.label(t)}</Link>
          {(device || (page && page.path !== currentSection.to)) && <><span aria-hidden="true">/</span><strong>{device ? app.deviceName(decodeURIComponent(path.split("/")[3] || "")) : page!.title(t)}</strong></>}
        </div>
        <div className="cx-top-actions">
          <span className="cx-hide-narrow"><StatusButton status={status} t={t} onClick={() => setStatusOpen(true)} /></span>
          <button className="cx-icon-btn" aria-label={t("待审批", "Pending approvals") + " " + approvals.length} onClick={() => navigate("/app/inbox")}><Bell size={19} aria-hidden="true" />{approvals.length > 0 && <span className="cx-count">{approvals.length}</span>}</button>
          {inWorkspace && hasWorkspace && <><button className="cx-icon-btn" aria-label={t("操作日志", "Activity log")} onClick={() => workspace.setOutputOpen(true)}><Activity size={18} aria-hidden="true" /></button>
            <button className="cx-icon-btn" aria-label={t("刷新状态", "Refresh status")} disabled={workspace.state.refreshing} onClick={workspace.refresh}><RefreshCw size={18} aria-hidden="true" /></button></>}
          <button className="cx-icon-btn cx-lang" aria-label={t("切换到 English", "Switch to 中文")} title={t("语言", "Language")} onClick={() => setLocale(locale.startsWith("zh") ? "en" : "zh-cn")}><Languages size={16} aria-hidden="true" /><span>{locale.startsWith("zh") ? "中" : "EN"}</span></button>
          <TerminalWindowToggle />
          <button className="cx-icon-btn" aria-label={dark ? t("切换浅色主题", "Switch to light theme") : t("切换深色主题", "Switch to dark theme")} onClick={() => setTheme(dark ? "light" : "dark")}>{dark ? <Sun size={18} aria-hidden="true" /> : <Moon size={18} aria-hidden="true" />}</button>
        </div>
      </header>
      <AccessSurfaces />
      <main id="main-content" ref={main} tabIndex={-1} className="cx-content">
        {error && <Alert action={<button className="cx-btn small" onClick={() => void refresh()}>{t("重试", "Retry")}</button>}>{error}</Alert>}
        {inWorkspace && workspace.error && <Alert tone="warn" action={<button className="cx-btn small" onClick={workspace.refresh}>{t("重试", "Retry")}</button>}><strong>{t("无法获取最新状态", "Unable to get the latest status")}</strong> {workspace.error}{workspace.snapshot && " " + t("下方保留上次成功更新的数据。", "The last successful snapshot is shown below.")}</Alert>}
        {notice && <div className="cx-notice" role="status"><span>{notice}</span><button className="cx-icon-btn" aria-label={t("关闭通知", "Dismiss notification")} onClick={() => setNotice("")}><X size={16} aria-hidden="true" /></button></div>}
        {workspace.notice && <div className="cx-notice" role="status"><span>{workspace.notice.message}</span><button className="cx-icon-btn" aria-label={t("关闭通知", "Dismiss notice")} onClick={workspace.dismissNotice}><X size={16} aria-hidden="true" /></button></div>}
        {tabs.length > 0 && <Tabs label={t("分区", "Sections")} items={tabs.map(item => item.to === "/overview" ? { ...item, to: inWorkspace ? path : "/overview" } : item)} />}
        {inWorkspace && hasWorkspace && <SubTabs label={t("工作区", "Workspace")} items={workspaceTabs} />}
        {loading ? <Loading label={t("正在连接你的设备…", "Connecting to your devices…")} /> : <Routes>
          <Route path="/" element={<Navigate to="/app/home" replace />} />
          {all.map(item => <Route key={item.path} path={item.path} element={render(item)} />)}
          <Route path="/app/device/:id/:tab?" element={<DeviceDetail />} />
          <Route path="/terminal" element={<TerminalRoute />} />
          <Route path="/app/advanced" element={workspace.available === null ? <Loading label={t("正在连接工作区…", "Connecting to the workspace…")} /> : <Navigate to={hasWorkspace ? "/overview" : "/app/this"} replace />} />
          <Route path="*" element={<Navigate to="/app/home" replace />} />
        </Routes>}
      </main>
      <footer className="cx-footer"><Lock size={12} aria-hidden="true" />{t("你的设备，你的数据，你来决定。", "Your devices. Your data. Your decisions.")}</footer>
    </div>
    <TerminalWindow suspended={menu} />
    <Overlays />
    <PairingSheet />
    {statusOpen && <Sheet title={t("当前状态", "Current status")} t={t} onClose={() => setStatusOpen(false)}>
      <p>{status?.state === "local" ? t("暂时连不上你的其他设备。本机的一切照常可用，恢复连接后会自动同步。", "None of your other devices are reachable right now. Everything on this device keeps working and syncs once they reconnect.")
        : status?.state === "offline" ? t("本机节点暂时离线。", "This device's node is offline.")
        : t("可用模型：", "Available models: ") + (status?.models_available || 0)}</p>
      <p>{t("待审批：", "Pending approvals: ")}{approvals.length}</p>
      <div className="cx-actions"><button className="cx-btn" onClick={() => { setStatusOpen(false); navigate("/app/models"); }}>{t("管理模型", "Manage models")}</button><button className="cx-btn" onClick={() => { setStatusOpen(false); navigate("/app/inbox"); }}>{t("打开待办", "Open inbox")}</button></div>
    </Sheet>}
  </div>;
}
