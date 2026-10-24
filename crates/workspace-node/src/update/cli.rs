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
use serde_json::{Value, json};
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
    },
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
    let root = super::runtime_root(peers.parent().unwrap_or(&peers));
    let fleet = Fleet::new(peers);
    match command {
        Command::Status { node, all } => {
            let mut out = Vec::new();
            if node.is_empty() && !all {
                out.push(local_status(&root)?);
            }
            for id in targets(&fleet, node, all).await? {
                out.push(
                    match fleet.get(&id, &format!("/api/v1/nodes/{id}/update")).await {
                        Ok(value) => value,
                        Err(error) => json!({"node": id, "error": format!("{error:#}")}),
                    },
                );
            }
            client::print(json!({"devices": out}));
        }
        Command::Apply {
            spec,
            node,
            all,
            force,
            no_wait,
        } => {
            if node.is_empty() && !all {
                return apply_local(&root, &spec, force).await;
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
        Command::Adopt => adopt(&root)?,
        Command::Launch { args } => {
            ensure!(
                Layout::new(&root).pointer()?.is_some(),
                "nothing to launch: run `cranibox update adopt` once on this device first"
            );
            let stop = tokio_util::sync::CancellationToken::new();
            let signal = stop.clone();
            // Service managers stop a service with SIGTERM; without it the launcher would
            // die and leave the node running in its own process group.
            #[cfg(unix)]
            let mut terminate =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            tokio::spawn(async move {
                #[cfg(unix)]
                tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
                #[cfg(not(unix))]
                let _ = tokio::signal::ctrl_c().await;
                signal.cancel();
            });
            // `--config` is a global option, so the runtime would silently fall back to the
            // default settings (and its single-instance lock) unless the launcher forwards it.
            let mut runtime_args: Vec<std::ffi::OsString> = vec!["--config".into(), config.into()];
            runtime_args.extend(args.into_iter().map(Into::into));
            let code = launcher::run(LauncherConfig::new(&root, runtime_args), stop).await?;
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

fn local_status(root: &Path) -> Result<Value> {
    let updater = updater(root)?;
    let runtime = updater.info()?;
    // This process is only the command line; the managed pointer names what the node runs.
    Ok(json!({
        "node": "this device",
        "version": runtime.version.clone().unwrap_or_else(super::product_version),
        "runtime": runtime,
        "status": updater.status()?,
    }))
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

async fn apply_local(root: &Path, spec: &str, force: bool) -> Result<()> {
    let updater = updater(root)?;
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
fn adopt(root: &Path) -> Result<()> {
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
    println!(
        "Adopted {version} in {}. Start the device with `cranibox update launch` from now on.",
        root.display()
    );
    Ok(())
}
