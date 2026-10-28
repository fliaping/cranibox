//! The desktop app as a second update package. Remote updates replace only the runtime; an
//! update started from the app's own window also replaces the app, then the app restarts.
use super::{npm, npm::Registries, version::Version};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Native operations this shell offers the runtime: 1 adds `request_permissions` and
/// `install_app`. Shells that predate the declaration set no level and count as 0.
pub const SHELL_API: u32 = 1;

#[cfg(target_os = "macos")]
const APP_FILE: &str = "app/Cranibox.app";
#[cfg(windows)]
const APP_FILE: &str = "app/cranibox-setup.exe";
#[cfg(not(any(target_os = "macos", windows)))]
const APP_FILE: &str = "app/Cranibox.AppImage";

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ShellInfo {
    pub version: Option<String>,
    pub api: u32,
    pub outdated: bool,
    pub can_install: bool,
    pub reason: Option<String>,
}

/// The desktop shell this runtime runs under; None for a headless node.
pub fn shell_info() -> Option<ShellInfo> {
    std::env::var_os("CRANIBOX_SHELL_URL")?;
    let version = std::env::var("CRANIBOX_SHELL_VERSION").ok();
    let api = std::env::var("CRANIBOX_SHELL_API")
        .ok()
        .and_then(|api| api.parse().ok())
        .unwrap_or(0);
    Some(describe(
        version,
        api,
        &super::product_version(),
        install_support(),
    ))
}

fn describe(
    version: Option<String>,
    api: u32,
    runtime: &str,
    support: Result<(), String>,
) -> ShellInfo {
    let outdated = api < SHELL_API || version.as_deref() != Some(runtime);
    let support = if api == 0 {
        Err("the installed desktop app is too old to update itself; install the new version once by hand".into())
    } else {
        support
    };
    ShellInfo {
        version,
        api,
        outdated,
        can_install: support.is_ok(),
        reason: support.err(),
    }
}

/// Whether this installation can replace itself.
pub fn install_support() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    if cfg!(target_os = "macos") {
        let bundle = bundle_of(&exe).ok_or("the app is not running from Cranibox.app")?;
        let parent = bundle.parent().ok_or("the app bundle has no folder")?;
        tempfile::tempfile_in(parent)
            .map(drop)
            .map_err(|_| format!("{} is not writable", parent.display()))
    } else if cfg!(windows) {
        let roots = ["ProgramFiles", "ProgramFiles(x86)"]
            .into_iter()
            .filter_map(std::env::var_os)
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        if under_any(&exe, &roots) {
            Err("installed for all users; run the new installer by hand".into())
        } else {
            Ok(())
        }
    } else if cfg!(target_os = "linux") {
        match std::env::var_os("APPIMAGE") {
            Some(_) => Ok(()),
            None => Err("installed from a package; update it with the package manager".into()),
        }
    } else {
        Err("unsupported platform".into())
    }
}

/// The `*.app` bundle that holds `exe`.
fn bundle_of(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .find(|path| {
            path.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("app"))
        })
        .map(Path::to_path_buf)
}

/// Case-insensitive path prefix test (Windows paths).
fn under_any(path: &Path, roots: &[PathBuf]) -> bool {
    let path = path.to_string_lossy().to_lowercase().replace('/', "\\");
    roots.iter().any(|root| {
        let root = root.to_string_lossy().to_lowercase().replace('/', "\\");
        let root = root.trim_end_matches('\\');
        !root.is_empty() && path.starts_with(&format!("{root}\\"))
    })
}

pub fn staging_dir(root: &Path, version: &Version) -> PathBuf {
    root.join("app-staging").join(version.to_string())
}

/// Download and verify the desktop package of exactly `version`, replacing any earlier staging.
pub async fn stage(
    registries: &Registries,
    entry: &str,
    version: &Version,
    root: &Path,
    progress: impl FnMut(u64, Option<u64>),
) -> Result<PathBuf> {
    let package = npm::desktop_package(entry)?;
    let release = registries.resolve(&package, &version.to_string()).await?;
    ensure!(
        release.version == version.to_string(),
        "{package} resolved to {} instead of {version}",
        release.version
    );
    fs::create_dir_all(root)?;
    let download = tempfile::Builder::new()
        .prefix(".update-download-")
        .tempdir_in(root)?;
    let tarball = download.path().join("app.tgz");
    registries
        .download(&release, &tarball, super::manager::MAX_DOWNLOAD, progress)
        .await?;
    let extracted = download.path().join("app");
    npm::extract(
        &tarball,
        &extracted,
        super::manager::MAX_FILES,
        super::manager::MAX_EXTRACTED,
    )?;
    ensure!(
        extracted.join(APP_FILE).exists(),
        "{package}@{version} has no {APP_FILE}"
    );
    let parent = root.join("app-staging");
    if parent.exists() {
        for entry in fs::read_dir(&parent)? {
            fs::remove_dir_all(entry?.path())?;
        }
    }
    fs::create_dir_all(&parent)?;
    let dest = staging_dir(root, version);
    fs::rename(&extracted, &dest).context("publish the staged desktop app")?;
    Ok(dest)
}

