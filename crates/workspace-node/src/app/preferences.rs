//! Interface preferences kept beside the node configuration. The desktop app serves
//! its interface on a new loopback port each launch, so browser storage (per origin)
//! cannot remember them.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    /// `en` or `zh-cn`; unset follows the browser.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// `light`, `dark` or `system`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// The setup guide was finished or dismissed.
    pub onboarded: bool,
}

pub fn path(peers: &Path) -> PathBuf {
    peers.with_file_name("ui-preferences.json")
}

/// Missing or unreadable preferences are the defaults.
pub fn read(peers: &Path) -> Preferences {
    std::fs::read(path(peers))
        .ok()
        .filter(|bytes| bytes.len() <= 4096)
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Merge `change` (any subset of the fields) into the saved preferences.
pub fn update(peers: &Path, change: &serde_json::Value) -> Result<Preferences> {
    let mut value = serde_json::to_value(read(peers))?;
    let (Some(saved), Some(change)) = (value.as_object_mut(), change.as_object()) else {
        anyhow::bail!("Preferences must be an object");
    };
    for (key, field) in change {
        saved.insert(key.clone(), field.clone());
    }
    let preferences: Preferences = serde_json::from_value(value)?;
    ensure!(
        preferences
            .language
            .as_deref()
            .is_none_or(|l| matches!(l, "en" | "zh-cn")),
        "Language must be en or zh-cn"
    );
    ensure!(
        preferences
            .theme
            .as_deref()
            .is_none_or(|t| matches!(t, "light" | "dark" | "system")),
        "Theme must be light, dark or system"
    );
    agent_workspace_runtime::config::write_private(
        &path(peers),
        &serde_json::to_vec_pretty(&preferences)?,
        true,
    )?;
    Ok(preferences)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_merge_validate_and_survive_rereading() {
        let root = tempfile::tempdir().unwrap();
        let peers = root.path().join("peers.json");
        assert_eq!(read(&peers), Preferences::default());
        update(&peers, &serde_json::json!({"language": "zh-cn"})).unwrap();
        let saved = update(&peers, &serde_json::json!({"onboarded": true})).unwrap();
        assert_eq!(saved.language.as_deref(), Some("zh-cn"));
        assert!(saved.onboarded && read(&peers) == saved);
        assert!(update(&peers, &serde_json::json!({"language": "fr"})).is_err());
        assert!(update(&peers, &serde_json::json!({"other": 1})).is_err());
        assert_eq!(read(&peers), saved);
    }
}
