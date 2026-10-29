use super::*;
use crate::app::{error, remote_error};
use cranibox_contracts::app::AppError;
use serde_json::{Value, json};

fn invalid(e: impl std::fmt::Display) -> AppError {
    error(400, "invalid", e.to_string())
}
/// `gateway`: this device's model proxy address, resolved only for the review test.
pub(crate) async fn route(
    peers: &Path,
    method: &str,
    path: &str,
    query: &str,
    body: Value,
    gateway: Option<String>,
) -> Result<Value, AppError> {
    let store = Store::new(peers);
    let query: BTreeMap<_, _> = url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect();
    if method == "GET"
        && let Some(id) = path.strip_prefix("/access/requests/")
    {
        ensure_id(id)?;
        if let Some(node) = query.get("node") {
            return agent_workspace_runtime::client::Fleet::new(peers.to_owned())
                .get(node, &format!("/api/v1/access/requests/{id}"))
                .await
                .map_err(remote_error);
        }
        return Err(error(
            400,
            "invalid",
            "requester polling requires target node",
        ));
    }
    match (method, path) {
        ("GET", "/access") => serde_json::to_value(store.view().map_err(invalid)?).map_err(invalid),
        ("GET", "/access/audit") => {
            let limit = query
                .get("limit")
                .map(|s| s.parse::<usize>())
                .transpose()
                .map_err(invalid)?
                .unwrap_or(100);
            let before = query
                .get("before")
                .map(|s| s.parse::<u64>())
                .transpose()
                .map_err(invalid)?;
            let capability = query
                .get("capability")
                .map(|s| serde_json::from_value::<Capability>(json!(s)))
                .transpose()
                .map_err(invalid)?;
            let risk = query
                .get("risk")
                .map(|s| serde_json::from_value::<Risk>(json!(s)))
                .transpose()
                .map_err(invalid)?;
            let text = |key: &str| {
                query
                    .get(key)
                    .map(|s| s.trim().chars().take(200).collect::<String>())
                    .filter(|s| !s.is_empty())
            };
            let audit_query = AuditQuery {
                limit,
                before,
                node: text("node"),
                capability,
                category: text("event"),
                kind: text("kind"),
                risk,
                result: text("result"),
                reviewed: query.get("reviewed").is_some_and(|v| v == "true"),
                q: text("q"),
            };
            let view = tokio::task::spawn_blocking(move || store.query(&audit_query))
                .await
                .map_err(invalid)?
                .map_err(invalid)?;
            serde_json::to_value(view).map_err(invalid)
        }
        ("GET", "/access/review") => Ok(json!({
            "settings": store.review_settings(),
            "has_key": store.has_review_key(),
        })),
        ("PUT", "/access/review") => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Change {
                settings: ReviewSettings,
                key: Option<String>,
            }
            let change: Change = serde_json::from_value(body).map_err(invalid)?;
            store
                .set_review(change.settings, change.key.as_deref())
                .map_err(invalid)?;
            Ok(json!({"settings": store.review_settings(), "has_key": store.has_review_key()}))
        }
        ("POST", "/access/review/test") => {
            let settings = store.review_settings();
            match decision::test(&settings, store.review_key(), gateway).await {
                Ok(review) => Ok(json!({"ok": true, "review": review})),
                Err(e) => Ok(json!({"ok": false, "error": operations::redact(&format!("{e:#}"))})),
            }
        }
        ("POST", "/access/grants") => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Grant {
                node: String,
                capabilities: Vec<Capability>,
                scope: DurationScope,
            }
            let grant: Grant = serde_json::from_value(body).map_err(invalid)?;
            let approvals = store
                .grant(&grant.node, &grant.capabilities, grant.scope)
                .map_err(invalid)?;
            Ok(json!({"approvals": approvals}))
        }
        ("POST", "/access/revoke") => {
            #[derive(Default, Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Revoke {
                id: Option<String>,
            }
            let r: Revoke = if body.is_null() {
                Revoke::default()
            } else {
                serde_json::from_value(body).map_err(invalid)?
            };
            store.revoke(r.id.as_deref(), false).map_err(invalid)?;
            Ok(json!({}))
        }
        ("POST", path) if path.starts_with("/access/requests/") => {
            let id = path.trim_start_matches("/access/requests/");
            ensure_id(id)?;
            let input = serde_json::from_value(body).map_err(invalid)?;
            serde_json::to_value(
                store
                    .decide(id, input)
                    .map_err(|e| error(409, "request_unavailable", e.to_string()))?,
            )
            .map_err(invalid)
        }
        ("PUT", path) if path.starts_with("/access/") && path != "/access/review" => {
            let node = path.trim_start_matches("/access/");
            let changes = serde_json::from_value(body).map_err(invalid)?;
            store.change(node, changes).map_err(invalid)?;
            Ok(json!({}))
        }
        _ => Err(error(404, "not_found", "Unknown access operation")),
    }
}
fn ensure_id(id: &str) -> Result<(), AppError> {
    uuid::Uuid::parse_str(id).map(|_| ()).map_err(invalid)
}
