//! What a device tells its peers about itself: a friendly name and system details.
use serde::Serialize;
use std::sync::OnceLock;

#[derive(Clone, Debug, Serialize)]
pub struct DeviceInfo {
    /// The name people know the device by (macOS Computer Name, otherwise the hostname).
    pub name: String,
    pub os: &'static str,
    pub os_version: Option<String>,
    pub arch: &'static str,
    pub model: Option<String>,
    /// `desktop`, `laptop` or `server`.
    pub kind: &'static str,
}

/// This device's details, read once.
pub fn local() -> &'static DeviceInfo {
    static INFO: OnceLock<DeviceInfo> = OnceLock::new();
    INFO.get_or_init(read)
}

#[cfg(not(target_os = "linux"))]
fn command(program: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !text.is_empty() && text.len() <= 128).then_some(text)
}

fn hostname() -> Option<String> {
    #[cfg(unix)]
    {
        let mut buffer = [0u8; 256];
        // SAFETY: the buffer is valid for its length; gethostname NUL-terminates on success.
        let result = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) };
        if result == 0 {
            let end = buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());
            let name = String::from_utf8_lossy(&buffer[..end]).trim().to_owned();
            if !name.is_empty() {
                return Some(name);
            }
        }
        None
    }
    #[cfg(windows)]
    {
        std::env::var("COMPUTERNAME").ok().filter(|n| !n.is_empty())
    }
}

fn read() -> DeviceInfo {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    #[cfg(target_os = "macos")]
    {
        let model = command("/usr/sbin/sysctl", &["-n", "hw.model"]);
        let laptop = model.as_deref().is_some_and(|m| m.contains("MacBook"));
        DeviceInfo {
            name: command("/usr/sbin/scutil", &["--get", "ComputerName"])
                .or_else(hostname)
                .unwrap_or_else(|| "Mac".into()),
            os,
            os_version: command("/usr/bin/sw_vers", &["-productVersion"])
                .map(|v| format!("macOS {v}")),
            arch,
            model,
            kind: if laptop { "laptop" } else { "desktop" },
        }
    }
    #[cfg(target_os = "linux")]
    {
        let release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
        let os_version = release
            .lines()
            .find_map(|l| l.strip_prefix("PRETTY_NAME="))
            .map(|v| v.trim_matches('"').to_owned())
            .filter(|v| !v.is_empty());
        let battery = std::fs::read_dir("/sys/class/power_supply").is_ok_and(|entries| {
            entries
                .flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with("BAT"))
        });
        let graphical =
            std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some();
        DeviceInfo {
            name: hostname().unwrap_or_else(|| "Linux".into()),
            os,
            os_version,
            arch,
            model: std::fs::read_to_string("/sys/devices/virtual/dmi/id/product_name")
                .ok()
                .map(|m| m.trim().to_owned())
                .filter(|m| !m.is_empty() && m.len() <= 128),
            kind: if battery {
                "laptop"
            } else if graphical {
                "desktop"
            } else {
                "server"
            },
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        DeviceInfo {
            name: hostname().unwrap_or_else(|| "Windows".into()),
            os,
            os_version: command("cmd", &["/C", "ver"]),
            arch,
            model: None,
            kind: "desktop",
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn local_device_has_a_name_and_platform() {
        let info = super::local();
        assert!(!info.name.is_empty() && info.name.len() <= 256);
        assert_eq!(info.os, std::env::consts::OS);
        assert!(matches!(info.kind, "desktop" | "laptop" | "server"));
    }
}
