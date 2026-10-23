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
  models_available: number;
  approvals_pending: number;
};
/** Every device is an equal peer: online or offline, a connection path and capabilities. */
export type Device = {
  node_id: string;
  name: string;
  kind: string;
  online: boolean;
  path: "direct" | "relay" | "local" | "unknown" | string;
  rtt_ms: number | null;
  /** "this" marks the local device, "claude" the device holding Claude credentials. */
  badges: string[];
  guest: boolean;
  same_identity?: boolean;
  caps: Record<string, boolean>;
  /** e.g. "macOS 26.6". */
  os_version?: string | null;
  /** Hardware model, e.g. "Mac16,8". */
  model?: string | null;
  /** Unix seconds when the device last answered. */
  last_seen?: number | null;
};
export type Identity = { code: string; owner_pub: string; recovery: boolean; recovery_kind?: "passphrase" | "code" | null; epoch: number | null; members: number };
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
export type CapabilityId = "terminal" | "desktop" | "computer" | "browser";
export type PermissionState = "granted" | "denied" | "unknown" | "not_required";
export type CapabilityAction =
  | "install_plan"
  | "install"
  | "start"
  | "stop"
  | "enable"
  | "disable"
  | "request_permissions"
  | "open_settings"
  | "use_profile"
  | "refresh";
export type Capability = {
  id: CapabilityId | string;
  provider: {
    kind: string;
    installed: boolean;
    running: boolean;
    version?: string | null;
    detail?: string | null;
  };
  permissions: { id: "screen_recording" | "accessibility" | string; state: PermissionState }[];
  sharing: { enabled: boolean; address?: string | null; mode?: "profile" | "managed" | "address" | null };
  actions: CapabilityAction[];
  error?: string | null;
};
export type Capabilities = { platform: string; items: Capability[] };
export type InstallPlan = {
  plan_id: string;
  commands: string[];
  url?: string | null;
  expires_at?: number | null;
};
export const grantKeys = [
  "exec",
  "terminals",
  "projects",
  "desktop",
  "computer",
  "browser",
  "messages",
  "activity",
  "spaces",
  "llm",
  "vault",
] as const;
export type GrantKey = (typeof grantKeys)[number];
export type Grant = { permissions: Partial<Record<GrantKey, boolean>> };
/** Errors keep their HTTP status so pages can tell "not available yet" apart. */
export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly code = "",
  ) {
    super(message);
  }
}
export const notAvailable = (error: unknown) =>
  error instanceof ApiError &&
  (error.status === 404 || error.status === 405 || error.code === "not_found");
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
      const text = await response.text();
      let value: any = null;
      try {
        value = text ? JSON.parse(text) : null;
      } catch {
        if (response.ok) throw new ApiError("Invalid response", response.status);
      }
      if (!response.ok)
        throw new ApiError(
          value?.message || value?.error || value?.code || "Request failed (" + response.status + ")",
          response.status,
          typeof value?.code === "string" ? value.code : "",
        );
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
