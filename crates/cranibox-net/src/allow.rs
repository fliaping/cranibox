//! Hot-reloadable set of EndpointIds allowed to use the relay and the mesh.
use iroh::EndpointId;
use std::{
    collections::HashSet,
    sync::{Arc, RwLock},
};
use tokio::sync::watch;

#[derive(Clone, Debug)]
pub struct Allowlist {
    ids: Arc<RwLock<HashSet<EndpointId>>>,
    changed: watch::Sender<u64>,
}
impl Default for Allowlist {
    fn default() -> Self {
        Self {
            ids: Arc::default(),
            changed: watch::channel(0).0,
        }
    }
}
impl Allowlist {
    pub fn new(ids: impl IntoIterator<Item = EndpointId>) -> Self {
        let list = Self::default();
        list.set(ids);
        list
    }
    pub fn contains(&self, id: &EndpointId) -> bool {
        self.ids.read().unwrap().contains(id)
    }
    /// Replace the set; returns the ids that were removed (to disconnect them).
    pub fn set(&self, ids: impl IntoIterator<Item = EndpointId>) -> Vec<EndpointId> {
        let next: HashSet<_> = ids.into_iter().collect();
        let mut current = self.ids.write().unwrap();
        if *current == next {
            return Vec::new();
        }
        let removed = current.difference(&next).copied().collect();
        *current = next;
        drop(current);
        self.changed.send_modify(|v| *v += 1);
        removed
    }
    pub fn insert(&self, id: EndpointId) {
        if self.ids.write().unwrap().insert(id) {
            self.changed.send_modify(|v| *v += 1);
        }
    }
    pub fn snapshot(&self) -> HashSet<EndpointId> {
        self.ids.read().unwrap().clone()
    }
    pub fn len(&self) -> usize {
        self.ids.read().unwrap().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }
}
