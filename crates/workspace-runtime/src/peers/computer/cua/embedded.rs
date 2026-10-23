//! cua-driver embedded in the Cranibox macOS app. The app starts the driver itself
//! (`CUA_DRIVER_EMBEDDED=1`), so the driver is in the app's TCC responsibility chain:
//! one Screen Recording and Accessibility grant, to Cranibox, and no separate
//! CuaDriver.app. The binary is downloaded on demand (pinned version and SHA-256)
//! into the node's directory instead of growing the installer by ~50 MB.
use anyhow::{Context, Result, ensure};
use sha2::Digest;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Mutex, OnceLock},
    time::Duration,
};

pub const VERSION: &str = "0.37.0";
const ARCHIVE: &str = "cua-driver-rs-0.37.0-darwin-universal-binary.tar.gz";
const ARCHIVE_SHA256: &str = "7d491f67f54be5f3b380ffd85c64b242eaefd05dad6df78012cde227c95aee2a";
const ARCHIVE_MAX: u64 = 128 * 1024 * 1024;
/// The bundle identifier the driver reports as its host (tauri.conf.json).
const HOST_BUNDLE_ID: &str = "com.cranibox.node";

static HOME: OnceLock<PathBuf> = OnceLock::new();
static DAEMON: Mutex<Option<std::process::Child>> = Mutex::new(None);

