use anyhow::{Context, Result};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
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
    /// Explicit trust groups; ungrouped devices cannot control other devices.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub mesh_groups: BTreeMap<String, String>,
    /// Previous registration IDs remain usable after an explicit identity migration.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub node_aliases: BTreeMap<String, String>,
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
                && value.nodes.values().collect::<BTreeSet<_>>().len() == value.nodes.len()
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
        anyhow::ensure!(
            value
                .mesh_groups
                .iter()
                .all(|(id, group)| value.nodes.contains_key(id)
                    && crate::protocol::valid_node_id(group)),
            "invalid device group"
        );
        anyhow::ensure!(
            value.node_aliases.len() <= 512
                && value
                    .node_aliases
                    .iter()
                    .all(|(alias, target)| crate::protocol::valid_node_id(alias)
                        && !value.nodes.contains_key(alias)
                        && crate::protocol::valid_node_id(target)
                        && !value.node_aliases.contains_key(target)),
            "invalid node identity aliases"
        );
        Ok(value)
    }
    pub fn resolve_node<'a>(&'a self, id: &'a str) -> &'a str {
        self.node_aliases.get(id).map_or(id, String::as_str)
    }
    /// Only an administrator may approve this mapping; device claims are not proof.
    pub fn migrate_node(&mut self, id: &str, target: &str) -> Result<()> {
        anyhow::ensure!(
            crate::protocol::valid_node_id(target),
            "invalid target node ID"
        );
        let id = self.resolve_node(id).to_owned();
        if id == target {
            anyhow::ensure!(self.nodes.contains_key(target), "unknown node");
            return Ok(());
        }
        anyhow::ensure!(
            !self.nodes.contains_key(target)
                && self
                    .node_aliases
                    .get(target)
                    .is_none_or(|owner| owner == &id),
            "target node ID is already registered or reserved"
        );
        anyhow::ensure!(
            self.node_aliases.len() < 512 || self.node_aliases.contains_key(target),
            "node identity alias limit reached"
        );
        let token = self.nodes.remove(&id).context("unknown node")?;
        self.node_aliases.remove(target);
        self.nodes.insert(target.to_owned(), token);
        if let Some(group) = self.mesh_groups.remove(&id) {
            self.mesh_groups.insert(target.to_owned(), group);
        }
        for previous in self.node_aliases.values_mut() {
            if previous == &id {
                *previous = target.to_owned();
            }
        }
        self.node_aliases.insert(id, target.to_owned());
        Ok(())
    }
    pub fn revoke_node(&mut self, id: &str) -> Result<()> {
        let id = self.resolve_node(id).to_owned();
        anyhow::ensure!(self.nodes.remove(&id).is_some(), "unknown node");
        self.mesh_groups.remove(&id);
        Ok(())
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
fn credentials_lock_file(path: &Path) -> Result<fs::File> {
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
    Ok(options.open(PathBuf::from(lock_path))?)
}
pub fn lock_credentials(path: &Path) -> Result<fs::File> {
    let file = credentials_lock_file(path)?;
    fs2::FileExt::lock_exclusive(&file)?;
    Ok(file)
}
/// Background readers yield to an operator's in-flight request instead of
/// occupying a blocking thread waiting for the same remote connection.
pub(crate) fn try_lock_credentials(path: &Path) -> Result<Option<fs::File>> {
    let file = credentials_lock_file(path)?;
    match fs2::FileExt::try_lock_exclusive(&file) {
        Ok(()) => Ok(Some(file)),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
        Err(error) => Err(error.into()),
    }
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Some NAS ACLs add inherited permissions even when create mode is
            // 0600. Restrict the opened inode before writing private contents.
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
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

#[cfg(test)]
mod identity_tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn private_writes_restrict_creation_and_replacement_and_preserve_no_clobber() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("private.json");
        write_private(&path, b"first", false).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(write_private(&path, b"must not replace", false).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"first");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        write_private(&path, b"replacement", true).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"replacement");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }
    fn credentials() -> Credentials {
        Credentials {
            controller_token: "0".repeat(64),
            nodes: BTreeMap::from([
                ("laptop".into(), "1".repeat(64)),
                ("other".into(), "2".repeat(64)),
            ]),
            mesh_groups: BTreeMap::from([("laptop".into(), "personal".into())]),
            node_aliases: BTreeMap::new(),
        }
    }
    #[test]
    fn migration_preserves_token_group_and_flattens_aliases_in_both_directions() {
        let mut value = credentials();
        value.migrate_node("laptop", "node-first").unwrap();
        assert_eq!(value.nodes["node-first"], "1".repeat(64));
        assert_eq!(value.mesh_groups["node-first"], "personal");
        assert_eq!(value.resolve_node("laptop"), "node-first");
        let before = serde_json::to_value(&value).unwrap();
        value.migrate_node("laptop", "node-first").unwrap();
        assert_eq!(serde_json::to_value(&value).unwrap(), before);
        value.migrate_node("laptop", "node-second").unwrap();
        assert_eq!(value.resolve_node("node-first"), "node-second");
        assert_eq!(value.resolve_node("laptop"), "node-second");
        value.migrate_node("node-first", "laptop").unwrap();
        assert_eq!(value.resolve_node("node-first"), "laptop");
        assert_eq!(value.resolve_node("node-second"), "laptop");
        assert!(!value.node_aliases.contains_key("laptop"));
        assert_eq!(value.nodes["laptop"], "1".repeat(64));
        assert_eq!(value.mesh_groups["laptop"], "personal");
    }
    #[test]
    fn conflicts_and_invalid_sources_leave_credentials_unchanged() {
        let mut value = credentials();
        value.migrate_node("other", "node-other").unwrap();
        let before = serde_json::to_value(&value).unwrap();
        for (source, target) in [
            ("laptop", "node-other"),
            ("laptop", "other"),
            ("laptop", "../invalid"),
            ("missing", "new-id"),
        ] {
            assert!(value.migrate_node(source, target).is_err());
            assert_eq!(serde_json::to_value(&value).unwrap(), before);
        }
    }
    #[test]
    fn aliases_remain_reserved_after_revocation_and_survive_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("runtime.json");
        let mut value = credentials();
        value.migrate_node("laptop", "node-device").unwrap();
        value.revoke_node("laptop").unwrap();
        assert!(!value.nodes.contains_key("node-device"));
        assert!(!value.mesh_groups.contains_key("node-device"));
        write_private(&path, &serde_json::to_vec(&value).unwrap(), false).unwrap();
        let mut reopened = Credentials::read(&path).unwrap();
        assert_eq!(reopened.resolve_node("laptop"), "node-device");
        assert!(reopened.migrate_node("other", "laptop").is_err());
        assert!(reopened.revoke_node("laptop").is_err());
    }
    #[test]
    fn invalid_alias_maps_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("runtime.json");
        for aliases in [
            BTreeMap::from([("old".into(), "old".into())]),
            BTreeMap::from([("laptop".into(), "new".into())]),
            BTreeMap::from([
                ("old".into(), "middle".into()),
                ("middle".into(), "laptop".into()),
            ]),
            BTreeMap::from([("old".into(), "../invalid".into())]),
        ] {
            let mut value = credentials();
            value.node_aliases = aliases;
            write_private(&path, &serde_json::to_vec(&value).unwrap(), true).unwrap();
            assert!(Credentials::read(&path).is_err());
        }
    }
}
