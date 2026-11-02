# Caddy Proxy Manager

code-server extension for viewing and managing Cranibox Caddy routes.

It reads state from `/config/proxyctl` and uses `workspacectl proxy` for
mutations. It does not edit generated Caddy JSON directly.

Features:

- Show proxy environment and named routes
- Add a named route
- Remove a named route
- Run `workspacectl proxy check`
- Show generated Caddy config
- Tail Caddy access logs and service logs
- Roll back the last route change
- Open a public route URL

After changes, reload code-server with `Developer: Reload Window`.
