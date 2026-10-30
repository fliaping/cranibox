//! `cranibox update …`: update this device or other devices of your identity.
use super::{
    launcher::{self, LauncherConfig},
    layout::Layout,
    manager::{Updater, default_entry_package},
    npm::{self, Registries},
    version::Version,
};
use agent_workspace_runtime::client::{self, Fleet};
use anyhow::{Context, Result, bail, ensure};
use clap::Subcommand;
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Version, platform and update state of devices (this device when none is named).
    Status {
        /// Device ID; repeat for several.
        #[arg(long)]
        node: Vec<String>,
        /// Every reachable device of your identity.
        #[arg(long)]
        all: bool,
        /// Resolve latest, next and dev on each target's platform.
        #[arg(long)]
        check: bool,
    },
    /// Check the available latest, next and dev versions.
    Check {
        #[arg(long)]
        node: Vec<String>,
    },
    /// Print an autostart definition and installation instructions; write no files.
    ServiceUnit,
    /// Update to a version or a dist-tag (latest, next, dev), then restart into it.
    Apply {
        spec: String,
        #[arg(long)]
        node: Vec<String>,
        /// Every reachable other device of your identity (update this one separately, last).
        #[arg(long)]
        all: bool,
        /// Allow the same or an older version, or one that failed before.
        #[arg(long)]
        force: bool,
        /// Return once the update started instead of waiting for the new version.
        #[arg(long)]
        no_wait: bool,
    },
    /// Go back to the previous version.
    Rollback {
        #[arg(long)]
        node: Vec<String>,
    },
    /// Install this program as the first managed version (once per device).
    Adopt,
    /// Run the node from the managed versions, restarting and rolling back by itself.
    /// This is what the device's autostart entry should run.
    Launch {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

pub async fn run(peers: PathBuf, config: PathBuf, command: Command) -> Result<()> {
    let state = peers.parent().unwrap_or(&peers).to_owned();
    let root = super::runtime_root(&state);
    let fleet = Fleet::new(peers);
    match command {
        Command::Check { node } => print_status(&fleet, &root, node, false, true).await?,
        Command::ServiceUnit => println!(
            "{}",
            super::service::definition(
                std::env::consts::OS,
                &std::env::current_exe()?,
                &std::path::absolute(&config)?,
            )?
        ),
        Command::Status { node, all, check } => {
            print_status(&fleet, &root, node, all, check).await?
        }
        Command::Apply {
            spec,
            node,
            all,
            force,
            no_wait,
        } => {
            if node.is_empty() && !all {
                return apply_local(&root, &state, &spec, force).await;
            }
            let mut failed = Vec::new();
            for id in targets(&fleet, node, all).await? {
                if let Err(error) = apply_remote(&fleet, &id, &spec, force, no_wait).await {
                    eprintln!("{id}: {error:#}");
                    failed.push(id);
                }
            }
            ensure!(failed.is_empty(), "update failed on: {}", failed.join(", "));
        }
        Command::Rollback { node } => {
            if node.is_empty() {
                let status = updater(&root)?.rollback()?;
                request_local_restart(&root, status.to.as_deref())?;
                client::print(json!(status));
            }
            for id in node {
                let status = fleet
                    .post(
                        &id,
                        &format!("/api/v1/nodes/{id}/update/rollback"),
                        json!({}),
                    )
                    .await?;
                client::print(status);
            }
        }
        Command::Adopt => {
            let version = adopt(&root)?;
            println!(
                "Adopted {version} in {}. Start the device with cranibox update launch from now on.",
                root.display()
            );
        }
        Command::Launch { args } => {
            ensure!(
                Layout::new(&root).pointer()?.is_some(),
                "nothing to launch: run `cranibox update adopt` once on this device first"
            );
            // `--config` is a global option, so the runtime would silently fall back to the
            // default settings (and its single-instance lock) unless the launcher forwards it.
            let mut runtime_args: Vec<std::ffi::OsString> = vec!["--config".into(), config.into()];
            runtime_args.extend(args.into_iter().map(Into::into));
            super::start_terminal_host(&state).await;
            let code = launch_foreground(&root, runtime_args).await?;
            std::process::exit(code);
        }
    }
    Ok(())
}

fn updater(root: &Path) -> Result<Updater> {
    Ok(Updater::new(
        root.to_owned(),
        Registries::from_env()?,
        default_entry_package(),
    ))
}

async fn print_status(
    fleet: &Fleet,
    root: &Path,
    node: Vec<String>,
    all: bool,
    check: bool,
) -> Result<()> {
    let mut out = Vec::new();
    if node.is_empty() && !all {
        let updater = updater(root)?;
        let version = updater
            .info()?
            .version
            .unwrap_or_else(super::product_version);
        out.push(updater.snapshot("this device", version, check).await?);
    }
    for id in targets(fleet, node, all).await? {
        let path = format!("/api/v1/nodes/{id}/update?check={}", u8::from(check));
        out.push(match fleet.get(&id, &path).await {
            Ok(value) => value,
            Err(error) => json!({"node": id, "error": format!("{error:#}")}),
        });
    }
    client::print(json!({"devices": out}));
    Ok(())
}

/// The named devices, or every reachable other device for `--all`.
async fn targets(fleet: &Fleet, named: Vec<String>, all: bool) -> Result<Vec<String>> {
    if !all {
        return Ok(named);
    }
    let nodes = fleet.nodes().await?;
    Ok(nodes["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|n| n["online"] == true && n["this"] != true)
        .filter_map(|n| n["id"].as_str().map(str::to_owned))
        .chain(named)
        .collect())
}

async fn apply_local(root: &Path, state: &Path, spec: &str, force: bool) -> Result<()> {
    let updater = updater(root)?.with_terminal_state(state.to_owned());
    let status = updater.apply(spec, force).await?;
    let to = status.to.clone();
    client::print(json!(status));
    request_local_restart(root, to.as_deref())?;
    eprintln!("Activated. A running node restarts into it within a few seconds.");
    Ok(())
}

/// A running node in another process watches for this file (see `watch_restart_file`).
fn request_local_restart(root: &Path, version: Option<&str>) -> Result<()> {
    std::fs::write(root.join("restart.request"), version.unwrap_or("")).context("request restart")
}

async fn apply_remote(
    fleet: &Fleet,
    node: &str,
    spec: &str,
    force: bool,
    no_wait: bool,
) -> Result<()> {
    let started = fleet
        .post(
            node,
            &format!("/api/v1/nodes/{node}/update"),
            json!({"spec": spec, "force": force}),
        )
        .await?;
    let id = started["id"]
        .as_str()
        .context("no update id returned")?
        .to_owned();
    eprintln!("{node}: update started ({id})");
    if no_wait {
        client::print(started);
        return Ok(());
    }
    let deadline = Instant::now() + Duration::from_secs(600);
    let (mut shown, mut target): (String, Option<String>) = (String::new(), None);
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_secs(2)).await;
        match fleet
            .get(node, &format!("/api/v1/nodes/{node}/update/{id}"))
            .await
        {
            Ok(status) => {
                let phase = status["phase"].as_str().unwrap_or("").to_owned();
                target = status["to"].as_str().map(str::to_owned).or(target);
                if phase != shown {
                    eprintln!("{node}: {phase}");
                    shown = phase.clone();
                }
                match phase.as_str() {
                    "done" => {
                        client::print(status);
                        return Ok(());
                    }
                    "failed" | "rolled_back" => bail!(
                        "{}",
                        status["error"]
                            .as_str()
                            .unwrap_or("update did not complete")
                    ),
                    _ => {}
                }
            }
            // The connection drops while the node restarts into the new version.
            Err(_) => {
                if let Ok(info) = fleet
                    .get(node, &format!("/api/v1/nodes/{node}/update"))
                    .await
                    && target.is_some()
                    && info["version"].as_str() == target.as_deref()
                    && info["runtime"]["state"] == "confirmed"
                {
                    client::print(info);
                    return Ok(());
                }
            }
        }
    }
    bail!("timed out waiting for {node} to finish updating")
}

