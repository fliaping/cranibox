//! A GUI app on macOS (and some Linux desktops) starts with a minimal PATH such as
//! `/usr/bin:/bin:/usr/sbin:/sbin`, so agents installed in `~/.local/bin` or Homebrew
//! look missing and remote commands cannot find `node` or `codex`. Take the PATH of
//! the user's login shell, plus the usual tool directories.
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

const MARKER: &str = "__CRANIBOX_PATH__";

/// The PATH this process should use. Call before starting any thread.
pub fn resolve() -> Option<OsString> {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let joined = merge(&current, login_shell_path(), home.as_deref())?;
    (joined != current).then_some(joined)
}
/// `current` first, then the login shell's entries, then the usual tool directories
/// that exist.
fn merge(current: &OsString, shell: Option<OsString>, home: Option<&Path>) -> Option<OsString> {
    let mut paths: Vec<PathBuf> = std::env::split_paths(current).collect();
    for path in shell.iter().flat_map(std::env::split_paths) {
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    let mut usual = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/opt/homebrew/sbin"),
        PathBuf::from("/usr/local/bin"),
    ];
    if let Some(home) = home {
        usual.splice(
            0..0,
            [".local/bin", ".cargo/bin", ".bun/bin", ".npm-global/bin"].map(|d| home.join(d)),
        );
    }
    for path in usual {
        if path.is_dir() && !paths.contains(&path) {
            paths.push(path);
        }
    }
    std::env::join_paths(paths).ok()
}

/// `$SHELL -ilc` prints its PATH between markers; startup noise is ignored.
fn login_shell_path() -> Option<OsString> {
    let shell = std::env::var_os("SHELL")
        .map(PathBuf::from)
        .filter(|s| s.is_absolute() && s.exists())
        .unwrap_or_else(|| PathBuf::from("/bin/zsh"));
    if !Path::new(&shell).exists() {
        return None;
    }
    let mut child = Command::new(&shell)
        .args(["-ilc", &format!("printf '{MARKER}%s{MARKER}' \"$PATH\"")])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // A broken shell profile must not keep the app from starting.
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut output = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut output).ok()?;
    let path = output.split(MARKER).nth(1)?;
    (!path.is_empty() && path.len() <= 32 * 1024).then(|| OsString::from(path))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn keeps_the_current_order_and_adds_the_shell_and_existing_tool_directories() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".local/bin")).unwrap();
        let path = merge(
            &OsString::from("/usr/bin:/bin"),
            Some(OsString::from("/bin:/custom/bin")),
            Some(home.path()),
        )
        .unwrap();
        let parts: Vec<PathBuf> = std::env::split_paths(&path).collect();
        assert_eq!(
            &parts[..3],
            &["/usr/bin", "/bin", "/custom/bin"].map(PathBuf::from)
        );
        assert!(parts.contains(&home.path().join(".local/bin")));
        assert!(
            !parts.contains(&home.path().join(".bun/bin")),
            "only directories that exist"
        );
    }
}
