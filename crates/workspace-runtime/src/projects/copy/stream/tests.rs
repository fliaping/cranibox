use super::*;

struct Fixture {
    _temp: tempfile::TempDir,
    state: PathBuf,
    root: PathBuf,
    store: Store,
    id: String,
    manifest: Vec<u8>,
    bytes: Vec<u8>,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let bytes = (0..CHUNK * 2 + 13)
            .map(|i| (i % 251) as u8)
            .collect::<Vec<_>>();
        let entry = |path: &str, data: &[u8]| super::super::File {
            path: path.into(),
            size: data.len(),
            sha256: digest(data),
            executable: false,
            data: String::new(),
        };
        let manifest = serde_json::to_vec(&Package {
            version: 2,
            source: "source".into(),
            project_id: uuid::Uuid::new_v4().to_string(),
            replica_id: uuid::Uuid::new_v4().to_string(),
            name: "Chunked project".into(),
            created_at: now(),
            directories: vec!["src".into()],
            files: vec![
                entry("00-empty", b""),
                entry("src/main.rs", &bytes),
                entry("zz-empty", b""),
            ],
            skipped: vec![],
        })
        .unwrap();
        Self {
            state: root.join("peers.json"),
            store: Store::new(root.join("projects.json")),
            root,
            _temp: temp,
            id: uuid::Uuid::new_v4().to_string(),
            manifest,
            bytes,
        }
    }
    fn run(&self, command: Command) -> Result<Value> {
        manage(
            &self.state,
            &self.store,
            "target",
            "caller",
            "private-token",
            command,
            |_| Ok(()),
        )
    }
    fn begin(&self) -> Command {
        Command::Begin {
            id: self.id.clone(),
            expected_root: self.root.clone(),
            digest: digest(&self.manifest),
            manifest: serde_json::from_slice(&self.manifest).unwrap(),
        }
    }
    fn write(&self, offset: usize) -> Command {
        let bytes = &self.bytes[offset..(offset + CHUNK).min(self.bytes.len())];
        Command::Write {
            id: self.id.clone(),
            digest: digest(&self.manifest),
            chunk: Chunk {
                index: 1,
                offset,
                data: STANDARD.encode(bytes),
                sha256: digest(bytes),
            },
        }
    }
    fn finish(&self) -> Command {
        Command::Finish {
            id: self.id.clone(),
            digest: digest(&self.manifest),
        }
    }
    fn record(&self) -> Receiving {
        read_receiving(&self.state, &self.id, "caller", "private-token").unwrap()
    }
}

#[test]
fn durable_chunks_recover_lost_acknowledgements_and_publication_without_overwriting_edits() {
    let f = Fixture::new();
    assert_eq!(f.run(f.begin()).unwrap()["progress"]["file_index"], 1);
    assert!(f.run(f.finish()).is_err());
    f.run(f.write(0)).unwrap();
    // A retry of a successfully stored but unacknowledged chunk is read-only.
    assert_eq!(f.run(f.write(0)).unwrap()["progress"]["bytes"], CHUNK);
    let pending = f.record().pending();
    fs::OpenOptions::new()
        .append(true)
        .open(pending.join("src/main.rs"))
        .unwrap()
        .write_all(b"unacknowledged crash tail")
        .unwrap();
    // Only private bytes beyond the durable offset may be replaced on resume.
    f.run(f.write(CHUNK)).unwrap();
    let last = f.run(f.write(CHUNK * 2)).unwrap();
    assert_eq!(last["progress"]["file_index"], 3);
    assert_eq!(last["progress"]["bytes"], f.bytes.len());
    // Simulate a restart after rename but before the copied checkpoint.
    let mut record = f.record();
    record.receipt.state = "publishing".into();
    save(&receiving_path(&f.state, &f.id).unwrap(), &record).unwrap();
    fs::rename(pending, &record.receipt.path).unwrap();
    assert_eq!(f.run(f.finish()).unwrap()["state"], "completed");
    let path = record.receipt.path.join("src/main.rs");
    assert_eq!(fs::read(&path).unwrap(), f.bytes);
    fs::write(&path, b"target edits").unwrap();
    assert_eq!(f.run(f.begin()).unwrap()["state"], "completed");
    assert_eq!(f.run(f.finish()).unwrap()["state"], "completed");
    assert_eq!(fs::read(&path).unwrap(), b"target edits");
    assert_eq!(
        f.store
            .apply_owned(super::super::Command::List {}, "target")
            .unwrap()
            .projects[0]
            .replicas
            .len(),
        1
    );
}

