//! Invitations to join another device's identity, received from an already paired
//! device. Each waits in the Inbox until the owner accepts (this device then pairs
//! with the invitation as an owner device) or declines. Kept in memory: an
//! invitation lives for minutes, and a restart simply drops it.
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};

pub const TTL_SECS: u64 = 10 * 60;
const MAX_PENDING: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invitation {
    pub id: String,
    /// The inviting device: the identity this device would join.
    pub from: String,
    pub name: String,
    pub link: String,
    pub created_at: u64,
}

static PENDING: LazyLock<Mutex<BTreeMap<PathBuf, Vec<Invitation>>>> =
    LazyLock::new(Default::default);

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Record an invitation for the node whose configuration is `peers`. A newer
/// invitation from the same device replaces the older one.
pub fn offer(peers: &Path, from: &str, name: &str, link: &str) -> anyhow::Result<String> {
    anyhow::ensure!(
        link.starts_with("cranibox://pair/") && link.len() <= 4096,
        "Not a pairing invitation"
    );
    let invitation = Invitation {
        id: uuid::Uuid::new_v4().to_string(),
        from: from.to_owned(),
        name: name.chars().filter(|c| !c.is_control()).take(64).collect(),
        link: link.to_owned(),
        created_at: now(),
    };
    let mut all = PENDING.lock().unwrap();
    let list = all.entry(peers.to_owned()).or_default();
    list.retain(|i| i.from != from && now() < i.created_at + TTL_SECS);
    if list.len() >= MAX_PENDING {
        list.remove(0);
    }
    let id = invitation.id.clone();
    list.push(invitation);
    Ok(id)
}
pub fn pending(peers: &Path) -> Vec<Invitation> {
    let mut all = PENDING.lock().unwrap();
    let list = all.entry(peers.to_owned()).or_default();
    list.retain(|i| now() < i.created_at + TTL_SECS);
    list.clone()
}
/// Remove and return one invitation (accepted or declined).
pub fn take(peers: &Path, id: &str) -> Option<Invitation> {
    let mut all = PENDING.lock().unwrap();
    let list = all.get_mut(peers)?;
    let index = list.iter().position(|i| i.id == id)?;
    Some(list.remove(index))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invitations_are_per_node_replace_older_ones_and_are_taken_once() {
        let a = Path::new("/tmp/cranibox-join-a/peers.json");
        let b = Path::new("/tmp/cranibox-join-b/peers.json");
        assert!(offer(a, "node-x", "Mac", "https://evil.example").is_err());
        offer(a, "node-x", "Mac", "cranibox://pair/one").unwrap();
        let second = offer(a, "node-x", "Mac\u{7}", "cranibox://pair/two").unwrap();
        let listed = pending(a);
        assert_eq!(listed.len(), 1);
        assert_eq!(
            (listed[0].link.as_str(), listed[0].name.as_str()),
            ("cranibox://pair/two", "Mac")
        );
        assert!(pending(b).is_empty());
        assert_eq!(take(a, &second).unwrap().from, "node-x");
        assert!(take(a, &second).is_none());
    }
}
