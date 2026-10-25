//! npm metadata, verified downloads, and extraction of native platform packages.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use flate2::read::GzDecoder;
use reqwest::{Client, Url, redirect::Policy};
use serde_json::Value;
use sha2::{Digest, Sha512};

#[derive(Clone, Debug)]
pub struct Release {
    pub package: String,
    pub version: String,
    pub tarball: String,
    pub integrity: String,
}

pub struct Registries {
    bases: Vec<Url>,
    client: Client,
}

impl Registries {
    pub fn new(bases: Vec<String>) -> Result<Self> {
        ensure!(!bases.is_empty(), "no npm registries configured");
        let bases = bases
            .into_iter()
            .map(|base| {
                let mut url = http_url(base.trim())?;
                ensure!(
                    url.query().is_none() && url.fragment().is_none(),
                    "registry URL must not have a query or fragment"
                );
                if !url.path().ends_with('/') {
                    url.set_path(&format!("{}/", url.path()));
                }
                Ok(url)
            })
            .collect::<Result<_>>()?;
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(300))
            // Check every hop, not just the URL supplied by the registry.
            .redirect(Policy::custom(|attempt| {
                if attempt.previous().len() > 5 {
                    attempt.error("more than five redirects")
                } else if http_url(attempt.url().as_str()).is_err() {
                    attempt.error("redirect must use HTTP(S) without credentials")
                } else {
                    attempt.follow()
                }
            }))
            // Integrity covers the tarball bytes, before any HTTP decoding.
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd()
            .build()?;
        Ok(Self { bases, client })
    }

    /// Use comma-separated `CRANIBOX_NPM_REGISTRY`, or npmmirror then npmjs.
    pub fn from_env() -> Result<Self> {
        let bases = match std::env::var("CRANIBOX_NPM_REGISTRY") {
            Ok(value) => value.split(',').map(str::to_owned).collect(),
            Err(std::env::VarError::NotPresent) => vec![
                "https://registry.npmmirror.com".into(),
                "https://registry.npmjs.org".into(),
            ],
            Err(error) => return Err(error).context("invalid CRANIBOX_NPM_REGISTRY"),
        };
        Self::new(bases)
    }

    /// Resolve an exact semver or dist-tag, trying every registry in order.
    /// Releases must have a plain semver and a valid SHA-512 SRI digest.
    pub async fn resolve(&self, package: &str, spec: &str) -> Result<Release> {
        validate_package(package)?;
        ensure!(
            plain_semver(spec)
                || (spec.starts_with(|c: char| c.is_ascii_alphabetic())
                    && spec
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))),
            "expected an exact semver or npm dist-tag"
        );
        let mut attempts = Vec::new();
        for base in &self.bases {
            // Keep the scope marker, but encode the slash as one path segment.
            let url = base.join(&package.replace('/', "%2F"))?;
            match self.resolve_at(url, package, spec).await {
                Ok(release) => return Ok(release),
                Err(error) => attempts.push(format!("{base}: {error:#}")),
            }
        }
        bail!(
            "could not resolve {package}@{spec}:\n{}",
            attempts.join("\n")
        )
    }

    /// Resolve tags without requiring a release manifest or downloading a tarball.
    /// Missing tags/packages are a successful empty result; outages remain errors.
    pub async fn tags(&self, package: &str) -> Result<super::manager::Available> {
        validate_package(package)?;
        let mut available = super::manager::Available::default();
        let mut reached = false;
        let mut errors = Vec::new();
        for base in &self.bases {
            let url = base.join(&package.replace('/', "%2F"))?;
            match self.metadata(url).await {
                Ok(metadata) => {
                    let mut tags = super::manager::Available::default();
                    let parsed = (|| -> Result<()> {
                        for (name, slot) in [
                            ("latest", &mut tags.latest),
                            ("next", &mut tags.next),
                            ("dev", &mut tags.dev),
                        ] {
                            if let Some(value) = metadata["dist-tags"].get(name) {
                                let version = value.as_str().context("invalid dist-tag")?;
                                ensure!(plain_semver(version), "invalid dist-tag version");
                                if !staging_placeholder(version) {
                                    *slot = Some(version.to_owned());
                                }
                            }
                        }
                        Ok(())
                    })();
                    if let Err(error) = parsed {
                        errors.push(format!("{base}: {error:#}"));
                        continue;
                    }
                    reached = true;
                    available.latest = available.latest.or(tags.latest);
                    available.next = available.next.or(tags.next);
                    available.dev = available.dev.or(tags.dev);
                    if available.latest.is_some()
                        && available.next.is_some()
                        && available.dev.is_some()
                    {
                        break;
                    }
                }
                Err(error)
                    if error
                        .downcast_ref::<reqwest::Error>()
                        .is_some_and(|e| e.status() == Some(reqwest::StatusCode::NOT_FOUND)) =>
                {
                    reached = true
                }
                Err(error) => errors.push(format!("{base}: {error:#}")),
            }
        }
        ensure!(reached, "could not check {package}: {}", errors.join("; "));
        Ok(available)
    }

    async fn metadata(&self, url: Url) -> Result<Value> {
        let mut response = self.client.get(url).send().await?.error_for_status()?;
        // Full packuments grow over time, but must not consume unbounded memory.
        const MAX_METADATA: usize = 16 * 1024 * 1024;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                chunk.len() <= MAX_METADATA - bytes.len(),
                "npm metadata exceeds size cap"
            );
            bytes.extend_from_slice(&chunk);
        }
        let metadata: Value = serde_json::from_slice(&bytes).context("invalid npm metadata")?;
        ensure!(metadata.is_object(), "invalid npm metadata");
        Ok(metadata)
    }

    async fn resolve_at(&self, url: Url, package: &str, spec: &str) -> Result<Release> {
        let metadata = self.metadata(url).await?;
        let version = if plain_semver(spec) {
            spec
        } else {
            metadata["dist-tags"][spec]
                .as_str()
                .with_context(|| format!("missing dist-tag {spec}"))?
        };
        ensure!(plain_semver(version), "invalid release version {version:?}");
        ensure!(
            !staging_placeholder(version),
            "no published release for {spec}"
        );
        let manifest = &metadata["versions"][version];
        ensure!(manifest.is_object(), "missing version {version}");
        ensure!(
            manifest["version"].as_str() == Some(version),
            "manifest version mismatch"
        );
        let integrity = manifest["dist"]["integrity"]
            .as_str()
            .context("missing SHA-512 dist.integrity")?;
        sha512_digests(integrity)?;
        let tarball = manifest["dist"]["tarball"]
            .as_str()
            .context("missing dist.tarball")?;
        http_url(tarball)?;
        Ok(Release {
            package: package.into(),
            version: version.into(),
            tarball: tarball.into(),
            integrity: integrity.into(),
        })
    }

    /// Stream through `dest.part`, then rename after SHA-512 verification.
    /// Existing partial files are never overwritten. Progress reports received bytes.
    pub async fn download(
        &self,
        release: &Release,
        dest: &Path,
        max_bytes: u64,
        mut progress: impl FnMut(u64, Option<u64>),
    ) -> Result<()> {
        let expected = sha512_digests(&release.integrity)?;
        let url = http_url(&release.tarball)?;
        let mut response = self.client.get(url).send().await?.error_for_status()?;
        let total = response.content_length();
        ensure!(
            total.is_none_or(|size| size <= max_bytes),
            "tarball exceeds size cap"
        );
        let dest = dest.to_owned();
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<Option<Vec<u8>>>(2);
        let writer = tokio::task::spawn_blocking(move || -> Result<()> {
            let mut part_name = dest.as_os_str().to_owned();
            part_name.push(".part");
            let part_path = PathBuf::from(part_name);
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&part_path)
                .context("create download partial file")?;
            let mut partial = PartialDownload {
                path: part_path,
                file: Some(file),
                published: false,
            };
            let mut hash = Sha512::new();
            loop {
                match receiver.blocking_recv() {
                    Some(Some(bytes)) => {
                        partial
                            .file
                            .as_mut()
                            .context("partial file closed")?
                            .write_all(&bytes)?;
                        hash.update(&bytes);
                    }
                    Some(None) => break,
                    // The network side reports the error; dropping the guard cleans up.
                    None => return Ok(()),
                }
            }
            let actual: [u8; 64] = hash.finalize().into();
            ensure!(
                expected.contains(&actual),
                "tarball SHA-512 integrity mismatch"
            );
            partial
                .file
                .as_mut()
                .context("partial file closed")?
                .sync_all()?;
            // Windows also requires the file handle to be closed before rename.
            drop(partial.file.take());
            fs::rename(&partial.path, dest).context("publish verified tarball")?;
            partial.published = true;
            Ok(())
        });
        let received = async {
            let mut done = 0_u64;
            progress(done, total);
            while let Some(chunk) = response.chunk().await? {
                ensure!(
                    chunk.len() as u64 <= max_bytes - done,
                    "tarball exceeds size cap"
                );
                done += chunk.len() as u64;
                sender
                    .send(Some(chunk.to_vec()))
                    .await
                    .context("download writer stopped")?;
                progress(done, total);
            }
            sender.send(None).await.context("download writer stopped")?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        drop(sender);
        // Wait for cleanup even when the network or size check failed.
        let written = writer.await.context("download writer panicked")?;
        written?;
        received
    }
}

