use anyhow::{Context, Result};
use rand::RngCore;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use subtle::ConstantTimeEq;

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

#[cfg(all(test, unix))]
mod identity_tests {
    use super::*;
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
}
