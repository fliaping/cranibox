# Persistent Debian desktop applications

Double-click a local `.deb` in the Webtop file manager, choose **PRoot** or
**Native**, review the package details, and choose Install. After installation,
visible desktop applications offer a **Launch application** button. Installation
success means apt completed; launch logs and error dialogs diagnose startup.

The default PRoot mode resolves dependencies with apt in a
shared Ubuntu 24.04 environment under `/config`. Successful installations add
application-menu entries with a **(Persistent)** suffix. Double-clicking a newer
package version updates the same environment and refreshes its launchers.

The application files, dependencies and dpkg database live in that environment.
They survive container replacement when the same `/config` volume is retained;
there is no package replay on startup. Applications use `/config` as their home,
and share the desktop's display, audio and session sockets. Package installation
uses its own rootfs and does not modify the container's package database.

## Setup and commands

The image must provide Python 3.12+, dpkg-deb, tar, desktop MIME tools, and
KDialog on KDE/Wayland, Zenity, or xmessage on X11. PRoot must be available at
`/config/.local/bin/proot`. KDE uses its native confirmation, progress and result
dialogs; `qdbus6` (or `qdbus`) controls its progress window.
The optional `/config/.local/bin/ncat` must be static to support opening browser
links through the host desktop. These runtimes are already present in this
workspace. Only the `/config` volume is required.

```bash
agent-workspace-manager install deb-runtime
agent-workspace-deb init
agent-workspace-deb status
agent-workspace-deb list
agent-workspace-deb-installer --dry-run /config/Downloads/application.deb
agent-workspace-deb-installer --yes /config/Downloads/application.deb
agent-workspace-deb remove --dry-run package-name
agent-workspace-deb remove --yes package-name
agent-workspace-deb purge --dry-run --autoremove package-name
agent-workspace-deb purge --yes --autoremove --keep-user-config package-name
agent-workspace-deb upgrade --yes
agent-workspace-deb repair --yes
agent-workspace-deb sync
```

Initialization downloads a checksum-pinned Ubuntu Base archive, then signed
Ubuntu repository packages, including GPG for vendor repository setup scripts.
It runs automatically on the first installation.
An interrupted dependency bootstrap can be retried with `init`. A dry run
requires an initialized environment and uses the existing apt indexes.
Package operations set `DPKG_DEB_THREADS_MAX=1` to avoid parallel xz
decompression crashes observed with large deb archives under PRoot. If an
earlier installation stopped during unpacking, installing the same deb again
retries the package and configures pending dependencies. Error dialogs include
the underlying dpkg failure; full output remains in the installation log.

Installing, updating, removing and repairing packages can run while applications
are open. Package operations are serialized with each other. Restart affected
applications after updating their files or shared dependencies. Initialization,
backup and restore still require applications to exit, because they prepare,
snapshot or replace the whole environment. These operations also block new
application launches until finished. The package list retains
installation history, including removed and failed packages. `repair` operates
on the shared environment; `sync` refreshes package status and menu entries.

Use `purge` to remove package configuration inside the rootfs, exported launchers,
all installer-managed cached versions of that package, its application log, and
its managed installation-history record. `--autoremove` also purges dependencies
apt considers unused; inspect `--dry-run` first because the environment is shared.
Original downloads outside the managed cache, backups, and shared apt/installer
audit logs are retained. `remove` keeps its existing behavior.

User configuration is preserved by default (`--keep-user-config`). To delete
application data, explicitly identify its directories relative to `/config`:

```bash
agent-workspace-deb purge --dry-run --delete-user-config --user-config-path .config/Code code
agent-workspace-deb purge --yes --delete-user-config --user-config-path .config/Code code
```

Repeat `--user-config-path` for additional application directories. Close the
application before deleting its data. Only the specified directories and the
package's launch overrides are removed; directory ownership is not guessed.
Paths shared with another installation should be kept. Broad paths, traversal,
symlinks, and known shared runtime directories are rejected. These new options
apply to `agent-workspace-deb` (PRoot); the Native command is unchanged.

## Persistent paths

| Contents | Path |
| --- | --- |
| Ubuntu rootfs, applications, dependencies and dpkg database | `/config/.local/share/agent-workspace/deb-runtime/rootfs` |
| Managed-package manifest and readiness marker | `/config/.local/share/agent-workspace/deb-runtime` |
| Downloaded base image and original deb archives | `/config/.cache/agent-workspace/debs` |
| Exported application launchers | `/config/.local/share/applications/agent-workspace-deb-*.desktop` |
| Copied application icons | `/config/.local/share/icons/agent-workspace-deb` |
| Installer summary and full apt output | `/config/.local/log/agent-workspace/deb-installer.log` and `deb-runtime.log` |
| Desktop invocation and dialog errors | `/config/.local/log/agent-workspace/deb-installer-launch.log` |
| Application output from menu launches | `/config/.local/log/agent-workspace/deb-apps/<package>.log` |
| Per-application overrides | `/config/.config/agent-workspace/deb-apps.json` |
| Environment backups and previous rootfs | `/config/.cache/agent-workspace/deb-backups` |

