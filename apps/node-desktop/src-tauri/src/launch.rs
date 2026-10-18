use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// A persistent executable, rather than the temporary program inside an AppImage.
pub struct Launcher {
    executable: PathBuf,
    appimage: bool,
}

impl Launcher {
    pub fn current() -> Result<Self> {
        let executable = std::env::current_exe().context("Find application executable")?;
        #[cfg(target_os = "linux")]
        let appimage = std::env::var_os("APPIMAGE").map(PathBuf::from);
        #[cfg(not(target_os = "linux"))]
        let appimage = None;
        Self::from_paths(executable, appimage)
    }

    fn from_paths(executable: PathBuf, appimage: Option<PathBuf>) -> Result<Self> {
        let is_appimage = appimage.is_some();
        let executable = appimage.unwrap_or(executable);
        ensure!(
            executable.is_absolute(),
            "Application path must be absolute"
        );
        let text = executable
            .to_str()
            .context("Application path is not UTF-8")?;
        ensure!(
            !text.chars().any(char::is_control),
            "Application path contains a control character"
        );
        Ok(Self {
            executable,
            appimage: is_appimage,
        })
    }

    fn arguments(&self, args: &[&str]) -> Vec<String> {
        // The AppImage runtime consumes this flag before invoking the application.
        // It works without FUSE and does not depend on the caller's environment.
        let mut result = Vec::new();
        if self.appimage {
            result.push("--appimage-extract-and-run".to_owned());
        }
        result.extend(args.iter().map(|arg| (*arg).to_owned()));
        result
    }

    pub fn mcp_config(&self, config: &Path, include_runs: bool) -> Value {
        let entry = |args: &[&str]| {
            let mut args: Vec<Value> = self
                .arguments(args)
                .into_iter()
                .map(Value::String)
                .collect();
            args.extend([json!("--config"), json!(config)]);
            json!({"command": self.executable, "args": args})
        };
        let mut servers = json!({
            "cranibox-devices": entry(&["mcp"]),
            "cranibox-messages": entry(&["peer", "messages", "mcp"]),
        });
        if include_runs {
            servers["cranibox-runs"] = entry(&["peer", "runs", "mcp"]);
        }
        json!({"mcpServers": servers})
    }

    #[cfg(target_os = "linux")]
    fn desktop_entry(&self) -> String {
        let executable = self.executable.to_str().unwrap();
        let mut command = desktop_quote(executable);
        // GIO checks that the executable exists before expanding %% field codes.
        // Pass paths containing % as an argument to env so that check stays valid.
        if executable.contains('%') {
            command = format!("/usr/bin/env -- {command}");
        }
        for arg in self.arguments(&["--background"]) {
            command.push(' ');
            command.push_str(&desktop_quote(&arg));
        }
        format!(
            "[Desktop Entry]\nType=Application\nVersion=1.0\nName=Cranibox Node\nExec={command}\nStartupNotify=false\nTerminal=false\n"
        )
    }
}

// Desktop entries have two escaping layers: string decoding, then Exec quoting.
#[cfg(target_os = "linux")]
fn desktop_quote(value: &str) -> String {
    let mut result = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' => result.push_str("\\\\\\\\"),
            '"' | '$' | '\u{60}' => {
                result.push_str("\\\\");
                result.push(ch);
            }
            '%' => result.push_str("%%"),
            _ => result.push(ch),
        }
    }
    result.push('"');
    result
}

#[cfg(target_os = "linux")]
pub struct Autostart {
    entry: PathBuf,
    legacy: Option<PathBuf>,
}

