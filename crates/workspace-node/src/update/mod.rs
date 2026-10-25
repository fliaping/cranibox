//! Remote self-update: npm download, versioned runtime directories and rollback.
pub(crate) mod app;
pub mod cli;
pub mod duo;
pub mod launcher;
pub mod layout;
pub mod manager;
pub mod npm;
pub mod platform;
pub mod service;
pub mod version;

use std::{
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

static RESTART: AtomicBool = AtomicBool::new(false);
static HOOK: Mutex<Option<CancellationToken>> = Mutex::new(None);
static VERSION: OnceLock<String> = OnceLock::new();

/// Record the product version once at startup (the node library has its own crate version).
pub fn set_product_version(version: &str) {
    let _ = VERSION.set(version.to_owned());
}

/// Version of the program answering a status request.
pub fn product_version() -> String {
    // A managed runtime is named by the directory it was installed under.
    std::env::var("CRANIBOX_RUNTIME_VERSION")
        .ok()
        .or_else(|| VERSION.get().cloned())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned())
}

/// Runtime root: set by the launcher, else next to the node's configuration.
pub fn runtime_root(config_dir: &Path) -> PathBuf {
    std::env::var_os("CRANIBOX_RUNTIME_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| config_dir.join("runtime"))
}

const RESTART_GRACE: Duration = Duration::from_secs(10);

/// A host that can restart through the launcher registers its shutdown token here.
pub fn install_restart_hook(stop: CancellationToken) {
    *HOOK.lock().unwrap() = Some(stop);
}

/// Ask this process to stop so the launcher starts the activated version. False when the
/// host registered no hook (an embedded node, or a node not started by the launcher).
pub fn request_restart() -> bool {
    let hook = HOOK.lock().unwrap().clone();
    match hook {
        Some(stop) if std::env::var_os("CRANIBOX_RUNTIME_MANAGED").is_some() => {
            RESTART.store(true, Ordering::SeqCst);
            stop.cancel();
            // A graceful stop waits for open connections (a window's event streams, terminal
            // sockets) and can hang for good; the launcher is waiting to start the new version.
            std::thread::spawn(|| {
                std::thread::sleep(RESTART_GRACE);
                eprintln!("update: shutdown did not finish in {RESTART_GRACE:?}; exiting for the restart");
                std::process::exit(launcher::RESTART_EXIT_CODE);
            });
            true
        }
        _ => false,
    }
}

pub fn restart_requested() -> bool {
    RESTART.load(Ordering::SeqCst)
}

/// Let a successful HTTP response reach its caller before disconnecting the node.
pub async fn restart_after_response() {
    tokio::time::sleep(Duration::from_secs(2)).await;
    if !request_restart() {
        eprintln!("update: this process cannot restart itself");
    }
}

pub fn valid_spec(spec: &str) -> bool {
    matches!(spec, "latest" | "next" | "dev") || npm::plain_semver(spec)
}

/// The local `cranibox update` command runs in another process: it drops `restart.request`
/// in the runtime root and the node picks it up here.
pub fn watch_restart_file(root: PathBuf, stop: CancellationToken) {
    tokio::spawn(async move {
        let file = root.join("restart.request");
        loop {
            tokio::select! {
                _ = stop.cancelled() => return,
                _ = tokio::time::sleep(Duration::from_secs(2)) => {}
            }
            // The file names the version to run: a stale request for the version that is
            // already running (written while no node was up) must not restart it.
            if let Ok(wanted) = std::fs::read_to_string(&file)
                && std::fs::remove_file(&file).is_ok()
                && wanted.trim() != std::env::var("CRANIBOX_RUNTIME_VERSION").unwrap_or_default()
            {
                request_restart();
            }
        }
    });
}

/// Mark the running version healthy once it has stayed up for `after`.
pub fn confirm_when_stable(root: PathBuf, after: Duration, stop: CancellationToken) {
    tokio::spawn(async move {
        tokio::select! {
            _ = stop.cancelled() => return,
            _ = tokio::time::sleep(after) => {}
        }
        let updater = manager::Updater::new(
            root,
            match npm::Registries::from_env() {
                Ok(registries) => registries,
                Err(_) => return,
            },
            manager::default_entry_package(),
        );
        if let Err(error) = updater.confirm_healthy() {
            eprintln!("update: confirm healthy: {error:#}");
        }
    });
}
