use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[default]
    X11vnc,
    Wayvnc,
}
impl Provider {
    pub fn current() -> Self {
        if wayland_session() {
            Self::Wayvnc
        } else {
            Self::X11vnc
        }
    }
    pub fn package(self) -> &'static str {
        match self {
            Self::X11vnc => "x11vnc",
            Self::Wayvnc => "wayvnc",
        }
    }
    pub fn display(self) -> String {
        std::env::var(match self {
            Self::X11vnc => "DISPLAY",
            Self::Wayvnc => "WAYLAND_DISPLAY",
        })
        .unwrap_or_default()
    }
    pub fn problem(self) -> Option<&'static str> {
        if !cfg!(target_os = "linux") {
            return Some(
                "Managed VNC currently supports Linux X11 and compatible wlroots Wayland sessions. Connect an existing local VNC server on this platform.",
            );
        }
        match self {
            Self::X11vnc if wayland_session() => Some(
                "Managed x11vnc cannot share a Wayland desktop. Use wayvnc on a compatible wlroots session, or connect an existing VNC server.",
            ),
            Self::Wayvnc if !wayland_session() => Some(
                "Managed wayvnc requires a Wayland login session. Start the node inside the desktop you want to share.",
            ),
            Self::Wayvnc
                if selkies_wayland_session()
                    && std::env::var("WAYLAND_DISPLAY")
                        .unwrap_or_default()
                        .is_empty() =>
            {
                Some(
                    "Selkies is running a Wayland desktop, but this background node has no selected Wayland session. Use the existing Selkies browser entry, or run the node inside a compatible wlroots desktop. x11vnc cannot share this desktop through XWayland.",
                )
            }
            Self::Wayvnc
                if unsupported_desktop(&format!(
                    "{}:{}",
                    std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
                    std::env::var("XDG_SESSION_DESKTOP").unwrap_or_default()
                )) =>
            {
                Some(
                    "wayvnc does not support GNOME, KDE or Weston. Connect a compatible local VNC server, or use a wlroots desktop such as Sway.",
                )
            }
            _ => None,
        }
    }
    pub fn validate(self, display: &str, output: &str) -> Result<()> {
        match self {
            Self::X11vnc => {
                let display = display.strip_prefix(':').unwrap_or_default();
                let parts: Vec<_> = display.split('.').collect();
                ensure!(
                    parts.len() <= 2
                        && parts.iter().all(|part| !part.is_empty()
                            && part.len() <= 5
                            && part.bytes().all(|b| b.is_ascii_digit())
                            && part.parse::<u16>().is_ok())
                        && output.is_empty(),
                    "Use a local X11 display such as :0 or :1. Output selection is available only for wayvnc."
                );
            }
            Self::Wayvnc => {
                ensure!(
                    !display.is_empty()
                        && display.len() <= 96
                        && display
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')),
                    "Use a local Wayland socket name such as wayland-0, without a path or command."
                );
                ensure!(
                    output.len() <= 96
                        && output
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')),
                    "Use a Wayland output name such as DP-1, or leave it empty for the first display."
                );
            }
        }
        Ok(())
    }
}
pub(super) fn wayland_session() -> bool {
    session_is_wayland(
        &std::env::var("XDG_SESSION_TYPE").unwrap_or_default(),
        &std::env::var("WAYLAND_DISPLAY").unwrap_or_default(),
        &std::env::var("PIXELFLUX_WAYLAND").unwrap_or_default(),
    )
}
pub(super) fn selkies_wayland_session() -> bool {
    std::env::var("PIXELFLUX_WAYLAND").is_ok_and(|s| s.eq_ignore_ascii_case("true"))
        && wayland_session()
}
fn session_is_wayland(session: &str, display: &str, selkies: &str) -> bool {
    // Image services inherit DISPLAY for XWayland, but do not necessarily inherit
    // the login session's WAYLAND_DISPLAY/XDG_SESSION_TYPE. An explicitly selected
    // X11 session still takes precedence over the image's Selkies default.
    !display.is_empty()
        || session.eq_ignore_ascii_case("wayland")
        || (!session.eq_ignore_ascii_case("x11") && selkies.eq_ignore_ascii_case("true"))
}
fn unsupported_desktop(value: &str) -> bool {
    value.split([':', ';', ' ']).any(|part| {
        matches!(
            part.to_ascii_lowercase().as_str(),
            "gnome" | "ubuntu" | "ubuntu-wayland" | "kde" | "plasma" | "plasmawayland" | "weston"
        )
    })
}
pub(super) fn supported_version(value: &str) -> bool {
    let Some(version) = value.lines().find_map(|line| line.strip_prefix("wayvnc: ")) else {
        return false;
    };
    let mut parts = version.trim().trim_start_matches('v').split('.');
    let major = parts.next().and_then(|v| v.parse::<u32>().ok());
    let minor = parts.next().and_then(|v| v.parse::<u32>().ok());
    matches!((major, minor), (Some(major), Some(minor)) if major > 0 || minor >= 8)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selkies_wayland_is_detected_without_login_environment() {
        for (session, display, selkies, expected) in [
            ("", "", "true", true),
            ("", "", "TRUE", true),
            ("", "", "false", false),
            ("", "", "", false),
            ("wayland", "", "", true),
            ("", "wayland-0", "", true),
            ("x11", "", "true", false),
            ("X11", "", "true", false),
            ("x11", "wayland-0", "true", true),
        ] {
            assert_eq!(session_is_wayland(session, display, selkies), expected);
        }
    }
    #[test]
    fn only_local_display_and_output_names_are_accepted() {
        assert!(Provider::Wayvnc.validate("wayland-0", "DP-1").is_ok());
        assert!(Provider::Wayvnc.validate("wayland_1", "").is_ok());
        for value in [
            "",
            "../wayland-0",
            "/tmp/wayland-0",
            "wayland-0;touch x",
            "-C /tmp/conf",
        ] {
            assert!(Provider::Wayvnc.validate(value, "").is_err());
        }
        for value in ["DP-1\npassword=x", "--config=/tmp/conf", "DP 1"] {
            assert!(Provider::Wayvnc.validate("wayland-0", value).is_err());
        }
        assert!(Provider::X11vnc.validate(":1.0", "").is_ok());
        assert!(Provider::X11vnc.validate(":1", "DP-1").is_err());
    }
    #[test]
    fn compositor_limits_and_minimum_version_are_explicit() {
        for desktop in ["GNOME", "ubuntu:GNOME", "KDE", "plasma", "weston"] {
            assert!(unsupported_desktop(desktop));
        }
        for desktop in ["sway", "wlroots", ""] {
            assert!(!unsupported_desktop(desktop));
        }
        for version in [
            "wayvnc: 0.8.0\nneatvnc: 0.8.0",
            "wayvnc: 0.9.1",
            "wayvnc: 1.0.0",
        ] {
            assert!(supported_version(version));
        }
        for version in ["wayvnc: 0.7.2", "wayvnc: 0.garbage.0", "other: 0.9.1"] {
            assert!(!supported_version(version));
        }
    }
}
