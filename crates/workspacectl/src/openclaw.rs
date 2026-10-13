use crate::common::*;
use anyhow::{Context as _, Result, bail, ensure};
use fs2::FileExt;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

fn valid_version(version: &str) -> bool {
    regex::Regex::new(r"^\d{4}\.\d{1,2}\.\d+(?:[-+][A-Za-z0-9.-]+)?$")
        .unwrap()
        .is_match(version)
}
fn package_version(entry: &Path) -> Result<String> {
    let value: Value = serde_json::from_str(&fs::read_to_string(
        entry
            .parent()
            .context("missing package directory")?
            .join("package.json"),
    )?)?;
    let version = value["version"]
        .as_str()
        .context("package version is missing")?;
    ensure!(
        valid_version(version),
        "OpenClaw package has an invalid version"
    );
    Ok(version.into())
}
fn service_command(text: &str) -> Result<(PathBuf, Vec<String>)> {
    let commands = text
        .lines()
        .filter_map(|line| line.strip_prefix("ExecStart="))
        .collect::<Vec<_>>();
    ensure!(
        commands.len() == 1,
        "Expected exactly one Gateway ExecStart"
    );
    let mut args = shell_words::split(commands[0])?;
    // Service units use Unix paths regardless of the host running this parser.
    if args.first().is_some_and(|value| {
        value
            .rsplit('/')
            .next()
            .is_some_and(|name| name.starts_with("node"))
    }) && args.len() > 1
    {
        args.remove(0);
    }
    ensure!(
        args.len() >= 2
            && args[1] == "gateway"
            && matches!(
                args[0].rsplit('/').next(),
                Some("openclaw" | "openclaw.mjs")
            ),
        "Unsupported Gateway ExecStart; expected an absolute openclaw gateway command"
    );
    let path = PathBuf::from(&args[0]);
    ensure!(
        args[0].starts_with('/'),
        "Gateway ExecStart must use an absolute OpenClaw path"
    );
    Ok((path, args[1..].to_vec()))
}
fn managed_unit(original: &str, entry: &Path, node: &str) -> Result<String> {
    let (_, mut args) = service_command(original)?;
    args.insert(0, entry.to_string_lossy().into_owned());
    args.insert(0, node.into());
    let command = format!("ExecStart={}\n", shell_words::join(args));
    let mode = "Environment=OPENCLAW_SUPERVISOR_MODE=external\n";
    let mut lines = original
        .split_inclusive('\n')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    for line in &mut lines {
        if line.starts_with("ExecStart=") {
            *line = command.clone();
        }
    }
    if let Some(line) = lines
        .iter_mut()
        .find(|line| line.starts_with("Environment=OPENCLAW_SUPERVISOR_MODE="))
    {
        *line = mode.into();
    } else {
        let index = lines
            .iter()
            .position(|line| line.trim() == "[Service]")
            .context("Gateway unit has no [Service] section")?;
        lines.insert(index + 1, mode.into());
    }
    Ok(lines.concat())
}
async fn restart(ctx: &Context) -> Result<()> {
    let log = ctx.logs().join("openclaw-workspace-update.log");
    let mut command = ctx.command("systemctl");
    command
        .args(["--user", "restart", "openclaw-gateway.service"])
        .kill_on_drop(true);
    let mut child = detach(&mut command, &log)?;
    let status = tokio::time::timeout(Duration::from_secs(40), child.wait())
        .await
        .context("Gateway restart timed out")??;
    ensure!(
        status.success(),
        "Gateway restart failed; see {}",
        log.display()
    );
    Ok(())
}
async fn verify(ctx: &Context, entry: &Path, timeout: u64) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout);
    while tokio::time::Instant::now() < deadline {
        let show = ctx
            .probe(
                &[
                    "systemctl",
                    "--user",
                    "show",
                    "openclaw-gateway.service",
                    "--property=ActiveState,MainPID",
                ],
                5.,
            )
            .await;
        let values = show
            .stdout
            .lines()
            .filter_map(|line| line.split_once('='))
            .collect::<std::collections::BTreeMap<_, _>>();
        let pid = values
            .get("MainPID")
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        if values.get("ActiveState") == Some(&"active") && pid > 0 {
            let command = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
            if command
                .split(|value| *value == 0)
                .any(|part| part == entry.to_string_lossy().as_bytes())
                && ctx
                    .http
                    .get("http://127.0.0.1:18789/")
                    .timeout(Duration::from_secs(2))
                    .send()
                    .await
                    .is_ok_and(|response| response.status().as_u16() < 500)
            {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    bail!("Gateway did not start the requested release and answer on port 18789")
}
fn guard(ctx: &Context) -> Result<()> {
    let gateway_pid = read(
        ctx.root
            .join(".local/run/user-systemd/openclaw-gateway.pid"),
    )
    .trim()
    .parse::<u32>()
    .unwrap_or(0);
    let mut ancestor = std::process::id();
    while ancestor > 1 {
        ensure!(
            ancestor != gateway_pid,
            "Run this update from Control Center or a terminal outside the OpenClaw Gateway"
        );
        ancestor = read(format!("/proc/{ancestor}/status"))
            .lines()
            .find_map(|line| {
                line.strip_prefix("PPid:")
                    .and_then(|value| value.trim().parse::<u32>().ok())
            })
            .unwrap_or(0);
    }
    Ok(())
}
async fn switch_unit(
    unit: &Path,
    original: &[u8],
    next: &str,
    activate: impl std::future::Future<Output = Result<()>>,
    recover: impl std::future::Future<Output = Result<()>>,
) -> Result<()> {
    let permissions = mode(unit, 0o644);
    atomic_write(unit, next, permissions)?;
    if let Err(error) = activate.await {
        atomic_write(unit, original, permissions)?;
        return match recover.await {
            Ok(()) => Err(error.context("Previous Gateway restored")),
            Err(recovery) => {
                Err(error.context(format!("Automatic service recovery failed: {recovery:#}")))
            }
        };
    }
    Ok(())
}
pub async fn update(ctx: &Context) -> Result<i32> {
    let unit = ctx.units().join("openclaw-gateway.service");
    ensure!(unit.is_file(), "OpenClaw gateway service is not installed");
    guard(ctx)?;
    fs::create_dir_all(ctx.state())?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(ctx.state().join("openclaw-update.lock"))?;
    lock.try_lock_exclusive()
        .context("An OpenClaw update is already running")?;
    let original = fs::read(&unit)?;
    let text = std::str::from_utf8(&original)?;
    let (old_entry, _) = service_command(text)?;
    let node = command_path("node");
    let npm = command_path("npm");
    ensure!(
        !node.is_empty() && !npm.is_empty(),
        "Node.js and npm are required"
    );
    let version: Value = serde_json::from_str(
        &ctx.probe(&[&npm, "view", "openclaw@latest", "version", "--json"], 30.)
            .await
            .checked("npm")?,
    )?;
    let version = version
        .as_str()
        .context("npm returned a non-string version")?;
    ensure!(
        valid_version(version),
        "npm returned an invalid OpenClaw version"
    );
    let releases = ctx.root.join("opt/openclaw/releases");
    fs::create_dir_all(&releases)?;
    let release = releases.join(version);
    let entry = release.join("node_modules/openclaw/openclaw.mjs");
    if !entry.is_file() || package_version(&entry).ok().as_deref() != Some(version) {
        ensure!(
            !release.exists(),
            "Incomplete release already exists: {}",
            release.display()
        );
        let staging = tempfile::Builder::new()
            .prefix(".staging-")
            .tempdir_in(&releases)?;
        println!("Installing OpenClaw {version} into {}", release.display());
        ctx.probe(
            &[
                &npm,
                "install",
                "--prefix",
                &staging.path().to_string_lossy(),
                "--no-save",
                "--no-audit",
                "--no-fund",
                &format!("openclaw@{version}"),
            ],
            1800.,
        )
        .await
        .checked("npm")?;
        let staged = staging.path().join("node_modules/openclaw/openclaw.mjs");
        ensure!(
            staged.is_file() && package_version(&staged)? == version,
            "Installed package differs from npm latest"
        );
        ctx.probe(&[&node, &staged.to_string_lossy(), "--version"], 30.)
            .await
            .checked("OpenClaw")?;
        fs::rename(staging.path(), &release)?;
    }
    let next = managed_unit(text, &entry, &node)?;
    if next == text {
        verify(ctx, &entry, 45).await?;
        println!("OpenClaw {version} is already active");
        return Ok(0);
    }
    let backup = ctx.state().join(format!(
        "openclaw-gateway.service.{}.{}.bak",
        chrono::Utc::now().timestamp(),
        std::process::id()
    ));
    atomic_write(&backup, &original, 0o600)?;
    println!(
        "Switching Gateway; previous service definition saved at {}",
        backup.display()
    );
    switch_unit(
        &unit,
        &original,
        &next,
        async {
            restart(ctx).await?;
            verify(ctx, &entry, 45).await
        },
        async {
            restart(ctx).await?;
            verify(ctx, &old_entry, 30).await
        },
    )
    .await?;
    let alias = ctx.root.join(".local/bin/openclaw");
    fs::create_dir_all(alias.parent().unwrap())?;
    if fs::symlink_metadata(&alias).is_err()
        || (alias.is_symlink()
            && fs::canonicalize(&alias).is_ok_and(|path| path.starts_with(&releases)))
    {
        #[cfg(unix)]
        {
            let temporary = alias.with_file_name(format!(".openclaw.{}.tmp", std::process::id()));
            if temporary.is_symlink() {
                fs::remove_file(&temporary)?;
            }
            std::os::unix::fs::symlink(&entry, &temporary)?;
            fs::rename(temporary, &alias)?;
        }
    } else {
        eprintln!(
            "Gateway updated, but existing CLI alias was preserved: {}",
            alias.display()
        );
    }
    println!("OpenClaw {version} is running; Gateway and HTTP checks passed");
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn restores_unit_before_restarting_previous_gateway() {
        let temp = tempfile::tempdir().unwrap();
        let unit = temp.path().join("gateway.service");
        let original = b"[Service]\nEnvironment=TOKEN=secret\nExecStart=/old/openclaw gateway\n";
        fs::write(&unit, original).unwrap();
        let error = switch_unit(
            &unit,
            original,
            "new release",
            async {
                assert_eq!(fs::read_to_string(&unit).unwrap(), "new release");
                bail!("health check failed")
            },
            async {
                assert_eq!(fs::read(&unit).unwrap(), original);
                Ok(())
            },
        )
        .await
        .unwrap_err();
        assert!(format!("{error:#}").contains("Previous Gateway restored"));
        assert!(format!("{error:#}").contains("health check failed"));
    }
    #[tokio::test]
    async fn reports_failed_recovery_and_keeps_original_unit() {
        let temp = tempfile::tempdir().unwrap();
        let unit = temp.path().join("gateway.service");
        fs::write(&unit, "original").unwrap();
        let error = switch_unit(
            &unit,
            b"original",
            "new",
            async { bail!("new release failed") },
            async { bail!("previous restart failed") },
        )
        .await
        .unwrap_err();
        assert_eq!(fs::read_to_string(unit).unwrap(), "original");
        assert!(format!("{error:#}").contains("previous restart failed"));
        assert!(format!("{error:#}").contains("new release failed"));
    }
    #[test]
    fn rejects_shell_and_relative_units() {
        for line in [
            "/bin/bash -c 'openclaw gateway'",
            "openclaw gateway",
            "C:/bin/openclaw gateway",
            r"/tmp/other\openclaw gateway",
            "/usr/bin/node /tmp/openclaw.mjs gateway\nExecStart=/tmp/openclaw gateway",
        ] {
            assert!(service_command(&format!("[Service]\nExecStart={line}\n")).is_err());
        }
    }
    #[test]
    fn preserves_arguments_and_environment() {
        let original = "[Service]\nEnvironment=TOKEN=secret\nExecStart=/usr/bin/node /old/openclaw.mjs gateway --bind loopback --port 18789\n[Install]\nWantedBy=default.target\n";
        let next = managed_unit(original, Path::new("/new/openclaw.mjs"), "/usr/bin/node").unwrap();
        assert!(next.contains("Environment=TOKEN=secret\n"));
        assert!(next.contains("gateway --bind loopback --port 18789\n"));
        assert!(next.contains("OPENCLAW_SUPERVISOR_MODE=external"));
        assert_eq!(
            managed_unit(&next, Path::new("/new/openclaw.mjs"), "/usr/bin/node").unwrap(),
            next
        );
    }
}
