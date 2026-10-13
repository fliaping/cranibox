# Common Software

This document records reusable software capabilities that should stay useful
across Agent Workspace deployments. Keep runtime state under `/config`.

## Custom-domain routing

Purpose: lightweight HTTP routing for subdomain-based service exposure.
`workspacectl` is the supported user and Agent interface; it installs and uses
the Caddy routing backend automatically.

Configure or migrate the root domain:

```bash
workspacectl network domain workspace.example.com
```

Current routing model:

```text
ping-code.h1.fliaping.com:7555       -> code-server
ping-<port>.h1.fliaping.com:7555     -> 127.0.0.1:<port>
api-ping-<port>.h1.fliaping.com:7555 -> 127.0.0.1:<port>
```

`PROXY_HOST_PREFIXES` accepts multiple comma-separated prefixes. Gateway auth is
handled outside Caddy, so Caddy treats all prefixes uniformly.

Common commands:

```bash
workspacectl routes
workspacectl proxy add app 127.0.0.1:3000
workspacectl proxy remove app
workspacectl proxy check
tail -f /config/proxyctl/access.log
```

For a loopback service that validates both HTTP `Host` and WebSocket `Origin`,
persist both upstream values on its trusted gateway route:

```bash
workspacectl proxy add dashboard 127.0.0.1:9119 \
  --upstream-host 127.0.0.1 \
  --upstream-origin http://127.0.0.1
```

Services that enforce trusted proxy attribution, including OpenClaw, also need
the route-level `--rewrite-client-ip` option so client-supplied forwarding
headers are replaced instead of preserved.

The persisted `/config/proxyctl` directory is an internal implementation detail.
It contains Caddy and routing state, but no separate CLI is required.

## Tailscale userspace network

Purpose: private tailnet access without opening a public port or granting the
container `NET_ADMIN`.

```bash
agent-workspace-manager install tailscale
workspacectl tailscale login
workspacectl tailscale serve
systemctl --user status tailscaled-workspace.service
```

Runtime, identity, and logs persist under `/config/opt/tailscale`,
`/config/.local/share/tailscale`, and `/config/.local/log/user-systemd`.

## DeepSeek Harness

Purpose: official plugin-based DeepSeek coding Agent with Web and headless
surfaces.

```bash
agent-workspace-manager install agent deepseek-harness
systemctl --user status deepseek-harness.service
deepseek-harness headless "inspect this workspace"
```

The Web UI is loopback-only on `127.0.0.1:3080` and opens through the existing
code-server gateway session at `/proxy/3080/`. Runtime state and credentials
persist under `/config/.dsh`; the official UI keeps stored API keys write-only.

## code-server

Purpose: browser-based VS Code environment and workspace access.

Service:

```bash
systemctl --user status code-server.service
systemctl --user restart code-server.service
```

Main URL:

```text
https://ping-code.h1.fliaping.com:7555
```

Auth is disabled in code-server; authentication is expected at the gateway.

## Desktop browser control

Purpose: let a trusted Agent inspect and operate the Chromium window already in
use on the Selkies desktop, including its current tabs and signed-in state.

```bash
workspacectl browser status
workspacectl browser setup
workspacectl browser open
```

The setup command adds `chrome-devtools-mcp` to installed Agents and points it at
the managed Chromium endpoint on `127.0.0.1:9222`. The desktop's default browser
launcher always uses `/config/.config/agent-browser`, so users and Agents share
one visible browser without per-session approval prompts. The endpoint is not
published outside the container.

The active Chromium profile is visibly named `Agent Workspace (Managed)` by
default, making a separately launched unmanaged Chromium profile easy to spot.
Override it with `AGENT_WORKSPACE_BROWSER_PROFILE_NAME` when needed.

For an existing workspace, close legacy Chromium once before the first managed
launch. The launcher copies `/config/.config/chromium` atomically, excludes
runtime lock files, and retains the original profile for rollback. A container
Agent receives the managed profile's live tabs, cookies, local storage, and
authenticated sessions, so use this mode only with trusted Agents.

The managed *user data directory* is `/config/.config/agent-browser`. Chromium
profiles remain nested inside it: a migrated installation can therefore still
show the profile name `Work` while its on-disk profile path is
`/config/.config/agent-browser/Default`.

Verify a desktop launch with either of these checks:

```bash
workspacectl browser status
# Expected when running: state ready, endpoint ready, profile agent-browser

# In Chromium, chrome://version should show:
# Profile Path: /config/.config/agent-browser/Default
```

If Control Center reports `Restart in managed mode`, another Chromium process
is using a non-managed profile. Close that browser window once and reopen **Web
Browser** from the desktop or run `workspacectl browser open`.

## Desktop Computer Use

Purpose: let trusted Agents see and operate native applications on the exact
Selkies desktop visible to the user.

```bash
workspacectl desktop setup
workspacectl desktop status
workspacectl desktop screenshot --output /config/Downloads/desktop.png
workspacectl desktop emergency-stop
workspacectl desktop resume
```

Selkies' PixelFlux backend currently binds its internal port `8764` to the
container interface; never publish it or attach the workspace to an untrusted
Docker network. The persistent `agent-desktop-bridge` user service listens only
on `127.0.0.1:8765`, records active
MCP sessions, normalizes Computer Use actions, and blocks input after an
emergency stop. `agent-desktop-mcp` is registered globally for installed Codex,
Claude Code, Hermes, and DeepSeek Harness clients. Browser tasks should use the
managed Chromium CDP path; this full-screen capability is for native GUI apps.

Window focus uses the Wayland foreign-toplevel protocol when `wlrctl` is
available and falls back to XWayland window IDs. Window enumeration is marked
partial because the compositor does not expose a stable complete listing API.

## Desktop Debian Package Installer

Purpose: install a local `.deb` using persistent PRoot or normal system
installation with saved packages and startup restoration.

Double-click a `.deb` in the file manager. The registered desktop handler shows
the package name, version, architecture, description, and source path, then
offers an installation mode and explains its persistence and permissions. After
confirmation, apt resolves dependencies in the selected environment. Managed
desktop menu entries use a `(Persistent)` or `(Native)` suffix.

Agents should inspect the planned package changes first:

```bash
agent-workspace-deb-installer --dry-run /path/to/package.deb
```

Only after explicit user approval, an Agent can perform a non-interactive
installation:

```bash
agent-workspace-deb-installer --yes /path/to/package.deb
```

Installations are serialized with a lock and appended to
`/config/.local/log/agent-workspace/deb-installer.log` (or
`deb-installer-native.log`). In PRoot mode, application files, dependencies and
the package database remain under `/config` across container rebuilds. Native
mode saves original packages and a restore manifest under `/config`; missing
packages are reinstalled at startup, and dependencies may require network
access. The installer and its desktop association are installed by
`agent-workspace-manager install deb-runtime` (also included in
`install workspace-controls`). Installation and package maintenance can run
while applications are open; PRoot initialization, backup and restore require
applications to exit. Detected Electron applications receive configurable
compatibility and Wayland display settings. Normal Ubuntu-compatible desktop
packages are supported;
drivers and system-service packages may require another approach.

See [Persistent Debian applications](persistent-deb-apps.md) for setup,
removal, backup/restore, application overrides and compatibility details.

## User Service Manager

Purpose: persistent service control inside the container without full systemd.

Unit files live in:

```text
/config/.config/systemd/user
```

Common commands:

```bash
systemctl --user status <service>
systemctl --user restart <service>
systemctl --user show <service> --property ActiveState,SubState,MainPID,Result
tail -n 100 /config/.local/log/user-systemd/<service>.log
```

## Custom s6 Services

Purpose: persistent s6 service definitions that survive container rebuilds.

Put service directories under:

```text
/config/custom-services.d/<name>/run
```

The startup registration flow recreates runtime links under `/run/service`.

## Code-Server Extensions

Purpose: in-browser operational tools.

Install or refresh:

```bash
agent-workspace-manager install code-server-extensions
```

Current reusable extensions:

- `control-center`: unified onboarding and daily control for Agents, services,
  desktop, custom-domain and Tailscale access, proxy routes, and diagnostics.
- `selkies-desktop`: embedded desktop integration opened from Control Center.

Legacy `service-manager` and `caddy-proxy-manager` sources remain available for
focused development, but the normal installer removes their separate Activity
Bar entries after installing Control Center.

## Homebrew

Purpose: persistent user-space package manager for optional tools.

Path:

```text
/config/.linuxbrew
```

Use:

```bash
brew install <package>
brew list
```

## Node Runtime

Purpose: persistent JavaScript runtime and global tools.

Path:

```text
/config/node
```

Use normal Node/npm commands after the runtime is on `PATH`.
