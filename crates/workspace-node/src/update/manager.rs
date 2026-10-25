//! Persisted update pipeline. The launcher owns restart and recovery supervision.

use std::{
    fs::{self, File, OpenOptions},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use super::{
    layout::{Layout, PointerState},
    npm::{self, Registries},
    platform,
    version::Version,
};

const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;
const MAX_EXTRACTED: u64 = 2 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 10_000;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Resolving,
    Downloading,
    Verifying,
    Staged,
    Activated,
    Done,
    Failed,
    RolledBack,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Status {
    pub id: String,
    pub spec: String,
    pub phase: Phase,
    pub from: Option<String>,
    pub to: Option<String>,
    pub error: Option<String>,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub started_at: u64,
    pub updated_at: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct RuntimeInfo {
    pub version: Option<String>,
    pub os: &'static str,
    pub arch: &'static str,
    pub managed: bool,
    pub state: Option<String>,
    pub can_update: bool,
    pub reasons: Vec<String>,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Available {
    pub latest: Option<String>,
    pub next: Option<String>,
    pub dev: Option<String>,
}

pub struct Updater {
    root: PathBuf,
    terminal_state: Option<PathBuf>,
    registries: Registries,
    entry_package: String,
}

struct UpdateLock(File);

impl Drop for UpdateLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

// Also remove staging when an apply future is dropped before activation.
struct Stage<'a> {
    layout: &'a Layout,
    version: &'a Version,
}

impl Drop for Stage<'_> {
    fn drop(&mut self) {
        self.layout.abort_stage(self.version);
    }
}

impl Updater {
    pub async fn available(&self) -> Result<Available> {
        self.registries
            .tags(&npm::platform_package(&self.entry_package)?)
            .await
    }

    /// Shared CLI, mesh and app status, with registry failures kept separate.
    pub async fn snapshot(
        &self,
        node: &str,
        version: String,
        check: bool,
    ) -> Result<serde_json::Value> {
        let mut value = serde_json::json!({
            "node": node, "version": version, "runtime": self.info()?, "status": self.status()?,
            "available": null, "available_error": null,
        });
        if check {
            match self.available().await {
                Ok(available) => value["available"] = serde_json::to_value(available)?,
                Err(error) => value["available_error"] = format!("{error:#}").into(),
            }
        }
        Ok(value)
    }

    /// Hold the update lock and publish resolving before returning an id to the caller.
    pub fn start(
        self,
        spec: String,
        force: bool,
    ) -> Result<(String, impl Future<Output = ()> + Send)> {
        let id = uuid::Uuid::new_v4().to_string();
        let (lock, pointer, status) = self.prepare(id.clone(), &spec)?;
        Ok((id, async move {
            if let Ok(status) = self.apply_prepared(lock, pointer, status, force).await
                && matches!(status.phase, Phase::Activated)
            {
                super::restart_after_response().await;
            }
        }))
    }

    pub fn new(root: PathBuf, registries: Registries, entry_package: String) -> Self {
        Self {
            root,
            terminal_state: None,
            registries,
            entry_package,
        }
    }

    pub fn with_terminal_state(mut self, state: PathBuf) -> Self {
        self.terminal_state = Some(state);
        self
    }

    pub fn info(&self) -> Result<RuntimeInfo> {
        let pointer = Layout::new(&self.root).pointer()?;
        let mut reasons = platform::blockers();
        if pointer.is_none() {
            reasons.push("not started by the cranibox launcher".into());
        }
        if let Err(error) = npm::platform_package(&self.entry_package) {
            reasons.push(error.to_string());
        }
        if self.is_locked()?
            || pointer
                .as_ref()
                .is_some_and(|p| p.state == PointerState::Pending)
        {
            reasons.push("an update is already in progress".into());
        }
        Ok(RuntimeInfo {
            version: pointer.as_ref().map(|p| p.current.clone()),
            os: platform::os_name(),
            arch: std::env::consts::ARCH,
            managed: pointer.is_some(),
            state: pointer.map(|p| match p.state {
                PointerState::Pending => "pending".into(),
                PointerState::Confirmed => "confirmed".into(),
            }),
            can_update: reasons.is_empty(),
            reasons,
        })
    }

    /// Read the last status, including after a process restart.
    pub fn status(&self) -> Result<Option<Status>> {
        match fs::read(self.root.join("update-state.json")) {
            Ok(bytes) => Ok(Some(
                serde_json::from_slice(&bytes).context("parse update-state.json")?,
            )),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).context("read update-state.json"),
        }
    }

    /// Activate a verified runtime. The caller restarts by exiting with code 75.
    /// Forced installed versions are reused; forcing the current version is idempotent.
    pub async fn apply(&self, spec: &str, force: bool) -> Result<Status> {
        self.apply_as(uuid::Uuid::new_v4().to_string(), spec, force)
            .await
    }

    /// Like [`apply`](Self::apply) with a caller-chosen id, so a request handler can hand the
    /// id to the requester before the download finishes.
    pub async fn apply_as(&self, id: String, spec: &str, force: bool) -> Result<Status> {
        let (lock, pointer, status) = self.prepare(id, spec)?;
        self.apply_prepared(lock, pointer, status, force).await
    }

    fn prepare(
        &self,
        id: String,
        spec: &str,
    ) -> Result<(UpdateLock, super::layout::Pointer, Status)> {
        let lock = self.lock()?;
        let layout = Layout::new(&self.root);
        let pointer = layout
            .pointer()?
            .context("not started by the cranibox launcher")?;
        ensure!(
            pointer.state != PointerState::Pending,
            "an update is already in progress"
        );
        let mut status = new_status(id, spec, Some(pointer.current.clone()));
        self.save(&mut status)?;
        Ok((lock, pointer, status))
    }

    async fn apply_prepared(
        &self,
        _lock: UpdateLock,
        pointer: super::layout::Pointer,
        mut status: Status,
        force: bool,
    ) -> Result<Status> {
        let layout = Layout::new(&self.root);
        let result = self
            .apply_inner(&layout, &pointer, &mut status, force)
            .await;
        if let Err(error) = result {
            status.phase = Phase::Failed;
            status.error = Some(format!("{error:#}"));
            self.save(&mut status)
                .with_context(|| format!("update failed: {error:#}; persist failure"))?;
            return Err(error);
        }
        Ok(status)
    }

    async fn apply_inner(
        &self,
        layout: &Layout,
        pointer: &super::layout::Pointer,
        status: &mut Status,
        force: bool,
    ) -> Result<()> {
        let blockers = platform::blockers();
        ensure!(blockers.is_empty(), "{}", blockers.join("; "));
        let package = npm::platform_package(&self.entry_package)?;
        let release = self.registries.resolve(&package, &status.spec).await?;
        status.to = Some(release.version.clone());
        self.save(status)?;
        let version = Version::parse(&release.version)?;
        let current = Version::parse(&pointer.current)?;
        ensure!(
            force || version > current,
            "version {} is not newer than {}; use force to retry or downgrade",
            version,
            current
        );
        ensure!(
            force || !pointer.failed.contains(&release.version),
            "version {} previously failed; use force to retry",
            version
        );
        // Layout rejects these names; check before calling its infallible version_dir.
        ensure!(
            !release.version.contains('+'),
            "build metadata is not allowed in runtime directory names"
        );
        ensure!(
            !release.version.to_ascii_lowercase().ends_with(".staging"),
            "reserved staging directory suffix"
        );

        let installed = layout
            .versions()?
            .iter()
            .any(|v| v.to_string() == release.version);
        if installed {
            // Never replace an installed executable: it may still be running on Windows.
            self.phase(status, Phase::Verifying)?;
            platform::verify_runtime(
                &layout
                    .version_dir(&version)
                    .join(npm::executable_relative_path()),
            )?;
        } else {
            let staging = layout.begin_stage(&version)?;
            let _stage = Stage {
                layout,
                version: &version,
            };
            let download = tempfile::Builder::new()
                .prefix(".update-download-")
                .tempdir_in(&self.root)?;
            let tarball = download.path().join("runtime.tgz");
            self.phase(status, Phase::Downloading)?;
            let mut last_write = Instant::now();
            let mut progress_error = None;
            self.registries
                .download(&release, &tarball, MAX_DOWNLOAD, |done, total| {
                    status.downloaded = done;
                    status.total = total;
                    if progress_error.is_none()
                        && (done == 0 || last_write.elapsed() >= PROGRESS_INTERVAL)
                    {
                        progress_error = self.save(status).err();
                        last_write = Instant::now();
                    }
                })
                .await?;
            if let Some(error) = progress_error {
                return Err(error).context("persist download progress");
            }
            self.phase(status, Phase::Verifying)?;
            // begin_stage reserves an empty directory; extract publishes to a missing path.
            fs::remove_dir(&staging)?;
            npm::extract(&tarball, &staging, MAX_FILES, MAX_EXTRACTED)?;
            platform::verify_runtime(&staging.join(npm::executable_relative_path()))?;
            layout.commit_stage(&version)?;
        }
        self.phase(status, Phase::Staged)?;
        // Persist activation intent first so a restarted healthy runtime can finish it.
        // A failed pointer switch is recorded as Failed by apply.
        self.phase(status, Phase::Activated)?;
        if pointer.current != release.version {
            if force {
                layout.activate_forced(&version)?;
            } else {
                layout.activate(&version)?;
            }
        }
        // Activation is the commit point. Cleanup failure must not report a failed update
        // after the pointer has switched; a later update can retry pruning.
        if let Some(state) = &self.terminal_state {
            let _ = layout.prune_with_terminal_state(3, state);
        } else {
            let _ = layout.prune(3);
        }
        Ok(())
    }

    /// Select the previous installed runtime and persist the rollback reason.
    pub fn rollback(&self, reason: &str) -> Result<Status> {
        let _lock = self.lock()?;
        let layout = Layout::new(&self.root);
        let before = layout
            .pointer()?
            .context("not started by the cranibox launcher")?;
        let mut status = self.status()?.unwrap_or_else(|| {
            new_status(
                uuid::Uuid::new_v4().to_string(),
                "rollback",
                Some(before.current.clone()),
            )
        });
        let pointer = layout.rollback(reason)?;
        status.phase = Phase::RolledBack;
        status.from = Some(before.current);
        status.to = Some(pointer.current);
        status.error = Some(reason.into());
        self.save(&mut status)?;
        Ok(status)
    }

    /// Confirm only the runtime identified by CRANIBOX_RUNTIME_VERSION.
    /// Unmanaged processes and a health check from the old runtime are no-ops.
    pub fn confirm_healthy(&self) -> Result<()> {
        self.confirm_version(std::env::var("CRANIBOX_RUNTIME_VERSION").ok().as_deref())
    }

    fn confirm_version(&self, current: Option<&str>) -> Result<()> {
        let layout = Layout::new(&self.root);
        let Some(pointer) = layout.pointer()? else {
            return Ok(());
        };
        if current != Some(pointer.current.as_str()) {
            return Ok(());
        }
        let _lock = self.lock()?;
        // Re-read after locking: an apply may have selected another version meanwhile.
        let pointer = layout.pointer()?.context("runtime pointer disappeared")?;
        if current != Some(pointer.current.as_str()) {
            return Ok(());
        }
        if pointer.state == PointerState::Pending {
            layout.confirm(&Version::parse(&pointer.current)?)?;
        }
        if let Some(mut status) = self.status()?
            && matches!(status.phase, Phase::Activated)
        {
            if status.to.as_deref() == current {
                self.phase(&mut status, Phase::Done)?;
            } else {
                // The launcher rolled the new version back before it became healthy.
                status.error = Some(format!(
                    "the new version did not become healthy; restored {}",
                    current.unwrap_or("the previous version")
                ));
                self.phase(&mut status, Phase::RolledBack)?;
            }
        }
        Ok(())
    }

    fn phase(&self, status: &mut Status, phase: Phase) -> Result<()> {
        status.phase = phase;
        self.save(status)
    }

    fn save(&self, status: &mut Status) -> Result<()> {
        status.updated_at = unix_time();
        let mut temporary = tempfile::Builder::new()
            .prefix(".update-state-")
            .suffix(".tmp")
            .tempfile_in(&self.root)?;
        serde_json::to_writer(&mut temporary, status)?;
        temporary.write_all(b"\n")?;
        temporary.as_file().sync_all()?;
        replace_status(temporary, &self.root.join("update-state.json"))?;
        #[cfg(unix)]
        File::open(&self.root)?.sync_all()?;
        Ok(())
    }

    fn lock(&self) -> Result<UpdateLock> {
        fs::create_dir_all(&self.root)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join("update.lock"))?;
        fs2::FileExt::try_lock_exclusive(&file)
            .context("an update is already in progress or update.lock cannot be locked")?;
        Ok(UpdateLock(file))
    }

    fn is_locked(&self) -> Result<bool> {
        let file = match OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.root.join("update.lock"))
        {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error).context("open update.lock"),
        };
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => {
                drop(UpdateLock(file));
                Ok(false)
            }
            Err(error)
                if error.kind() == ErrorKind::WouldBlock
                    || error.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
            {
                Ok(true)
            }
            Err(error) => Err(error).context("inspect update.lock"),
        }
    }
}

