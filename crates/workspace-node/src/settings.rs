use agent_workspace_runtime::{
    client::{Api, ApiOptions},
    config,
    protocol::valid_node_id,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub server: String,
    pub id: String,
    pub name: Option<String>,
    pub token_file: PathBuf,
    #[serde(default)]
    pub allow_insecure: bool,
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(valid_node_id(&self.id), "invalid node ID");
        let url = config::check_url(&self.server, true, self.allow_insecure)?;
        anyhow::ensure!(
            url.path() == "/",
            "server must be a WebSocket origin without a path"
        );
        anyhow::ensure!(
            self.name
                .as_ref()
                .is_none_or(|name| name.len() <= 128 && !name.chars().any(char::is_control)),
            "invalid node name"
        );
        config::read_token(&self.token_file)?;
        Ok(())
    }
    pub fn read(path: &Path) -> Result<Self> {
        let settings: Self =
            serde_json::from_slice(&std::fs::read(path).context("read node settings")?)?;
        settings.validate()?;
        Ok(settings)
    }
    pub fn write(&self, path: &Path) -> Result<()> {
        self.validate()?;
        config::write_private(path, &serde_json::to_vec_pretty(self)?, true)
    }
    pub fn api(&self) -> Result<Api> {
        let mut url = config::check_url(&self.server, true, self.allow_insecure)?;
        let scheme = if url.scheme() == "wss" {
            "https"
        } else {
            "http"
        };
        url.set_scheme(scheme)
            .map_err(|_| anyhow::anyhow!("invalid server URL"))?;
        Api::mesh(ApiOptions {
            url: url.to_string(),
            token_file: Some(self.token_file.clone()),
            credentials: PathBuf::new(),
            allow_insecure: self.allow_insecure,
        })
    }
}
pub fn default_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    base.unwrap_or_else(|| PathBuf::from("."))
        .join("cranibox-node/settings.json")
}