pub enum Relaunch {
    /// Start the replaced program the usual way.
    Restart,
    /// Exit, then let this detached command install and start the new version.
    AfterExit(Command),
}

/// Replace this installation with the staged `version` (the shell calls this; it never takes
/// a path from the runtime).
pub fn install(root: &Path, version: &Version) -> Result<Relaunch> {
    let staging = staging_dir(root, version);
    let staged = staging.join(APP_FILE);
    ensure!(staged.exists(), "desktop app {version} is not staged");
    support()?;
    let relaunch = install_staged(&staged)?;
    if matches!(relaunch, Relaunch::Restart) {
        let _ = fs::remove_dir_all(&staging);
    }
    Ok(relaunch)
}

fn support() -> Result<()> {
    install_support().map_err(|reason| anyhow::anyhow!("{reason}"))
}

#[cfg(target_os = "macos")]
fn install_staged(staged: &Path) -> Result<Relaunch> {
    let bundle = bundle_of(&std::env::current_exe()?).context("not running from an app bundle")?;
    verify_bundle(staged, &bundle)?;
    let incoming = sibling(&bundle, "incoming")?;
    remove(&incoming)?;
    run(Command::new("/usr/bin/ditto").arg(staged).arg(&incoming))?;
    swap(&incoming, &bundle)?;
    Ok(Relaunch::Restart)
}

#[cfg(target_os = "macos")]
fn verify_bundle(staged: &Path, running: &Path) -> Result<()> {
    run(Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(staged))
    .context("the new desktop app's signature is invalid")?;
    let details = Command::new("/usr/bin/codesign")
        .arg("-dv")
        .arg(staged)
        .output()?;
    ensure!(
        String::from_utf8_lossy(&details.stderr)
            .lines()
            .any(|line| line == "Identifier=com.cranibox.node"),
        "the new desktop app is not com.cranibox.node"
    );
    let current = Command::new("/usr/bin/codesign")
        .args(["-d", "-r-"])
        .arg(running)
        .output()?;
    if let Some(requirement) = designated(&String::from_utf8_lossy(&current.stdout)) {
        run(Command::new("/usr/bin/codesign")
            .arg("--verify")
            .arg(format!("-R={requirement}"))
            .arg(staged))
        .context("the new desktop app is signed by another identity")?;
    }
    Ok(())
}

/// The running app's designated requirement, unless it is ad-hoc (pinned to one build).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn designated(output: &str) -> Option<&str> {
    output
        .lines()
        .find_map(|line| line.trim_start_matches("# ").strip_prefix("designated => "))
        .filter(|requirement| !requirement.contains("cdhash"))
}

#[cfg(windows)]
fn install_staged(staged: &Path) -> Result<Relaunch> {
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "Wait-Process -Id $env:CRANIBOX_UPDATE_PID -ErrorAction SilentlyContinue; \
             Start-Process -Wait -FilePath $env:CRANIBOX_UPDATE_SETUP -ArgumentList '/S'; \
             Start-Process -FilePath $env:CRANIBOX_UPDATE_APP",
        ])
        .env("CRANIBOX_UPDATE_PID", std::process::id().to_string())
        .env("CRANIBOX_UPDATE_SETUP", staged)
        .env("CRANIBOX_UPDATE_APP", std::env::current_exe()?);
    Ok(Relaunch::AfterExit(command))
}

#[cfg(not(any(target_os = "macos", windows)))]
fn install_staged(staged: &Path) -> Result<Relaunch> {
    let target = PathBuf::from(std::env::var_os("APPIMAGE").context("not running as an AppImage")?);
    let incoming = sibling(&target, "incoming")?;
    remove(&incoming)?;
    fs::copy(staged, &incoming)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&incoming, fs::Permissions::from_mode(0o755))?;
    }
    fs::rename(&incoming, &target).context("replace the AppImage")?;
    Ok(Relaunch::Restart)
}