/// Make the running program the first managed version: stage a copy, activate, and mark it
/// confirmed because it is demonstrably working.
pub fn adopt(root: &Path) -> Result<String> {
    let layout = Layout::new(root);
    ensure!(
        layout.pointer()?.is_none(),
        "this device already has managed versions in {}",
        root.display()
    );
    let version = Version::parse(&super::product_version())
        .context("this program's version is not a release version")?;
    install(&layout, &version, &std::env::current_exe()?)?;
    layout.activate_forced(&version)?;
    layout.confirm(&version)?;
    Ok(version.to_string())
}

/// Copy a program into the managed versions as `version`.
fn install(layout: &Layout, version: &Version, exe: &Path) -> Result<()> {
    let staging = layout.begin_stage(version)?;
    let target = staging.join(npm::executable_relative_path());
    let copy = std::fs::create_dir_all(target.parent().context("no parent")?)
        .and_then(|()| std::fs::copy(exe, &target).map(drop));
    if let Err(error) = copy {
        layout.abort_stage(version);
        return Err(error).context("copy the program into the managed versions");
    }
    layout.commit_stage(version)?;
    Ok(())
}

/// What starting this program did to the managed runtimes.
#[derive(Debug, PartialEq, Eq)]
pub enum Startup {
    /// The first managed version: this program.
    Adopted(String),
    /// This program is newer than the managed runtime and runs next, pending confirmation.
    Activated(String),
    /// The managed runtime stays: it is this version or newer, or this version was
    /// installed here before (and failed, or the owner went back from it).
    Kept,
}

