use anyhow::{Context, Result};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use subtle::ConstantTimeEq;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    pub controller_token: String,
    #[serde(default)]
    pub nodes: BTreeMap<String, String>,
}
impl Credentials {
    pub fn read(path: &Path) -> Result<Self> {
        let value: Self =
            serde_json::from_slice(&fs::read(path).context("read credentials file")?)?;
        anyhow::ensure!(
            value.controller_token.len() == 64
                && value
                    .controller_token
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit())
                && value.nodes.len() <= 128
                && value.nodes.values().all(|v| v.len() == 64
                    && v.bytes().all(|b| b.is_ascii_hexdigit())
                    && v != &value.controller_token),
            "invalid credentials"
        );
        anyhow::ensure!(
            value
                .nodes
                .keys()
                .all(|id| crate::protocol::valid_node_id(id)),
            "invalid node ID in credentials"
        );
        Ok(value)
    }
}
pub fn matches(actual: &str, expected: &str) -> bool {
    actual.len() == expected.len() && bool::from(actual.as_bytes().ct_eq(expected.as_bytes()))
}
pub fn token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn lock_credentials(path: &Path) -> Result<fs::File> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".lock");
    let file = options.open(PathBuf::from(lock_path))?;
    fs2::FileExt::lock_exclusive(&file)?;
    Ok(file)
}
pub fn write_private(path: &Path, bytes: &[u8], replace: bool) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".workspace-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if replace {
            fs::rename(&temp, path)?;
        } else {
            // hard_link is an atomic no-clobber operation, including under concurrent enrollment.
            fs::hard_link(&temp, path)?;
            fs::remove_file(&temp)?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
pub fn read_token(path: &Path) -> Result<String> {
    let value = fs::read_to_string(path)
        .context("read token file")?
        .trim()
        .to_owned();
    anyhow::ensure!(
        value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid token file"
    );
    Ok(value)
}
pub fn default_credentials() -> PathBuf {
    let root = std::env::var_os("AGENT_WORKSPACE_CONFIG_ROOT").unwrap_or_else(|| "/config".into());
    PathBuf::from(root).join(".config/agent-workspace/runtime.json")
}
pub fn check_url(value: &str, websocket: bool, allow_insecure: bool) -> Result<url::Url> {
    let url = url::Url::parse(value)?;
    let secure = if websocket { "wss" } else { "https" };
    let plain = if websocket { "ws" } else { "http" };
    anyhow::ensure!(
        url.scheme() == secure || url.scheme() == plain,
        "invalid URL scheme"
    );
    anyhow::ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "URL must not contain credentials, query or fragment"
    );
    let local = url.host_str().is_some_and(|s| {
        s == "localhost"
            || s.parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    anyhow::ensure!(
        url.scheme() == secure || local || allow_insecure,
        "remote connections require TLS; --allow-insecure is for a trusted private network only"
    );
    Ok(url)
}
