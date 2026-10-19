import {
  useCallback,
  useEffect,
  useState,
  useRef,
  type ReactNode,
} from "react";
import {
  Bell,
  Bot,
  ChevronRight,
  CircleCheck,
  House,
  Inbox,
  Laptop,
  ListTodo,
  LoaderCircle,
  Menu,
  Monitor,
  Moon,
  Plus,
  Settings,
  ShieldCheck,
  Sparkles,
  Sun,
  X,
} from "lucide-react";
import {
  type Api,
  type Account,
  type Accepted,
  type Agent,
  type Approval,
  type Device,
  type Invite,
  type Model,
  type Pill,
  type Preset,
  type SignIn,
  type TaskSummary,
  agentNames,
} from "./api";
import {
  ApprovalCard,
  Card,
  Pill as Tag,
  QrPanel,
  Sheet,
  StatusPill,
} from "./components";
import "./personal.css";
type Page = "home" | "inbox" | "devices" | "models" | "settings" | "advanced";
const pages: Page[] = ["home", "inbox", "devices", "models", "settings"];
function stored(key: string, fallback: string) {
  try {
    return localStorage.getItem(key) || fallback;
  } catch {
    return fallback;
  }
}
function pageFromHash(): Page {
  const path = location.hash.replace(/^#\/?/, "");
  return path === "app/advanced"
    ? "advanced"
    : path.startsWith("app/") && pages.includes(path.slice(4) as Page)
      ? (path.slice(4) as Page)
      : path && !path.startsWith("app/")
        ? "advanced"
        : "home";
}
export function PersonalApp({
  api,
  language = "en",
  advanced,
  openExternal,
  onNavigate,
  chatUrl = "/proxy/3080/",
}: {
  api: Api;
  language?: string;
  advanced?: ReactNode;
  openExternal?: (url: string) => Promise<unknown>;
  onNavigate?: (path: string) => void;
  chatUrl?: string;
}) {
  const [locale, setLocale] = useState(() =>
      stored("cranibox.language", language),
    ),
    [theme, setTheme] = useState(() => stored("cranibox.theme", "light"));
  const t = useCallback(
    (zh: string, en: string) => (locale === "zh-cn" ? zh : en),
    [locale],
  );
  const [page, setPage] = useState<Page>(pageFromHash),
    [navOpen, setNavOpen] = useState(false),
    [taskOpen, setTaskOpen] = useState(false),
    [statusOpen, setStatusOpen] = useState(false);
  const [status, setStatus] = useState<Pill | null>(null),
    [devices, setDevices] = useState<Device[]>([]),
    [approvals, setApprovals] = useState<Approval[]>([]),
    [agents, setAgents] = useState<Agent[]>([]),
    [models, setModels] = useState<Model[]>([]),
    [accounts, setAccounts] = useState<Account[]>([]),
    [presets, setPresets] = useState<Preset[]>([]);
  const [loading, setLoading] = useState(true),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [notice, setNotice] = useState("");
  const [sheet, setSheet] = useState<"pair" | "subscription" | null>(null),
    [purpose, setPurpose] = useState("owner_device"),
    [invite, setInvite] = useState<Invite | null>(null),
    [pairLink, setPairLink] = useState(""),
    [accepted, setAccepted] = useState<Accepted | null>(null),
    [pairError, setPairError] = useState("");
  const [onboarding, setOnboarding] = useState(
      () => stored("cranibox.onboarded", "") !== "1",
    ),
    [onboardStep, setOnboardStep] = useState(0);
  const [tab, setTab] = useState("signin"),
    [provider, setProvider] = useState("codex"),
    [region, setRegion] = useState(""),
    [key, setKey] = useState(""),
    [label, setLabel] = useState(""),
    [endpoint, setEndpoint] = useState(""),
    [customModel, setCustomModel] = useState(""),
    [signIn, setSignIn] = useState<SignIn | null>(null),
    [callback, setCallback] = useState(""),
    [subscriptionError, setSubscriptionError] = useState("");
  const [selectedDevice, setSelectedDevice] = useState("");
  const [tasks, setTasks] = useState<TaskSummary[]>([]),
    [tasksLoading, setTasksLoading] = useState(false),
    [tasksError, setTasksError] = useState("");
  useEffect(() => {
    if (!taskOpen) return;
    let active = true;
    let pending = false;
    const load = async () => {
      if (pending) return;
      pending = true;
      try {
        const items = await api.call<TaskSummary[]>("GET", "/tasks");
        if (active) { setTasks(items); setTasksError(""); }
      } catch (error) {
        if (active) setTasksError(error instanceof Error ? error.message : String(error));
      } finally {
        pending = false;
        if (active) setTasksLoading(false);
      }
    };
    setTasksLoading(true);
    void load();
    const timer = setInterval(() => void load(), 5000);
    return () => { active = false; clearInterval(timer); };
  }, [taskOpen, api]);
  const labels: Record<Page, string> = {
    home: t("首页", "Home"),
    inbox: t("收件箱", "Inbox"),
    devices: t("设备", "Devices"),
    models: t("模型", "Models"),
    settings: t("设置", "Settings"),
    advanced: t("高级", "Advanced"),
  };
  const icons = {
    home: House,
    inbox: Inbox,
    devices: Laptop,
    models: Sparkles,
    settings: Settings,
  };
  const refreshing = useRef<Promise<void> | null>(null);
  const refresh = useCallback(() => {
    if (refreshing.current) return refreshing.current;
    const pending = (async () => {
      const results = await Promise.allSettled([
        api.call<Pill>("GET", "/status").then(setStatus),
        api.call<Device[]>("GET", "/devices").then(setDevices),
        api.call<Approval[]>("GET", "/approvals").then(setApprovals),
        api.call<Agent[]>("GET", "/agents").then(setAgents),
        api.call<Model[]>("GET", "/models/models").then(setModels),
        api.call<Account[]>("GET", "/models/accounts").then(setAccounts),
        api.call<Preset[]>("GET", "/models/presets").then(setPresets),
      ]);
      const failed = results.find((r) => r.status === "rejected");
      setError(
        failed?.status === "rejected"
          ? String(
              failed.reason instanceof Error
                ? failed.reason.message
                : failed.reason,
            )
          : "",
      );
      setLoading(false);
    })().finally(() => {
      refreshing.current = null;
    });
    refreshing.current = pending;
    return pending;
  }, [api]);
  useEffect(() => {
    void refresh();
    const timer = setInterval(() => {
      if (!document.hidden) void refresh();
    }, 10000);
    return () => clearInterval(timer);
  }, [refresh]);
  useEffect(
    () =>
      api.subscribe?.(() => {
        if (!document.hidden) void refresh();
      }),
    [api, refresh],
  );
  useEffect(() => {
    const change = () => {
      setPage(pageFromHash());
      setNavOpen(false);
    };
    window.addEventListener("hashchange", change);
    return () => window.removeEventListener("hashchange", change);
  }, []);
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    try {
      localStorage.setItem("cranibox.theme", theme);
      localStorage.setItem("cranibox.language", locale);
    } catch {}
  }, [theme, locale]);
  useEffect(() => {
    document.title = labels[page] + " · Cranibox";
  }, [page, locale]);
  function navigate(target: Page, path = target === "advanced" ? "/overview" : "/app/" + target) {
    setPage(target);
    setNavOpen(false);
    if (onNavigate) onNavigate(path);
    else location.hash = path;
  }
  async function run(action: () => Promise<unknown>) {
    if (busy) return;
    setBusy(true);
    setError("");
    try {
      await action();
      await refresh();
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  }
  function finishOnboarding() {
    try {
      localStorage.setItem("cranibox.onboarded", "1");
    } catch {}
    setOnboarding(false);
    navigate("home");
  }
  function openPair() {
    setInvite(null);
    setAccepted(null);
    setPairError("");
    setSheet("pair");
  }
  async function createInvite() {
    setBusy(true);
    setPairError("");
    try {
      setInvite(await api.call<Invite>("POST", "/pair/invite", { purpose }));
    } catch (error) {
      setPairError(String(error instanceof Error ? error.message : error));
    } finally {
      setBusy(false);
    }
  }
  async function acceptPair() {
    setBusy(true);
    setPairError("");
    try {
      setAccepted(
        await api.call<Accepted>("POST", "/pair/accept", {
          link: pairLink.trim(),
        }),
      );
    } catch (error) {
      setPairError(String(error instanceof Error ? error.message : error));
    } finally {
      setBusy(false);
    }
  }
  async function confirmPair(matches: boolean) {
    if (!accepted) return;
    setBusy(true);
    setPairError("");
    try {
      await api.call(
        "POST",
        "/pair/" + encodeURIComponent(accepted.flow_id) + "/confirm",
        { match: matches },
      );
      if (matches) {
        setNotice(t("设备已配对", "Device paired"));
        setSheet(null);
        setOnboardStep(2);
        await refresh();
      } else {
        setAccepted(null);
        setPairError(
          t(
            "配对已取消。请重新生成邀请。",
            "Pairing cancelled. Create a new invitation.",
          ),
        );
      }
    } catch (error) {
      setPairError(String(error instanceof Error ? error.message : error));
    } finally {
      setBusy(false);
    }
  }
  function addSubscription() {
    setRegion("");
    setKey("");
    setSignIn(null);
    setCallback("");
    setSubscriptionError("");
    setSheet("subscription");
  }
  async function external(url: string) {
    if (!/^https?:\/\//i.test(url))
      throw Error(t("无效的登录地址", "Invalid sign-in URL"));
    if (openExternal) await openExternal(url);
    else window.open(url, "_blank", "noopener,noreferrer");
  }
  async function submitSubscription(event: React.FormEvent) {
    event.preventDefault();
    setBusy(true);
    setSubscriptionError("");
    try {
      if (tab === "signin") {
        const flow = await api.call<SignIn>("POST", "/models/signin/start", {
          provider,
          region: region || null,
        });
        setSignIn(flow);
        await external(flow.url);
      } else {
        const body =
          tab === "custom"
            ? {
                kind: "custom",
                label,
                key,
                endpoints: { chat: endpoint },
                models: customModel
                  .split(",")
                  .map((s) => s.trim())
                  .filter(Boolean),
              }
            : {
                kind: "api_key",
                preset: provider,
                key,
                label: label || null,
                region: region || null,
              };
        await api.call("POST", "/models/accounts", body);
        setKey("");
        setSheet(null);
        await refresh();
      }
    } catch (error) {
      setSubscriptionError(
        String(error instanceof Error ? error.message : error),
      );
    } finally {
      setBusy(false);
    }
  }
  const currentDevice = devices.find((d) => d.badges.includes("this"));
  const available = models.filter((m) => m.availability.kind !== "unavailable");
  const dsh = agents.find((a) => a.id === "deepseek-harness");
  const agentPicker = (
    <div className="pa-agent-list">
      {agents.map((agent) => (
        <div className="pa-agent-row" key={agent.id}>
          <span className="pa-card-icon">
            <Bot size={20} aria-hidden="true" />
          </span>
          <div className="pa-grow">
            <strong>{agentNames[agent.id] || agent.id}</strong>
            <div className="pa-row pa-wrap">
              {agent.recommended && (
                <Tag tone="good">
                  {t("推荐 · 默认", "Recommended · Default")}
                </Tag>
              )}
              <small>
                {agent.installed
                  ? t("已安装", "Installed")
                  : t("未安装", "Not installed")}
              </small>
            </div>
          </div>
          <label className="pa-model-select">
            <span className="pa-sr">
              {(agentNames[agent.id] || agent.id) + t("使用的模型", " model")}
            </span>
            <select
              disabled={busy || (!available.length && !agent.managed)}
              value={agent.managed ? agent.model || "auto" : "unmanaged"}
              onChange={(e) =>
                void run(() =>
                  api.call("PUT", "/agents/" + agent.id, {
                    managed: e.target.value !== "unmanaged",
                    model: ["auto", "unmanaged"].includes(e.target.value)
                      ? null
                      : e.target.value,
                  }),
                )
              }
            >
              <option value="auto">{t("自动", "Automatic")}</option>
              <option value="unmanaged">
                {t("自行管理", "Manage myself")}
              </option>
              {available.map((model) => (
                <option key={model.id} value={model.id}>
                  {model.display || model.id}
                </option>
              ))}
            </select>
          </label>
        </div>
      ))}
    </div>
  );
  const pairContent = (
    <>
      {!accepted ? (
        <>
          <div className="pa-segment" aria-label={t("设备类型", "Device type")}>
            {[
              ["owner_device", t("我的设备", "My device")],
              ["guest", t("访客设备", "Guest device")],
            ].map(([value, label]) => (
              <button
                key={value}
                aria-pressed={purpose === value}
                onClick={() => {
                  setPurpose(value);
                  setInvite(null);
                }}
              >
                {label}
              </button>
            ))}
          </div>
          <p className="pa-muted">
            {purpose === "guest"
              ? t(
                  "访客只能使用你允许的功能，无法访问模型订阅。",
                  "Guests can use only the capabilities you allow. Model subscriptions stay private.",
                )
              : t(
                  "你的设备共享设备名单，并接收加密的模型订阅。",
                  "Your devices share a device list and receive encrypted model subscriptions.",
                )}
          </p>
          {purpose === "guest" && (
            <p className="pa-muted">
              {t(
                "配对完成后，在访客设备卡片上选择允许的功能。",
                "After pairing, choose allowed capabilities on the guest device card.",
              )}
            </p>
          )}
          {invite ? (
            <QrPanel invite={invite} t={t} />
          ) : (
            <button
              className="pa-primary pa-full"
              disabled={busy}
              onClick={() => void createInvite()}
            >
              {t("生成二维码和链接", "Create QR code and link")}
            </button>
          )}
          <div className="pa-divider">
            {t("或打开收到的邀请", "Or open an invitation")}
          </div>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void acceptPair();
            }}
          >
            <label>
              {t("配对链接", "Pairing link")}
              <input
                autoFocus
                placeholder="cranibox://pair/…"
                value={pairLink}
                onChange={(e) => setPairLink(e.target.value)}
                required
              />
            </label>
            <button className="pa-full" disabled={busy || !pairLink.trim()}>
              {t("继续核对", "Continue to verification")}
            </button>
          </form>
        </>
      ) : (
        <div className="pa-sas">
          <ShieldCheck size={40} aria-hidden="true" />
          <h3>
            {t(
              "两台设备上的数字一致吗？",
              "Do the codes match on both devices?",
            )}
          </h3>
          <p>
            {t(
              "请在另一台设备上查看，确认每一位都相同。",
              "Check the other device. Every digit must match.",
            )}
          </p>
          <code>{accepted.sas}</code>
          <div className="pa-actions">
            <button disabled={busy} onClick={() => void confirmPair(false)}>
              {t("不一致，取消", "Different, cancel")}
            </button>
            <button
              className="pa-primary"
              disabled={busy}
              onClick={() => void confirmPair(true)}
            >
              {t("两边一致", "Codes match")}
            </button>
          </div>
        </div>
      )}
      {pairError && (
        <p className="pa-error" role="alert">
          {pairError}
        </p>
      )}
    </>
  );
  return (
    <div
      className={page === "advanced" ? "pa-legacy" : "personal"}
      data-theme={theme}
    >
      {page !== "advanced" && (
        <a
          className="pa-skip"
          href="#pa-main"
          onClick={(e) => {
            e.preventDefault();
            document.getElementById("pa-main")?.focus();
          }}
        >
          {t("跳至主要内容", "Skip to main content")}
        </a>
      )}
      {page === "advanced" ? (
        <div className="pa-advanced">
          <header className="pa-advanced-bar">
            <button onClick={() => navigate("settings")}>
              ← {t("返回设置", "Back to Settings")}
            </button>
            <p>
              {t(
                "高级功能 · 面向熟悉网络和命令行的用户",
                "Advanced · For users familiar with networks and the command line",
              )}
            </p>
          </header>
          {advanced || (
            <Card>
              {t(
                "此环境未提供高级工具。",
                "Advanced tools are not available in this environment.",
              )}
            </Card>
          )}
        </div>
      ) : (
        <>
          <aside className={"pa-sidebar " + (navOpen ? "is-open" : "")}>
            <a className="pa-brand" href="#/app/home">
              <span className="pa-brand-icon">
                <Sparkles size={22} aria-hidden="true" />
              </span>
              <span>
                Cranibox
                <small>{t("你的个人 Agent", "Your personal agent")}</small>
              </span>
            </a>
            <nav aria-label={t("主导航", "Main navigation")}>
              {pages.map((id) => {
                const Icon = icons[id as keyof typeof icons];
                return (
                  <button
                    key={id}
                    aria-current={page === id ? "page" : undefined}
                    onClick={() => navigate(id)}
                  >
                    <Icon size={20} aria-hidden="true" />
                    <span>{labels[id]}</span>
                    {id === "inbox" && approvals.length > 0 && (
                      <b>{approvals.length}</b>
                    )}
                  </button>
                );
              })}
            </nav>
            <div className="pa-sidebar-bottom">
              <span className="pa-card-icon">
                <ShieldCheck size={18} aria-hidden="true" />
              </span>
              <div>
                <strong>{t("由你掌控", "Yours to control")}</strong>
                <small>
                  {t("设备间端到端加密", "Encrypted between devices")}
                </small>
              </div>
            </div>
          </aside>
          {navOpen && (
            <button
              className="pa-nav-scrim"
              aria-label={t("关闭导航", "Close navigation")}
              onClick={() => setNavOpen(false)}
            />
          )}
          <div className="pa-shell">
            <header className="pa-topbar">
              <div className="pa-row">
                <button
                  className="pa-icon pa-mobile"
                  aria-label={t("打开导航", "Open navigation")}
                  aria-expanded={navOpen}
                  onClick={() => setNavOpen(!navOpen)}
                >
                  <Menu aria-hidden="true" />
                </button>
                <label className="pa-device-picker">
                  <Monitor size={16} aria-hidden="true" />
                  <span className="pa-sr">
                    {t("当前设备", "Current device")}
                  </span>
                  <select
                    value={selectedDevice || currentDevice?.node_id || ""}
                    onChange={(e) => setSelectedDevice(e.target.value)}
                  >
                    {devices
                      .filter((d) => !d.guest)
                      .map((d) => (
                        <option
                          key={d.node_id}
                          value={d.node_id}
                          disabled={!d.badges.includes("this")}
                        >
                          {d.name}
                          {d.badges.includes("this")
                            ? t(" · 本机", " · This device")
                            : ""}
                        </option>
                      ))}
                  </select>
                </label>
              </div>
              <div className="pa-row">
                <StatusPill
                  status={status}
                  t={t}
                  onClick={() => setStatusOpen(true)}
                />
                <button
                  className="pa-icon pa-bell"
                  aria-label={
                    t("待审批", "Pending approvals") + " " + approvals.length
                  }
                  onClick={() => navigate("inbox")}
                >
                  <Bell size={19} aria-hidden="true" />
                  {approvals.length > 0 && <b>{approvals.length}</b>}
                </button>
              </div>
            </header>
            <main
              id="pa-main"
              className={"pa-main pa-page-" + page}
              tabIndex={-1}
            >
              {error && (
                <div role="alert" className="pa-error pa-row">
                  <span className="pa-grow">{error}</span>
                  <button onClick={() => void refresh()}>
                    {t("重试", "Retry")}
                  </button>
                </div>
              )}
              {notice && (
                <div className="pa-notice" role="status">
                  {notice}
                  <button
                    className="pa-icon"
                    aria-label={t("关闭通知", "Dismiss notification")}
                    onClick={() => setNotice("")}
                  >
                    <X size={16} aria-hidden="true" />
                  </button>
                </div>
              )}
              {loading ? (
                <div className="pa-empty" role="status">
                  <LoaderCircle className="pa-spin" aria-hidden="true" />
                  <p>{t("正在准备你的空间…", "Getting your space ready…")}</p>
                </div>
              ) : (
                <>
                  {page === "home" && (
                    <>
                      <div className="pa-page-heading">
                        <div>
                          <p className="pa-eyebrow">
                            {t(
                              "一点想法，无限可能",
                              "A LITTLE IDEA. A WORLD OF POSSIBILITY.",
                            )}
                          </p>
                          <h1>
                            {t(
                              "今天，一起做些什么？",
                              "What shall we do today?",
                            )}
                          </h1>
                          <p>
                            {t(
                              "交给 Agent。需要你决定时，我会来问你。",
                              "Hand it to your agent. You decide when it matters.",
                            )}
                          </p>
                        </div>
                        <button onClick={() => setTaskOpen(true)}>
                          <ListTodo size={18} aria-hidden="true" />
                          {t("任务", "Tasks")}
                        </button>
                      </div>
                      {dsh?.installed ? (
                        <div className="pa-conversation">
                          <iframe
                            title={t(
                              "与 DeepSeek Harness 对话",
                              "Chat with DeepSeek Harness",
                            )}
                            src={chatUrl}
                            allow="clipboard-write"
                            sandbox="allow-scripts allow-same-origin allow-forms allow-downloads allow-popups"
                          />
                        </div>
                      ) : (
                        <Card className="pa-welcome-card">
                          <span className="pa-orb">
                            <Bot size={34} aria-hidden="true" />
                          </span>
                          <h2>
                            {t(
                              "你的 Agent，准备出发",
                              "Your agent, ready to begin",
                            )}
                          </h2>
                          <p>
                            {t(
                              "DeepSeek Harness 是默认 Agent。先安装，再添加一个模型，就能开始对话。",
                              "DeepSeek Harness is your default agent. Install it and add a model to start a conversation.",
                            )}
                          </p>
                          <div className="pa-actions">
                            <button
                              className="pa-primary"
                              onClick={() => navigate("advanced")}
                            >
                              {t(
                                "安装 DeepSeek Harness",
                                "Install DeepSeek Harness",
                              )}
                            </button>
                            <button onClick={addSubscription}>
                              <Plus size={16} aria-hidden="true" />
                              {t("添加模型", "Add a model")}
                            </button>
                          </div>
                          <small>
                            DSH 0.1.1-rc.2 · {t("版本已锁定", "Pinned version")}
                          </small>
                        </Card>
                      )}
                      {approvals.length > 0 && (
                        <button
                          className="pa-pending"
                          onClick={() => navigate("inbox")}
                        >
                          <Bell size={18} aria-hidden="true" />
                          {t(
                            "有 " + approvals.length + " 件事需要你确认",
                            approvals.length + " things need your attention",
                          )}
                          <ChevronRight size={18} aria-hidden="true" />
                        </button>
                      )}
                    </>
                  )}
                  {page === "inbox" && (
                    <>
                      <div className="pa-page-heading">
                        <div>
                          <h1>
                            {t("留给你决定的事", "A moment for your judgment")}
                          </h1>
                          <p>
                            {t(
                              "每一个重要决定，都先经过你。",
                              "Every important decision comes to you first.",
                            )}
                          </p>
                        </div>
                        <Tag>
                          {approvals.length} {t("待审批", "pending")}
                        </Tag>
                      </div>
                      {approvals.length ? (
                        approvals.map((card) => (
                          <ApprovalCard
                            key={card.id}
                            card={card}
                            t={t}
                            busy={busy}
                            onDecide={(decision) =>
                              void run(() =>
                                api.call(
                                  "POST",
                                  "/approvals/" + encodeURIComponent(card.id),
                                  { decision },
                                ),
                              )
                            }
                          />
                        ))
                      ) : (
                        <div className="pa-empty">
                          <CircleCheck size={38} aria-hidden="true" />
                          <h2>{t("都处理好了", "You’re all caught up")}</h2>
                          <p>
                            {t(
                              "需要你确认时，会出现在这里。",
                              "When your agent needs a decision, you’ll find it here.",
                            )}
                          </p>
                        </div>
                      )}
                    </>
                  )}
                  {page === "devices" && (
                    <>
                      <div className="pa-page-heading">
                        <div>
                          <h1>
                            {t(
                              "你的设备，一起工作",
                              "Your devices, working together",
                            )}
                          </h1>
                          <p>
                            {t(
                              "无论身在何处，熟悉的 Agent 都在。",
                              "Your familiar agent, wherever you are.",
                            )}
                          </p>
                        </div>
                        <button className="pa-primary" onClick={openPair}>
                          <Plus size={17} aria-hidden="true" />
                          {t("添加设备", "Add device")}
                        </button>
                      </div>
                      {[false, true].map((guest) => (
                        <section key={String(guest)} className="pa-section">
                          <h2>
                            {guest
                              ? t("访客设备", "Guest devices")
                              : t("我的设备", "My devices")}
                          </h2>
                          <div className="pa-device-grid">
                            {devices
                              .filter((d) => d.guest === guest)
                              .map((device) => (
                                <Card key={device.node_id}>
                                  <div className="pa-row">
                                    <span className="pa-card-icon">
                                      <Laptop size={23} aria-hidden="true" />
                                    </span>
                                    <div className="pa-grow">
                                      <h3>{device.name}</h3>
                                      <small>
                                        {device.online
                                          ? t("在线", "Online")
                                          : t("离线", "Offline")}
                                        {device.online &&
                                          " · " +
                                            (device.path === "relay"
                                              ? t(
                                                  "经由其他设备",
                                                  "Via another device",
                                                )
                                              : device.path === "direct"
                                                ? t("直连", "Direct")
                                                : t("本机", "Local"))}
                                        {device.rtt_ms !== null
                                          ? " · " + device.rtt_ms + " ms"
                                          : ""}
                                      </small>
                                    </div>
                                    <span
                                      className={
                                        "pa-dot " +
                                        (device.online ? "online" : "")
                                      }
                                    />
                                  </div>
                                  <div className="pa-row pa-wrap">
                                    {device.badges.map((b) => (
                                      <Tag
                                        key={b}
                                        tone={b === "hub" ? "good" : ""}
                                      >
                                        {b === "this"
                                          ? t("本机", "This device")
                                          : b === "hub"
                                            ? t("常驻节点", "Hub")
                                            : "Claude"}
                                      </Tag>
                                    ))}
                                  </div>
                                  {guest && (
                                    <div className="pa-cap-list">
                                      {Object.entries(device.caps)
                                        .filter(
                                          ([k]) =>
                                            !["llm", "vault"].includes(k),
                                        )
                                        .map(([cap, on]) => (
                                          <label className="pa-check" key={cap}>
                                            <input
                                              type="checkbox"
                                              checked={on}
                                              disabled={busy}
                                              onChange={(e) =>
                                                void run(() =>
                                                  api.call(
                                                    "PUT",
                                                    "/devices/" +
                                                      device.node_id,
                                                    {
                                                      caps: {
                                                        ...device.caps,
                                                        [cap]: e.target.checked,
                                                      },
                                                    },
                                                  ),
                                                )
                                              }
                                            />
                                            {cap}
                                          </label>
                                        ))}
                                      <button
                                        className="pa-danger"
                                        disabled={busy}
                                        onClick={() =>
                                          void run(() =>
                                            api.call(
                                              "PUT",
                                              "/devices/" + device.node_id,
                                              { revoke: true },
                                            ),
                                          )
                                        }
                                      >
                                        {t("撤销访问", "Revoke access")}
                                      </button>
                                    </div>
                                  )}
                                </Card>
                              ))}
                          </div>
                          {!devices.some((d) => d.guest === guest) && (
                            <p className="pa-muted">
                              {guest
                                ? t("还没有访客设备。", "No guest devices yet.")
                                : t(
                                    "添加你的第一台设备。",
                                    "Add your first device.",
                                  )}
                            </p>
                          )}
                        </section>
                      ))}
                    </>
                  )}
                  {page === "models" && (
                    <>
                      <div className="pa-page-heading">
                        <div>
                          <h1>
                            {t(
                              "给想法一点动力",
                              "Give your ideas a little power",
                            )}
                          </h1>
                          <p>
                            {t(
                              "连接你的订阅，再选择哪些设备可以使用。",
                              "Connect your subscriptions. Choose which devices can use them.",
                            )}
                          </p>
                        </div>
                        <button
                          className="pa-primary"
                          onClick={addSubscription}
                        >
                          <Plus size={17} aria-hidden="true" />
                          {t("添加订阅", "Add subscription")}
                        </button>
                      </div>
                      <section className="pa-section">
                        <h2>{t("我的订阅", "My subscriptions")}</h2>
                        <div className="pa-device-grid">
                          {accounts.map((account) => (
                            <Card key={account.id}>
                              <div className="pa-row">
                                <span className="pa-card-icon">
                                  <Sparkles size={21} aria-hidden="true" />
                                </span>
                                <div className="pa-grow">
                                  <h3>{account.label || account.provider}</h3>
                                  <small>{account.provider}</small>
                                </div>
                                <Tag
                                  tone={
                                    account.state.kind === "ready"
                                      ? "good"
                                      : "warn"
                                  }
                                >
                                  {account.state.kind === "ready"
                                    ? t("可用", "Ready")
                                    : account.state.kind === "needs_sign_in"
                                      ? t("需重新登录", "Sign in again")
                                      : account.state.kind === "quota_exhausted"
                                        ? t("额度已用完", "Quota used")
                                        : t("需要检查", "Needs attention")}
                                </Tag>
                              </div>
                              {account.kind === "claude_oauth" ? (
                                <>
                                  <p className="pa-muted">
                                    {t(
                                      "凭据只保留在 Claude 节点，不同步到其他设备。",
                                      "Credentials stay on the Claude node and never sync to other devices.",
                                    )}
                                  </p>
                                  <label>
                                    {t("Claude 节点", "Claude node")}
                                    <select
                                      value={
                                        devices.find((d) =>
                                          d.badges.includes("claude"),
                                        )?.node_id || account.home_node
                                      }
                                      disabled={busy}
                                      onChange={(e) =>
                                        void run(() =>
                                          api.call(
                                            "POST",
                                            "/models/claude/set-node",
                                            { node: e.target.value },
                                          ),
                                        )
                                      }
                                    >
                                      {devices
                                        .filter((d) => !d.guest)
                                        .map((d) => (
                                          <option
                                            key={d.node_id}
                                            value={d.node_id}
                                          >
                                            {d.name}
                                          </option>
                                        ))}
                                    </select>
                                  </label>
                                </>
                              ) : (
                                <>
                                  <label className="pa-check">
                                    <input
                                      type="checkbox"
                                      checked={
                                        account.all_nodes ||
                                        account.enabled_on.includes(
                                          currentDevice?.node_id || "",
                                        )
                                      }
                                      disabled={busy || !currentDevice}
                                      onChange={(e) =>
                                        void run(() =>
                                          api.call(
                                            "POST",
                                            "/models/accounts/" +
                                              account.id +
                                              "/enable",
                                            {
                                              node: currentDevice?.node_id,
                                              all_nodes: null,
                                              on: e.target.checked,
                                            },
                                          ),
                                        )
                                      }
                                    />
                                    {t("在本机启用", "Enable on this device")}
                                  </label>
                                  <label className="pa-check">
                                    <input
                                      type="checkbox"
                                      checked={account.all_nodes}
                                      disabled={busy}
                                      onChange={(e) =>
                                        void run(() =>
                                          api.call(
                                            "POST",
                                            "/models/accounts/" +
                                              account.id +
                                              "/enable",
                                            {
                                              node: null,
                                              all_nodes: true,
                                              on: e.target.checked,
                                            },
                                          ),
                                        )
                                      }
                                    />
                                    {t(
                                      "在所有节点启用",
                                      "Enable on all devices",
                                    )}
                                  </label>
                                  <details>
                                    <summary>
                                      {t(
                                        "逐台选择设备",
                                        "Choose devices individually",
                                      )}
                                    </summary>
                                    {devices
                                      .filter((d) => !d.guest)
                                      .map((d) => (
                                        <label
                                          className="pa-check"
                                          key={d.node_id}
                                        >
                                          <input
                                            type="checkbox"
                                            disabled={busy || account.all_nodes}
                                            checked={
                                              account.all_nodes ||
                                              account.enabled_on.includes(
                                                d.node_id,
                                              )
                                            }
                                            onChange={(e) =>
                                              void run(() =>
                                                api.call(
                                                  "POST",
                                                  "/models/accounts/" +
                                                    account.id +
                                                    "/enable",
                                                  {
                                                    node: d.node_id,
                                                    all_nodes: null,
                                                    on: e.target.checked,
                                                  },
                                                ),
                                              )
                                            }
                                          />
                                          {d.name}
                                        </label>
                                      ))}
                                  </details>
                                </>
                              )}
                              {account.state.kind === "needs_sign_in" && (
                                <button
                                  onClick={() => {
                                    setProvider(account.provider);
                                    setTab("signin");
                                    addSubscription();
                                  }}
                                >
                                  {t("重新登录", "Sign in again")}
                                </button>
                              )}
                            </Card>
                          ))}
                        </div>
                        {accounts.length === 0 && (
                          <Card>
                            <p>
                              {t(
                                "还没有订阅。从你已经在用的账号开始。",
                                "No subscriptions yet. Start with an account you already use.",
                              )}
                            </p>
                            <button onClick={addSubscription}>
                              {t(
                                "添加第一个订阅",
                                "Add your first subscription",
                              )}
                            </button>
                          </Card>
                        )}
                      </section>
                      <section className="pa-section">
                        <h2>
                          {t("各 Agent 用哪个模型", "Models for your agents")}
                        </h2>
                        <Card>{agentPicker}</Card>
                      </section>
                    </>
                  )}
                  {page === "settings" && (
                    <>
                      <div className="pa-page-heading">
                        <div>
                          <h1>{t("按你的习惯来", "Make yourself at home")}</h1>
                          <p>
                            {t(
                              "简单一点，专注你想做的事。",
                              "Keep things simple. Focus on what you want to do.",
                            )}
                          </p>
                        </div>
                      </div>
                      <Card>
                        <div className="pa-setting">
                          <div>
                            <h2>{t("外观", "Appearance")}</h2>
                            <p>
                              {t(
                                "选择舒服的明暗。",
                                "Find your comfortable light.",
                              )}
                            </p>
                          </div>
                          <button
                            onClick={() =>
                              setTheme(theme === "light" ? "dark" : "light")
                            }
                          >
                            {theme === "light" ? (
                              <Moon size={18} aria-hidden="true" />
                            ) : (
                              <Sun size={18} aria-hidden="true" />
                            )}
                            {theme === "light"
                              ? t("深色", "Dark")
                              : t("浅色", "Light")}
                          </button>
                        </div>
                        <div className="pa-setting">
                          <label htmlFor="pa-language">
                            {t("语言", "Language")}
                          </label>
                          <select
                            id="pa-language"
                            value={locale}
                            onChange={(e) => setLocale(e.target.value)}
                          >
                            <option value="zh-cn">简体中文</option>
                            <option value="en">English</option>
                          </select>
                        </div>
                      </Card>
                      <section className="pa-section">
                        <h2>Agent</h2>
                        <Card>{agentPicker}</Card>
                      </section>
                      <Card>
                        <div className="pa-setting">
                          <div>
                            <h2>{t("高级", "Advanced")}</h2>
                            <p>
                              {t(
                                "面向熟悉网络和命令行的用户。",
                                "For users familiar with networks and the command line.",
                              )}
                            </p>
                          </div>
                          <button onClick={() => navigate("advanced")}>
                            {t("打开高级功能", "Open advanced tools")}
                            <ChevronRight size={17} aria-hidden="true" />
                          </button>
                        </div>
                      </Card>
                    </>
                  )}
                </>
              )}
            </main>
            <footer className="pa-footer">
              <ShieldCheck size={13} aria-hidden="true" />
              {t("你的设备，你的选择。", "Your devices. Your decisions.")}
            </footer>
          </div>
        </>
      )}
      {statusOpen && (
        <Sheet
          title={t("当前状态", "Current status")}
          t={t}
          onClose={() => setStatusOpen(false)}
        >
          <p>
            {status?.state === "local"
              ? t(
                  "常驻节点暂时离线，已在本机运行。恢复后会自动切回。",
                  "Your hub is offline. Running locally until it is stable again.",
                )
              : t("模型可用数量：", "Available models: ") +
                (status?.models_available || 0)}
          </p>
          <p>
            {t("待审批：", "Pending approvals: ")}
            {approvals.length}
          </p>
          <button
            className="pa-full"
            onClick={() => {
              setStatusOpen(false);
              navigate("models");
            }}
          >
            {t("管理模型", "Manage models")}
          </button>
        </Sheet>
      )}
      {taskOpen && (
        <Sheet
          title={t("任务", "Tasks")}
          t={t}
          onClose={() => setTaskOpen(false)}
        >
          {tasksLoading && <p role="status">{t("正在读取任务…", "Loading tasks…")}</p>}
          {tasksError && <p role="alert">{tasksError}</p>}
          {tasks.map((task) => (
            <Card key={task.id}>
              <div className="pa-between">
                <h3>{task.title}</h3>
                <Tag>{({
                  queued: t("等待中", "Queued"), available: t("待开始", "Ready"),
                  running: t("进行中", "Running"), cancel_requested: t("正在停止", "Stopping"),
                  succeeded: t("已完成", "Completed"), failed: t("失败", "Failed"),
                  cancelled: t("已取消", "Cancelled"),
                } as Record<string, string>)[task.state] || t("状态待更新", "Awaiting update")}</Tag>
              </div>
              {task.progress && <p>{task.progress}</p>}
              {task.result && <p>{task.result}</p>}
              {task.questions?.filter((q) => q.answer === null).map((q) => (
                <p key={q.id}><strong>{t("需要你回答：", "Your answer is needed: ")}</strong>{q.text}</p>
              ))}
            </Card>
          ))}
          {!tasksLoading && !tasksError && tasks.length === 0 && <div className="pa-empty">
            <ListTodo size={32} aria-hidden="true" />
            <p>
              {t(
                "在对话里告诉 Agent 你想做什么。",
                "Tell your agent what you’d like to do in the conversation.",
              )}
            </p>
          </div>}
            <button className="pa-full"
              onClick={() => {
                setTaskOpen(false);
                navigate("advanced", "/tasks");
              }}
            >
              {t("查看所有任务", "View all tasks")}
            </button>
        </Sheet>
      )}
      {sheet === "pair" && (
        <Sheet
          title={t("添加设备", "Add device")}
          t={t}
          onClose={() => setSheet(null)}
        >
          {pairContent}
        </Sheet>
      )}
      {sheet === "subscription" && (
        <Sheet
          title={t("添加订阅", "Add subscription")}
          t={t}
          onClose={() => {
            setSheet(null);
            setKey("");
          }}
        >
          <div className="pa-segment pa-wrap">
            {[
              ["signin", t("账号登录", "Sign in")],
              ["plan", t("国内套餐", "Regional plans")],
              ["key", t("粘贴 Key", "API key")],
              ["custom", t("自定义", "Custom")],
            ].map(([id, title]) => (
              <button
                key={id}
                aria-pressed={tab === id}
                onClick={() => {
                  setTab(id);
                  setSignIn(null);
                  setRegion("");
                  setProvider(
                    id === "plan"
                      ? presets.find((p) => p.region.includes("cn"))?.id ||
                          "deepseek"
                      : id === "key"
                        ? "deepseek"
                        : "codex",
                  );
                }}
              >
                {title}
              </button>
            ))}
          </div>
          {!signIn ? (
            <form onSubmit={(e) => void submitSubscription(e)}>
              {tab !== "custom" && (
                <label>
                  {t("服务商", "Provider")}
                  <select
                    value={provider}
                    onChange={(e) => {
                      setProvider(e.target.value);
                      setRegion("");
                    }}
                  >
                    {(tab === "signin"
                      ? [
                          ["codex", "ChatGPT"],
                          ["copilot", "GitHub Copilot"],
                          ["gemini", "Gemini"],
                          ["kimi-code", "Kimi Code"],
                          ["claude", "Claude"],
                        ]
                      : presets
                          .filter(
                            (p) => tab !== "plan" || p.region.includes("cn"),
                          )
                          .map((p) => [p.id, p.label])
                    ).map(([id, label]) => (
                      <option key={id} value={id}>
                        {label}
                      </option>
                    ))}
                  </select>
                </label>
              )}
              {tab !== "signin" &&
                tab !== "custom" &&
                (presets.find((p) => p.id === provider)?.region.length || 0) >
                  1 && (
                  <label>
                    {t("服务区域", "Service region")}
                    <select
                      value={region}
                      onChange={(e) => setRegion(e.target.value)}
                    >
                      <option value="">
                        {t("默认区域", "Default region")}
                      </option>
                      {presets
                        .find((p) => p.id === provider)
                        ?.region.map((r) => (
                          <option key={r} value={r}>
                            {r === "cn"
                              ? t("中国大陆", "China")
                              : r === "global"
                                ? t("国际", "Global")
                                : r}
                          </option>
                        ))}
                    </select>
                  </label>
                )}
              {provider === "claude" && tab === "signin" && (
                <p className="pa-muted">
                  {t(
                    "此设备将成为 Claude 节点。Claude 凭据不会同步。",
                    "This device will be the Claude node. Claude credentials never sync.",
                  )}
                </p>
              )}
              {tab !== "signin" && (
                <>
                  <label>
                    {t("名称（可选）", "Name (optional)")}
                    <input
                      value={label}
                      onChange={(e) => setLabel(e.target.value)}
                      autoComplete="off"
                    />
                  </label>
                  <label>
                    {t("API Key", "API key")}
                    <input
                      value={key}
                      type="password"
                      autoComplete="off"
                      onChange={(e) => setKey(e.target.value)}
                      required
                    />
                  </label>
                </>
              )}
              {tab === "custom" && (
                <>
                  <label>
                    {t("服务地址", "API base URL")}
                    <input
                      type="url"
                      required
                      value={endpoint}
                      onChange={(e) => setEndpoint(e.target.value)}
                      placeholder="https://api.example.com"
                    />
                  </label>
                  <label>
                    {t("模型 ID（逗号分隔）", "Model IDs (comma separated)")}
                    <input
                      required
                      value={customModel}
                      onChange={(e) => setCustomModel(e.target.value)}
                    />
                  </label>
                </>
              )}
              <button className="pa-primary pa-full" disabled={busy}>
                {busy
                  ? t("请稍候…", "Please wait…")
                  : tab === "signin"
                    ? t("在浏览器中继续", "Continue in browser")
                    : t("保存订阅", "Save subscription")}
              </button>
            </form>
          ) : (
            <div>
              <h3>
                {t("完成浏览器中的登录", "Finish signing in in your browser")}
              </h3>
              {signIn.code && (
                <div className="pa-sas">
                  <code>{signIn.code}</code>
                </div>
              )}
              <button onClick={() => void external(signIn.url)}>
                {t("再次打开登录页", "Open sign-in page again")}
              </button>
              <form
                onSubmit={(e) => {
                  e.preventDefault();
                  void run(async () => {
                    await api.call(
                      "POST",
                      "/models/signin/" +
                        encodeURIComponent(signIn.flow_id) +
                        "/complete",
                      signIn.kind === "device_code"
                        ? { callback_url: null, code: null }
                        : /^https?:/.test(callback)
                          ? { callback_url: callback, code: null }
                          : { callback_url: null, code: callback },
                    );
                    setSheet(null);
                  });
                }}
              >
                {signIn.kind !== "device_code" && (
                  <label>
                    {t(
                      "回调链接或授权码",
                      "Callback URL or authorization code",
                    )}
                    <input
                      required
                      value={callback}
                      onChange={(e) => setCallback(e.target.value)}
                    />
                  </label>
                )}
                <button className="pa-primary pa-full" disabled={busy}>
                  {t("完成登录", "Complete sign-in")}
                </button>
              </form>
            </div>
          )}
          {subscriptionError && (
            <p className="pa-error" role="alert">
              {subscriptionError}
            </p>
          )}
        </Sheet>
      )}
      {onboarding && !loading && page !== "advanced" && (
        <Sheet
          title={
            onboardStep === 0
              ? t("很高兴，在这里遇见你", "A little space for big ideas")
              : onboardStep === 1
                ? t("让你的设备彼此认识", "Introduce your devices")
                : t("最后，给 Agent 一个模型", "Give your agent a model")
          }
          t={t}
          onClose={finishOnboarding}
        >
          <div
            className="pa-onboard-progress"
            aria-label={t("引导进度", "Setup progress")}
          >
            {[0, 1, 2].map((step) => (
              <span key={step} className={step <= onboardStep ? "done" : ""} />
            ))}
          </div>
          {onboardStep === 0 ? (
            <div className="pa-onboarding">
              <span className="pa-orb">
                <Sparkles size={36} aria-hidden="true" />
              </span>
              <p>
                {t(
                  "一个懂你的 Agent，连接你的设备。重要的决定，始终交给你。",
                  "An agent for your ideas, across your devices. Important decisions always stay with you.",
                )}
              </p>
              <button
                className="pa-primary pa-full"
                onClick={() => setOnboardStep(1)}
              >
                {t("开始设置", "Let’s get started")}
              </button>
              <button className="pa-full" onClick={() => setOnboardStep(2)}>
                {t("先在这台设备使用", "Use on this device first")}
              </button>
            </div>
          ) : onboardStep === 1 ? (
            <>
              <p>
                {t(
                  "在常驻节点上打开「添加设备」，粘贴邀请链接或用相机扫描二维码后打开。",
                  "Open Add device on your hub. Paste its invitation link, or scan the QR code with your camera and open it.",
                )}
              </p>
              {pairContent}
              <button className="pa-full" onClick={() => setOnboardStep(2)}>
                {t("暂时跳过", "Skip for now")}
              </button>
            </>
          ) : (
            <>
              <p>
                {t(
                  "你可以连接已有的订阅，之后随时更改。",
                  "Connect a subscription you already have. You can change it any time.",
                )}
              </p>
              <button
                className="pa-primary pa-full"
                onClick={() => {
                  finishOnboarding();
                  addSubscription();
                }}
              >
                {t("添加订阅", "Add subscription")}
              </button>
              <button className="pa-full" onClick={finishOnboarding}>
                {t("稍后添加，进入首页", "Add later, go home")}
              </button>
            </>
          )}
        </Sheet>
      )}
    </div>
  );
}
