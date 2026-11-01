# Desktop on/off (headless mode)

Status: implemented. Owner surface: image (`Dockerfile`, `scripts/cranibox-desktop`,
`addons/desktop-power/`), `workspacectl desktop on|off|power`, Control Center Desktop page.

## Goal

The graphical desktop is the largest memory consumer in the workspace image. Users want to
boot without it and switch it on and off at runtime, without restarting the container and
without disturbing agents, terminals, code-server, Control Center, the node or the gateway.
The desktop web page (Selkies, port 3000/3001) must stay reachable at all times and carry the
switch itself.

## What "desktop" means here

The gated s6 services (upstream linuxserver/webtop):

| Service | Role |
|---|---|
| `svc-pulseaudio` | desktop audio |
| `svc-xorg` | Xvfb (X11 mode); idle in Wayland mode |
| `svc-selkies` | Selkies server, pixelflux compositor and the video/audio stream (port 8082) |
| `svc-xsettingsd` | X settings daemon |
| `svc-de` | labwc + XFCE session, Xwayland, desktop apps and the desktop browser |
| `svc-watchdog` | upstream desktop watchdog |

nginx (ports 3000/3001), Control Center, code-server, the cranibox node/gateway and all
user services are not gated.

## Design

- **Gate.** At build time each gated service's `run` is renamed `run.upstream` and replaced
  with `exec cranibox-desktop gate <svc> run.upstream`. The gate execs the upstream script when
  the desktop is on and otherwise execs `sleep infinity`, so s6 still sees a healthy,
  supervised service and dependency ordering is unchanged.
- **State.** `/run/cranibox-desktop/state` (tmpfs). If it is missing, the boot state comes from
  `CRANIBOX_DESKTOP` (`on` by default; `off|false|0|no|disabled|headless` mean off). A container
  restart therefore returns to the boot setting.
- **Off.** Write state `off`, `s6-svc -r` the services in reverse order (each restarts into the
  idle placeholder), then terminate leftovers: processes in the services' sessions, desktop
  programs owned by `abc`, non-headless Chromium, the session D-Bus and system D-Bus helpers the
  desktop activated (`udisksd`, `polkitd`). TERM, then KILL after 6 s. Runtime files the
  upstream scripts wait on (`/de-pid`, `/defaults/pid`, `/tmp/.X1-lock`,
  `/dev/shm/audio.lock`, `$XDG_RUNTIME_DIR/wayland-*`) are removed so the next start is clean.
- **On.** Clean the same runtime files, write state `on`, `s6-svc -r` the services in order and
  wait for the stream port.
- **Control API.** `svc-cranibox-desktop` (root, s6 longrun) serves `127.0.0.1:3079`:
  `GET /status`, `POST /on`, `POST /off`, `POST /power {"state"}`. Changes run in the background
  (202) and return 409 while another change is in progress. nginx proxies
  `<SUBFOLDER>cranibox-desktop/` to it, behind the same Basic auth as the desktop page.
  POST requires the header `X-Cranibox-Desktop: 1`, which a cross-site form cannot send and a
  cross-origin `fetch` cannot send without a preflight that is never granted.
- **Desktop page.** The Selkies dashboard's module script is replaced with
  `cranibox-desktop.js`, which asks `/status` first. Off: a full-page card ("桌面已关闭") with the
  current container memory, the expected extra memory and a "开启桌面" button. On: the original
  Selkies client loads, plus a small "关闭桌面" tab on the right edge that asks for confirmation.
  The page reloads when another surface changes the state. If the API is unreachable the page
  falls back to plain Selkies.
- **Memory figures.** Desktop memory is the sum of `Pss_Anon + Pss_Shmem` of desktop processes;
  container memory is cgroup `memory.current - inactive_file`, matching `docker stats`.

## Measurements

`ubuntu-xfce`, amd64, idle desktop without the desktop browser, reclaimable page cache
dropped: about 260 MiB with the desktop on, about 78 MiB off (cgroup anon ~220 MB vs ~65 MB).
Off takes about 1 s; on takes 3–5 s until the stream is ready. A running desktop browser adds
its own memory to the savings.

## Limits

- Turning off closes all desktop windows; unsaved work is lost. The page and Control Center ask
  for confirmation.
- Desktop Computer Use and managed desktop Chromium are unavailable while off; agents should use
  headless tools or run `workspacectl desktop on` when a task truly needs the GUI.
- Remote viewers that connect straight to the stream port (8082) see no "desktop is off" card;
  they simply cannot connect until the desktop is on.
- Verified on the Wayland `ubuntu-xfce` variant. X11 mode and the LXQt/KDE variants use the same
  services and gate but were not exercised end to end.