/// Start the installer command so it outlives this process.
pub fn spawn_detached(mut command: Command) -> Result<()> {
    #[cfg(windows)]
    let child = {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::{
            CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, DETACHED_PROCESS,
        };
        let flags = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW;
        command.creation_flags(flags | CREATE_BREAKAWAY_FROM_JOB);
        command.spawn().or_else(|_| {
            command.creation_flags(flags);
            command.spawn()
        })
    };
    #[cfg(not(windows))]
    let child = command.spawn();
    child.context("start the desktop app installer")?;
    Ok(())
}

/// `.<name>.<suffix>` next to `path`.
#[cfg_attr(windows, allow(dead_code))]
fn sibling(path: &Path, suffix: &str) -> Result<PathBuf> {
    let name = path.file_name().context("path has no name")?;
    Ok(path.with_file_name(format!(".{}.{suffix}", name.to_string_lossy())))
}

#[cfg_attr(windows, allow(dead_code))]
fn remove(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path)?,
        Ok(_) => fs::remove_file(path)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

/// Put `incoming` at `bundle`, keeping the old one as `.<name>.previous`; undone on failure.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn swap(incoming: &Path, bundle: &Path) -> Result<()> {
    let previous = sibling(bundle, "previous")?;
    remove(&previous)?;
    fs::rename(bundle, &previous).context("move the current app aside")?;
    if let Err(error) = fs::rename(incoming, bundle) {
        let _ = fs::rename(&previous, bundle);
        return Err(error).context("move the new app into place");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn run(command: &mut Command) -> Result<()> {
    let output = command.output()?;
    if !output.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_level_and_version_decide_outdated_and_installability() {
        let current = describe(Some("1.2.0".into()), SHELL_API, "1.2.0", Ok(()));
        assert!(!current.outdated && current.can_install && current.reason.is_none());
        let behind = describe(Some("1.1.0".into()), SHELL_API, "1.2.0", Ok(()));
        assert!(behind.outdated && behind.can_install);
        let old = describe(None, 0, "1.2.0", Ok(()));
        assert!(old.outdated && !old.can_install);
        assert!(old.reason.unwrap().contains("by hand"));
        let package = describe(Some("1.2.0".into()), SHELL_API, "1.2.0", Err("deb".into()));
        assert_eq!(
            (package.can_install, package.reason.as_deref()),
            (false, Some("deb"))
        );
    }

    #[test]
    fn finds_bundles_and_program_files() {
        assert_eq!(
            bundle_of(Path::new(
                "/Applications/Cranibox.app/Contents/MacOS/cranibox"
            )),
            Some(PathBuf::from("/Applications/Cranibox.app"))
        );
        assert_eq!(bundle_of(Path::new("/usr/local/bin/cranibox")), None);
        let roots = [PathBuf::from(r"C:\Program Files")];
        assert!(under_any(
            Path::new(r"c:\program files\Cranibox\cranibox.exe"),
            &roots
        ));
        assert!(!under_any(
            Path::new(r"C:\Program Files (x86)\x.exe"),
            &roots
        ));
        assert!(!under_any(
            Path::new(r"C:\Users\me\AppData\Local\Cranibox\cranibox.exe"),
            &roots
        ));
        assert!(!under_any(Path::new(r"C:\x.exe"), &[PathBuf::new()]));
    }

    #[test]
    fn designated_requirement_ignores_ad_hoc_signatures() {
        let signed =
            "# designated => identifier \"com.cranibox.node\" and certificate leaf = H\"ab\"\n";
        assert_eq!(
            designated(signed),
            Some("identifier \"com.cranibox.node\" and certificate leaf = H\"ab\"")
        );
        assert_eq!(designated("designated => cdhash H\"12\"\n"), None);
        assert_eq!(designated(""), None);
    }

    #[test]
    fn swap_keeps_the_previous_bundle_and_undoes_a_failed_move() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("Cranibox.app");
        let incoming = dir.path().join(".Cranibox.app.incoming");
        fs::create_dir_all(bundle.join("old")).unwrap();
        fs::create_dir_all(incoming.join("new")).unwrap();
        fs::create_dir_all(dir.path().join(".Cranibox.app.previous/older")).unwrap();
        swap(&incoming, &bundle).unwrap();
        assert!(bundle.join("new").exists() && !incoming.exists());
        assert!(dir.path().join(".Cranibox.app.previous/old").exists());
        // Nothing to move in: the current app goes back.
        let error = swap(&incoming, &bundle).unwrap_err();
        assert!(format!("{error:#}").contains("move the new app"));
        assert!(bundle.join("new").exists());
    }
}