struct PartialDownload {
    path: PathBuf,
    file: Option<File>,
    published: bool,
}

impl Drop for PartialDownload {
    fn drop(&mut self) {
        drop(self.file.take());
        if !self.published {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn http_url(value: &str) -> Result<Url> {
    let url = Url::parse(value).context("invalid HTTP URL")?;
    ensure!(
        matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
        "URL must use HTTP(S)"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "URL credentials are unsupported"
    );
    Ok(url)
}

fn sha512_digests(integrity: &str) -> Result<Vec<[u8; 64]>> {
    let mut digests = Vec::new();
    for token in integrity.split_whitespace() {
        if let Some(encoded) = token.strip_prefix("sha512-") {
            let digest = STANDARD
                .decode(encoded)
                .context("invalid SHA-512 SRI encoding")?;
            digests.push(
                digest
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("SHA-512 SRI must contain 64 bytes"))?,
            );
        }
    }
    ensure!(
        !digests.is_empty(),
        "dist.integrity requires SHA-512 (no SHA-1 fallback)"
    );
    Ok(digests)
}

fn validate_package(package: &str) -> Result<()> {
    let valid_part = |part: &str| {
        part.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
            && part
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._-".contains(&c))
    };
    let valid = if let Some(scoped) = package.strip_prefix('@') {
        scoped
            .split_once('/')
            .is_some_and(|(scope, name)| valid_part(scope) && valid_part(name))
    } else {
        valid_part(package)
    };
    ensure!(valid, "invalid npm package name");
    Ok(())
}

/// npm points `latest` at `0.0.0-stage` while a package's first release awaits staged approval.
fn staging_placeholder(version: &str) -> bool {
    version.starts_with("0.0.0-")
}
pub(super) fn plain_semver(version: &str) -> bool {
    let identifiers = |value: &str, numeric_zeroes: bool| {
        value.split('.').all(|part| {
            !part.is_empty()
                && part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
                && (numeric_zeroes
                    || part.len() == 1
                    || !part.starts_with('0')
                    || !part.bytes().all(|c| c.is_ascii_digit()))
        })
    };
    let (version, build) = version
        .split_once('+')
        .map_or((version, None), |(v, b)| (v, Some(b)));
    if build.is_some_and(|build| !identifiers(build, true)) {
        return false;
    }
    let (core, pre) = version
        .split_once('-')
        .map_or((version, None), |(v, p)| (v, Some(p)));
    if pre.is_some_and(|pre| !identifiers(pre, false)) {
        return false;
    }
    let parts: Vec<_> = core.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|c| c.is_ascii_digit())
                && (part.len() == 1 || !part.starts_with('0'))
        })
}

