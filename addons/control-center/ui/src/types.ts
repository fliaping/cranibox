export interface Agent { id: string; label: string; installed: boolean; selected?: boolean; version?: string; launch?: string; web_url?: string; web_path?: string; web_label?: string; web_state?: string }
export interface Service { id: string; name: string; kind: string; scope?: string; status: string; pid?: number | null }
export interface Check { id: string; label?: string; state: string; detail?: string }
export interface Snapshot {
  schema_version: number;
  revision?: string;
  generated_at?: string;
  locale?: { current: string };
  summary?: { healthy?: boolean; ready_agents?: number; running_services?: number; service_count?: number; global_skills?: number; mcp_servers?: number };
  workspace?: { root?: string; user?: string; docker?: { mode?: string; detail?: string } };
  access?: { hostname?: string; code_server_local?: string; desktop_local?: string };
  bootstrap?: { foundation_ready?: boolean; onboarding?: { complete?: boolean } };
  agents?: Agent[];
  services?: Service[];
  mcp?: { count?: number; global_count?: number; manager?: { installed?: boolean }; items?: { id: string; name: string; detail?: string; managed?: boolean; enabled?: boolean; transport?: string; agents: { id: string; label: string; enabled?: boolean }[] }[] };
  skills?: { count?: number; shared_count?: number; root?: string; items?: { id: string; name: string; description?: string; path?: string; shared?: boolean; agents: string[] }[] };
  browser?: { installed?: boolean; supported?: boolean; state?: string; running?: boolean; remote_debugging?: boolean; mcp_configured?: boolean; managed_default?: boolean; version?: string; profile?: string; profile_name?: string; desired_profile_name?: string; profile_name_applied?: boolean; profile_exists?: boolean; migration_pending?: boolean; debug_port?: number; mcp_agents?: string[]; exposed?: boolean };
  desktop?: { installed?: boolean; runtime_available?: boolean; running?: boolean; state?: string; service?: string; local_url?: string; code_server_path?: string; computer_use?: { installed?: boolean; running?: boolean; paused?: boolean; state?: string; backend_available?: boolean; loopback_only?: boolean; bridge_url?: string; native_url?: string; native_bind?: string; backend?: string; backend_diagnostic?: { detail?: string }; resolution?: { width: number; height: number }; active_sessions?: { id: string; last_action?: string; idle_seconds?: number }[]; mcp_agents?: string[]; capabilities?: string[] } };
  network?: { installed?: boolean; mode?: string; root_domain?: string; routes?: { name: string; host?: string; target: string; public_url?: string }[] };
  tailscale?: { installed?: boolean; state?: string; serve_enabled?: boolean; url?: string; ips?: string[]; dns_name?: string };
  local_node?: { id: string; online: boolean; terminal_sessions: boolean };
  metrics?: Metrics;
  ports?: { command: string; pid?: number; address: string; user?: string }[];
  capabilities?: Check[];
  doctor?: { healthy?: boolean; checks?: Check[] };
}
export interface Prompt { id: string; kind: 'input' | 'pick' | 'confirm'; title?: string; prompt?: string; value?: string; placeHolder?: string; canPickMany?: boolean; items?: { label: string; description?: string; picked?: boolean }[]; choices?: string[] }
export interface RemoteNode { desktop_relay?: boolean; mesh_group?: string | null; terminal?: boolean; terminal_sessions?: boolean; id: string; aliases?: string[]; name?: string; os?: string; arch?: string; online: boolean; last_seen?: number }
export interface Metrics { rss_bytes?: number; cpu_percent?: number; uptime_seconds: number; collections: number; collection_ms: number; maximum_collection_ms: number; cache_hits: number; stream_messages: number; stream_bytes: number; scope: string; unified_server?: boolean }
export interface Operation { id: string; title?: string; node?: string; program?: string; args?: string[]; outcome: string; created_at: number; finished_at?: number; output?: string; stdout?: string; stderr?: string; exit_code?: number; error?: string; truncated?: boolean }
export interface PollState { cursor: number; snapshot?: Snapshot | null; snapshotAt?: number; busy: boolean; refreshing?: boolean; output?: string; prompt?: Prompt | null; forceWelcome?: boolean; error?: string; outputDelta?: string; outputReset?: boolean; events?: { type: string; message?: string; url?: string }[] }
export interface BootConfig { csrf: string; language: string; cursor: number; workspaceUrl: string; desktopUrl: string; /** "node": CLI node or desktop app; "workspace": Docker container server with its consoles. */ edition?: 'workspace' | 'node'; /** This device's node ID. */ node?: string; /** True only inside the desktop app, which serves /api/shell. */ shell?: boolean; /** @deprecated alias of edition. */ mode?: 'workspace' | 'node'; /** Saved beside the node configuration (the desktop app's origin changes per launch). */ preferences?: Preferences }
export interface Preferences { language?: 'en' | 'zh-cn'; theme?: 'light' | 'dark' | 'system'; onboarded?: boolean }
export type UpdateChannel = 'latest' | 'next' | 'dev';
export type UpdatePhase = 'resolving' | 'downloading' | 'verifying' | 'staged' | 'activated' | 'done' | 'failed' | 'rolled_back';
export interface UpdateStatus {
  id: string; spec: string; phase: UpdatePhase;
  from: string | null; to: string | null; error: string | null;
  downloaded: number; total: number | null; started_at: number; updated_at: number;
}
export interface NodeUpdate {
  node: string; self: boolean; version: string;
  runtime: { managed: boolean; state: 'pending' | 'confirmed' | null; version: string | null; os: 'linux' | 'macos' | 'windows'; arch: 'x86_64' | 'aarch64'; can_update: boolean; reasons: string[] };
  status: UpdateStatus | null;
  available?: Record<UpdateChannel, string | null> | null;
  available_error?: string | null;
}
export type UpdateRequest = { node: string; action: 'apply'; spec: string; force?: boolean } | { node: string; action: 'rollback' | 'adopt' };
export interface UpdateAccepted { id: string }
export interface UpdateAdopted { adopted: string; needs_restart: boolean }
declare global { interface Window { workspaceWeb: BootConfig } }