/// On every start: a newly installed app or package replaces an older managed runtime,
/// instead of the device running the version it had before. Never downgrades, and a version
/// installed here before is left alone: it failed, or the owner rolled back from it. The new
/// version starts pending: if it does not confirm, the launcher rolls back to the one it replaced.
pub fn adopt_or_upgrade(root: &Path) -> Result<Startup> {
    let version = Version::parse(&super::product_version())
        .context("this program's version is not a release version")?;
    upgrade_to(root, &version, &std::env::current_exe()?)
}

fn upgrade_to(root: &Path, version: &Version, exe: &Path) -> Result<Startup> {
    let layout = Layout::new(root);
    let Some(pointer) = layout.pointer()? else {
        install(&layout, version, exe)?;
        layout.activate_forced(version)?;
        layout.confirm(version)?;
        return Ok(Startup::Adopted(version.to_string()));
    };
    let current = Version::parse(&pointer.current).context("current runtime version")?;
    let name = version.to_string();
    if *version <= current || pointer.failed.contains(&name) || layout.version_dir(version).exists()
    {
        return Ok(Startup::Kept);
    }
    install(&layout, version, exe)?;
    layout.activate(version)?;
    Ok(Startup::Activated(name))
}

/// Run the managed runtime in the foreground until a stop signal; returns the exit code.
async fn launch_foreground(root: &Path, runtime_args: Vec<std::ffi::OsString>) -> Result<i32> {
    let stop = tokio_util::sync::CancellationToken::new();
    let signal = stop.clone();
    // Service managers stop a service with SIGTERM; without it the launcher would
    // die and leave the node running in its own process group.
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::spawn(async move {
        #[cfg(unix)]
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
        #[cfg(not(unix))]
        let _ = tokio::signal::ctrl_c().await;
        signal.cancel();
    });
    launcher::run(LauncherConfig::new(root, runtime_args), stop).await
}

