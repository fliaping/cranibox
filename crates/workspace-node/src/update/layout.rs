use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use super::version::Version;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PointerState {
    Pending,
    Confirmed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Pointer {
    pub schema: u32,
    pub current: String,
    pub previous: Option<String>,
    pub state: PointerState,
    pub activated_at: u64,
    pub attempts: u32,
    pub last_confirmed: Option<String>,
    pub failed: Vec<String>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum LaunchDecision {
    Run {
        version: Version,
    },
    RolledBack {
        from: Version,
        to: Version,
        reason: String,
    },
    Broken {
        reason: String,
    },
}

pub struct Layout {
    root: PathBuf,
}

struct LayoutLock(File);

impl Drop for LayoutLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

impl Layout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Committed directories only, in increasing SemVer precedence.
    pub fn versions(&self) -> Result<Vec<Version>> {
        let entries = match fs::read_dir(self.root.join("versions")) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut versions = Vec::new();
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            if let Some(name) = entry.file_name().to_str()
                && let Ok(version) = directory_version(name)
            {
                versions.push(version);
            }
        }
        versions.sort();
        Ok(versions)
    }

    /// Reading never repairs, replaces, or removes a malformed pointer.
    pub fn pointer(&self) -> Result<Option<Pointer>> {
        let bytes = match fs::read(self.root.join("current.json")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("read current.json"),
        };
        let pointer: Pointer = serde_json::from_slice(&bytes).context("parse current.json")?;
        ensure!(
            pointer.schema == 1,
            "unsupported pointer schema {}",
            pointer.schema
        );
        for name in std::iter::once(&pointer.current)
            .chain(pointer.previous.iter())
            .chain(pointer.last_confirmed.iter())
            .chain(pointer.failed.iter())
        {
            directory_version(name).context("invalid version in current.json")?;
        }
        Ok(Some(pointer))
    }

    pub fn begin_stage(&self, v: &Version) -> Result<PathBuf> {
        let name = directory_name(v)?;
        let _lock = self.lock()?;
        let versions = self.root.join("versions");
        fs::create_dir_all(&versions)?;
        ensure_absent(&versions.join(&name))?;
        let staging = versions.join(format!("{name}.staging"));
        remove_entry(&staging)?;
        fs::create_dir(&staging)?;
        Ok(staging)
    }

    pub fn commit_stage(&self, v: &Version) -> Result<PathBuf> {
        let name = directory_name(v)?;
        let _lock = self.lock()?;
        let versions = self.root.join("versions");
        let installed = versions.join(&name);
        ensure_absent(&installed)?;
        let staging = versions.join(format!("{name}.staging"));
        ensure_directory(&staging)?;
        fs::rename(&staging, &installed).context("commit staged runtime")?;
        sync_directory(&versions)?;
        Ok(installed)
    }

    pub fn abort_stage(&self, v: &Version) {
        if let Ok(name) = directory_name(v)
            && let Ok(_lock) = self.lock()
        {
            let _ = remove_entry(&self.root.join("versions").join(format!("{name}.staging")));
        }
    }

    /// Panics for build metadata or the reserved `.staging` suffix (ignoring case).
    /// Fallible operations return an error for the same input.
    pub fn version_dir(&self, v: &Version) -> PathBuf {
        self.root
            .join("versions")
            .join(directory_name(v).expect("version cannot be used as a runtime directory"))
    }

    /// Atomically select an installed runtime, resetting its confirmation budget.
    pub fn activate(&self, v: &Version) -> Result<Pointer> {
        self.activate_inner(v, false)
    }

    pub fn activate_forced(&self, v: &Version) -> Result<Pointer> {
        self.activate_inner(v, true)
    }

    fn activate_inner(&self, v: &Version, forced: bool) -> Result<Pointer> {
        let name = directory_name(v)?;
        let _lock = self.lock()?;
        ensure_directory(&self.version_dir(v))?;
        let old = self.pointer()?;
        if let Some(old) = &old {
            ensure!(old.current != name, "version is already current");
            ensure!(
                forced || !old.failed.contains(&name),
                "version previously failed"
            );
        }
        let pointer = Pointer {
            schema: 1,
            current: name,
            previous: old.as_ref().map(|p| p.current.clone()),
            state: PointerState::Pending,
            activated_at: unix_time()?,
            attempts: 0,
            last_confirmed: old.as_ref().and_then(|p| p.last_confirmed.clone()),
            failed: old.map_or_else(Vec::new, |p| p.failed),
        };
        self.save(&pointer)?;
        Ok(pointer)
    }

    pub fn begin_launch(
        &self,
        max_attempts: u32,
        confirm_window: Duration,
    ) -> Result<LaunchDecision> {
        self.begin_launch_at(unix_time()?, max_attempts, confirm_window)
    }

    /// Pending attempts are persisted before allowing the launcher to start.
    pub fn begin_launch_at(
        &self,
        now: u64,
        max_attempts: u32,
        confirm_window: Duration,
    ) -> Result<LaunchDecision> {
        let _lock = self.lock()?;
        let Some(mut pointer) = self.pointer()? else {
            return Ok(LaunchDecision::Broken {
                reason: "no runtime has been activated".into(),
            });
        };
        let version = directory_version(&pointer.current)?;
        if let Err(error) = ensure_directory(&self.version_dir(&version)) {
            return Ok(LaunchDecision::Broken {
                reason: format!("current runtime is unavailable: {error}"),
            });
        }
        if pointer.state == PointerState::Confirmed {
            return Ok(LaunchDecision::Run { version });
        }
        let overflow = pointer.attempts.checked_add(1).is_none();
        pointer.attempts = pointer.attempts.saturating_add(1);
        let reason = if overflow || pointer.attempts > max_attempts {
            Some("runtime exceeded its launch attempt limit")
        } else if Duration::from_secs(now.saturating_sub(pointer.activated_at)) > confirm_window {
            Some("runtime did not confirm before its deadline")
        } else {
            None
        };
        if let Some(reason) = reason {
            // Check recovery before changing the pointer; an unavailable fallback is never selected.
            match self.rollback_pointer(&pointer) {
                Ok(rolled_back) => {
                    let to = directory_version(&rolled_back.current)?;
                    self.save(&rolled_back)?;
                    return Ok(LaunchDecision::RolledBack {
                        from: version,
                        to,
                        reason: reason.into(),
                    });
                }
                Err(error) => {
                    self.save(&pointer)?;
                    return Ok(LaunchDecision::Broken {
                        reason: format!("{reason}; {error}"),
                    });
                }
            }
        }
        self.save(&pointer)?;
        Ok(LaunchDecision::Run { version })
    }

    pub fn confirm(&self, v: &Version) -> Result<Pointer> {
        let name = directory_name(v)?;
        let _lock = self.lock()?;
        let mut pointer = self.pointer()?.context("no runtime has been activated")?;
        ensure!(
            pointer.current == name,
            "only the current runtime can confirm"
        );
        ensure_directory(&self.version_dir(v))?;
        pointer.state = PointerState::Confirmed;
        pointer.last_confirmed = Some(name);
        pointer.attempts = 0;
        self.save(&pointer)?;
        Ok(pointer)
    }

    /// Recovery clears `previous` so a second rollback cannot select the failed runtime.
    pub fn rollback(&self, reason: &str) -> Result<Pointer> {
        let _lock = self.lock()?;
        let pointer = self.pointer()?.context("no runtime has been activated")?;
        let pointer = self
            .rollback_pointer(&pointer)
            .with_context(|| format!("rollback: {reason}"))?;
        self.save(&pointer)?;
        Ok(pointer)
    }

    fn rollback_pointer(&self, pointer: &Pointer) -> Result<Pointer> {
        let previous = pointer.previous.as_ref().context("no previous runtime")?;
        ensure!(
            *previous != pointer.current,
            "previous runtime equals current"
        );
        let version = directory_version(previous)?;
        ensure_directory(&self.version_dir(&version)).context("previous runtime is unavailable")?;
        let mut recovered = pointer.clone();
        if !recovered.failed.contains(&pointer.current) {
            recovered.failed.push(pointer.current.clone());
        }
        recovered.current = previous.clone();
        recovered.previous = None;
        recovered.state = PointerState::Confirmed;
        recovered.attempts = 0;
        Ok(recovered)
    }

    /// Keep protected runtimes plus `keep` newest others. The returned deletions are sorted.
    /// All staging directories and `.current-*.tmp` pointer remnants are disposable.
    /// Call after staging has finished: the lock does not cover a caller writing stage contents.
    pub fn prune(&self, keep: usize) -> Result<Vec<Version>> {
        let _lock = self.lock()?;
        let pointer = self.pointer()?;
        let protected: HashSet<_> = pointer
            .iter()
            .flat_map(|p| {
                std::iter::once(p.current.as_str())
                    .chain(p.previous.as_deref())
                    .chain(p.last_confirmed.as_deref())
            })
            .collect();
        let mut others = Vec::new();
        for version in self.versions()? {
            if !protected.contains(version.to_string().as_str()) {
                others.push(version);
            }
        }
        let delete_count = others.len().saturating_sub(keep);
        let mut deleted = Vec::new();
        for version in others.into_iter().take(delete_count) {
            fs::remove_dir_all(self.version_dir(&version))?;
            deleted.push(version);
        }
        let versions = self.root.join("versions");
        match fs::read_dir(&versions) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry?;
                    if let Some(name) = entry.file_name().to_str()
                        && let Some(version) = name.strip_suffix(".staging")
                        && directory_version(version).is_ok()
                    {
                        remove_entry(&entry.path())?;
                    }
                }
                sync_directory(&versions)?;
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if let Some(name) = entry.file_name().to_str()
                && name.starts_with(".current-")
                && name.ends_with(".tmp")
                && !entry.file_type()?.is_dir()
            {
                fs::remove_file(entry.path())?;
            }
        }
        sync_directory(&self.root)?;
        Ok(deleted)
    }

    fn lock(&self) -> Result<LayoutLock> {
        fs::create_dir_all(&self.root)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join("layout.lock"))?;
        fs2::FileExt::lock_exclusive(&file).context("lock runtime layout")?;
        Ok(LayoutLock(file))
    }

    fn save(&self, pointer: &Pointer) -> Result<()> {
        let mut temporary = tempfile::Builder::new()
            .prefix(".current-")
            .suffix(".tmp")
            .tempfile_in(&self.root)?;
        serde_json::to_writer(&mut temporary, pointer)?;
        temporary.write_all(b"\n")?;
        temporary.as_file().sync_all()?;
        replace_pointer(temporary, &self.root.join("current.json"))?;
        sync_directory(&self.root)
    }
}