/// Override the entry package with CRANIBOX_NPM_PACKAGE.
pub fn default_entry_package() -> String {
    std::env::var("CRANIBOX_NPM_PACKAGE").unwrap_or_else(|_| "@fliaping/cranibox".into())
}

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn new_status(id: String, spec: &str, from: Option<String>) -> Status {
    let now = unix_time();
    Status {
        id,
        spec: spec.into(),
        phase: Phase::Resolving,
        from,
        to: None,
        error: None,
        downloaded: 0,
        total: None,
        started_at: now,
        updated_at: now,
    }
}

#[cfg(not(windows))]
fn replace_status(temporary: tempfile::NamedTempFile, destination: &Path) -> Result<()> {
    temporary
        .persist(destination)
        .map_err(|error| error.error)?;
    Ok(())
}

// The fixture is intentionally an unsigned shell script. macOS signature validation
// is tested through platform's pure helpers, not bypassed for a fake runtime.
#[cfg(all(test, target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
pub(crate) mod tests {
    use super::*;
    use std::sync::Arc;

    use axum::{
        Router,
        body::Body,
        http::{Response, StatusCode, Uri},
    };
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use flate2::{Compression, write::GzEncoder};
    use serde_json::json;
    use sha2::{Digest, Sha512};
    use tokio::{
        sync::{Notify, Semaphore},
        task::JoinHandle,
    };