## Backup and restore

This section covers PRoot snapshots. Native startup replay is described below.

```bash
agent-workspace-deb backup
agent-workspace-deb restore --yes /config/.cache/agent-workspace/deb-backups/deb-runtime-TIMESTAMP.tar.gz
```

`backup` prints the archive path. It includes the rootfs, manifest and readiness
marker. It does **not** include application home data under `/config`, per-app
overrides, or cached deb archives; back up the full `/config` volume for those.
Although stored below `.cache`, backup files should be preserved if needed.

Restore accepts a compatible native-architecture Ubuntu 24.04 backup, validates
archive paths, swaps in the restored environment and regenerates menu entries.
The previous environment is retained in a printed `previous-*` directory for
recovery. Allow free disk space for both environments and the compressed
archive. Restore does not merge newer package contents into the restored rootfs.

## Application adjustments and troubleshooting

Package-specific environment and arguments can be configured without editing
exported launchers. Keys are Debian package names as shown by `list`:

```json
{
  "example-app": {
    "extra_args": ["--example-option"],
    "environment": {"GDK_BACKEND": "x11"}
  }
}
```

Replace the example with options actually supported by the application. Values
are passed as arguments and environment variables without shell evaluation.
To inspect a launch interactively:

```bash
agent-workspace-deb run --package package-name -- /usr/bin/application
```

### Automatic desktop compatibility

Managed launchers share a compatibility policy derived from the ChatGPT and VS
Code fixes. It runs at each launch, so existing menu entries and newly installed
packages use it without rewriting launchers or adding package-name rules.
Detection resolves the executable inside the selected filesystem and looks for
`icudtl.dat`, `resources.pak`, and `resources/app.asar` or
`resources/app/package.json` beside it. A `bin` wrapper one level below the
payload is supported, as are symlinks and simple `env NAME=value app` launchers.
Unrecognized layouts, shell command strings, and commands without `--package`
receive no automatic Electron flags. Known crash helpers and explicit
`ELECTRON_RUN_AS_NODE=1` invocations are excluded. This is a conservative layout
heuristic, not a guarantee that every Electron package is recognized.

| Detected Electron application | PRoot | Native |
| --- | --- | --- |
| Chromium sandbox | Add `--no-sandbox` | Preserve application default |
| Rendering | Add `--disable-gpu` | Preserve application default |
| Display with an existing Wayland socket | Add `--ozone-platform=wayland` | Add `--ozone-platform=wayland` |
| Display without a Wayland socket | Preserve application default | Preserve application default |

The PRoot default disables the detected application's Chromium sandbox to avoid
the observed zygote failure; the installer explains this before installation.
Software rendering addresses the observed EGL/GPU failures. Wayland selection
lets the tested applications follow Selkies compositor scaling without hardcoding
a scale factor. These defaults are scoped to detected Electron commands through
managed launchers, not system-wide environment variables. Native installation
does not automatically disable sandboxing or GPU acceleration.

If Electron applications look twice as large as KDE at the same Selkies
scale, inspect the separate GTK text multiplier:

```bash
workspacectl desktop scaling status
```

In the tested KDE Wayland session, a leftover value of `2.0` made VS Code
report `devicePixelRatio=3` with the compositor at 150%. Restoring the text
multiplier to `1.0` brought it back to `1.5`. If that extra text scaling is
unintentional, use the explicit repair command, which backs up the old value:

```bash
workspacectl desktop scaling repair
# Undo a managed repair:
workspacectl desktop scaling restore
```

The command discovers the current user's KDE Wayland session and its session
bus, so it also works from an Agent terminal. The setting persists in the
desktop user's `/config/.config/dconf/user`; keeping the `/config` volume
preserves it across container rebuilds. The original value is stored under
`/config/.local/share/agent-workspace/desktop-scaling/original.json`. Repeated
repairs keep the first backup. Restore consumes that backup after verification.
The installer does not apply this optional global preference automatically.

Dynamic verification used the same running VS Code process: changing Selkies
from 300% to 150% changed its reported device pixel ratio from 3 to 1.5.
ChatGPT menu and sidebar sizes halved in matching framebuffer screenshots.
These checks require no fixed application scale flag or polling service.
Status reports compositor and text settings, not live Electron measurements.

Reopen affected applications if their layout does not settle after the change.
Verify the live compositor scale and application size before changing KDE
fonts or panel height; do not automatically reset accessibility preferences
from the package installer.

Explicit `extra_args` and original launcher switches suppress conflicting
automatic defaults. Existing arguments and file operands are preserved. A
per-package `compatibility` object can independently control the policy:

```json
{
  "example-app": {
    "compatibility": {
      "profile": "auto",
      "display": "x11",
      "gpu": "default",
      "sandbox": "default"
    }
  }
}
```

All fields default to `auto`. `profile` accepts `auto`, `electron` (opt in an
unrecognized wrapper), or `none` (disable automatic flags). `display` accepts
`auto`, `wayland`, `x11`, or `default`; `gpu` accepts `auto`, `software`, or
`default`; `sandbox` accepts `auto`, `disabled`, or `default`. `default` means
leave that feature to the application. For older Electron builds without
working Wayland support, set `display` to `x11` or `default`. Explicit flags
remain in force even with `profile: none`; remove unwanted `extra_args` too.
PRoot and Native use their respective override files.