/// Select the package supported by the release matrix, rejecting musl Linux.
pub fn platform_package(entry: &str) -> Result<String> {
    let libc = if cfg!(target_env = "gnu") {
        "gnu"
    } else if cfg!(target_env = "musl") {
        "musl"
    } else {
        ""
    };
    platform_package_for(entry, std::env::consts::OS, std::env::consts::ARCH, libc)
}

fn platform_package_for(entry: &str, os: &str, arch: &str, libc: &str) -> Result<String> {
    validate_package(entry)?;
    if os == "linux" {
        ensure!(
            libc == "gnu",
            "Cranibox requires glibc Linux; musl/Alpine and other libc implementations are unsupported"
        );
    }
    let suffix = match (os, arch) {
        ("macos", "aarch64") => "darwin-arm64",
        ("linux", "x86_64") => "linux-x64-gnu",
        ("windows", "x86_64") => "win32-x64",
        _ => bail!("unsupported Cranibox npm platform: {os}/{arch}/{libc}"),
    };
    Ok(format!("{entry}-{suffix}"))
}

/// Native executable path after stripping the npm `package/` prefix.
pub fn executable_relative_path() -> &'static str {
    if cfg!(windows) {
        "bin/cranibox.exe"
    } else {
        "bin/cranibox"
    }
}

/// Extract regular files and directories into a new destination atomically.
/// `max_files` counts all entries, including directories; returned paths include
/// explicit directories, except the stripped package root. Extended tar headers
/// are rejected, so they cannot bypass path validation or allocation limits.
pub fn extract(tgz: &Path, dest: &Path, max_files: usize, max_bytes: u64) -> Result<Vec<PathBuf>> {
    ensure_missing(dest)?;
    let parent = dest
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temp = tempfile::Builder::new()
        .prefix(".cranibox-npm-")
        .tempdir_in(parent)?;
    // Bound headers, padding, and trailing data as well as file contents.
    let budget = max_bytes
        .saturating_add((max_files as u64).saturating_mul(1024))
        .saturating_add(10240);
    let input = GzDecoder::new(File::open(tgz)?).take(budget.saturating_add(1));
    let mut archive = tar::Archive::new(input);
    let mut paths = Vec::new();
    let mut total = 0_u64;
    for (index, entry) in archive.entries()?.raw(true).enumerate() {
        ensure!(index < max_files, "tarball exceeds file-count cap");
        let mut entry = entry.context("invalid tar entry")?;
        let kind = entry.header().entry_type();
        ensure!(
            kind.is_file() || kind.is_dir(),
            "tarball contains a link or unsupported entry type"
        );
        let path = safe_relative_path(&entry.path_bytes())?;
        let size = entry.size();
        ensure!(
            size <= max_bytes - total,
            "tarball exceeds uncompressed size cap"
        );
        total += size;
        if kind.is_dir() {
            ensure!(size == 0, "tar directory contains data");
            if path.as_os_str().is_empty() {
                continue;
            }
            fs::create_dir_all(temp.path().join(&path))?;
        } else {
            ensure!(
                !path.as_os_str().is_empty(),
                "package root must be a directory"
            );
            let target = temp.path().join(&path);
            fs::create_dir_all(target.parent().context("missing file parent")?)?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)?;
            ensure!(
                io::copy(&mut entry, &mut file)? == size,
                "truncated tar entry"
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = if entry.header().mode()? & 0o111 != 0 {
                    0o755
                } else {
                    0o644
                };
                file.set_permissions(fs::Permissions::from_mode(mode))?;
            }
        }
        paths.push(path);
    }
    // Read through the gzip trailer to verify its checksum, even after tar EOF.
    let mut input = archive.into_inner();
    io::copy(&mut input, &mut io::sink())?;
    ensure!(input.limit() != 0, "tarball exceeds archive size cap");
    ensure_missing(dest)?;
    fs::rename(temp.path(), dest).context("publish extracted npm package")?;
    let _ = temp.keep();
    Ok(paths)
}

