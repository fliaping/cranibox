//! Bounded, private operation records. Recover running records without replaying commands.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

pub const LIMIT: usize = 128;
pub struct Journal {
    directory: PathBuf,
    records: BTreeMap<String, Value>,
    dirty: BTreeSet<String>,
}
impl Journal {
    pub fn open(directory: PathBuf) -> Result<Self> {
        fs::create_dir_all(&directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        }
        let mut journal = Self {
            directory,
            records: BTreeMap::new(),
            dirty: BTreeSet::new(),
        };
        for entry in fs::read_dir(&journal.directory)? {
            let entry = entry?;
            if entry.path().extension().is_none_or(|s| s != "json") {
                continue;
            }
            ensure!(
                entry.file_type()?.is_file(),
                "history record must be a regular file"
            );
            ensure!(
                entry.metadata()?.len() <= 512 * 1024,
                "history record exceeds limit"
            );
            let mut value: Value = serde_json::from_slice(&fs::read(entry.path())?)
                .context("invalid operation history")?;
            let id = value["id"]
                .as_str()
                .context("history ID missing")?
                .to_owned();
            ensure!(
                Self::valid_id(&id) && entry.file_name().to_string_lossy() == format!("{id}.json"),
                "invalid history ID"
            );
            if value["outcome"] == "running" {
                value["outcome"] = json!("interrupted");
                value["error"] = json!("service restarted; command was not replayed");
                journal.dirty.insert(id.clone());
            }
            journal.records.insert(id, value);
        }
        journal.prune()?;
        journal.flush()?;
        Ok(journal)
    }
    fn valid_id(id: &str) -> bool {
        !id.is_empty() && id.len() <= 64 && id.bytes().all(|c| c.is_ascii_hexdigit() || c == b'-')
    }
    pub fn insert(&mut self, value: Value) -> Result<()> {
        let id = value["id"]
            .as_str()
            .context("history ID missing")?
            .to_owned();
        ensure!(Self::valid_id(&id), "invalid history ID");
        ensure!(
            self.records.contains_key(&id)
                || self.records.len() < LIMIT
                || self.records.values().any(|v| v["outcome"] != "running"),
            "operation history full"
        );
        ensure!(
            serde_json::to_vec(&value)?.len() <= 512 * 1024,
            "history record exceeds limit"
        );
        self.records.insert(id.clone(), value);
        self.dirty.insert(id);
        self.prune()?;
        self.flush()
    }
    pub fn update(&mut self, id: &str, update: impl FnOnce(&mut Value)) {
        if let Some(value) = self.records.get_mut(id) {
            update(value);
            self.dirty.insert(id.to_owned());
        }
    }
    pub fn get(&self, id: &str) -> Option<Value> {
        self.records.get(id).cloned()
    }
    pub fn list(&self) -> Vec<Value> {
        let mut values: Vec<_> = self.records.values().cloned().collect();
        values.sort_by_key(|v| std::cmp::Reverse(v["created_at"].as_u64().unwrap_or(0)));
        values
    }
    fn prune(&mut self) -> Result<()> {
        while self.records.len() > LIMIT {
            let id = self
                .records
                .iter()
                .filter(|(_, v)| v["outcome"] != "running")
                .min_by_key(|(_, v)| v["created_at"].as_u64().unwrap_or(0))
                .map(|(id, _)| id.clone())
                .context("operation history full")?;
            self.records.remove(&id);
            self.dirty.remove(&id);
            let path = self.directory.join(format!("{id}.json"));
            if path.exists() {
                fs::remove_file(path)?;
            }
        }
        Ok(())
    }
    pub fn flush(&mut self) -> Result<()> {
        for id in self.dirty.clone() {
            if let Some(value) = self.records.get(&id) {
                crate::config::write_private(
                    &self.directory.join(format!("{id}.json")),
                    &serde_json::to_vec(value)?,
                    true,
                )?;
            }
            self.dirty.remove(&id);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restart_marks_interrupted_and_keeps_output() {
        let root = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(root.path().into()).unwrap();
        journal
            .insert(json!({"id":"abc123","outcome":"running","created_at":1,"output":"hello"}))
            .unwrap();
        let reopened = Journal::open(root.path().into()).unwrap();
        assert_eq!(reopened.get("abc123").unwrap()["outcome"], "interrupted");
        assert_eq!(reopened.get("abc123").unwrap()["output"], "hello");
        assert!(journal.insert(json!({"id":"../escape"})).is_err());
    }
    #[test]
    fn full_running_history_rejects_without_growing() {
        let root = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(root.path().into()).unwrap();
        for i in 0..LIMIT {
            journal
                .insert(json!({"id":format!("{i:x}"),"outcome":"running","created_at":i}))
                .unwrap();
        }
        assert!(
            journal
                .insert(json!({"id":"ffff","outcome":"running"}))
                .is_err()
        );
        assert_eq!(journal.list().len(), LIMIT);
        assert!(journal.get("ffff").is_none());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), LIMIT);
    }
    #[test]
    fn evicts_old_completed_records() {
        let root = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(root.path().into()).unwrap();
        for i in 0..130 {
            journal
                .insert(json!({"id":format!("{i:x}"),"outcome":"exited","created_at":i}))
                .unwrap();
        }
        assert_eq!(journal.list().len(), LIMIT);
        assert!(journal.get("0").is_none());
        assert_eq!(
            Journal::open(root.path().into()).unwrap().list().len(),
            LIMIT
        );
    }
}