Preview the effective arguments from a desktop terminal without starting or
restarting anything (the current session determines Wayland availability):

```bash
agent-workspace-deb run --dry-run --package chatgpt -- chatgpt
agent-workspace-deb run --dry-run --package code -- code
agent-workspace-deb-native run --dry-run --package package-name -- /usr/bin/application
```

Desktop logs include the detected profile, selected defaults, and effective
arguments. Before launch, a best-effort same-user process check reports main
Electron processes in that payload directory with different compatibility
switches. A desktop notification asks the user to save work and fully quit the
application before reopening; the installer does not kill running applications.
Processes hidden by `/proc` permissions cannot be checked.

The tested ChatGPT build can forward a second launch to an existing background
process and exit zero. Use the application's Quit action (Ctrl+Q), then reopen
it. Closing a window or observing a new launch-log entry alone does not prove
that new display settings reached the running application. The process check
is advisory; launch success does not certify a visible window or scaling.

Desktop logs record launch times, arguments and exit codes. Nonzero exits and
recognized Chromium sandbox failures during the first 30 seconds show an error
through KDialog or Zenity, including when the application returns zero. Other
zero-exit failures still require inspecting the log.
Use `agent-workspace-deb launch package-name` to dispatch its visible launcher.
`launcher package-name` prints its path, excluding hidden URL handlers.

Normal same-architecture desktop packages compatible with Ubuntu 24.04 are the
target. Arbitrary deb compatibility is not guaranteed: drivers, kernel modules,
systemd-dependent services, hardware-specific GPU stacks and browser/Electron
sandboxes may require a different installation method or app-specific changes.
Sandbox restrictions are not disabled globally. PRoot uses the host kernel and
is not a security boundary; install trusted software only. Running applications
can access files in `/config`, and all persistent deb applications share one
dependency environment.

Browser links opened via the guest's PATH-resolved `xdg-open` use the host
desktop. File-manager targets should be shared paths under `/config` or `/tmp`.
Applications that directly invoke `/usr/bin/xdg-open` or use a separate desktop
portal may need their own integration. CLI-only packages may have no application
menu entry. Packages requiring a newer Ubuntu release can fail dependency
resolution even when they work in the host container.

## Native installation and startup replay

Choose **Native** in the double-click installer, or run:

```bash
agent-workspace-deb-installer --native --yes /config/Downloads/application.deb
agent-workspace-deb-native status
agent-workspace-deb-native restore --yes
agent-workspace-deb-native remove --yes package-name
```

Native mode installs normally into the container's system paths using
passwordless sudo and apt. It persists original archives in
`/config/.cache/agent-workspace/deb-native` and desired packages in
`/config/.local/share/agent-workspace/deb-native/packages.json`. Do not clear this
cache if startup restoration is needed. Application home data stays under
`/config`; system configuration, services and data outside that directory are
not backed up by saving a deb.

`agent-workspace-manager install deb-runtime` installs the persistent service
`/config/custom-services.d/deb-native-restore/run` and registers it with the
image's custom-service registrar. On startup it validates cached checksums and
metadata, then reinstalls missing, older or unconfigured managed packages.
Already installed equal/newer versions skip apt and repository requests.
Install and replay serialize through one native operation lock; apt additionally
waits for the system package lock. The restore runs asynchronously with desktop
startup, once per service start, and does not continuously retry failures.

Restore results are in `deb-native/restore-status.json` beside the manifest.
Logs are under `/config/.local/log/agent-workspace`: `deb-native.log`,
`deb-native-restore.log`, and `deb-installer-native.log`. After correcting a
network or dependency issue, retry with `restore --yes`. Failed installations
retain their desired record for retry. `remove --yes` also removes that record
so startup does not reinstall the application; the original archive is retained.

Dependencies are resolved against the current container's repositories. This
does not guarantee offline restoration or compatibility across distribution
changes. No extra Docker mount is needed beyond `/config`. Native GUI programs
run as the desktop user and have separate optional overrides in
`/config/.config/agent-workspace/deb-native-apps.json`. Managed menu entries use
the **(Native)** suffix; existing PRoot installations remain available.

## Validation

Unit tests cover desktop argument handling, file isolation rules, locks,
manifest updates, archive restore and the desktop URL bridge:

```bash
python3 -m unittest discover -s tests -p 'test_deb*.py' -v
```

The opt-in integration test builds a temporary local package, installs a real
dependency, checks maintainer scripts, upgrades and removes the package, then
backs up and restores the environment. It verifies that the host dpkg database
is unchanged. It needs network access and free disk space; generated backup
archives and the previous environment remain available after the test.

```bash
python3 tests/smoke_deb_runtime.py
```

`python3 tests/smoke_deb_native.py` exercises real native installation, launcher
export, execution, no-op replay, and recovery after removing only the test
package from the host. It cleans up the installed test package and desired
record. This simulates package loss without rebuilding the running container.
