//! The app is also the `cranibox` command. The Linux package and the Windows installer
//! put it on PATH; on macOS the app links it into /usr/local/bin when asked.
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
const LINK: &str = "/usr/local/bin/cranibox";

/// The persistent program: the AppImage file rather than its temporary mount.
fn program() -> Result<PathBuf> {
    #[cfg(target_os = "linux")]
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        return Ok(PathBuf::from(appimage));
    }
    std::env::current_exe().context("Find application executable")
}

/// Whether `cranibox` found on `path` runs this program.
fn runs(found: &Path, program: &Path) -> bool {
    let canonical = |path: &Path| std::fs::canonicalize(path).ok();
    if canonical(found).is_some_and(|found| Some(found) == canonical(program)) {
        return true;
    }
    // Windows: the installer's bin\cranibox.cmd starts ..\cranibox.exe.
    found
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd"))
        && found
            .parent()
            .and_then(Path::parent)
            .zip(program.parent())
            .is_some_and(|(found, program)| canonical(found) == canonical(program))
}

/// The `cranibox` a terminal with this PATH would run.
fn find(path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["cranibox.cmd", "cranibox.exe"]
    } else {
        &["cranibox"]
    };
    std::env::split_paths(path?)
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| candidate.is_file())
}

pub fn status() -> Result<Value> {
    let program = program()?;
    #[cfg(windows)]
    {
        // A running app keeps the PATH it started with; the installer's entry counts.
        let script = program
            .parent()
            .map(|dir| dir.join("bin").join("cranibox.cmd"));
        if let Some(script) = script.filter(|s| s.is_file()) {
            return Ok(
                json!({"program": program, "command": script, "ready": true, "installable": false}),
            );
        }
    }
    let command = find(std::env::var_os("PATH").as_deref());
    let ready = command.as_deref().is_some_and(|c| runs(c, &program));
    Ok(json!({
        "program": program,
        "command": command,
        "ready": ready,
        "installable": cfg!(target_os = "macos") && !ready,
    }))
}

/// Link /usr/local/bin/cranibox to this app after the system asks for an administrator.
#[cfg(target_os = "macos")]
pub async fn install() -> Result<Value> {
    let program = program()?;
    let output = tokio::process::Command::new("/usr/bin/osascript")
        .args([
            "-e",
            "on run argv",
            "-e",
            "do shell script \"mkdir -p /usr/local/bin && ln -sf \" & quoted form of (item 1 of argv) & \" \" & quoted form of (item 2 of argv) with administrator privileges",
            "-e",
            "end run",
        ])
        .arg(&program)
        .arg(LINK)
        .output()
        .await
        .context("Run osascript")?;
    anyhow::ensure!(
        output.status.success(),
        "The command line was not installed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    status()
}
#[cfg(not(target_os = "macos"))]
pub async fn install() -> Result<Value> {
    anyhow::bail!("The installer already puts cranibox on PATH on this system")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_link_on_path_runs_the_app_and_another_program_does_not() {
        let root = std::env::temp_dir().join(format!("cranibox-cli-{}", std::process::id()));
        let (app, bin, other) = (root.join("app"), root.join("bin"), root.join("other"));
        for dir in [&app, &bin, &other] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let program = app.join("cranibox-app");
        std::fs::write(&program, "").unwrap();
        std::fs::write(other.join("cranibox"), "").unwrap();
        std::os::unix::fs::symlink(&program, bin.join("cranibox")).unwrap();

        let path = std::env::join_paths([&other, &bin]).unwrap();
        let found = find(Some(&path)).unwrap();
        assert_eq!(found, other.join("cranibox"));
        assert!(!runs(&found, &program));
        let path = std::env::join_paths([&bin]).unwrap();
        assert!(runs(&find(Some(&path)).unwrap(), &program));
        assert!(find(Some(&std::env::join_paths([&app]).unwrap())).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