fn directory_name(version: &Version) -> Result<String> {
    let name = version.to_string();
    ensure!(
        !name.contains('+'),
        "build metadata is not allowed in runtime directory names"
    );
    // A prerelease can otherwise alias another version's staging directory.
    ensure!(
        !name.to_ascii_lowercase().ends_with(".staging"),
        "reserved staging directory suffix"
    );
    Ok(name)
}

fn directory_version(name: &str) -> Result<Version> {
    let version = Version::parse(name)?;
    directory_name(&version)?;
    Ok(version)
}

fn ensure_absent(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
        Ok(_) => anyhow::bail!("runtime destination already exists: {}", path.display()),
    }
}

fn ensure_directory(path: &Path) -> Result<()> {
    ensure!(
        fs::symlink_metadata(path)
            .with_context(|| format!("runtime directory {}", path.display()))?
            .file_type()
            .is_dir(),
        "runtime is not a directory: {}",
        path.display()
    );
    #[cfg(any(windows, target_os = "macos"))]
    {
        // SemVer identifiers are case sensitive even on a case insensitive filesystem.
        let name = path.file_name().context("runtime directory has no name")?;
        let parent = path.parent().context("runtime directory has no parent")?;
        let mut exact = false;
        for entry in fs::read_dir(parent)? {
            if entry?.file_name().as_os_str() == name {
                exact = true;
                break;
            }
        }
        ensure!(
            exact,
            "runtime directory name differs in case: {}",
            path.display()
        );
    }
    Ok(())
}

