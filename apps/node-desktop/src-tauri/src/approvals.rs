//! Native surfaces share the App API's expiry and decision checks.
use crate::{AppState, show};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::Arc;
use tauri::{
    Manager,
    menu::{MenuItem, Submenu},
};
use tauri_plugin_notification::{Action, ActionType, NotificationExt};

pub fn inbox(app: &tauri::AppHandle) {
    show(app);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.eval("location.assign('/app/inbox')");
    }
}

pub fn decide(app: &tauri::AppHandle, id: String, decision: String) {
    let handle = app.clone();
    let state = app.state::<Arc<AppState>>().inner().clone();
    tauri::async_runtime::spawn(async move {
        // Never trust a stale tray item or notification: the service rechecks
        // the current queue, expiration and offered actions before release.
        if crate::managed::dispatch(
            &state,
            "POST",
            &format!("/approvals/{id}"),
            json!({"decision":decision}),
        )
        .await
        .is_err()
        {
            inbox(&handle);
        }
    });
}

pub fn setup(app: &tauri::AppHandle) -> tauri_plugin_notification::Result<()> {
    app.notification().register_action_types(vec![
        ActionType::builder("approval")
            .actions(vec![
                Action::builder("allow_once", "允许一次 / Allow once")
                    .requires_authentication(true)
                    .build(),
                Action::builder("deny", "拒绝 / Deny")
                    .requires_authentication(true)
                    .destructive(true)
                    .build(),
            ])
            .build(),
    ])?;
    let handle = app.clone();
    app.notification().on_action(move |action| {
        if matches!(action.action_id(), "allow_once" | "deny")
            && let Some(id) = action
                .notification()
                .and_then(|n| n.extra().get("approval_id"))
                .and_then(Value::as_str)
        {
            decide(&handle, id.to_owned(), action.action_id().to_owned());
            return;
        }
        if action.action_id() != "dismiss" {
            inbox(&handle);
        }
    })
}

pub fn refresh(
    app: &tauri::AppHandle,
    menu: &Submenu<tauri::Wry>,
    cards: &[Value],
    seen: &mut BTreeSet<String>,
) -> tauri::Result<()> {
    for item in menu.items()? {
        menu.remove(&item)?;
    }
    let active: BTreeSet<_> = cards
        .iter()
        .filter_map(|c| c["id"].as_str().map(str::to_owned))
        .collect();
    seen.retain(|id| active.contains(id));
    for (index, card) in cards.iter().enumerate() {
        let Some(id) = card["id"].as_str() else {
            continue;
        };
        if !seen.contains(id) {
            // Keep message content and command details off the lock screen.
            if app
                .notification()
                .builder()
                .title("Cranibox · 待审批 / Approval needed")
                .body("有新的请求等待你的决定 / A new request needs your decision")
                .action_type_id("approval")
                .extra("approval_id", id)
                .show()
                .is_ok()
            {
                seen.insert(id.to_owned());
            }
        }
        if index >= 5 {
            continue;
        }
        let entry = Submenu::with_id(
            app,
            format!("request:{id}"),
            format!("请求 / Request {}", index + 1),
            true,
        )?;
        for (decision, label) in [
            ("allow_once", "允许一次 / Allow once"),
            ("deny", "拒绝 / Deny"),
        ] {
            if card["actions"]
                .as_array()
                .is_some_and(|items| items.iter().any(|v| v == decision))
            {
                entry.append(&MenuItem::with_id(
                    app,
                    format!("approval:{decision}:{id}"),
                    label,
                    true,
                    None::<&str>,
                )?)?;
            }
        }
        menu.append(&entry)?;
    }
    menu.set_enabled(!cards.is_empty())?;
    Ok(())
}