    const ENTRY: &str = "@fliaping/cranibox";
    const SCRIPT: &[u8] = b"#!/bin/sh\nexit 0\n";

    pub(crate) struct Fixture {
        pub(crate) root: tempfile::TempDir,
        pub(crate) base: String,
        server: JoinHandle<()>,
        requested: Arc<Notify>,
        release: Arc<Semaphore>,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.server.abort();
        }
    }

    impl Fixture {
        pub(crate) async fn new(
            current: &str,
            missing_executable: bool,
            bad_integrity: bool,
            held: bool,
        ) -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let bytes = tarball(missing_executable);
            let integrity = format!(
                "sha512-{}",
                STANDARD.encode(Sha512::digest(if bad_integrity {
                    b"wrong".as_slice()
                } else {
                    &bytes
                }))
            );
            let package = npm::platform_package(ENTRY).unwrap();
            let mut versions = serde_json::Map::new();
            for version in [
                "0.9.0",
                "1.0.0",
                "1.1.0",
                "2.0.0",
                "1.2.0-dev.9",
                "1.2.0-dev.17",
                "1.1.0+build",
                "1.1.0-test.staging",
            ] {
                versions.insert(
                    version.into(),
                    json!({
                        "name": package, "version": version,
                        "dist": { "tarball": format!("{base}/runtime.tgz"), "integrity": integrity }
                    }),
                );
            }
            let metadata = serde_json::to_vec(&json!({
                "dist-tags": {"latest": "1.1.0", "dev": "1.2.0-dev.17"},
                "versions": versions,
            }))
            .unwrap();
            let requested = Arc::new(Notify::new());
            let release = Arc::new(Semaphore::new(usize::from(!held)));
            let router = Router::new().fallback({
                let requested = requested.clone();
                let release = release.clone();
                let package_path = format!("/{}", package.replace('/', "%2F"));
                move |uri: Uri| {
                    let bytes = bytes.clone();
                    let metadata = metadata.clone();
                    let requested = requested.clone();
                    let release = release.clone();
                    let package_path = package_path.clone();
                    async move {
                        let (status, body) = if uri.path() == "/runtime.tgz" {
                            requested.notify_one();
                            let _permit = release.acquire().await.unwrap();
                            (StatusCode::OK, bytes)
                        } else if uri.path() == package_path {
                            (StatusCode::OK, metadata)
                        } else {
                            (StatusCode::NOT_FOUND, Vec::new())
                        };
                        Response::builder()
                            .status(status)
                            .body(Body::from(body))
                            .unwrap()
                    }
                }
            });
            let server = tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            });
            let root = scratch();
            let layout = Layout::new(root.path());
            install(&layout, current);
            let version = Version::parse(current).unwrap();
            layout.activate(&version).unwrap();
            layout.confirm(&version).unwrap();
            Self {
                root,
                base,
                server,
                requested,
                release,
            }
        }

        fn updater(&self) -> Updater {
            Updater::new(
                self.root.path().into(),
                Registries::new(vec![self.base.clone()]).unwrap(),
                ENTRY.into(),
            )
        }

        fn layout(&self) -> Layout {
            Layout::new(self.root.path())
        }

        fn assert_clean(&self) {
            for entry in fs::read_dir(self.root.path().join("versions")).unwrap() {
                assert!(
                    !entry
                        .unwrap()
                        .file_name()
                        .to_str()
                        .unwrap()
                        .ends_with(".staging")
                );
            }
            for entry in fs::read_dir(self.root.path()).unwrap() {
                assert!(
                    !entry
                        .unwrap()
                        .file_name()
                        .to_str()
                        .unwrap()
                        .starts_with(".update-")
                );
            }
        }
    }

    fn scratch() -> tempfile::TempDir {
        let parent = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        tempfile::tempdir_in(parent).unwrap()
    }

    fn tarball(missing_executable: bool) -> Vec<u8> {
        let gzip = GzEncoder::new(Vec::new(), Compression::default());
        let mut archive = tar::Builder::new(gzip);
        let (path, bytes) = if missing_executable {
            ("package/package.json", b"{}".as_slice())
        } else {
            ("package/bin/cranibox", SCRIPT)
        };
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive.append_data(&mut header, path, bytes).unwrap();
        archive.into_inner().unwrap().finish().unwrap()
    }

    fn install(layout: &Layout, version: &str) {
        let version = Version::parse(version).unwrap();
        let stage = layout.begin_stage(&version).unwrap();
        fs::create_dir(stage.join("bin")).unwrap();
        let exe = stage.join(npm::executable_relative_path());
        fs::write(&exe, SCRIPT).unwrap();
        platform::verify_runtime(&exe).unwrap();
        layout.commit_stage(&version).unwrap();
    }

    #[tokio::test]
    async fn applies_new_version_and_persists_across_restart() {
        let fixture = Fixture::new("1.0.0", false, false, false).await;
        let updater = fixture.updater();
        assert!(updater.info().unwrap().can_update);
        let status = updater.apply("latest", false).await.unwrap();
        assert!(matches!(status.phase, Phase::Activated));
        assert!(uuid::Uuid::parse_str(&status.id).is_ok());
        assert_eq!(status.from.as_deref(), Some("1.0.0"));
        assert_eq!(status.to.as_deref(), Some("1.1.0"));
        assert!(status.downloaded > 0);
        assert_eq!(status.total, Some(status.downloaded));
        assert!(status.updated_at >= status.started_at);
        assert_eq!(
            serde_json::to_value(&status).unwrap(),
            serde_json::to_value(fixture.updater().status().unwrap().unwrap()).unwrap()
        );
        let pointer = fixture.layout().pointer().unwrap().unwrap();
        assert_eq!(pointer.current, "1.1.0");
        assert_eq!(pointer.previous.as_deref(), Some("1.0.0"));
        assert_eq!(pointer.state, PointerState::Pending);
        assert!(
            fixture
                .layout()
                .version_dir(&Version::parse("1.0.0").unwrap())
                .is_dir()
        );
        let exe = fixture
            .layout()
            .version_dir(&Version::parse("1.1.0").unwrap())
            .join(npm::executable_relative_path());
        assert!(std::process::Command::new(exe).status().unwrap().success());
        let info = updater.info().unwrap();
        assert_eq!(info.state.as_deref(), Some("pending"));
        assert!(!info.can_update);
        assert!(
            info.reasons
                .iter()
                .any(|reason| reason.contains("already in progress"))
        );
        assert!(updater.apply("2.0.0", true).await.is_err());
        assert_eq!(updater.status().unwrap().unwrap().id, status.id);
        fixture.assert_clean();
    }

    #[tokio::test]
    async fn same_and_older_versions_require_force_without_overwriting_installed_files() {
        let fixture = Fixture::new("1.0.0", false, false, false).await;
        let updater = fixture.updater();
        let before = fixture.layout().pointer().unwrap();
        for spec in ["1.0.0", "0.9.0"] {
            assert!(
                updater
                    .apply(spec, false)
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("not newer")
            );
            assert!(matches!(
                updater.status().unwrap().unwrap().phase,
                Phase::Failed
            ));
            assert_eq!(fixture.layout().pointer().unwrap(), before);
        }
        let same = updater.apply("1.0.0", true).await.unwrap();
        assert!(matches!(same.phase, Phase::Activated));
        assert_eq!(same.downloaded, 0);
        assert_eq!(fixture.layout().pointer().unwrap(), before);
        let older = updater.apply("0.9.0", true).await.unwrap();
        assert!(matches!(older.phase, Phase::Activated));
        assert_eq!(
            fixture.layout().pointer().unwrap().unwrap().current,
            "0.9.0"
        );
        updater.confirm_version(Some("0.9.0")).unwrap();
        // Selecting a previously installed version also succeeds without replacing it.
        updater.apply("1.0.0", false).await.unwrap();
        assert_eq!(
            fixture.layout().pointer().unwrap().unwrap().current,
            "1.0.0"
        );
        fixture.assert_clean();
    }

    #[tokio::test]
    async fn failed_versions_require_force_and_rollback_is_persisted() {
        let fixture = Fixture::new("1.0.0", false, false, false).await;
        let updater = fixture.updater();
        let applied = updater.apply("2.0.0", false).await.unwrap();
        let rolled_back = updater.rollback("health check failed").unwrap();
        assert!(matches!(rolled_back.phase, Phase::RolledBack));
        assert_eq!(rolled_back.id, applied.id);
        assert_eq!(rolled_back.from.as_deref(), Some("2.0.0"));
        assert_eq!(rolled_back.to.as_deref(), Some("1.0.0"));
        assert_eq!(rolled_back.error.as_deref(), Some("health check failed"));
        assert!(matches!(
            fixture.updater().status().unwrap().unwrap().phase,
            Phase::RolledBack
        ));
        let before = fixture.layout().pointer().unwrap();
        assert_eq!(before.as_ref().unwrap().failed, ["2.0.0"]);
        assert!(
            updater
                .apply("2.0.0", false)
                .await
                .unwrap_err()
                .to_string()
                .contains("previously failed")
        );
        assert_eq!(fixture.layout().pointer().unwrap(), before);
        assert!(matches!(
            updater.apply("2.0.0", true).await.unwrap().phase,
            Phase::Activated
        ));
        assert_eq!(
            fixture.layout().pointer().unwrap().unwrap().state,
            PointerState::Pending
        );
    }

    #[tokio::test]
    async fn extraction_and_integrity_failures_leave_pointer_and_staging_clean() {
        for (missing, corrupt, message) in [
            (true, false, "runtime executable is missing"),
            (false, true, "integrity mismatch"),
        ] {
            let fixture = Fixture::new("1.0.0", missing, corrupt, false).await;
            let updater = fixture.updater();
            let before = fixture.layout().pointer().unwrap();
            let error = updater.apply("latest", false).await.unwrap_err();
            assert!(format!("{error:#}").contains(message), "{error:#}");
            assert_eq!(fixture.layout().pointer().unwrap(), before);
            let status = fixture.updater().status().unwrap().unwrap();
            assert!(matches!(status.phase, Phase::Failed));
            assert!(status.error.unwrap().contains(message));
            assert_eq!(fixture.layout().versions().unwrap().len(), 1);
            assert!(updater.info().unwrap().can_update);
            fixture.assert_clean();
        }
    }

    #[tokio::test]
    async fn second_concurrent_apply_and_rollback_are_rejected() {
        let fixture = Fixture::new("1.0.0", false, false, true).await;
        let first = fixture.updater();
        let second = fixture.updater();
        let (result, ()) = tokio::join!(first.apply("latest", false), async {
            tokio::time::timeout(Duration::from_secs(10), fixture.requested.notified())
                .await
                .unwrap();
            let status = second.status().unwrap().unwrap();
            assert!(matches!(status.phase, Phase::Downloading));
            assert!(!second.info().unwrap().can_update);
            assert!(
                second
                    .apply("2.0.0", true)
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("already in progress")
            );
            assert!(second.rollback("concurrent").is_err());
            assert_eq!(second.status().unwrap().unwrap().id, status.id);
            fixture.release.add_permits(1);
        });
        assert!(matches!(result.unwrap().phase, Phase::Activated));
        fixture.assert_clean();
    }

    #[tokio::test]
    async fn health_confirmation_requires_matching_runtime_and_finishes_status() {
        let fixture = Fixture::new("1.0.0", false, false, false).await;
        fixture.updater().apply("latest", false).await.unwrap();
        let restarted = fixture.updater();
        for version in [None, Some("1.0.0"), Some("1.1.0+other")] {
            restarted.confirm_version(version).unwrap();
            assert_eq!(
                fixture.layout().pointer().unwrap().unwrap().state,
                PointerState::Pending
            );
            assert!(matches!(
                restarted.status().unwrap().unwrap().phase,
                Phase::Activated
            ));
        }
        restarted.confirm_version(Some("1.1.0")).unwrap();
        let pointer = fixture.layout().pointer().unwrap().unwrap();
        assert_eq!(pointer.state, PointerState::Confirmed);
        assert_eq!(pointer.last_confirmed.as_deref(), Some("1.1.0"));
        assert!(matches!(
            restarted.status().unwrap().unwrap().phase,
            Phase::Done
        ));
        assert!(restarted.info().unwrap().can_update);
        restarted.confirm_version(Some("1.1.0")).unwrap();
        // Recover if the process died after confirming the pointer but before saving Done.
        let mut status = restarted.status().unwrap().unwrap();
        status.phase = Phase::Activated;
        restarted.save(&mut status).unwrap();
        restarted.confirm_version(Some("1.1.0")).unwrap();
        assert!(matches!(
            restarted.status().unwrap().unwrap().phase,
            Phase::Done
        ));
    }

    #[tokio::test]
    async fn dev_prereleases_use_numeric_semver_order() {
        let fixture = Fixture::new("1.2.0-dev.9", false, false, false).await;
        let updater = fixture.updater();
        let status = updater.apply("dev", false).await.unwrap();
        assert_eq!(status.to.as_deref(), Some("1.2.0-dev.17"));
        updater.confirm_version(Some("1.2.0-dev.17")).unwrap();
        assert!(updater.apply("1.2.0-dev.9", false).await.is_err());
    }

    #[tokio::test]
    async fn unmanaged_info_and_confirmation_do_not_create_a_layout() {
        let root = scratch();
        let missing = root.path().join("missing");
        let updater = Updater::new(
            missing.clone(),
            Registries::new(vec!["http://127.0.0.1:1".into()]).unwrap(),
            ENTRY.into(),
        );
        let info = updater.info().unwrap();
        assert!(!info.managed);
        assert!(!info.can_update);
        assert!(info.version.is_none());
        assert!(info.state.is_none());
        assert!(
            info.reasons
                .iter()
                .any(|reason| reason == "not started by the cranibox launcher")
        );
        assert!(updater.status().unwrap().is_none());
        updater.confirm_version(Some("1.0.0")).unwrap();
        assert!(!missing.exists());
        assert!(
            updater
                .apply("latest", false)
                .await
                .unwrap_err()
                .to_string()
                .contains("not started by")
        );
    }

    #[tokio::test]
    async fn invalid_specs_and_layout_names_fail_without_staging_or_panicking() {
        let fixture = Fixture::new("1.0.0", false, false, false).await;
        let updater = fixture.updater();
        let before = fixture.layout().pointer().unwrap();
        for spec in ["../bad", "missing-tag", "1.1.0+build", "1.1.0-test.staging"] {
            assert!(updater.apply(spec, true).await.is_err(), "{spec}");
            assert!(matches!(
                updater.status().unwrap().unwrap().phase,
                Phase::Failed
            ));
            assert_eq!(fixture.layout().pointer().unwrap(), before);
            fixture.assert_clean();
        }
    }

    #[tokio::test]
    async fn stale_resolving_status_does_not_hold_the_update_lock() {
        let fixture = Fixture::new("1.0.0", false, false, false).await;
        let updater = fixture.updater();
        updater
            .save(&mut new_status(
                "x".into(),
                "abandoned",
                Some("1.0.0".into()),
            ))
            .unwrap();
        assert!(updater.info().unwrap().can_update);
        updater.apply("latest", false).await.unwrap();
    }

    #[tokio::test]
    async fn pruning_keeps_current_previous_and_three_other_versions() {
        let fixture = Fixture::new("1.0.0", false, false, false).await;
        for version in ["0.1.0", "0.2.0", "0.3.0", "0.4.0", "0.5.0"] {
            install(&fixture.layout(), version);
        }
        fixture.updater().apply("latest", false).await.unwrap();
        let versions: Vec<_> = fixture
            .layout()
            .versions()
            .unwrap()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(versions, ["0.3.0", "0.4.0", "0.5.0", "1.0.0", "1.1.0"]);
    }
}

#[cfg(windows)]
fn replace_status(temporary: tempfile::NamedTempFile, destination: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

    let source = temporary.into_temp_path().keep()?;
    let source_wide: Vec<_> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination_wide: Vec<_> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // SAFETY: Both paths are live NUL-terminated UTF-16, and the rename stays in root.
    let replaced = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            destination_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        let error = std::io::Error::last_os_error();
        let _ = fs::remove_file(source);
        return Err(error).context("replace update-state.json");
    }
    Ok(())
}
