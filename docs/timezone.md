# Workspace timezone

Run `workspacectl timezone set <IANA-zone>` to select a timezone, for example
`workspacectl timezone set America/Phoenix` (UTC-7 year-round).
For another location, use `workspacectl timezone set Europe/London`.
Use an IANA region, rather than a fixed offset, so daylight-saving rules remain correct.

- `workspacectl timezone status --json`: configured, system and calling-process timezone.
- `workspacectl timezone rollback`: switch to the previous configured timezone.
- `workspacectl timezone apply`: restore the persisted system timezone, including at boot.
- `workspacectl timezone set America/Phoenix --config-only`: write durable settings without changing the current system timezone.

Settings live in `/config/.config/agent-workspace/timezone`. Shell startup files and
the managed browser launcher read that setting. The persistent custom s6 service
`workspace-timezone` reapplies `/etc/localtime` when the container starts. System
application requires root or passwordless `sudo`. Keep the normal `/config` mount.

Existing processes retain their environments. Reopen terminal sessions and restart
the managed browser when convenient; status deliberately does not claim to verify
the browser. Verify its effective setting using `Intl.DateTimeFormat().resolvedOptions().timeZone`
and `new Date().getTimezoneOffset()` (Phoenix: 420).

## Container startup environment

Every set/rollback command also generates `timezone.env` and
`timezone.compose.yaml` alongside the timezone setting. The env file contains
`TZ=America/Phoenix` when that timezone is selected and can be passed to `docker run --env-file`.
For Compose, include the generated override last:

```sh
docker compose -f docker-compose.yml -f ./home/.config/agent-workspace/timezone.compose.yaml up -d
```

Run this on the **outer Docker host**. Replace `./home` with the host directory
mounted at `/config`. The override targets the `agent-workspace` service; adjust
the service name for a differently named deployment. It sets TZ explicitly, so
an inherited host TZ cannot change the requested value through interpolation.
The command may recreate the container and interrupt running sessions. A plain
`docker restart` does not update container creation environment variables. The
timezone command generates deployment artifacts but does not recreate containers.

This capability does not change proxy routing, locale, fonts, user agent, cookies or
credentials. It configures the requested timezone; a third-party score is not a
measurement of account suspension probability.