#[cfg(target_os = "linux")]
impl Autostart {
    pub fn current() -> Result<Self> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
        Self::from_paths(home.as_deref(), config.as_deref())
    }

    fn from_paths(home: Option<&Path>, config: Option<&Path>) -> Result<Self> {
        let home = home.filter(|path| path.is_absolute());
        let config = config
            .filter(|path| path.is_absolute())
            .map(Path::to_owned)
            .or_else(|| home.map(|path| path.join(".config")))
            .context("Cannot locate the user autostart directory")?;
        let entry = config.join("autostart/Cranibox Node.desktop");
        let legacy = home
            .map(|path| path.join(".config/autostart/Cranibox Node.desktop"))
            .filter(|path| *path != entry);
        Ok(Self { entry, legacy })
    }

    pub fn is_enabled(&self) -> Result<bool> {
        for path in std::iter::once(&self.entry).chain(self.legacy.iter()) {
            match std::fs::read_to_string(path) {
                Ok(text) => {
                    let disabled = text.lines().any(|line| {
                        matches!(
                            line.trim(),
                            "Hidden=true" | "X-GNOME-Autostart-enabled=false"
                        )
                    });
                    return Ok(!disabled);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error).context("Read login startup entry"),
            }
        }
        Ok(false)
    }

    pub fn set_enabled(&self, launcher: &Launcher, enabled: bool) -> Result<()> {
        if enabled {
            agent_workspace_runtime::config::write_private(
                &self.entry,
                launcher.desktop_entry().as_bytes(),
                true,
            )
            .context("Save login startup entry")?;
        } else {
            remove_entry(&self.entry)?;
        }
        if let Some(path) = &self.legacy {
            remove_entry(path)?;
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn remove_entry(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("Remove login startup entry"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launcher(appimage: bool) -> Launcher {
        Launcher::from_paths(
            std::env::current_exe().unwrap(),
            appimage.then(|| std::env::temp_dir().join("Cranibox Node.AppImage")),
        )
        .unwrap()
    }

    #[test]
    fn mcp_keeps_persistent_paths_and_appimage_runtime_arguments() {
        let launcher = launcher(true);
        let config = std::env::temp_dir().join("profile/settings.json");
        let result = launcher.mcp_config(&config, true);
        for (name, command) in [
            ("cranibox-devices", vec!["mcp"]),
            ("cranibox-messages", vec!["peer", "messages", "mcp"]),
            ("cranibox-runs", vec!["peer", "runs", "mcp"]),
        ] {
            let server = &result["mcpServers"][name];
            assert_eq!(server["command"], json!(launcher.executable));
            let args = server["args"].as_array().unwrap();
            assert_eq!(args[0], "--appimage-extract-and-run");
            assert_eq!(
                &args[1..1 + command.len()],
                json!(command).as_array().unwrap()
            );
            assert_eq!(args[args.len() - 2], "--config");
            assert_eq!(args[args.len() - 1], json!(config));
            assert!(server.get("env").is_none());
        }
    }

    #[test]
    fn installed_mcp_does_not_require_appimage_or_enable_runs() {
        let launcher = launcher(false);
        let config = std::env::temp_dir().join("profile/settings.json");
        let result = launcher.mcp_config(&config, false);
        assert_eq!(result["mcpServers"].as_object().unwrap().len(), 2);
        assert_eq!(result["mcpServers"]["cranibox-devices"]["args"][0], "mcp");
        assert!(result["mcpServers"]["cranibox-runs"].is_null());
    }

    #[test]
    fn invalid_launcher_paths_are_rejected() {
        assert!(Launcher::from_paths(PathBuf::from("relative"), None).is_err());
        let path = std::env::temp_dir().join("Cranibox\nNode");
        assert!(Launcher::from_paths(path, None).is_err());
    }

    #[cfg(target_os = "linux")]
    fn temporary_directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "cranibox-launch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn autostart_migrates_legacy_entry_to_xdg_and_disables_both() {
        let root = temporary_directory();
        let home = root.join("home");
        let config = root.join("custom-config");
        let autostart = Autostart::from_paths(Some(&home), Some(&config)).unwrap();
        let legacy = autostart.legacy.as_ref().unwrap();
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(legacy, "[Desktop Entry]\nExec=/old/path\n").unwrap();
        assert!(autostart.is_enabled().unwrap());
        autostart.set_enabled(&launcher(true), true).unwrap();
        assert!(autostart.is_enabled().unwrap());
        assert!(!legacy.exists());
        let text = std::fs::read_to_string(&autostart.entry).unwrap();
        assert!(text.contains("--appimage-extract-and-run"));
        assert!(text.contains("--background"));
        std::fs::write(legacy, "old duplicate").unwrap();
        autostart.set_enabled(&launcher(true), false).unwrap();
        assert!(!autostart.is_enabled().unwrap());
        assert!(!legacy.exists());
        autostart.set_enabled(&launcher(true), false).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn relative_xdg_directory_is_ignored_and_hidden_entry_is_disabled() {
        let root = temporary_directory();
        let autostart = Autostart::from_paths(Some(&root), Some(Path::new("relative"))).unwrap();
        assert_eq!(
            autostart.entry,
            root.join(".config/autostart/Cranibox Node.desktop")
        );
        assert!(!autostart.is_enabled().unwrap());
        autostart.set_enabled(&launcher(false), true).unwrap();
        std::fs::write(&autostart.entry, "[Desktop Entry]\nHidden=true\n").unwrap();
        assert!(!autostart.is_enabled().unwrap());
        assert!(Autostart::from_paths(None, Some(Path::new("relative"))).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires the Linux gio desktop launcher"]
    fn gio_launches_exact_executable_with_spaces_and_reserved_characters() {
        use std::os::unix::fs::PermissionsExt;
        let root = temporary_directory();
        let executable = root.join("Cranibox Node %f $HOME \u{60}test\u{60} \\\".AppImage");
        let output = root.join("arguments.json");
        let script = format!(
            "#!/usr/bin/python3\nimport json,sys\nfrom pathlib import Path\nPath({}).write_text(json.dumps(sys.argv[1:]))\n",
            json!(output.to_str().unwrap())
        );
        std::fs::write(&executable, script).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let launcher = Launcher::from_paths(executable.clone(), Some(executable)).unwrap();
        let autostart = Autostart::from_paths(None, Some(&root)).unwrap();
        autostart.set_enabled(&launcher, true).unwrap();
        let result = std::process::Command::new("gio")
            .arg("launch")
            .arg(&autostart.entry)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        for _ in 0..100 {
            if output.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let args: Value = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        assert_eq!(args, json!(["--appimage-extract-and-run", "--background"]));
        std::fs::remove_dir_all(root).unwrap();
    }
}
