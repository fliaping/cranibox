//! Private key storage: OS secret service when available, 0600 durable fallback for headless nodes.
use super::*;
use rand08::RngCore;
use std::{
    fs,
    io::Write,
    process::{Child, Command, Output, Stdio},
    time::{Duration, Instant},
};
/// A credential-backend abstraction also usable with Keychain/DPAPI in desktop hosts.
pub trait KeyStore {
    fn read(&self, name: &str) -> Result<Option<SecretBox>, StoreError>;
    fn write(&self, name: &str, value: &SecretBox) -> Result<(), StoreError>;
}
/// Linux Secret Service via its standard client; secret values only travel through stdin/stdout.
pub struct SecretService;
/// Native Keychain / Windows credential storage, or Linux Secret Service.
pub struct PlatformKeyStore;
impl KeyStore for PlatformKeyStore {
    fn read(&self, name: &str) -> Result<Option<SecretBox>, StoreError> {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            let entry = keyring::Entry::new("cranibox-device-key", name).map_err(other)?;
            match entry.get_secret() {
                Ok(secret) => Ok(Some(SecretBox::new(secret))),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(e) => Err(other(e)),
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        SecretService.read(name)
    }
    fn write(&self, name: &str, value: &SecretBox) -> Result<(), StoreError> {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            keyring::Entry::new("cranibox-device-key", name)
                .map_err(other)?
                .set_secret(value.expose())
                .map_err(other)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        SecretService.write(name, value)
    }
}
fn finish(mut child: Child) -> Result<Output, StoreError> {
    let started = Instant::now();
    loop {
        match child.try_wait().map_err(other)? {
            Some(_) => return child.wait_with_output().map_err(other),
            None if started.elapsed() < Duration::from_secs(3) => {
                std::thread::sleep(Duration::from_millis(20))
            }
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(other("secret service timed out"));
            }
        }
    }
}
impl KeyStore for SecretService {
    fn read(&self, name: &str) -> Result<Option<SecretBox>, StoreError> {
        let output = Command::new("secret-tool")
            .args(["lookup", "application", "cranibox", "identity", name])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(other)?;
        let output = finish(output)?;
        if !output.status.success() {
            return Ok(None);
        }
        let bytes = Zeroizing::new(output.stdout);
        let value = B64.decode(bytes.trim_ascii()).map_err(other)?;
        Ok(Some(SecretBox::new(value)))
    }
    fn write(&self, name: &str, value: &SecretBox) -> Result<(), StoreError> {
        let mut child = Command::new("secret-tool")
            .args([
                "store",
                "--label=Cranibox device key",
                "application",
                "cranibox",
                "identity",
                name,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(other)?;
        let encoded = Zeroizing::new(B64.encode(value.expose()));
        child
            .stdin
            .take()
            .ok_or_else(|| other("stdin"))?
            .write_all(encoded.as_bytes())
            .map_err(other)?;
        if finish(child)?.status.success() {
            Ok(())
        } else {
            Err(other("secret service unavailable"))
        }
    }
}
pub fn load_or_create(
    name: &str,
    fallback: &Path,
    system: Option<&dyn KeyStore>,
) -> Result<SecretBox, StoreError> {
    use fs2::FileExt;
    let parent = fallback.parent().ok_or_else(|| other("key directory"))?;
    fs::create_dir_all(parent).map_err(other)?;
    let mut opts = fs::OpenOptions::new();
    opts.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        opts.mode(0o600);
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).map_err(other)?;
    }
    let lock = opts.open(fallback.with_extension("lock")).map_err(other)?;
    lock.lock_exclusive().map_err(other)?;
    let marker = fallback.with_extension("backend");
    // An existing fallback remains authoritative when a secret service appears later.
    match fs::read(fallback) {
        Ok(bytes) => {
            if bytes.len() != 32 {
                return Err(other("key length"));
            }
            return Ok(SecretBox::new(bytes));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(other(e)),
    }
    let mark_system = || -> Result<(), StoreError> {
        let mut file = opts.open(&marker).map_err(other)?;
        file.write_all(b"system").map_err(other)?;
        file.sync_all().map_err(other)?;
        #[cfg(unix)]
        fs::File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(other)?;
        Ok(())
    };
    if marker.exists() {
        // A temporarily locked keychain must never cause a replacement key.
        if fs::read(&marker).map_err(other)? != b"system" {
            return Err(other("invalid key backend"));
        }
        let secret = system
            .ok_or_else(|| other("system key store unavailable"))?
            .read(name)?
            .ok_or_else(|| other("system key missing"))?;
        if secret.expose().len() != 32 {
            return Err(other("key length"));
        }
        return Ok(secret);
    }
    if let Some(system) = system
        && let Ok(Some(secret)) = system.read(name)
    {
        if secret.expose().len() == 32 {
            mark_system()?;
            return Ok(secret);
        }
        return Err(other("key length"));
    }
    let mut key = Zeroizing::new([0u8; 32]);
    rand08::rngs::OsRng.fill_bytes(key.as_mut());
    let secret = SecretBox::new(key.to_vec());
    if let Some(system) = system
        && system.write(name, &secret).is_ok()
    {
        mark_system()?;
        return Ok(secret);
    }
    let parent = fallback.parent().ok_or_else(|| other("key directory"))?;
    fs::create_dir_all(parent).map_err(other)?;
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        opts.mode(0o600);
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).map_err(other)?;
    }
    let mut file = opts.open(fallback).map_err(other)?;
    file.write_all(secret.expose()).map_err(other)?;
    file.sync_all().map_err(other)?;
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(other)?;
    Ok(secret)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Stored;
    impl KeyStore for Stored {
        fn read(&self, _: &str) -> Result<Option<SecretBox>, StoreError> {
            Ok(Some(SecretBox::new(vec![7; 32])))
        }
        fn write(&self, _: &str, _: &SecretBox) -> Result<(), StoreError> {
            panic!("must reuse stored key")
        }
    }
    #[test]
    fn system_key_is_reused_and_existing_fallback_remains_authoritative() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("key");
        assert_eq!(
            load_or_create("node", &path, Some(&Stored))
                .unwrap()
                .expose(),
            &[7; 32]
        );
        assert!(!path.exists());
        assert!(load_or_create("node", &path, None).is_err());
        assert!(!path.exists());
        assert_eq!(
            load_or_create("node", &path, Some(&Stored))
                .unwrap()
                .expose(),
            &[7; 32]
        );
        let path = root.path().join("headless-key");
        let first = load_or_create("node", &path, None).unwrap();
        assert_eq!(
            load_or_create("node", &path, Some(&Stored))
                .unwrap()
                .expose(),
            first.expose()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::write(&path, b"truncated").unwrap();
        assert!(load_or_create("node", &path, Some(&Stored)).is_err());
    }
}
