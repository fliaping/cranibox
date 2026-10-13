// Synthetic data only: browser tests never connect to installed workspace tools.
export const snapshot = {
  schema_version: 1, revision: 'ui-preview', locale: { current: 'en' },
  summary: { healthy: true },
  workspace: { root: '/config/Workspace', user: 'workspace', docker: { mode: 'isolated' } },
  access: { hostname: 'cranibox-dev' },
  bootstrap: { foundation_ready: true, onboarding: { complete: true } },
  agents: [
    { id: 'codex', label: 'Codex', installed: true, selected: true, version: 'Preview version', launch: 'codex' },
    { id: 'claude-code', label: 'Claude Code', installed: true, version: 'Preview version', launch: 'claude' },
    { id: 'hermes', label: 'Hermes', installed: false, launch: 'hermes' },
    { id: 'deepseek-harness', label: 'DeepSeek', installed: true, version: 'Preview version', launch: 'dsh', web_path: '/proxy/8787/', web_label: 'Open DeepSeek' },
    { id: 'openclaw', label: 'OpenClaw', installed: true, version: 'Preview version', launch: 'openclaw', web_path: '/proxy/18789/', web_label: 'Open Gateway' },
  ],
  services: [
    { id: 'agent-workspace-web.service', name: 'Cranibox browser control center', kind: 'systemd', status: 'running', pid: 2401 },
    { id: 'openclaw-gateway.service', name: 'OpenClaw Gateway', kind: 'systemd', status: 'running', pid: 2402 },
    { id: 'long-job', name: 'Sample operation logs', kind: 'systemd', status: 'running', pid: 2403 },
    { id: 'agent-desktop-bridge.service', name: 'Desktop control bridge', kind: 'systemd', status: 'stopped' },
    { id: 'workspace-services', name: 'User service manager', kind: 's6', status: 'running', pid: 2404 },
    { id: 'sample-service-with-a-long-description.service', name: 'A sample service with a longer description for narrow screen layouts', kind: 'systemd', status: 'failed' },
  ],
  mcp: { count: 3, manager: { installed: true }, agents: [
    { id: 'codex', label: 'Codex', installed: true, configured: 2 },
    { id: 'claude-code', label: 'Claude Code', installed: true, configured: 1 },
  ], items: [
    { id: 'filesystem', name: 'Filesystem', detail: 'Workspace file access', managed: true, transport: 'stdio', agents: [{ id: 'codex', label: 'Codex', enabled: true }, { id: 'claude-code', label: 'Claude Code', enabled: true }] },
    { id: 'browser', name: 'Managed browser', detail: 'Shared Chromium session', managed: false, transport: 'http', agents: [{ id: 'codex', label: 'Codex', enabled: true }] },
    { id: 'desktop', name: 'Desktop control', detail: 'Computer Use tools', managed: true, transport: 'http', agents: [{ id: 'codex', label: 'Codex', enabled: false }] },
  ] },
  skills: { count: 3, shared_count: 2, root: '/config/.agents/skills', items: [
    { id: 'ui-ux-pro-max', name: 'ui-ux-pro-max', description: 'Design consistent, accessible interfaces for your applications.', shared: true, agents: ['codex', 'claude-code'] },
    { id: 'agent-browser', name: 'agent-browser', description: 'Automate browser workflows and verify your applications.', shared: true, agents: ['codex', 'claude-code'] },
    { id: 'project-context', name: 'project-context', description: 'Project-specific guidance.', shared: false, agents: ['codex'] },
  ] },
  browser: { installed: true, supported: true, running: true, state: 'ready', version: 'Chromium preview', profile_name: 'Workspace', desired_profile_name: 'Workspace', profile_name_applied: true, managed_default: true, mcp_configured: true, mcp_agents: ['codex', 'claude-code'], debug_port: 9222, exposed: false },
  desktop: { installed: true, runtime_available: true, running: true, state: 'running', service: 'selkies', computer_use: { installed: true, running: true, state: 'running', backend_available: true, backend: 'cua-driver', loopback_only: true, bridge_url: 'http://127.0.0.1:8090', native_bind: '127.0.0.1:8091', resolution: { width: 1920, height: 1080 }, mcp_agents: ['codex'], active_sessions: [] } },
  network: { installed: true, mode: 'ready', root_domain: 'workspace.example.com', routes: [{ name: 'preview', host: 'preview.workspace.example.com', target: '127.0.0.1:8080', public_url: 'https://preview.workspace.example.com' }] },
  tailscale: { installed: true, state: 'needs-login' },
  ports: [{ address: '127.0.0.1:9080', command: 'workspace-web', pid: 2401 }, { address: '127.0.0.1:8443', command: 'code-server', pid: 2405 }],
  capabilities: [{ id: 'browser', label: 'Managed browser', state: 'ready', detail: 'Shared profile configured' }, { id: 'tailscale', label: 'Tailscale', state: 'needs-attention', detail: 'Sign in to enable private network access' }],
  doctor: { healthy: true, checks: [{ id: 'storage', label: 'Persistent storage', state: 'ready', detail: '/config is writable' }, { id: 'runtime', label: 'Runtime services', state: 'ready', detail: 'Required services are available' }] },
};
