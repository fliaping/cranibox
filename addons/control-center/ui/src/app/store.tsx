import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { Account, Agent, Api, Approval, Device, Model, Pill, Preset } from "../personal/api";
import type { Translate } from "./ui";
import type { Preferences } from "../types";
import type { AccessView } from "../access/types";

export function stored(key: string, fallback: string) {
  try { return localStorage.getItem(key) || fallback; } catch { return fallback; }
}
export function store(key: string, value: string) {
  try { localStorage.setItem(key, value); } catch { /* Browser storage is optional. */ }
}
export type Theme = "light" | "dark" | "system";
/** Remember interface choices on the node; older servers without the route just ignore them. */
export async function savePreferences(api: Api, change: Preferences) {
  try {
    const saved = await api.call<Preferences>("PUT", "/preferences", change);
    if (window.workspaceWeb) window.workspaceWeb.preferences = saved;
  } catch { /* Browser storage still holds the choice. */ }
}
export function resolvedTheme(theme: Theme) {
  return theme === "system" ? (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light") : theme;
}
export type AppState = {
  api: Api; t: Translate; locale: string; setLocale: (locale: string) => void;
  theme: Theme; setTheme: (theme: Theme) => void;
  status: Pill | null; devices: Device[]; approvals: Approval[]; agents: Agent[];
  access: AccessView | null; accessError: string;
  models: Model[]; accounts: Account[]; presets: Preset[];
  loading: boolean; error: string; busy: boolean; notice: string;
  setNotice: (value: string) => void; setError: (value: string) => void;
  refresh: () => Promise<void>; run: (action: () => Promise<unknown>) => Promise<void>;
  thisDevice?: Device; deviceName: (node: string) => string;
  openExternal: (url: string) => Promise<void>;
  pairing: boolean; setPairing: (open: boolean) => void;
};
const Context = createContext<AppState | null>(null);
export function useOptionalApp() {
  return useContext(Context);
}
export function useApp() {
  const value = useContext(Context);
  if (!value) throw Error("App state provider missing");
  return value;
}
export function AppProvider({ api, language, openExternal, onLocale, children }: { api: Api; language: string; openExternal?: (url: string) => Promise<unknown>; onLocale?: (locale: string) => void; children: ReactNode }) {
  const [locale, setLocaleState] = useState(language);
  const [theme, setThemeState] = useState<Theme>(() => window.workspaceWeb?.preferences?.theme || stored("cranibox.theme", "system") as Theme);
  const t = useCallback<Translate>((zh, en) => (locale.startsWith("zh") ? zh : en), [locale]);
  const [status, setStatus] = useState<Pill | null>(null), [devices, setDevices] = useState<Device[]>([]),
    [approvals, setApprovals] = useState<Approval[]>([]), [agents, setAgents] = useState<Agent[]>([]),
    [models, setModels] = useState<Model[]>([]), [accounts, setAccounts] = useState<Account[]>([]),
    [presets, setPresets] = useState<Preset[]>([]);
  const [loading, setLoading] = useState(true), [busy, setBusy] = useState(false), [error, setError] = useState(""), [notice, setNotice] = useState("");
  const [pairing, setPairing] = useState(false);
  const [access, setAccess] = useState<AccessView | null>(null), [accessError, setAccessError] = useState("");
  const refreshing = useRef<Promise<void> | null>(null);
  const refresh = useCallback(() => {
    if (refreshing.current) return refreshing.current;
    const pending = (async () => {
      const results = await Promise.allSettled([
        api.call<AccessView>("GET", "/access").then(value => { if (value && Array.isArray(value.peers)) { setAccess(value); setAccessError(""); } }).catch(problem => setAccessError(problem instanceof Error ? problem.message : String(problem))),
        api.call<Pill>("GET", "/status").then(setStatus),
        api.call<Device[]>("GET", "/devices").then(setDevices),
        api.call<Approval[]>("GET", "/approvals").then(setApprovals),
        api.call<Agent[]>("GET", "/agents").then(setAgents),
        api.call<Model[]>("GET", "/models/models").then(setModels),
        api.call<Account[]>("GET", "/models/accounts").then(setAccounts),
        api.call<Preset[]>("GET", "/models/presets").then(setPresets),
      ]);
      const failed = results.find(result => result.status === "rejected");
      setError(failed?.status === "rejected" ? String(failed.reason instanceof Error ? failed.reason.message : failed.reason) : "");
      setLoading(false);
    })().finally(() => { refreshing.current = null; });
    refreshing.current = pending;
    return pending;
  }, [api]);
  useEffect(() => {
    void refresh();
    const timer = setInterval(() => { if (!document.hidden) void refresh(); }, 10000);
    return () => clearInterval(timer);
  }, [refresh]);
  useEffect(() => api.subscribe?.(() => { void refresh(); }), [api, refresh]);
  useEffect(() => {
    const apply = () => { document.documentElement.dataset.theme = resolvedTheme(theme); };
    apply();
    store("cranibox.theme", theme);
    if (theme !== "system") return;
    const query = matchMedia("(prefers-color-scheme: dark)");
    query.addEventListener("change", apply);
    return () => query.removeEventListener("change", apply);
  }, [theme]);
  const run = useCallback(async (action: () => Promise<unknown>) => {
    setBusy(true); setError("");
    try { await action(); await refresh(); }
    catch (problem) { setError(problem instanceof Error ? problem.message : String(problem)); }
    finally { setBusy(false); }
  }, [refresh]);
  const setLocale = useCallback((next: string) => {
    store("cranibox.language", next);
    setLocaleState(next);
    void savePreferences(api, { language: next === "en" ? "en" : "zh-cn" }).finally(() => onLocale?.(next));
  }, [api, onLocale]);
  const setTheme = useCallback((next: Theme) => {
    setThemeState(next);
    void savePreferences(api, { theme: next });
  }, [api]);
  const value = useMemo<AppState>(() => {
    // Hub-era servers may still send a "hub" badge; peers are equal, so it is ignored.
    const peers = devices.map(device => ({ ...device, badges: device.badges.filter(badge => badge !== "hub") }));
    const thisDevice = peers.find(device => device.badges.includes("this"));
    return {
      api, t, locale, setLocale, theme, setTheme, status, devices: peers, approvals, agents, models, accounts, presets, access, accessError,
      loading, error, busy, notice, setNotice, setError, refresh, run, thisDevice, pairing, setPairing,
      deviceName: node => peers.find(device => device.node_id === node)?.name || node,
      openExternal: async url => {
        if (!/^https?:\/\//i.test(url)) throw Error(t("无效的链接", "Invalid link"));
        if (openExternal) await openExternal(url);
        else window.open(url, "_blank", "noopener,noreferrer");
      },
    };
  }, [api, t, locale, setLocale, theme, setTheme, status, devices, approvals, agents, models, accounts, presets, access, accessError, loading, error, busy, notice, refresh, run, openExternal, pairing]);
  return <Context.Provider value={value}>{children}</Context.Provider>;
}