fn ensure_missing(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
        Ok(_) => bail!("destination already exists: {}", path.display()),
    }
}

fn safe_relative_path(raw: &[u8]) -> Result<PathBuf> {
    let raw = std::str::from_utf8(raw).context("tar path is not UTF-8")?;
    let raw = raw.strip_suffix('/').unwrap_or(raw);
    let mut parts = raw.split('/');
    ensure!(
        parts.next() == Some("package"),
        "tar path must start with package/"
    );
    let mut path = PathBuf::new();
    for part in parts {
        ensure!(
            !part.is_empty() && part != "." && part != "..",
            "unsafe tar path component"
        );
        // Apply Windows rules on every host: no drive paths, ADS, or aliases.
        ensure!(
            !part
                .chars()
                .any(|c| c.is_control() || "\\:<>\"|?*".contains(c))
                && !part.ends_with(['.', ' ']),
            "unsafe tar path component"
        );
        let stem = part
            .split('.')
            .next()
            .unwrap_or(part)
            .trim_end_matches(' ')
            .to_ascii_uppercase();
        ensure!(
            !matches!(
                stem.as_str(),
                "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
            ) && !["COM", "LPT"]
                .iter()
                .any(|prefix| stem.strip_prefix(prefix).is_some_and(|suffix| {
                    matches!(
                        suffix,
                        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                    )
                })),
            "reserved Windows tar path"
        );
        path.push(part);
    }
    Ok(path)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use axum::{
        Router,
        body::Body,
        http::{Response, StatusCode, Uri},
        response::IntoResponse,
    };
    use flate2::{Compression, write::GzEncoder};
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        task::JoinHandle,
    };

    const PACKAGE: &str = "@fliaping/cranibox-linux-x64-gnu";

    pub(crate) struct Server {
        pub(crate) base: String,
        task: JoinHandle<()>,
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn serve(router: Router) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Server { base, task }
    }

    pub(crate) async fn serve_body(status: StatusCode, body: Vec<u8>) -> Server {
        serve(Router::new().fallback(move || {
            let body = body.clone();
            async move { (status, body) }
        }))
        .await
    }

    async fn serve_raw(response: Vec<u8>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                if stream.read_exact(&mut byte).await.is_err() {
                    return;
                }
                request.push(byte[0]);
            }
            let _ = stream.write_all(&response).await;
            let _ = stream.shutdown().await;
        });
        Server { base, task }
    }

    fn scratch() -> tempfile::TempDir {
        // Keep all test writes in the authorized target directory or worktree.
        let parent = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        tempfile::tempdir_in(parent).unwrap()
    }

    fn sri(bytes: &[u8]) -> String {
        format!("sha512-{}", STANDARD.encode(Sha512::digest(bytes)))
    }

    pub(crate) fn packument() -> Value {
        json!({
            "dist-tags": {"latest": "1.2.3", "next": "1.2.3", "dev": "1.2.3"},
            "versions": {"1.2.3": {
                "name": PACKAGE, "version": "1.2.3",
                "dist": {"tarball": "http://127.0.0.1/package.tgz", "integrity": sri(b"package")}
            }}
        })
    }

    fn release(url: &str, bytes: &[u8]) -> Release {
        Release {
            package: PACKAGE.into(),
            version: "1.2.3".into(),
            tarball: url.into(),
            integrity: sri(bytes),
        }
    }

    #[tokio::test]
    async fn available_tags_absent_fallback_and_unreachable() {
        use crate::update::manager::Updater;
        let root = scratch();
        let full = serve_body(StatusCode::OK, serde_json::to_vec(&packument()).unwrap()).await;
        let mut metadata = packument();
        // npm's placeholder for a package whose first release awaits staged approval.
        metadata["dist-tags"] = json!({"dev":"1.1.1-dev.7","latest":"0.0.0-stage"});
        // Tag lookup needs only metadata; manifests and tarballs are not fetched.
        metadata["versions"] = json!({});
        let partial = serve_body(StatusCode::OK, serde_json::to_vec(&metadata).unwrap()).await;
        let absent = serve_body(StatusCode::OK, br#"{"dist-tags":{}}"#.to_vec()).await;
        let missing = serve_body(StatusCode::NOT_FOUND, Vec::new()).await;
        let down = serve_body(StatusCode::SERVICE_UNAVAILABLE, Vec::new()).await;
        for (bases, expected) in [
            (
                vec![full.base.clone()],
                json!({"latest":"1.2.3","next":"1.2.3","dev":"1.2.3"}),
            ),
            (
                vec![partial.base.clone()],
                json!({"latest":null,"next":null,"dev":"1.1.1-dev.7"}),
            ),
            (
                vec![absent.base.clone()],
                json!({"latest":null,"next":null,"dev":null}),
            ),
            (
                vec![missing.base.clone()],
                json!({"latest":null,"next":null,"dev":null}),
            ),
            (
                vec![down.base.clone(), partial.base.clone(), full.base.clone()],
                json!({"latest":"1.2.3","next":"1.2.3","dev":"1.1.1-dev.7"}),
            ),
        ] {
            let updater = Updater::new(
                root.path().into(),
                Registries::new(bases).unwrap(),
                "@fliaping/cranibox".into(),
            );
            assert_eq!(
                serde_json::to_value(updater.available().await.unwrap()).unwrap(),
                expected
            );
        }
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let closed = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let updater = Updater::new(
            root.path().into(),
            Registries::new(vec![closed, down.base.clone()]).unwrap(),
            "@fliaping/cranibox".into(),
        );
        assert!(updater.available().await.is_err());
        let status = updater
            .snapshot("self", "1.1.0".into(), true)
            .await
            .unwrap();
        assert!(status["available"].is_null());
        assert!(
            status["available_error"]
                .as_str()
                .unwrap()
                .contains("could not check")
        );
        assert_eq!(status["version"], "1.1.0");
    }

    // Set the raw name field so hostile paths aren't sanitized by the builder.
    fn tarball(entries: &[(&str, u8, u32, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
        for &(path, kind, mode, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_mode(mode);
            header.set_size(data.len() as u64);
            header.set_entry_type(tar::EntryType::new(kind));
            assert!(path.len() <= 100);
            header.as_mut_bytes()[..path.len()].copy_from_slice(path.as_bytes());
            if kind == b'1' || kind == b'2' {
                header.set_link_name("../outside").unwrap();
            }
            header.set_cksum();
            builder.append(&header, data).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    fn sample_tarball() -> Vec<u8> {
        tarball(&[
            ("package/", b'5', 0o755, b""),
            ("package/bin/", b'5', 0o755, b""),
            ("package/bin/cranibox", b'0', 0o6755, b"native executable"),
            ("package/package.json", b'0', 0o666, b"{}"),
        ])
    }

    #[tokio::test]
    async fn resolve_exact_tags_and_scoped_encoding() {
        let paths = Arc::new(Mutex::new(Vec::new()));
        let seen = paths.clone();
        let server = serve(Router::new().fallback(move |uri: Uri| {
            seen.lock().unwrap().push(uri.path().to_owned());
            async { axum::Json(packument()) }
        }))
        .await;
        let registries = Registries::new(vec![format!("{}/registry", server.base)]).unwrap();
        for spec in ["1.2.3", "latest", "next", "dev"] {
            let result = registries.resolve(PACKAGE, spec).await.unwrap();
            assert_eq!(result.package, PACKAGE);
            assert_eq!(result.version, "1.2.3");
            assert_eq!(result.integrity, sri(b"package"));
        }
        assert_eq!(
            *paths.lock().unwrap(),
            vec!["/registry/@fliaping%2Fcranibox-linux-x64-gnu"; 4]
        );
    }

    #[tokio::test]
    async fn resolve_rejects_missing_or_invalid_integrity() {
        for integrity in [
            Value::Null,
            json!("sha1-abc"),
            json!("sha512-invalid"),
            json!("sha512-YQ=="),
        ] {
            let mut metadata = packument();
            metadata["versions"]["1.2.3"]["dist"]["integrity"] = integrity;
            let server = serve_body(StatusCode::OK, serde_json::to_vec(&metadata).unwrap()).await;
            let error = Registries::new(vec![server.base.clone()])
                .unwrap()
                .resolve(PACKAGE, "latest")
                .await
                .unwrap_err();
            assert!(error.to_string().contains("SHA-512"));
        }
    }

    #[tokio::test]
    async fn resolve_falls_back_after_404_or_connection_failure() {
        let missing = serve_body(StatusCode::NOT_FOUND, Vec::new()).await;
        let good = serve_body(StatusCode::OK, serde_json::to_vec(&packument()).unwrap()).await;
        let unavailable = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let down = format!("http://{}", unavailable.local_addr().unwrap());
        drop(unavailable);
        for first in [missing.base.clone(), down] {
            let registries = Registries::new(vec![first, good.base.clone()]).unwrap();
            assert_eq!(
                registries.resolve(PACKAGE, "latest").await.unwrap().version,
                "1.2.3"
            );
        }
    }

    #[tokio::test]
    async fn resolve_falls_back_for_mirror_sync_lag() {
        let good = serve_body(StatusCode::OK, serde_json::to_vec(&packument()).unwrap()).await;
        for spec in ["1.2.3", "latest"] {
            let mut stale = packument();
            if spec == "latest" {
                stale["dist-tags"] = json!({});
            } else {
                stale["versions"] = json!({});
            }
            let mirror = serve_body(StatusCode::OK, serde_json::to_vec(&stale).unwrap()).await;
            let registries = Registries::new(vec![mirror.base.clone(), good.base.clone()]).unwrap();
            assert_eq!(
                registries.resolve(PACKAGE, spec).await.unwrap().version,
                "1.2.3"
            );
        }
    }

    #[tokio::test]
    async fn resolve_reports_every_failed_registry() {
        let first = serve_body(StatusCode::NOT_FOUND, Vec::new()).await;
        let second = serve_body(StatusCode::OK, b"{}".to_vec()).await;
        let registries = Registries::new(vec![first.base.clone(), second.base.clone()]).unwrap();
        let error = registries
            .resolve(PACKAGE, "latest")
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(&first.base) && error.contains(&second.base));
        assert!(error.contains("404") && error.contains("missing dist-tag"));
    }

    #[tokio::test]
    async fn resolve_rejects_unsafe_versions_and_manifest_mismatch() {
        for version in ["../evil", "1.2.3/evil", "1.2.3\\evil", "01.2.3", "1.2.3-01"] {
            let mut metadata = packument();
            metadata["dist-tags"]["latest"] = json!(version);
            let server = serve_body(StatusCode::OK, serde_json::to_vec(&metadata).unwrap()).await;
            assert!(
                Registries::new(vec![server.base.clone()])
                    .unwrap()
                    .resolve(PACKAGE, "latest")
                    .await
                    .is_err()
            );
        }
        let mut metadata = packument();
        metadata["versions"]["1.2.3"]["version"] = json!("1.2.4");
        let server = serve_body(StatusCode::OK, serde_json::to_vec(&metadata).unwrap()).await;
        assert!(
            Registries::new(vec![server.base.clone()])
                .unwrap()
                .resolve(PACKAGE, "1.2.3")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn download_verifies_tarball_and_reports_progress() {
        let bytes = sample_tarball();
        let server = serve_body(StatusCode::OK, bytes.clone()).await;
        let registries = Registries::new(vec![server.base.clone()]).unwrap();
        let root = scratch();
        let dest = root.path().join("download.tgz");
        let mut progress = Vec::new();
        registries
            .download(
                &release(&server.base, &bytes),
                &dest,
                bytes.len() as u64,
                |done, total| progress.push((done, total)),
            )
            .await
            .unwrap();
        assert_eq!(fs::read(&dest).unwrap(), bytes);
        assert!(!root.path().join("download.tgz.part").exists());
        assert_eq!(progress.first(), Some(&(0, Some(bytes.len() as u64))));
        assert_eq!(
            progress.last(),
            Some(&(bytes.len() as u64, Some(bytes.len() as u64)))
        );
        assert!(progress.windows(2).all(|pair| pair[0].0 <= pair[1].0));
    }

    #[tokio::test]
    async fn download_integrity_mismatch_removes_partial_and_preserves_dest() {
        let bytes = sample_tarball();
        let server = serve_body(StatusCode::OK, bytes).await;
        let root = scratch();
        let dest = root.path().join("download.tgz");
        fs::write(&dest, b"previous").unwrap();
        let error = Registries::new(vec![server.base.clone()])
            .unwrap()
            .download(&release(&server.base, b"wrong"), &dest, 10000, |_, _| {})
            .await
            .unwrap_err();
        assert!(error.to_string().contains("integrity mismatch"));
        assert_eq!(fs::read(dest).unwrap(), b"previous");
        assert!(!root.path().join("download.tgz.part").exists());
    }

    #[tokio::test]
    async fn download_size_cap_checks_content_length() {
        let server = serve_body(StatusCode::OK, vec![0; 20]).await;
        let root = scratch();
        let error = Registries::new(vec![server.base.clone()])
            .unwrap()
            .download(
                &release(&server.base, &[0; 20]),
                &root.path().join("download"),
                10,
                |_, _| {},
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("size cap"));
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn download_size_cap_checks_chunked_body_despite_lying_content_length() {
        // Transfer-Encoding defines the body framing; Content-Length is a lie.
        let response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 1\r\nConnection: close\r\n\r\n14\r\n01234567890123456789\r\n0\r\n\r\n";
        let server = serve_raw(response.to_vec()).await;
        let root = scratch();
        let error = Registries::new(vec![server.base.clone()])
            .unwrap()
            .download(
                &release(&server.base, b"01234567890123456789"),
                &root.path().join("download"),
                10,
                |_, _| {},
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("size cap"), "{error:#}");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn download_size_cap_checks_body_without_content_length() {
        let server =
            serve_raw(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n01234567890123456789".to_vec())
                .await;
        let root = scratch();
        let error = Registries::new(vec![server.base.clone()])
            .unwrap()
            .download(
                &release(&server.base, b"01234567890123456789"),
                &root.path().join("download"),
                10,
                |_, _| {},
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("size cap"));
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn download_network_failure_cleans_partial() {
        let server = serve_raw(
            b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nshort".to_vec(),
        )
        .await;
        let root = scratch();
        assert!(
            Registries::new(vec![server.base.clone()])
                .unwrap()
                .download(
                    &release(&server.base, b"short"),
                    &root.path().join("download"),
                    100,
                    |_, _| {}
                )
                .await
                .is_err()
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn download_preserves_existing_partial() {
        let server = serve_body(StatusCode::OK, b"data".to_vec()).await;
        let root = scratch();
        fs::write(root.path().join("download.part"), b"other download").unwrap();
        let error = Registries::new(vec![server.base.clone()])
            .unwrap()
            .download(
                &release(&server.base, b"data"),
                &root.path().join("download"),
                100,
                |_, _| {},
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("create download partial file"));
        assert_eq!(
            fs::read(root.path().join("download.part")).unwrap(),
            b"other download"
        );
        assert!(!root.path().join("download").exists());
    }

    #[tokio::test]
    async fn download_redirect_limit_and_scheme() {
        let server = serve(Router::new().fallback(|uri: Uri| async move {
            let count: usize = uri.path().trim_start_matches('/').parse().unwrap();
            if count == 0 {
                b"data".to_vec().into_response()
            } else {
                Response::builder()
                    .status(StatusCode::FOUND)
                    .header("location", format!("/{}", count - 1))
                    .body(Body::empty())
                    .unwrap()
            }
        }))
        .await;
        let root = scratch();
        let registries = Registries::new(vec![server.base.clone()]).unwrap();
        registries
            .download(
                &release(&format!("{}/5", server.base), b"data"),
                &root.path().join("five"),
                100,
                |_, _| {},
            )
            .await
            .unwrap();
        assert!(
            registries
                .download(
                    &release(&format!("{}/6", server.base), b"data"),
                    &root.path().join("six"),
                    100,
                    |_, _| {}
                )
                .await
                .is_err()
        );
        let bad = serve(Router::new().fallback(|| async {
            Response::builder()
                .status(StatusCode::FOUND)
                .header("location", "file:///outside")
                .body(Body::empty())
                .unwrap()
        }))
        .await;
        assert!(
            registries
                .download(
                    &release(&bad.base, b"data"),
                    &root.path().join("bad"),
                    100,
                    |_, _| {}
                )
                .await
                .is_err()
        );
        for url in ["file:///outside", "ftp://127.0.0.1/file"] {
            assert!(
                registries
                    .download(
                        &release(url, b"data"),
                        &root.path().join("bad"),
                        100,
                        |_, _| {}
                    )
                    .await
                    .is_err()
            );
        }
    }

    #[test]
    fn extraction_happy_path_and_permissions() {
        let root = scratch();
        let tgz = root.path().join("package.tgz");
        fs::write(&tgz, sample_tarball()).unwrap();
        let dest = root.path().join("extracted");
        let paths = extract(&tgz, &dest, 4, 19).unwrap();
        assert_eq!(
            paths,
            ["bin", "bin/cranibox", "package.json"].map(PathBuf::from)
        );
        assert_eq!(
            fs::read(dest.join("bin/cranibox")).unwrap(),
            b"native executable"
        );
        assert_eq!(fs::read(dest.join("package.json")).unwrap(), b"{}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(dest.join("bin/cranibox"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o7777,
                0o755
            );
            assert_eq!(
                fs::metadata(dest.join("package.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o7777,
                0o644
            );
        }
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    }

    fn assert_extraction_rejected(bytes: &[u8], max_files: usize, max_bytes: u64) -> String {
        let root = scratch();
        let tgz = root.path().join("package.tgz");
        fs::write(&tgz, bytes).unwrap();
        let dest = root.path().join("extracted");
        let error = extract(&tgz, &dest, max_files, max_bytes).unwrap_err();
        assert!(!dest.exists());
        assert_eq!(
            fs::read_dir(root.path()).unwrap().count(),
            1,
            "temp directory leaked"
        );
        error.to_string()
    }

    #[test]
    fn extraction_rejects_traversal_absolute_and_windows_paths() {
        for path in [
            "../evil",
            "package/../evil",
            "/evil",
            "/package/evil",
            "package/a/../../evil",
            "package/./evil",
            "package//evil",
            "C:/evil",
            "package/C:/evil",
            "package/..\\evil",
            "package/a:stream",
            "package/CON",
            "package/con .txt",
            "package/CONIN$",
            "package/CONOUT$",
            "package/nul.txt",
            "package/COM1",
            "package/LPT¹",
            "package/file.",
            "package/file ",
            "other/file",
        ] {
            assert_extraction_rejected(&tarball(&[(path, b'0', 0o644, b"evil")]), 10, 100);
        }
    }

    #[test]
    fn extraction_rejects_links_devices_fifos_and_extended_headers() {
        for kind in *b"12346xgLKS" {
            assert_extraction_rejected(&tarball(&[("package/link", kind, 0o644, b"")]), 10, 100);
        }
    }

    #[test]
    fn extraction_enforces_file_count_and_uncompressed_size() {
        assert!(assert_extraction_rejected(&sample_tarball(), 3, 100).contains("file-count cap"));
        assert!(
            assert_extraction_rejected(&sample_tarball(), 4, 18).contains("uncompressed size cap")
        );
        assert!(assert_extraction_rejected(&sample_tarball(), 0, 100).contains("file-count cap"));
    }

    #[test]
    fn extraction_rejects_existing_destination() {
        let root = scratch();
        let tgz = root.path().join("package.tgz");
        fs::write(&tgz, sample_tarball()).unwrap();
        let dest = root.path().join("extracted");
        fs::create_dir(&dest).unwrap();
        assert!(
            extract(&tgz, &dest, 10, 100)
                .unwrap_err()
                .to_string()
                .contains("already exists")
        );
        fs::write(dest.join("keep"), b"existing").unwrap();
        assert!(extract(&tgz, &dest, 10, 100).is_err());
        assert_eq!(fs::read(dest.join("keep")).unwrap(), b"existing");
    }

    #[cfg(unix)]
    #[test]
    fn extraction_rejects_dangling_destination_symlink() {
        let root = scratch();
        let tgz = root.path().join("package.tgz");
        fs::write(&tgz, sample_tarball()).unwrap();
        let dest = root.path().join("extracted");
        std::os::unix::fs::symlink(root.path().join("missing"), &dest).unwrap();
        assert!(extract(&tgz, &dest, 10, 100).is_err());
        assert!(fs::symlink_metadata(dest).unwrap().is_symlink());
    }

    #[test]
    fn extraction_cleans_up_after_duplicate_and_corrupt_gzip() {
        let duplicates = tarball(&[
            ("package/a", b'0', 0o644, b"a"),
            ("package/a", b'0', 0o644, b"b"),
        ]);
        assert_extraction_rejected(&duplicates, 10, 100);
        let mut corrupt = sample_tarball();
        let trailer = corrupt.len() - 8;
        corrupt[trailer] ^= 1;
        assert_extraction_rejected(&corrupt, 10, 100);
        assert_extraction_rejected(&sample_tarball()[..20], 10, 100);
    }

    #[test]
    fn extraction_caps_archive_padding() {
        let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
        gzip.write_all(&[0; 65536]).unwrap();
        assert!(
            assert_extraction_rejected(&gzip.finish().unwrap(), 10, 100)
                .contains("archive size cap")
        );
    }

    #[test]
    fn platform_mapping_matches_release_matrix() {
        for (os, arch, libc, suffix) in [
            ("macos", "aarch64", "", "darwin-arm64"),
            ("linux", "x86_64", "gnu", "linux-x64-gnu"),
            ("windows", "x86_64", "", "win32-x64"),
        ] {
            assert_eq!(
                platform_package_for("@fliaping/cranibox", os, arch, libc).unwrap(),
                format!("@fliaping/cranibox-{suffix}")
            );
        }
        for (os, arch, libc) in [
            ("linux", "x86_64", "musl"),
            ("linux", "aarch64", "gnu"),
            ("macos", "x86_64", ""),
            ("windows", "aarch64", ""),
            ("freebsd", "x86_64", ""),
        ] {
            assert!(platform_package_for("@fliaping/cranibox", os, arch, libc).is_err());
        }
        assert_eq!(
            executable_relative_path(),
            if cfg!(windows) {
                "bin/cranibox.exe"
            } else {
                "bin/cranibox"
            }
        );
        #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
        assert_eq!(platform_package("@fliaping/cranibox").unwrap(), PACKAGE);
    }

    #[test]
    fn strict_semver_and_registry_validation() {
        for version in [
            "0.0.0",
            "1.2.3",
            "1.2.3-dev.0",
            "1.2.3+build.01",
            "1.2.3-rc.1+build",
        ] {
            assert!(plain_semver(version), "{version}");
        }
        for version in [
            "1.2",
            "v1.2.3",
            "1.2.3.4",
            "1.2.3-",
            "1.2.3-01",
            "1.2.3+",
            "1.2.3+a+b",
            "1.2.3/evil",
            "1.2.3 ",
        ] {
            assert!(!plain_semver(version), "{version}");
        }
        assert!(Registries::new(vec![]).is_err());
        for url in [
            "",
            "file:///registry",
            "ftp://127.0.0.1",
            "http://127.0.0.1/?q=a",
            "http://127.0.0.1/#fragment",
        ] {
            assert!(Registries::new(vec![url.into()]).is_err());
        }
        for package in [
            "../evil",
            "@scope/../evil",
            "@scope/name?query",
            "@scope/name/other",
        ] {
            assert!(validate_package(package).is_err());
        }
    }
}
