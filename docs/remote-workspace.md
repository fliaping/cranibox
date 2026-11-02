# Remote Workspace Profiles

The Cranibox workspace image separates the reusable container environment from
deployment-specific ingress. Only `/config` needs to be durable.

## Direct profile

`docker-compose.remote.yml` is the default single-user profile:

```text
browser -- HTTPS/password --> Webtop :3001
browser -- HTTPS/password --> code-server :8443
                              |
                              +-- user services and Agent runtimes
```

Run `scripts/remote-up.sh`, choose one Agent, and let it generate `.env.remote`.
Codex is the default; Claude Code, Hermes, DeepSeek Harness, and no-Agent
bootstraps are available. Both Webtop and code-server use the generated password. Their default
certificates are self-signed, so this profile is best reached through a private
network or VPN.

The printed `localhost` URLs refer to the Docker host. From another machine,
use the server hostname or IP when ports `3001` and `8443` are reachable. If the
host firewall, WSL networking, or NAT does not publish them, keep the ports
private and create an SSH tunnel instead:

```bash
ssh -N \
  -L 3001:127.0.0.1:3001 \
  -L 8443:127.0.0.1:8443 \
  user@server
```

Then open the same `https://localhost:3001` and
`https://localhost:8443` addresses on the client.

The first boot sets `AGENT_WORKSPACE_BOOTSTRAP=remote`. The s6 bootstrap service
updates the application source and installs:

- official standalone code-server under `/config/opt/code-server`
- the unified Cranibox Control Center and desktop integration
- persistent custom-service registration
- the selected Agent CLI or Web runtime

The completion marker is stored under `/config/.local/state/agent-workspace`.
After installation, open code-server. Control Center opens automatically on the
first session and walks through Agent sign-in, the durable workspace, remote
access, and optional capabilities. It remains available from the single Cranibox
Activity Bar icon. Terminal-only users can run `codex`, `claude`, or
`hermes setup --portal` directly. DeepSeek Harness runs as a loopback user
service and opens on the same authenticated code-server origin at
`/proxy/3080/`. Seeded `AGENTS.md` and `CLAUDE.md` files teach
supported Agents about the persistence and service-management model.

Use `workspacectl status` as the terminal entry point for container operations;
`workspacectl status --json` is the stable Control Center contract. It exposes
service, log, port, route, and capability controls, and warns when a Docker
socket makes the privilege boundary larger than the workspace container.

## Authenticated gateway profile

The optional custom-domain profile matches deployments that already provide
wildcard DNS, TLS, and strong authentication:

```text
browser --> authenticating gateway --> Caddy :80 --> code-server :8443
                                                --> named local services
```

Configure code-server for the loopback HTTP hop, then configure the domain
through `workspacectl` (which installs the Caddy backend automatically):

```bash
CODE_SERVER_BIND=127.0.0.1:8443 \
CODE_SERVER_CERT=false \
CODE_SERVER_AUTH=none \
CODE_SERVER_ALLOW_NO_AUTH=true \
CODE_SERVER_PROXY_DOMAIN='ping-{{port}}.example.com' \
CODE_SERVER_RECONFIGURE=true \
agent-workspace-manager install code-server

PROXY_ROOT_DOMAIN=example.com \
PROXY_HOST_PREFIXES=ping,api-ping \
PROXY_PORT_ROUTING=direct \
CODE_SERVER_PROXY_DOMAIN='ping-{{port}}.example.com' \
workspacectl network domain example.com
```

`auth=none` and direct port routing are safe only when the upstream gateway
authenticates every public hostname. Without that guarantee, keep code-server
password authentication and `PROXY_PORT_ROUTING=code-server`.

Control Center can configure or migrate the same root domain without editing
Caddy JSON directly:

```bash
workspacectl network domain workspace.example.com
```

This configures the container-side HTTP router. DNS, public TLS, port mapping,
and gateway authentication remain deployment responsibilities.

## Private Tailscale profile

Tailscale is an optional alternative when a public domain or inbound port is
undesirable. Cranibox uses userspace networking, so the default mode
does not need `/dev/net/tun`, `NET_ADMIN`, or a Docker socket:

```bash
agent-workspace-manager install tailscale
workspacectl tailscale login
workspacectl tailscale serve
```

Login is interactive by default and Control Center never stores an auth key.
Serve publishes code-server only inside the tailnet; Selkies remains available
through code-server's same-origin `/proxy/3000/` path. The first `serve` on a
tailnet may print an admin-console link to enable Serve/HTTPS.

### Unattended login with an auth key

Images built from this change can join the tailnet at first boot. Set one of:

| Variable | Purpose |
|----------|---------|
| `TAILSCALE_AUTHKEY` (alias `TS_AUTHKEY`) | Auth key value |
| `TAILSCALE_AUTHKEY_FILE` | Path to a file inside the container that holds the key (preferred, e.g. a Docker secret) |
| `TAILSCALE_HOSTNAME` | Device name (default `agent-workspace`) |
| `TAILSCALE_ADVERTISE_TAGS` | `--advertise-tags` value, e.g. `tag:agent-workspace` |
| `TAILSCALE_EXTRA_ARGS` | Extra `tailscale up` arguments, split on whitespace |
| `TAILSCALE_SERVE` | `true` to run `workspacectl tailscale serve` after login |

With `scripts/remote-up.sh`, add them to `.env.remote`; `docker-compose.remote.yml`
passes them through. At startup a `custom-cont-init.d` hook removes the key from
the s6 container environment (whose files are world-readable), writes it to a
`0600` file under `/run/agent-workspace`, and `workspace-bootstrap` installs
Tailscale if needed and runs `workspacectl tailscale autoconnect`, which calls
`tailscale up --auth-key=file:<path>`. The staged file is deleted after the
attempt, and the step is skipped when the node is already logged in, so the key
is only used once. It is never printed, but the original variable stays visible
to `docker inspect` and `docker exec`; prefer `TAILSCALE_AUTHKEY_FILE`, and use a
one-off, pre-approved, tagged key with a short expiry.

## Persistence boundary

Persistent application state stays under:

```text
/config/agent-workspace-manager
/config/opt/code-server
/config/opt/tailscale
/config/.config/code-server
/config/.config/systemd/user
/config/.local/log/user-systemd
/config/.local/run/user-systemd
/config/.local/share/tailscale
/config/proxyctl
/config/custom-services.d
```

Image-layer paths provide stable OS dependencies and startup hooks only. A
container rebuild therefore needs the `/config` volume, not separate mounts for
individual tools.