pub fn url() -> String {
    format!("https://github.com/trycua/cua/releases/download/cua-driver-rs-v{VERSION}/{ARCHIVE}")
}
/// Where this node keeps the driver (`<node dir>/cua-driver`). Set once at start.
pub fn configure(node_dir: &Path) {
    let _ = HOME.set(node_dir.join("cua-driver"));
}
fn home() -> Option<&'static Path> {
    HOME.get().map(PathBuf::as_path)
}
/// Embedded mode applies only to the macOS app: the host must be an identifiable
/// application for macOS to attribute the driver to it.
pub fn active() -> bool {
    cfg!(target_os = "macos")
        && home().is_some()
        && std::env::current_exe()
            .is_ok_and(|exe| exe.to_string_lossy().contains(".app/Contents/MacOS/"))
}
pub fn binary() -> Option<PathBuf> {
    home().map(|h| h.join(super::BINARY))
}
fn socket() -> Option<PathBuf> {
    home().map(|h| h.join("cua.sock"))
}
/// Arguments for the MCP session that talks to this app's own daemon.
pub fn mcp_args() -> Option<Vec<String>> {
    Some(vec![
        "mcp".into(),
        "--embedded".into(),
        "--socket".into(),
        socket()?.to_string_lossy().into_owned(),
        "--host-bundle-id".into(),
        HOST_BUNDLE_ID.into(),
    ])
}
/// The steps `install` performs, shown for review.
pub fn plan() -> Vec<String> {
    vec![
        format!("Download {} (cua-driver {VERSION}, MIT)", url()),
        format!("Verify SHA-256 {ARCHIVE_SHA256}"),
        format!(
            "Extract cua-driver into {}",
            home().map_or_else(String::new, |h| h.display().to_string())
        ),
    ]
}
/// Download, verify and unpack the pinned driver. No installer script runs.
pub async fn install() -> Result<()> {
    let home = home().context("the node directory is not configured")?;
    std::fs::create_dir_all(home)?;
    let archive = home.join("download.tar.gz");
    let mut response = reqwest::Client::builder()
        .timeout(Duration::from_secs(600))
        .build()?
        .get(url())
        .send()
        .await?
        .error_for_status()?;
    ensure!(
        response.content_length().is_none_or(|n| n <= ARCHIVE_MAX),
        "The cua-driver download is unexpectedly large."
    );
    let mut hasher = sha2::Sha256::new();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            (bytes.len() + chunk.len()) as u64 <= ARCHIVE_MAX,
            "The cua-driver download is unexpectedly large."
        );
        hasher.update(&chunk);
        bytes.extend_from_slice(&chunk);
    }
    let digest: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    ensure!(
        digest == ARCHIVE_SHA256,
        "The cua-driver download does not match its published checksum."
    );
    std::fs::write(&archive, &bytes)?;
    let unpacked = home.join("unpacked");
    let _ = std::fs::remove_dir_all(&unpacked);
    std::fs::create_dir_all(&unpacked)?;
    let status = tokio::process::Command::new("/usr/bin/tar")
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(&unpacked)
        .arg(super::BINARY)
        .stdin(Stdio::null())
        .status()
        .await?;
    let _ = std::fs::remove_file(&archive);
    ensure!(status.success(), "Could not unpack cua-driver.");
    stop_daemon();
    let target = binary().context("the node directory is not configured")?;
    std::fs::rename(unpacked.join(super::BINARY), &target)?;
    let _ = std::fs::remove_dir_all(&unpacked);
    ensure!(super::executable(&target), "cua-driver is not executable.");
    Ok(())
}
/// The app's own daemon answers on its socket.
pub fn daemon_running() -> bool {
    let mut daemon = DAEMON.lock().unwrap();
    if let Some(child) = daemon.as_mut()
        && matches!(child.try_wait(), Ok(None))
    {
        return socket().is_some_and(|s| s.exists());
    }
    *daemon = None;
    false
}
/// Start the daemon as a direct child of this app, never through `open` or a shell.
pub async fn start_daemon(program: &Path) -> Result<()> {
    if daemon_running() {
        return Ok(());
    }
    let socket = socket().context("the node directory is not configured")?;
    // A daemon from an earlier run of the app (not our child any more) owns the
    // socket: ask it to quit before replacing it.
    if socket.exists() {
        let socket = socket.to_string_lossy();
        let _ = super::cli(
            program,
            &["stop", "--socket", &socket],
            Duration::from_secs(5),
        )
        .await;
    }
    let _ = std::fs::remove_file(&socket);
    let child = std::process::Command::new(program)
        .args(["serve", "--socket"])
        .arg(&socket)
        .env("CUA_DRIVER_EMBEDDED", "1")
        .env("CUA_DRIVER_HOST_BUNDLE_ID", HOST_BUNDLE_ID)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("start the embedded cua-driver")?;
    *DAEMON.lock().unwrap() = Some(child);
    for _ in 0..40 {
        if socket.exists() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    stop_daemon();
    anyhow::bail!("The embedded cua-driver did not start.")
}
pub fn stop_daemon() {
    if let Some(mut child) = DAEMON.lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// This app's own grants: the driver inherits them.
pub fn permissions() -> (Option<bool>, Option<bool>) {
    tcc::grants()
}
/// Ask macOS for both grants on behalf of this app.
pub fn request_permissions() {
    tcc::request();
}

#[cfg(target_os = "macos")]
mod tcc {
    use std::ffi::c_void;
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
        static kAXTrustedCheckOptionPrompt: *const c_void;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFBooleanTrue: *const c_void;
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
        fn CFDictionaryCreate(
            allocator: *const c_void,
            keys: *const *const c_void,
            values: *const *const c_void,
            count: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> *const c_void;
        fn CFRelease(object: *const c_void);
    }
    /// `(screen_recording, accessibility)`.
    pub fn grants() -> (Option<bool>, Option<bool>) {
        // SAFETY: argument-free TCC queries.
        unsafe {
            (
                Some(CGPreflightScreenCaptureAccess()),
                Some(AXIsProcessTrusted()),
            )
        }
    }
    pub fn request() {
        // SAFETY: a one-entry CFDictionary of CF constants, released after use. Both
        // calls only raise the system prompt (or open the matching settings pane).
        unsafe {
            let keys = [kAXTrustedCheckOptionPrompt];
            let values = [kCFBooleanTrue];
            let options = CFDictionaryCreate(
                std::ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                &raw const kCFTypeDictionaryKeyCallBacks,
                &raw const kCFTypeDictionaryValueCallBacks,
            );
            if !options.is_null() {
                AXIsProcessTrustedWithOptions(options);
                CFRelease(options);
            }
            CGRequestScreenCaptureAccess();
        }
    }
}
#[cfg(not(target_os = "macos"))]
mod tcc {
    pub fn grants() -> (Option<bool>, Option<bool>) {
        (None, None)
    }
    pub fn request() {}
}
