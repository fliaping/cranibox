export type Api = {
  call<T = unknown>(method: string, path: string, body?: unknown): Promise<T>;
  subscribe?(refresh: () => void): () => void;
};
export type TaskSummary = {
  id: string;
  title: string;
  direction: "incoming" | "outgoing";
  state: string;
  progress: string | null;
  result: string | null;
  questions: { id: string; text: string; answer: string | null }[] | null;
  updated_at: number;
};
export type Pill = {
  state: "connected" | "local" | "attention" | "no_models" | "offline";
  text_key: string;
  hub: string | null;
  models_available: number;
  approvals_pending: number;
};
export type Device = {
  node_id: string;
  name: string;
  kind: string;
  online: boolean;
  path: string;
  rtt_ms: number | null;
  badges: string[];
  guest: boolean;
  caps: Record<string, boolean>;
};
export type Approval = {
  id: string;
  kind: string;
  from_node: string | null;
  on_node: string;
  agent: string | null;
  title_key: string;
  title_args: Record<string, string>;
  detail: string | null;
  risk: string;
  created_at: number;
  expires_at: number | null;
  actions: string[];
};
export type Agent = {
  id: string;
  installed: boolean;
  managed: boolean;
  model: string | null;
  recommended: boolean;
};
export type Model = {
  id: string;
  display: string;
  availability: { kind: string; node?: string; code?: string };
};
export type Account = {
  id: string;
  provider: string;
  label: string;
  kind: string;
  home_node: string;
  sync: boolean;
  enabled_on: string[];
  all_nodes: boolean;
  state: { kind: string; message?: string };
};
export type Preset = { id: string; label: string; region: string[] };
export type Invite = { link: string; qr_svg: string; expires_at: number };
export type Accepted = { flow_id: string; sas: string };
export type SignIn = {
  kind: string;
  flow_id: string;
  url: string;
  code?: string;
  expires_at?: number;
};
export const agentNames: Record<string, string> = {
  "deepseek-harness": "DeepSeek Harness",
  codex: "Codex",
  "claude-code": "Claude Code",
  pi: "Pi",
  hermes: "Hermes",
  openclaw: "OpenClaw",
};
export function httpApi(csrf: string): Api {
  return {
    async call<T>(method: string, path: string, body?: unknown) {
      const response = await fetch("/api/v1/app" + path, {
        method,
        credentials: "same-origin",
        headers: {
          "content-type": "application/json",
          "x-workspace-csrf": csrf,
        },
        body: body === undefined ? undefined : JSON.stringify(body),
      });
      const value = await response.json();
      if (!response.ok)
        throw Error(value.message || value.code || "Request failed");
      return value as T;
    },
    subscribe(refresh) {
      const events = new EventSource("/api/v1/app/events");
      for (const type of ["status", "approval", "device", "account"])
        events.addEventListener(type, refresh);
      return () => events.close();
    },
  };
}