#[test]
fn chunks_reject_corruption_wrong_offsets_credentials_and_revocation() {
    let f = Fixture::new();
    f.run(f.begin()).unwrap();
    assert!(f.run(f.write(CHUNK)).is_err());
    let mut corrupt = f.write(0);
    if let Command::Write { chunk, .. } = &mut corrupt {
        chunk.sha256 = "0".repeat(64);
    }
    assert!(f.run(corrupt).is_err());
    assert_eq!(f.record().bytes, 0);
    assert!(
        manage(
            &f.state,
            &f.store,
            "target",
            "caller",
            "rotated",
            Command::Status { id: f.id.clone() },
            |_| Ok(())
        )
        .is_err()
    );
    assert!(
        manage(
            &f.state,
            &f.store,
            "target",
            "other",
            "private-token",
            Command::Status { id: f.id.clone() },
            |_| Ok(())
        )
        .is_err()
    );
    assert!(
        manage(
            &f.state,
            &f.store,
            "target",
            "caller",
            "private-token",
            f.write(0),
            |_| anyhow::bail!("revoked")
        )
        .is_err()
    );
    assert_eq!(f.record().bytes, 0);
    let mut changed = f.begin();
    if let Command::Begin { expected_root, .. } = &mut changed {
        *expected_root = f.root.join("other");
    }
    assert!(f.run(changed).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = receiving_path(&f.state, &f.id).unwrap();
        for private in [&path, &path.with_extension("manifest")] {
            assert_eq!(
                fs::metadata(private).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}

#[test]
fn manifest_limits_reject_oversized_and_embedded_content_before_creating_a_destination() {
    let f = Fixture::new();
    let mut manifest: Package = serde_json::from_slice(&f.manifest).unwrap();
    manifest.files = vec![super::super::File {
        path: "large.bin".into(),
        size: MAX_FILE + 1,
        sha256: "a".repeat(64),
        executable: false,
        data: String::new(),
    }];
    assert!(manifest.validate().is_err());
    manifest.files[0].size = MAX_FILE;
    assert!(manifest.validate().is_ok());
    manifest.files.push(super::super::File {
        path: "second.bin".into(),
        size: MAX_FILE,
        sha256: "b".repeat(64),
        executable: false,
        data: String::new(),
    });
    assert!(manifest.validate().is_ok());
    manifest.files.push(super::super::File {
        path: "over.bin".into(),
        size: 1,
        sha256: "c".repeat(64),
        executable: false,
        data: String::new(),
    });
    let expected = digest(&serde_json::to_vec(&manifest).unwrap());
    assert!(
        f.run(Command::Begin {
            id: f.id.clone(),
            expected_root: f.root.clone(),
            digest: expected,
            manifest,
        })
        .is_err()
    );
    assert!(!f.root.join(format!("cranibox-{}", f.id)).exists());
    let mut embedded: Package = serde_json::from_slice(&f.manifest).unwrap();
    embedded.files[0].data = STANDARD.encode(b"do not embed file contents");
    assert!(embedded.validate().is_err());
}

#[test]
fn full_receipt_capacity_still_allows_resuming_the_original_id() {
    let f = Fixture::new();
    f.run(f.begin()).unwrap();
    let directory = receiving_path(&f.state, &f.id)
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    for _ in 1..256 {
        fs::write(
            directory.join(format!("{}.json", uuid::Uuid::new_v4())),
            b"{}",
        )
        .unwrap();
    }
    assert_eq!(f.run(f.begin()).unwrap()["state"], "receiving");
    let mut fresh = f.begin();
    let next = uuid::Uuid::new_v4().to_string();
    if let Command::Begin { id, .. } = &mut fresh {
        *id = next.clone();
    }
    assert!(
        f.run(fresh)
            .unwrap_err()
            .to_string()
            .contains("receipt limit")
    );
    assert!(!f.root.join(format!("cranibox-{next}")).exists());
    assert_eq!(f.run(f.write(0)).unwrap()["progress"]["bytes"], CHUNK);
}

#[cfg(unix)]
#[test]
fn staging_symlinks_and_hardlinks_cannot_modify_outside_files() {
    let f = Fixture::new();
    f.run(f.begin()).unwrap();
    let outside = f.root.join("outside");
    fs::write(&outside, b"untouched").unwrap();
    let path = f.record().pending().join("src/main.rs");
    std::os::unix::fs::symlink(&outside, &path).unwrap();
    assert!(f.run(f.write(0)).is_err());
    fs::remove_file(&path).unwrap();
    fs::hard_link(&outside, &path).unwrap();
    assert!(f.run(f.write(0)).is_err());
    assert_eq!(fs::read(outside).unwrap(), b"untouched");
}