fn remove_entry(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            fs::remove_dir_all(path)?
        }
        Ok(_) => fs::remove_file(path)?,
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn unix_time() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<()> {
    // Windows uses a write-through rename instead of POSIX directory fsync.
    Ok(())
}

#[cfg(not(windows))]
fn replace_pointer(temporary: tempfile::NamedTempFile, destination: &Path) -> Result<()> {
    temporary
        .persist(destination)
        .map_err(|error| error.error)?;
    Ok(())
}

#[cfg(windows)]
fn replace_pointer(temporary: tempfile::NamedTempFile, destination: &Path) -> Result<()> {
    // Keep clears tempfile's temporary attribute and closes its handle before replacement.
    // `MoveFileExW(REPLACE_EXISTING)` fails with "access denied" while any process has the
    // old pointer open, which the launcher and status commands routinely do. The standard
    // library renames with POSIX semantics (replacing a file other handles keep open), and
    // the temporary file was synced before it was handed over.
    let source = temporary.into_temp_path().keep()?;
    if let Err(error) = fs::rename(&source, destination) {
        let _ = fs::remove_file(&source);
        return Err(error).context("replace current.json");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    fn fixture() -> (tempfile::TempDir, Layout) {
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
        let scratch = target.join("update-layout-tests");
        fs::create_dir_all(&scratch).unwrap();
        let root = tempfile::tempdir_in(scratch).unwrap();
        let layout = Layout::new(root.path());
        (root, layout)
    }

    fn version(value: &str) -> Version {
        Version::parse(value).unwrap()
    }

    fn install(layout: &Layout, value: &str) -> Version {
        let version = version(value);
        let stage = layout.begin_stage(&version).unwrap();
        fs::write(stage.join("runtime"), value).unwrap();
        layout.commit_stage(&version).unwrap();
        version
    }

    fn pending_pair(layout: &Layout) -> (Version, Version, Pointer) {
        let old = install(layout, "1.0.0");
        let new = install(layout, "2.0.0");
        layout.activate(&old).unwrap();
        layout.confirm(&old).unwrap();
        let pointer = layout.activate(&new).unwrap();
        (old, new, pointer)
    }

    #[test]
    fn stage_commit_activate() {
        let (_root, layout) = fixture();
        assert!(layout.pointer().unwrap().is_none());
        assert!(layout.versions().unwrap().is_empty());
        let v = version("1.2.0-dev.9");
        assert!(layout.commit_stage(&v).is_err());
        let staging = layout.begin_stage(&v).unwrap();
        fs::write(staging.join("partial"), b"stale").unwrap();
        assert_eq!(layout.begin_stage(&v).unwrap(), staging);
        assert_eq!(fs::read_dir(&staging).unwrap().count(), 0);
        fs::write(staging.join("runtime"), b"complete").unwrap();
        assert!(layout.versions().unwrap().is_empty());
        assert!(layout.activate(&v).is_err());
        let installed = layout.commit_stage(&v).unwrap();
        assert_eq!(installed, layout.version_dir(&v));
        assert!(!staging.exists());
        assert_eq!(fs::read(installed.join("runtime")).unwrap(), b"complete");
        assert!(layout.begin_stage(&v).is_err());
        assert!(layout.commit_stage(&v).is_err());
        let pointer = layout.activate(&v).unwrap();
        assert_eq!(pointer.current, v.to_string());
        assert_eq!(pointer.previous, None);
        assert_eq!(pointer.state, PointerState::Pending);
        assert_eq!(pointer.attempts, 0);
        assert_eq!(pointer, layout.pointer().unwrap().unwrap());
        install(&layout, "1.2.0-dev.17");
        install(&layout, "1.2.0");
        assert_eq!(
            layout
                .versions()
                .unwrap()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["1.2.0-dev.9", "1.2.0-dev.17", "1.2.0"]
        );
    }

    #[test]
    fn abort_and_existing_destination() {
        let (_root, layout) = fixture();
        let v = version("1.0.0");
        let stage = layout.begin_stage(&v).unwrap();
        fs::create_dir(layout.version_dir(&v)).unwrap();
        assert!(layout.commit_stage(&v).is_err());
        assert!(stage.is_dir());
        layout.abort_stage(&v);
        layout.abort_stage(&v);
        assert!(!stage.exists());
        assert!(layout.version_dir(&v).is_dir());
    }

    #[test]
    fn activation_rejections_and_forced_retry() {
        let (_root, layout) = fixture();
        let (old, new, _) = pending_pair(&layout);
        let before = layout.pointer().unwrap();
        assert!(layout.activate(&new).is_err());
        assert!(layout.activate_forced(&new).is_err());
        assert!(layout.activate(&version("3.0.0")).is_err());
        assert_eq!(layout.pointer().unwrap(), before);
        let recovered = layout.rollback("health check failed").unwrap();
        assert_eq!(recovered.current, old.to_string());
        assert_eq!(recovered.failed, [new.to_string()]);
        assert_eq!(recovered.previous, None);
        assert!(layout.activate(&new).is_err());
        let retried = layout.activate_forced(&new).unwrap();
        assert_eq!(retried.current, new.to_string());
        assert_eq!(retried.previous, Some(old.to_string()));
        assert_eq!(retried.state, PointerState::Pending);
        assert_eq!(retried.attempts, 0);
    }

    #[test]
    fn rollback_preserves_last_health_confirmation() {
        let (_root, layout) = fixture();
        let (healthy, pending, _) = pending_pair(&layout);
        let newest = install(&layout, "3.0.0");
        layout.activate(&newest).unwrap();
        let recovered = layout.rollback("newest failed").unwrap();
        assert_eq!(recovered.current, pending.to_string());
        assert_eq!(recovered.last_confirmed, Some(healthy.to_string()));
        assert_eq!(recovered.failed, [newest.to_string()]);
        assert_eq!(layout.prune(0).unwrap(), [newest]);
        assert!(layout.version_dir(&healthy).is_dir());
        assert!(layout.version_dir(&pending).is_dir());
    }

    #[test]
    fn runtime_names_are_case_sensitive() {
        let (_root, layout) = fixture();
        let installed = install(&layout, "1.0.0-dev");
        layout.activate(&installed).unwrap();
        let before = layout.pointer().unwrap();
        assert!(layout.activate(&version("1.0.0-DEV")).is_err());
        assert_eq!(layout.pointer().unwrap(), before);
    }

    #[test]
    fn launch_then_confirm() {
        let (_root, layout) = fixture();
        let (old, new, pointer) = pending_pair(&layout);
        assert_eq!(
            layout
                .begin_launch_at(pointer.activated_at, 3, Duration::from_secs(60))
                .unwrap(),
            LaunchDecision::Run {
                version: new.clone()
            }
        );
        assert_eq!(layout.pointer().unwrap().unwrap().attempts, 1);
        assert!(layout.confirm(&old).is_err());
        let confirmed = layout.confirm(&new).unwrap();
        assert_eq!(confirmed.state, PointerState::Confirmed);
        assert_eq!(confirmed.last_confirmed, Some(new.to_string()));
        assert_eq!(confirmed.attempts, 0);
        assert_eq!(
            layout.begin_launch_at(u64::MAX, 0, Duration::ZERO).unwrap(),
            LaunchDecision::Run { version: new }
        );
        assert_eq!(layout.pointer().unwrap().unwrap(), confirmed);
        assert!(matches!(
            layout.begin_launch(0, Duration::ZERO).unwrap(),
            LaunchDecision::Run { .. }
        ));
    }

    #[test]
    fn crash_loop_rolls_back() {
        let (_root, layout) = fixture();
        let (old, new, pointer) = pending_pair(&layout);
        for attempts in 1..=2 {
            assert_eq!(
                layout
                    .begin_launch_at(pointer.activated_at, 2, Duration::from_secs(60))
                    .unwrap(),
                LaunchDecision::Run {
                    version: new.clone()
                }
            );
            assert_eq!(layout.pointer().unwrap().unwrap().attempts, attempts);
        }
        match layout
            .begin_launch_at(pointer.activated_at, 2, Duration::from_secs(60))
            .unwrap()
        {
            LaunchDecision::RolledBack { from, to, reason } => {
                assert_eq!(from, new);
                assert_eq!(to, old);
                assert!(reason.contains("attempt"));
            }
            other => panic!("unexpected {other:?}"),
        }
        let recovered = layout.pointer().unwrap().unwrap();
        assert_eq!(recovered.current, old.to_string());
        assert_eq!(recovered.state, PointerState::Confirmed);
        assert_eq!(recovered.attempts, 0);
        assert_eq!(recovered.failed, [new.to_string()]);
    }

    #[test]
    fn never_confirmed_timeout_rolls_back() {
        let (_root, layout) = fixture();
        let (old, new, pointer) = pending_pair(&layout);
        let window = Duration::from_secs(60);
        assert!(matches!(
            layout
                .begin_launch_at(pointer.activated_at + 60, 100, window)
                .unwrap(),
            LaunchDecision::Run { .. }
        ));
        assert_eq!(layout.pointer().unwrap().unwrap().attempts, 1);
        assert!(
            matches!(layout.begin_launch_at(pointer.activated_at + 61, 100, window).unwrap(),
            LaunchDecision::RolledBack { from, to, reason } if from == new && to == old && reason.contains("deadline"))
        );
    }

    #[test]
    fn aged_first_launch_and_backwards_clock() {
        let (_root, layout) = fixture();
        let (_, _, pointer) = pending_pair(&layout);
        assert!(matches!(
            layout
                .begin_launch_at(pointer.activated_at - 1, 10, Duration::ZERO)
                .unwrap(),
            LaunchDecision::Run { .. }
        ));
        let mut pointer = layout.pointer().unwrap().unwrap();
        pointer.attempts = 0;
        layout.save(&pointer).unwrap();
        assert!(matches!(
            layout
                .begin_launch_at(pointer.activated_at + 1, 10, Duration::ZERO)
                .unwrap(),
            LaunchDecision::RolledBack { .. }
        ));
    }

    #[test]
    fn broken_without_previous() {
        let (_root, layout) = fixture();
        assert!(matches!(
            layout.begin_launch_at(0, 0, Duration::ZERO).unwrap(),
            LaunchDecision::Broken { .. }
        ));
        let v = install(&layout, "1.0.0");
        let pointer = layout.activate(&v).unwrap();
        assert!(layout.rollback("no fallback").is_err());
        assert_eq!(layout.pointer().unwrap().unwrap(), pointer);
        assert!(
            matches!(layout.begin_launch_at(pointer.activated_at, 0, Duration::from_secs(60)).unwrap(),
            LaunchDecision::Broken { reason } if reason.contains("no previous"))
        );
        let broken = layout.pointer().unwrap().unwrap();
        assert_eq!(broken.current, v.to_string());
        assert_eq!(broken.attempts, 1);
        assert_eq!(broken.state, PointerState::Pending);
    }

    #[test]
    fn missing_previous_is_never_selected() {
        let (_root, layout) = fixture();
        let (old, new, pointer) = pending_pair(&layout);
        fs::remove_dir_all(layout.version_dir(&old)).unwrap();
        assert!(layout.rollback("fallback disappeared").is_err());
        assert_eq!(layout.pointer().unwrap().unwrap(), pointer);
        assert!(matches!(
            layout
                .begin_launch_at(pointer.activated_at, 0, Duration::MAX)
                .unwrap(),
            LaunchDecision::Broken { .. }
        ));
        assert_eq!(layout.pointer().unwrap().unwrap().current, new.to_string());
        fs::remove_dir_all(layout.version_dir(&new)).unwrap();
        assert!(matches!(
            layout
                .begin_launch_at(pointer.activated_at, 0, Duration::MAX)
                .unwrap(),
            LaunchDecision::Broken { .. }
        ));
    }

    #[test]
    fn attempt_counter_cannot_wrap() {
        let (_root, layout) = fixture();
        let (_, _, mut pointer) = pending_pair(&layout);
        pointer.attempts = u32::MAX;
        layout.save(&pointer).unwrap();
        assert!(matches!(
            layout
                .begin_launch_at(pointer.activated_at, u32::MAX, Duration::MAX)
                .unwrap(),
            LaunchDecision::RolledBack { .. }
        ));
    }

    #[test]
    fn prune_protects_recovery_versions_and_newest_others() {
        let (_root, layout) = fixture();
        for value in ["1.0.0", "2.0.0", "3.0.0", "4.0.0", "5.0.0", "6.0.0"] {
            install(&layout, value);
        }
        layout.activate(&version("1.0.0")).unwrap();
        layout.confirm(&version("1.0.0")).unwrap();
        layout.activate(&version("2.0.0")).unwrap();
        let pointer = layout.activate(&version("3.0.0")).unwrap();
        assert_eq!(pointer.last_confirmed.as_deref(), Some("1.0.0"));
        assert_eq!(pointer.previous.as_deref(), Some("2.0.0"));
        assert_eq!(
            layout.prune(1).unwrap(),
            [version("4.0.0"), version("5.0.0")]
        );
        assert_eq!(
            layout.versions().unwrap(),
            [
                version("1.0.0"),
                version("2.0.0"),
                version("3.0.0"),
                version("6.0.0")
            ]
        );
        assert_eq!(layout.prune(0).unwrap(), [version("6.0.0")]);
        assert!(layout.version_dir(&version(&pointer.current)).is_dir());
        assert_eq!(layout.pointer().unwrap().unwrap(), pointer);
    }

    #[test]
    fn prune_cleans_stages_and_pointer_remnants() {
        let (root, layout) = fixture();
        let stage = layout.begin_stage(&version("1.0.0")).unwrap();
        fs::write(stage.join("partial"), b"partial").unwrap();
        let temporary = root.path().join(".current-interrupted.tmp");
        fs::write(&temporary, b"partial JSON").unwrap();
        let unrelated = root.path().join("unrelated.tmp");
        fs::write(&unrelated, b"keep").unwrap();
        assert!(layout.prune(0).unwrap().is_empty());
        assert!(!stage.exists());
        assert!(!temporary.exists());
        assert!(unrelated.exists());
        assert!(root.path().join("layout.lock").exists());
    }

    #[test]
    fn corrupt_pointer_is_untouched() {
        let (root, layout) = fixture();
        let v = install(&layout, "1.0.0");
        let pointer = layout.activate(&v).unwrap();
        let valid = serde_json::to_value(pointer).unwrap();
        let mut schema = valid.clone();
        schema["schema"] = 2.into();
        let mut unknown = valid.clone();
        unknown["surprise"] = true.into();
        let mut traversal = valid.clone();
        traversal["current"] = "../escape".into();
        let mut build = valid.clone();
        build["previous"] = "1.0.0+build".into();
        let mut failed = valid.clone();
        failed["failed"] = serde_json::json!(["../../escape"]);
        let mut state = valid;
        state["state"] = "unknown".into();
        let mut invalid = vec![b"{partial".to_vec(), b"{}".to_vec()];
        invalid.extend(
            [schema, unknown, traversal, build, failed, state]
                .into_iter()
                .map(|v| serde_json::to_vec(&v).unwrap()),
        );
        for bytes in invalid {
            fs::write(root.path().join("current.json"), &bytes).unwrap();
            assert!(layout.pointer().is_err());
            assert!(layout.activate(&v).is_err());
            assert!(layout.confirm(&v).is_err());
            assert!(layout.rollback("corrupt").is_err());
            assert!(layout.begin_launch_at(0, 0, Duration::ZERO).is_err());
            assert!(layout.prune(0).is_err());
            assert_eq!(fs::read(root.path().join("current.json")).unwrap(), bytes);
            assert!(layout.version_dir(&v).is_dir());
        }
    }

    #[test]
    fn rejects_reserved_directory_versions() {
        let (_root, layout) = fixture();
        for name in ["1.0.0+build", "1.0.0-dev.9.staging", "1.0.0-dev.9.STAGING"] {
            let v = version(name);
            assert!(layout.begin_stage(&v).is_err());
            assert!(layout.commit_stage(&v).is_err());
            assert!(layout.activate(&v).is_err());
            assert!(layout.activate_forced(&v).is_err());
            assert!(layout.confirm(&v).is_err());
            layout.abort_stage(&v);
        }
        assert!(layout.versions().unwrap().is_empty());
    }

    #[test]
    #[should_panic(expected = "version cannot be used as a runtime directory")]
    fn infallible_path_rejects_build_metadata() {
        Layout::new("unused").version_dir(&version("1.0.0+build"));
    }

    #[test]
    fn concurrent_activation_serializes() {
        let (_root, layout) = fixture();
        let (old, _, _) = pending_pair(&layout);
        layout.confirm(&version("2.0.0")).unwrap();
        let a = install(&layout, "3.0.0");
        let b = install(&layout, "4.0.0");
        let layout = Arc::new(layout);
        let barrier = Arc::new(Barrier::new(3));
        let handles: Vec<_> = [a.clone(), b.clone()]
            .into_iter()
            .map(|v| {
                let layout = Arc::clone(&layout);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    layout.activate(&v).unwrap()
                })
            })
            .collect();
        barrier.wait();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let final_pointer = layout.pointer().unwrap().unwrap();
        let earlier = results
            .iter()
            .find(|p| p.current != final_pointer.current)
            .unwrap();
        assert_eq!(earlier.previous.as_deref(), Some("2.0.0"));
        assert_eq!(
            final_pointer.previous.as_deref(),
            Some(earlier.current.as_str())
        );
        assert_eq!(final_pointer.last_confirmed.as_deref(), Some("2.0.0"));
        assert!(layout.version_dir(&old).is_dir());
    }

    #[test]
    fn concurrent_same_version_has_one_winner() {
        let (_root, layout) = fixture();
        let v = install(&layout, "1.0.0");
        let layout = Arc::new(layout);
        let barrier = Arc::new(Barrier::new(3));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let layout = Arc::clone(&layout);
                let barrier = Arc::clone(&barrier);
                let v = v.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    layout.activate(&v)
                })
            })
            .collect();
        barrier.wait();
        let wins = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(Result::is_ok)
            .count();
        assert_eq!(wins, 1);
        assert_eq!(layout.pointer().unwrap().unwrap().previous, None);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_runtime_is_not_launchable() {
        use std::os::unix::fs::symlink;

        let (root, layout) = fixture();
        let v = version("1.0.0");
        layout.begin_stage(&v).unwrap();
        let other = root.path().join("other");
        fs::create_dir(&other).unwrap();
        fs::write(other.join("keep"), b"keep").unwrap();
        symlink(&other, layout.version_dir(&v)).unwrap();
        assert!(layout.versions().unwrap().is_empty());
        assert!(layout.activate(&v).is_err());
        assert!(layout.commit_stage(&v).is_err());
        layout.prune(0).unwrap();
        assert!(other.join("keep").exists());
    }

    #[cfg(windows)]
    #[test]
    fn pointer_replacement_with_open_reader() {
        let (root, layout) = fixture();
        let v = install(&layout, "1.0.0");
        layout.activate(&v).unwrap();
        let reader = File::open(root.path().join("current.json")).unwrap();
        layout.confirm(&v).unwrap();
        assert_eq!(
            layout.pointer().unwrap().unwrap().state,
            PointerState::Confirmed
        );
        drop(reader);
    }
}
