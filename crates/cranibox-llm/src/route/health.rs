//! Debounced per-node health. Existing streams never consult this state again.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Mutex};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Remote,
    Suspect,
    Local,
    Probing,
}
#[derive(Clone, Debug)]
pub struct Health {
    pub mode: Mode,
    failures: u8,
    successes: u8,
    recovery_since: Option<u64>,
}
impl Default for Health {
    fn default() -> Self {
        Self {
            mode: Mode::Remote,
            failures: 0,
            successes: 0,
            recovery_since: None,
        }
    }
}
impl Health {
    pub fn available(&self) -> bool {
        matches!(self.mode, Mode::Remote | Mode::Suspect)
    }
    pub fn failed(&mut self) {
        self.successes = 0;
        self.recovery_since = None;
        self.failures = self.failures.saturating_add(1);
        self.mode = if self.failures >= 3 {
            Mode::Local
        } else if self.available() {
            Mode::Suspect
        } else {
            Mode::Local
        };
    }
    pub fn succeeded(&mut self, at: u64) {
        self.failures = 0;
        if self.available() {
            self.mode = Mode::Remote;
            self.successes = 0;
            self.recovery_since = None;
            return;
        }
        self.mode = Mode::Probing;
        self.successes = self.successes.saturating_add(1);
        let start = *self.recovery_since.get_or_insert(at);
        if self.successes >= 5 && at.saturating_sub(start) >= 30 {
            self.mode = Mode::Remote;
            self.recovery_since = None;
            self.successes = 0;
        }
    }
}
#[derive(Default)]
pub struct HealthBook(Mutex<BTreeMap<String, Health>>);
impl HealthBook {
    pub fn get(&self, node: &str) -> Health {
        self.0
            .lock()
            .expect("health lock")
            .get(node)
            .cloned()
            .unwrap_or_default()
    }
    pub fn failed(&self, node: &str) {
        self.0
            .lock()
            .expect("health lock")
            .entry(node.into())
            .or_default()
            .failed();
    }
    pub fn succeeded(&self, node: &str, at: u64) {
        self.0
            .lock()
            .expect("health lock")
            .entry(node.into())
            .or_default()
            .succeeded(at);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_requires_both_time_and_five_successes() {
        let mut h = Health::default();
        h.failed();
        h.failed();
        assert!(h.available());
        h.failed();
        assert_eq!(h.mode, Mode::Local);
        for t in [1, 2, 3, 4, 5] {
            h.succeeded(t);
        }
        assert_eq!(h.mode, Mode::Probing);
        h.succeeded(31);
        assert_eq!(h.mode, Mode::Remote);
        h.failed();
        h.failed();
        h.failed();
        h.succeeded(100);
        h.succeeded(140);
        assert!(!h.available());
        h.failed();
        assert_eq!(h.mode, Mode::Local);
    }
}