/// Remote updates are on by default: a plain `cranibox` or `cranibox serve` makes itself the
/// first managed version on first start and from then on only supervises the managed runtime
/// (same process, same service-manager unit). `Ok(None)` means run unmanaged as before:
/// another command, an opt-out (`CRANIBOX_NO_MANAGED`), or a runtime that cannot be adopted.
pub async fn auto_launch(args: &[String]) -> Result<Option<i32>> {
    if std::env::var_os("CRANIBOX_RUNTIME_MANAGED").is_some()
        || std::env::var_os("CRANIBOX_NO_MANAGED").is_some()
    {
        return Ok(None);
    }
    let mut config = None;
    let mut positional = Vec::new();
    let mut rest = args.iter().skip(1);
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--config" => config = rest.next().map(PathBuf::from),
            "-h" | "--help" | "-V" | "--version" => return Ok(None),
            _ if arg.starts_with("--config=") => config = Some(PathBuf::from(&arg[9..])),
            _ if arg.starts_with('-') => {}
            _ => positional.push(arg.as_str()),
        }
    }
    let service = matches!(
        positional.as_slice(),
        [] | ["serve", ..] | ["peer", "serve"]
    );
    if !service {
        return Ok(None);
    }
    let config = config.unwrap_or_else(crate::settings::default_path);
    let peers = crate::peer::config::path_for(&config);
    let root = super::runtime_root(peers.parent().unwrap_or(&peers));
    match adopt_or_upgrade(&root) {
        Ok(Startup::Activated(version)) => eprintln!("Starting the newly installed {version}"),
        Ok(_) => {}
        // A device already managed keeps its runtime; otherwise run unmanaged as before.
        Err(error) if Layout::new(&root).pointer()?.is_some() => {
            eprintln!("Keeping the managed runtime: {error:#}")
        }
        Err(error) => {
            eprintln!("Remote updates are not available on this device: {error:#}");
            return Ok(None);
        }
    }
    let runtime_args = args.iter().skip(1).map(Into::into).collect();
    super::start_terminal_host(peers.parent().unwrap_or(&peers)).await;
    Ok(Some(launch_foreground(&root, runtime_args).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::layout::PointerState;

    fn version(v: &str) -> Version {
        Version::parse(v).unwrap()
    }
    fn program(dir: &Path, body: &str) -> PathBuf {
        let exe = dir.join(format!("program-{body}"));
        std::fs::write(&exe, body).unwrap();
        exe
    }
    fn installed(root: &Path, v: &str) -> String {
        std::fs::read_to_string(
            Layout::new(root)
                .version_dir(&version(v))
                .join(npm::executable_relative_path()),
        )
        .unwrap()
    }

    #[test]
    fn first_start_adopts_this_program() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("runtime");
        let exe = program(dir.path(), "302");
        let outcome = upgrade_to(&root, &version("1.1.1-dev.302"), &exe).unwrap();
        assert_eq!(outcome, Startup::Adopted("1.1.1-dev.302".into()));
        let pointer = Layout::new(&root).pointer().unwrap().unwrap();
        assert_eq!(pointer.current, "1.1.1-dev.302");
        assert_eq!(pointer.state, PointerState::Confirmed);
        assert_eq!(installed(&root, "1.1.1-dev.302"), "302");
    }

    #[test]
    fn a_newer_installed_app_replaces_the_older_managed_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("runtime");
        upgrade_to(
            &root,
            &version("1.1.1-dev.292"),
            &program(dir.path(), "292"),
        )
        .unwrap();
        let outcome = upgrade_to(
            &root,
            &version("1.1.1-dev.302"),
            &program(dir.path(), "302"),
        )
        .unwrap();
        assert_eq!(outcome, Startup::Activated("1.1.1-dev.302".into()));
        let layout = Layout::new(&root);
        let pointer = layout.pointer().unwrap().unwrap();
        assert_eq!(pointer.current, "1.1.1-dev.302");
        assert_eq!(pointer.previous.as_deref(), Some("1.1.1-dev.292"));
        // Pending: the launcher confirms it or rolls back to the runtime it replaced.
        assert_eq!(pointer.state, PointerState::Pending);
        assert_eq!(installed(&root, "1.1.1-dev.302"), "302");
        assert_eq!(installed(&root, "1.1.1-dev.292"), "292");
        let rolled = layout.rollback("did not confirm").unwrap();
        assert_eq!(rolled.current, "1.1.1-dev.292");
        // The failed version is not tried again on the next start.
        let outcome = upgrade_to(
            &root,
            &version("1.1.1-dev.302"),
            &program(dir.path(), "302"),
        )
        .unwrap();
        assert_eq!(outcome, Startup::Kept);
        assert_eq!(layout.pointer().unwrap().unwrap().current, "1.1.1-dev.292");
    }

    #[test]
    fn an_older_or_equal_app_never_downgrades() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("runtime");
        upgrade_to(
            &root,
            &version("1.1.1-dev.302"),
            &program(dir.path(), "302"),
        )
        .unwrap();
        for older in ["1.1.1-dev.302", "1.1.1-dev.292", "1.1.0"] {
            let outcome = upgrade_to(&root, &version(older), &program(dir.path(), "old")).unwrap();
            assert_eq!(outcome, Startup::Kept, "{older}");
        }
        let layout = Layout::new(&root);
        assert_eq!(layout.pointer().unwrap().unwrap().current, "1.1.1-dev.302");
        assert_eq!(layout.versions().unwrap(), [version("1.1.1-dev.302")]);
    }

    #[test]
    fn an_owner_rollback_is_respected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("runtime");
        upgrade_to(
            &root,
            &version("1.1.1-dev.292"),
            &program(dir.path(), "292"),
        )
        .unwrap();
        upgrade_to(
            &root,
            &version("1.1.1-dev.302"),
            &program(dir.path(), "302"),
        )
        .unwrap();
        let layout = Layout::new(&root);
        layout.revert().unwrap();
        let outcome = upgrade_to(
            &root,
            &version("1.1.1-dev.302"),
            &program(dir.path(), "302"),
        )
        .unwrap();
        assert_eq!(outcome, Startup::Kept);
        assert_eq!(layout.pointer().unwrap().unwrap().current, "1.1.1-dev.292");
    }
}
