//! Printable autostart definitions. Installation is always left to the user.
use anyhow::{Result, bail, ensure};
use std::path::Path;

pub fn definition(os: &str, executable: &Path, config: &Path) -> Result<String> {
    let exe = executable
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("non-UTF-8 executable path"))?;
    let config = config
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("non-UTF-8 configuration path"))?;
    ensure!(
        !exe.contains(['\n', '\r']) && !config.contains(['\n', '\r']),
        "invalid service path"
    );
    match os {
        "linux" => {
            let quote = |s: &str| {
                format!(
                    "\"{}\"",
                    s.replace('\\', "\\\\")
                        .replace('"', "\\\"")
                        .replace('%', "%%")
                        .replace('$', "$$")
                )
            };
            Ok(format!(
                "# Save as ~/.config/systemd/user/cranibox.service\n# systemctl --user daemon-reload\n# systemctl --user enable --now cranibox.service\n[Unit]\nDescription=Cranibox runtime launcher\nAfter=network.target\n\n[Service]\nExecStart={} --config {} update launch\nRestart=always\nKillMode=control-group\n\n[Install]\nWantedBy=default.target\n",
                quote(exe),
                quote(config)
            ))
        }
        "macos" => {
            let xml = |s: &str| {
                s.replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
                    .replace('"', "&quot;")
                    .replace('\'', "&apos;")
            };
            Ok(format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- Save as ~/Library/LaunchAgents/com.cranibox.launcher.plist\nlaunchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.cranibox.launcher.plist\nlaunchctl enable gui/$(id -u)/com.cranibox.launcher\n-->\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>com.cranibox.launcher</string>\n<key>ProgramArguments</key><array><string>{}</string><string>--config</string><string>{}</string><string>update</string><string>launch</string></array>\n<key>RunAtLoad</key><true/>\n<key>KeepAlive</key><true/>\n</dict></plist>\n",
                xml(exe),
                xml(config)
            ))
        }
        "windows" => {
            ensure!(
                !exe.contains('"') && !config.contains('"'),
                "invalid Windows path"
            );
            Ok(format!(
                "REM Run in cmd.exe to install the logon task, then start it:\nschtasks /Create /TN Cranibox /SC ONLOGON /TR \"\\\"{exe}\\\" --config \\\"{config}\\\" update launch\" /F\nschtasks /Run /TN Cranibox\n"
            ))
        }
        _ => bail!("unsupported autostart platform: {os}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_definitions_preserve_launcher_and_configuration() {
        let exe = Path::new("/opt/Cranibox App/cranibox");
        let config = Path::new("/data/config & more/settings.json");
        let linux = definition("linux", exe, config).unwrap();
        assert!(linux.contains("Restart=always\nKillMode=control-group"));
        assert!(linux.contains("ExecStart=\"/opt/Cranibox App/cranibox\" --config \"/data/config & more/settings.json\" update launch"));
        assert!(linux.contains("systemctl --user enable --now"));
        assert!(!linux.contains("Restart=on-failure"));
        let mac = definition("macos", exe, config).unwrap();
        assert!(mac.contains("config &amp; more/settings.json</string>"));
        assert!(mac.contains("<string>update</string><string>launch</string>"));
        assert!(mac.contains("launchctl bootstrap"));
        let win = definition(
            "windows",
            Path::new(r"C:\Program Files\Cranibox\cranibox.exe"),
            config,
        )
        .unwrap();
        assert!(win.contains("schtasks /Create /TN Cranibox /SC ONLOGON"));
        assert!(win.contains("update launch"));
        assert!(win.contains("schtasks /Run /TN Cranibox"));
    }

    #[test]
    fn systemd_escapes_specifiers_and_environment_expansion() {
        let text =
            definition("linux", Path::new("/opt/a%$b"), Path::new("/data/settings")).unwrap();
        assert!(text.contains("a%%$$b"));
        assert!(definition("linux", Path::new("/bad\npath"), Path::new("/settings")).is_err());
    }
}
