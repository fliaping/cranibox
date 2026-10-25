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
                let status = updater(&root)?.rollback("requested on this device")?;
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
    let exe = std::env::current_exe()?;
    let staging = layout.begin_stage(&version)?;
    let target = staging.join(npm::executable_relative_path());
    let copy = std::fs::create_dir_all(target.parent().context("no parent")?)
        .and_then(|()| std::fs::copy(&exe, &target).map(drop));
    if let Err(error) = copy {
        layout.abort_stage(&version);
        return Err(error).context("copy the program into the managed versions");
    }
    layout.commit_stage(&version)?;
    layout.activate_forced(&version)?;
    layout.confirm(&version)?;
    Ok(version.to_string())
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
    if Layout::new(&root).pointer()?.is_none()
        && let Err(error) = adopt(&root)
    {
        eprintln!("Remote updates are not available on this device: {error:#}");
        return Ok(None);
    }
    let runtime_args = args.iter().skip(1).map(Into::into).collect();
    super::start_terminal_host(peers.parent().unwrap_or(&peers)).await;
    Ok(Some(launch_foreground(&root, runtime_args).await?))
}
