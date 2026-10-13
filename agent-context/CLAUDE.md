# Agent Workspace Container

You are running inside an Agent Workspace container. Treat `/config` as the
only durable filesystem area; image-layer paths such as `/etc`, `/usr/local`,
and `/run` can be replaced when the container is rebuilt.

- User projects live under `/config/Workspace`.
- Put persistent app state in `/config/.config`, `/config/.local`, or a named
  `/config/opt/<app>` directory.
- Start with `workspacectl status` to see the environment; use
  `workspacectl status --json` when structured state is useful.
- Use `workspacectl services`, `workspacectl service ...`, and
  `workspacectl logs ...` for persistent user services.
- Use `workspacectl s6 ...` for image-provided s6 services.
- Use `workspacectl ports` and `workspacectl routes` for network inspection.
- If the Rust gateway is installed, use `workspacectl nodes --json` to discover
  remote nodes, and `workspacectl node exec --node <id> -- <program> <args>`
  to run commands with the remote client's account permissions. Use
  `workspacectl node job <job-id>` and `workspacectl node cancel <job-id>`
  for asynchronous jobs. Commands are not replayed after disconnection.
  The `workspace-runtime mcp` stdio server exposes the same operations.
- Use `workspacectl install ...` for optional Agent Workspace capabilities.
- Use `workspacectl network domain <domain>` for the managed Caddy domain;
  DNS, TLS, and gateway authentication remain deployment responsibilities.
- Use `workspacectl tailscale status|login|serve` for private Tailnet access.
  Tailscale runs in userspace mode, so do not try to create a TUN device.
- Selkies Desktop is the container's graphical workspace. Inspect it with
  `workspacectl status --json` and manage its image service with
  `workspacectl s6 status|restart svc-selkies`.
- Use `workspacectl browser status` before operating the user's managed Chromium
  window. Configured container Agents can access all open tabs and signed-in
  sessions without a per-session prompt. Treat that access as sensitive and
  never publish its loopback debugging endpoint.
- Use `workspacectl desktop status` before controlling native GUI applications.
  Desktop Computer Use captures the same Selkies display and can send pointer
  and keyboard input. Prefer managed Chromium CDP for web tasks; reserve desktop
  control for native apps. Stop all Agent input immediately with
  `workspacectl desktop emergency-stop`, and resume only with explicit user
  intent. The bridge on `8765` is loopback-only. PixelFlux currently binds its
  internal upstream on `8764` to the container interface, so never publish or
  proxy either port and do not attach this workspace to an untrusted Docker network.
- The code-server Control Center is the user-facing view of these same commands;
  keep operations usable from both the UI and terminal.
- The container user may have passwordless `sudo`; use it only when an
  image-layer change is truly necessary.
- For a local Debian package, first run
  `agent-workspace-deb-installer --dry-run /path/to/package.deb`. A `.deb` can
  execute privileged maintainer scripts and changes the replaceable image
  layer; use `--yes` only after explicit user approval.
- A mounted `/var/run/docker.sock` can grant control outside this container.
  Inspect the active Docker mode before using it.

Preserve existing user files and unrelated working-tree changes. Prefer
reversible operations and verify service, port, and route changes after making
them.
