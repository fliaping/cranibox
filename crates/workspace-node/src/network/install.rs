//! Install only the two upstream binaries, from pinned HTTPS assets and SHA-256s.
use anyhow::{Context, Result, ensure};
use futures_util::StreamExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

pub(super) const VERSION: &str = "2.6.4";
const MAX_ARCHIVE: usize = 64 * 1024 * 1024;
const MAX_BINARY: u64 = 128 * 1024 * 1024;

fn asset() -> Result<(&'static str, &'static str)> {
    Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => (
            "linux-x86_64",
            "61b659eaedba658fa66fe47d17e1426cdd77e5d02fa15fed447bb4357c09dfd6",
        ),
        ("linux", "aarch64") => (
            "linux-aarch64",
            "f533ec25a7ea714e09f645615012200278058525795cc3bb690ff011aec1a70f",
        ),
        ("macos", "x86_64") => (
            "macos-x86_64",
            "89fc28a6e6995259d76ce3f11775220e8a21c760e94df91a6a9db30a69b6982e",
        ),
        ("macos", "aarch64") => (
            "macos-aarch64",
            "4be1882d1aa36d31c1d6ba0596f2cf8a097e371f8da124212324b2e0f8df7e4b",
        ),
        ("windows", "x86_64") => (
            "windows-x86_64",
            "27af91e270e554709b048bd32327fefd2dfce5062ae1e8701af7550c6f525f84",
        ),
        ("windows", "aarch64") => (
            "windows-arm64",
            "37023f8a3451c9234b17ee2089a03dc344ce90d803b5b359cb6c46682b0549b4",
        ),
        _ => anyhow::bail!(
            "Automatic EasyTier installation supports Windows, macOS and Linux on x86_64 or ARM64."
        ),
    })
}
pub(super) fn directory(peers: &Path) -> PathBuf {
    peers
        .with_file_name("tools")
        .join(format!("easytier-{VERSION}"))
}
fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}
pub(super) fn binaries(peers: &Path) -> Option<(PathBuf, PathBuf)> {
    let root = directory(peers);
    let core = root.join(exe("easytier-core"));
    let cli = root.join(exe("easytier-cli"));
    (core.is_file() && cli.is_file()).then_some((core, cli))
}
pub(super) fn plan(peers: &Path) -> Result<Value> {
    let (platform, sha256) = asset()?;
    Ok(
        json!({"provider":"easytier","version":VERSION,"platform":platform,
        "url":format!("https://github.com/EasyTier/EasyTier/releases/download/v{VERSION}/easytier-{platform}-v{VERSION}.zip"),
        "sha256":sha256,"directory":directory(peers),"requires_admin":false,
        "changes":["Download and verify the pinned upstream release","Install easytier-core and easytier-cli in this node's private tools directory"],
        "network_mode":"userspace","system_routes":false,"system_dns":false}),
    )
}
struct Stage(PathBuf);
impl Drop for Stage {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
pub(super) async fn install(peers: PathBuf) -> Result<()> {
    let plan = plan(&peers)?;
    let client = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .user_agent("cranibox-node")
        .build()?;
    let response = client
        .get(plan["url"].as_str().unwrap())
        .send()
        .await
        .context("Download EasyTier from GitHub")?;
    ensure!(
        response.status().is_success(),
        "EasyTier download failed (HTTP {}).",
        response.status()
    );
    ensure!(
        response
            .content_length()
            .is_none_or(|size| size <= MAX_ARCHIVE as u64),
        "EasyTier archive exceeds the download limit."
    );
    let mut data = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("Read EasyTier download")?;
        ensure!(
            data.len().saturating_add(chunk.len()) <= MAX_ARCHIVE,
            "EasyTier archive exceeds the download limit."
        );
        data.extend_from_slice(&chunk);
    }
    let expected = plan["sha256"].as_str().unwrap().to_owned();
    tokio::task::spawn_blocking(move || verified_extract(&peers, data, &expected)).await?
}
fn verified_extract(peers: &Path, data: Vec<u8>, expected: &str) -> Result<()> {
    ensure!(
        format!("{:x}", Sha256::digest(&data)) == expected,
        "EasyTier checksum mismatch; nothing was installed."
    );
    extract(peers, data)
}
fn extract(peers: &Path, data: Vec<u8>) -> Result<()> {
    let dest = directory(peers);
    ensure!(
        !dest.try_exists()?,
        "An EasyTier installation already exists; preserve it before reinstalling."
    );
    let parent = dest.parent().unwrap();
    std::fs::create_dir_all(parent)?;
    let stage = Stage(parent.join(format!(
        ".easytier-install-{}",
        uuid::Uuid::new_v4().simple()
    )));
    std::fs::create_dir(&stage.0)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stage.0, std::fs::Permissions::from_mode(0o700))?;
    }
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(data)).context("Read EasyTier ZIP archive")?;
    for binary in [exe("easytier-core"), exe("easytier-cli")] {
        let matches: Vec<_> = archive
            .file_names()
            .filter(|name| {
                Path::new(name)
                    .file_name()
                    .is_some_and(|n| n == binary.as_str())
            })
            .map(str::to_owned)
            .collect();
        ensure!(
            matches.len() == 1,
            "EasyTier archive must contain exactly one {binary}."
        );
        let mut source = archive.by_name(&matches[0])?;
        ensure!(
            !source.is_dir() && source.size() > 0 && source.size() <= MAX_BINARY,
            "Invalid EasyTier binary size."
        );
        // Select an explicit destination; never extract archive paths or symlinks.
        let mut output = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(stage.0.join(&binary))?;
        let copied = std::io::copy(&mut source.by_ref().take(MAX_BINARY + 1), &mut output)?;
        ensure!(
            copied == source.size(),
            "EasyTier binary is truncated or oversized."
        );
        output.flush()?;
        output.sync_all()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                stage.0.join(&binary),
                std::fs::Permissions::from_mode(0o700),
            )?;
        }
    }
    std::fs::rename(&stage.0, dest).context("Publish verified EasyTier installation")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn archive(names: &[String]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for name in names {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"verified binary fixture").unwrap();
        }
        zip.finish().unwrap().into_inner()
    }
    fn publish(peers: &Path, bytes: Vec<u8>) -> Result<()> {
        let expected = format!("{:x}", Sha256::digest(&bytes));
        verified_extract(peers, bytes, &expected)
    }
    #[test]
    fn checksum_failure_never_creates_an_installation() {
        let root = tempfile::tempdir().unwrap();
        let peers = root.path().join("peers.json");
        assert!(
            verified_extract(
                &peers,
                archive(&[exe("easytier-core"), exe("easytier-cli")]),
                &"0".repeat(64)
            )
            .is_err()
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
    #[test]
    fn incomplete_or_ambiguous_archive_is_never_published() {
        for names in [
            vec![exe("easytier-core")],
            vec![
                format!("one/{}", exe("easytier-core")),
                format!("two/{}", exe("easytier-core")),
                exe("easytier-cli"),
            ],
        ] {
            let root = tempfile::tempdir().unwrap();
            let peers = root.path().join("peers.json");
            assert!(publish(&peers, archive(&names)).is_err());
            assert!(!directory(&peers).exists());
            assert_eq!(
                std::fs::read_dir(root.path().join("tools"))
                    .unwrap()
                    .count(),
                0
            );
        }
    }
    #[test]
    fn archive_paths_cannot_escape_and_existing_installations_are_preserved() {
        let root = tempfile::tempdir().unwrap();
        let peers = root.path().join("peers.json");
        let names = [
            format!("../../{}", exe("easytier-core")),
            format!("upstream/{}", exe("easytier-cli")),
            "../../UNRELATED".into(),
        ];
        publish(&peers, archive(&names)).unwrap();
        let core = directory(&peers).join(exe("easytier-core"));
        assert_eq!(std::fs::read(core).unwrap(), b"verified binary fixture");
        assert!(!root.path().join("UNRELATED").exists());
        assert!(publish(&peers, archive(&names)).is_err());
        assert_eq!(std::fs::read_dir(directory(&peers)).unwrap().count(), 2);
    }
}
