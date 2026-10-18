//! Owner-authenticated catalog reconciliation. The reader sends only IDs and
//! hashes; the owner returns changed records and deletions. No remote edits,
//! event history, clock ordering, or authority are inferred from a cached index.
use super::{Catalog, Project};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const PROTOCOL: &str = "project-index-v1";
const MAX_PROJECTS: usize = 512;

fn digest(value: &impl Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn valid_id(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok()
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    pub digest: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Index {
    pub base: String,
    pub known: Vec<Entry>,
}
impl Index {
    pub fn from_catalog(catalog: &Catalog) -> Result<Self> {
        Ok(Self {
            base: digest(catalog)?,
            known: catalog
                .projects
                .iter()
                .map(|p| {
                    Ok(Entry {
                        id: p.id.clone(),
                        digest: digest(p)?,
                    })
                })
                .collect::<Result<_>>()?,
        })
    }
    fn validate(&self) -> Result<()> {
        let mut ids = BTreeSet::new();
        ensure!(
            valid_digest(&self.base)
                && self.known.len() <= MAX_PROJECTS
                && self
                    .known
                    .iter()
                    .all(|p| valid_id(&p.id) && valid_digest(&p.digest) && ids.insert(&p.id)),
            "invalid project sync index"
        );
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delta {
    pub base: String,
    pub revision: u64,
    pub digest: String,
    pub changed: Vec<Project>,
    pub removed: Vec<String>,
    pub order: Vec<String>,
}
impl Delta {
    /// Call only after filtering the authoritative catalog by target ownership.
    /// Comparing record hashes also handles old cursors, restored backups and
    /// missed deletions without retaining an unbounded event log.
    pub fn between(catalog: &Catalog, index: &Index) -> Result<Self> {
        index.validate()?;
        ensure!(
            catalog.projects.len() <= MAX_PROJECTS,
            "project sync catalog too large"
        );
        let known: BTreeMap<_, _> = index.known.iter().map(|p| (&p.id, &p.digest)).collect();
        let mut changed = Vec::new();
        let mut present = BTreeSet::new();
        for project in &catalog.projects {
            present.insert(&project.id);
            if known.get(&project.id).copied() != Some(&digest(project)?) {
                changed.push(project.clone());
            }
        }
        Ok(Self {
            base: index.base.clone(),
            revision: catalog.revision,
            digest: digest(catalog)?,
            changed,
            removed: index
                .known
                .iter()
                .filter(|p| !present.contains(&p.id))
                .map(|p| p.id.clone())
                .collect(),
            order: catalog.projects.iter().map(|p| p.id.clone()).collect(),
        })
    }

    /// Reject incomplete, reordered, duplicated or mismatched replies before
    /// replacing the reader's private snapshot. Target ownership is checked by
    /// the transport after reconstruction, including all unchanged records.
    pub fn apply(self, previous: &Catalog) -> Result<Catalog> {
        ensure!(self.base == digest(previous)?, "project sync base changed");
        ensure!(
            valid_digest(&self.digest)
                && self.changed.len() <= MAX_PROJECTS
                && self.removed.len() <= MAX_PROJECTS
                && self.order.len() <= MAX_PROJECTS,
            "invalid project sync delta"
        );
        let mut records: BTreeMap<_, _> = previous
            .projects
            .iter()
            .map(|p| (p.id.clone(), p.clone()))
            .collect();
        let mut touched = BTreeSet::new();
        for id in self.removed {
            ensure!(
                touched.insert(id.clone()) && records.remove(&id).is_some(),
                "invalid project sync deletion"
            );
        }
        for project in self.changed {
            ensure!(
                valid_id(&project.id) && touched.insert(project.id.clone()),
                "duplicate project sync record"
            );
            records.insert(project.id.clone(), project);
        }
        let mut catalog = Catalog {
            revision: self.revision,
            projects: Vec::new(),
        };
        for id in self.order {
            catalog.projects.push(
                records
                    .remove(&id)
                    .ok_or_else(|| anyhow::anyhow!("invalid project sync order"))?,
            );
        }
        ensure!(
            records.is_empty() && digest(&catalog)? == self.digest,
            "incomplete project sync catalog"
        );
        Ok(catalog)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::{Command, Store};
    fn register(path: &str) -> Command {
        Command::Register {
            project_id: None,
            name: path.into(),
            node: Some("owner".into()),
            path: path.into(),
        }
    }
    #[test]
    fn reconciles_changes_deletions_and_revision_rollback_without_history() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::new(temp.path().join("projects.json"));
        let before = store.apply_owned(register("/first"), "owner").unwrap();
        let index = Index::from_catalog(&before).unwrap();
        let unchanged = Delta::between(&before, &index).unwrap();
        assert!(unchanged.changed.is_empty() && unchanged.removed.is_empty());
        unchanged.apply(&before).unwrap();
        let replica = before.projects[0].replicas[0].id.clone();
        store
            .apply_owned(
                Command::Bind {
                    replica_id: replica.clone(),
                    terminal_id: uuid::Uuid::new_v4().to_string(),
                },
                "owner",
            )
            .unwrap();
        let after = store.apply_owned(register("/second"), "owner").unwrap();
        let delta = Delta::between(&after, &index).unwrap();
        assert_eq!(delta.changed.len(), 2);
        assert_eq!(
            serde_json::to_value(delta.apply(&before).unwrap()).unwrap(),
            serde_json::to_value(&after).unwrap()
        );
        let removed = store
            .apply_owned(
                Command::Forget {
                    replica_id: replica,
                },
                "owner",
            )
            .unwrap();
        let delta = Delta::between(&removed, &Index::from_catalog(&after).unwrap()).unwrap();
        assert_eq!(delta.removed, vec![before.projects[0].id.clone()]);
        delta.apply(&after).unwrap();
        let rollback = Delta::between(&before, &Index::from_catalog(&removed).unwrap()).unwrap();
        assert_eq!(
            serde_json::to_value(rollback.apply(&removed).unwrap()).unwrap(),
            serde_json::to_value(&before).unwrap()
        );
    }
    #[test]
    fn rejects_duplicate_indexes_wrong_bases_and_incomplete_or_duplicate_replies() {
        let catalog = Catalog::default();
        let index = Index::from_catalog(&catalog).unwrap();
        let mut delta = Delta::between(&catalog, &index).unwrap();
        delta.base = "0".repeat(64);
        assert!(delta.apply(&catalog).is_err());
        let mut delta = Delta::between(&catalog, &index).unwrap();
        delta.digest = "0".repeat(64);
        assert!(delta.apply(&catalog).is_err());
        let id = uuid::Uuid::new_v4().to_string();
        let duplicate = Index {
            base: index.base,
            known: vec![
                Entry {
                    id: id.clone(),
                    digest: "a".repeat(64),
                },
                Entry {
                    id,
                    digest: "a".repeat(64),
                },
            ],
        };
        assert!(Delta::between(&catalog, &duplicate).is_err());
    }
}
