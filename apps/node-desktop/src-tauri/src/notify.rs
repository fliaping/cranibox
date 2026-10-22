//! Owner notification for the tray: messages waiting in the inbound approval queue.
//! Reads the existing mailbox count; approving still happens in the Web UI or CLI.
use std::path::PathBuf;

/// Messages held for the owner, or 0 when the mailbox is unavailable.
pub async fn held(path: PathBuf) -> u64 {
    use agent_workspace_runtime::peers::{Store, messages::Command};
    Store::new(path)
        .messages(Command::Held)
        .await
        .ok()
        .and_then(|view| view["held"].as_array().map(|held| held.len() as u64))
        .unwrap_or(0)
}

/// Tray tooltip for the node status and the approval queue.
pub fn tooltip(status: &str, held: u64) -> String {
    match held {
        0 => format!("Cranibox — {status}"),
        n => format!("Cranibox — {status} · {n} 条消息待批准 / {n} awaiting approval"),
    }
}

/// Tray menu entry for the approval queue. Linux tray hosts do not show tooltips
/// (appindicator), so the count must also appear in the menu and the icon title.
pub fn menu_label(held: u64) -> String {
    match held {
        0 => "设备消息：没有待批准 / No messages awaiting approval".to_owned(),
        n => format!("{n} 条消息待批准，打开审批 / {n} awaiting approval — review"),
    }
}

/// Text next to the tray icon where the host supports it (Linux appindicator).
pub fn title(held: u64) -> Option<String> {
    (held > 0).then(|| format!("{held} 待批准"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tooltip_mentions_the_queue_only_when_messages_wait() {
        assert_eq!(tooltip("运行中", 0), "Cranibox — 运行中");
        assert!(tooltip("运行中", 2).ends_with("2 条消息待批准 / 2 awaiting approval"));
        assert!(menu_label(0).contains("没有待批准"));
        assert!(menu_label(3).starts_with("3 条消息待批准"));
        assert_eq!(title(0), None);
        assert_eq!(title(2).as_deref(), Some("2 待批准"));
    }
}
