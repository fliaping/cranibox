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
    if let Some(value) = remote_route(peers, &store, method, path, &body).await {
        return value;
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
        ("PUT", path)
            if path.starts_with("/access/")
                && path != "/access/review"
                && path != "/access/approver" =>
        {
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

/// The target's refusal body, if it sent one.
fn refusal_body(e: &anyhow::Error) -> Value {
    agent_workspace_runtime::client::http_body(e)
        .map(|(_, body)| body)
        .unwrap_or_default()
}
fn refusal_reason(e: &anyhow::Error) -> String {
    agent_workspace_runtime::client::http_body(e)
        .and_then(|(_, body)| {
            body["reason"]
                .as_str()
                .or(body["error"].as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "unreachable".into())
}
/// The owner's other devices: their waiting requests, and deciding one from here after
/// the person proves it is them.
async fn remote_route(
    peers: &Path,
    store: &Store,
    method: &str,
    path: &str,
    body: &Value,
) -> Option<Result<Value, AppError>> {
    Some(match (method, path) {
        ("GET", "/access/approver") => Ok(approver_view(store)),
        ("PUT", "/access/approver") => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Change {
                remote_enabled: bool,
            }
            let change: Change = match serde_json::from_value(body.clone()) {
                Ok(change) => change,
                Err(e) => return Some(Err(invalid(e))),
            };
            store
                .set_remote_enabled(change.remote_enabled)
                .map(|_| approver_view(store))
                .map_err(invalid)
        }
        ("GET", "/access/remote") => Ok(remote_list(peers, store).await),
        ("POST", "/access/remote/decide") => remote_decide(peers, store, body.clone()).await,
        _ => return None,
    })
}
fn approver_view(store: &Store) -> Value {
    let cert = store
        .roster()
        .and_then(|r| r.current())
        .and_then(|r| r.owner_cert);
    json!({
        // Remote approval needs the identity's recovery code or passphrase.
        "recovery": store.recovery_verifier().is_some(),
        "recovery_kind": cert.map(|c| if c.recovery_salt.is_some() { "passphrase" } else { "code" }),
        "remote_enabled": store.remote_enabled(),
        "owner_device": !store.owner_devices().is_empty(),
        "attempts": remote::ATTEMPTS,
    })
}
/// The error the approving person sees for the target's refusal `body`.
fn refused(e: &anyhow::Error) -> AppError {
    let body = refusal_body(e);
    let reason = refusal_reason(e);
    let (status, message) = match reason.as_str() {
        "wrong_code" => (401, "the recovery code or passphrase is not right"),
        "locked" => (
            423,
            "too many wrong recovery codes; remote approval is locked for now",
        ),
        "no_recovery" => (409, "that device has no recovery code for this identity"),
        _ => (409, "the other device refused the decision"),
    };
    let mut failure = error(status, &reason, message);
    let mut out = json!({"error": reason});
    for key in ["left", "until"] {
        if let Some(value) = body.get(key) {
            out[key] = value.clone();
        }
    }
    failure.body = Some(out);
    failure
}
/// Asks each of the owner's other devices what is waiting there, in parallel.
pub(super) async fn remote_list(peers: &Path, store: &Store) -> Value {
    let fleet = agent_workspace_runtime::client::Fleet::new(peers.to_owned());
    let devices = store.owner_devices();
    let answers = futures_util::future::join_all(devices.iter().map(|(node, _)| {
        let fleet = &fleet;
        async move {
            tokio::time::timeout(
                std::time::Duration::from_secs(4),
                fleet.get(node, "/api/v1/access/remote"),
            )
            .await
            .map_err(|_| anyhow::anyhow!("timed out"))
            .and_then(|r| r)
        }
    }))
    .await;
    let devices: Vec<Value> = devices
        .into_iter()
        .zip(answers)
        .map(|((node, name), answer)| {
            let list = answer.and_then(|value| {
                serde_json::from_value::<RemotePendingList>(value).map_err(anyhow::Error::from)
            });
            match list {
                Ok(list) if list.node == node => {
                    let requests: Vec<Value> = list
                        .requests
                        .into_iter()
                        .filter(|p| remote::op_hash(&p.request) == p.op_hash)
                        .map(|p| {
                            let once = p.once_only || remote::once_only(&p.request);
                            json!({
                                "request": p.request,
                                "op_hash": p.op_hash,
                                "once_only": once,
                                "challenge_expires_at": p.challenge_expires_at,
                            })
                        })
                        .collect();
                    json!({"node": node, "name": name, "online": true, "requests": requests})
                }
                Ok(_) => json!({"node": node, "name": name, "online": true, "error": "wrong_node", "requests": []}),
                Err(e) => json!({"node": node, "name": name, "online": false, "error": refusal_reason(&e), "requests": []}),
            }
        })
        .collect();
    json!({"devices": devices, "approver": approver_view(store)})
}
pub(super) async fn remote_decide(
    peers: &Path,
    store: &Store,
    body: Value,
) -> Result<Value, AppError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Auth {
        /// The recovery code or passphrase, typed just now. Used here only, never sent.
        code: String,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Decide {
        node: String,
        request: String,
        op_hash: String,
        decision: ApprovalDecision,
        #[serde(default)]
        scope: Option<DurationScope>,
        auth: Auth,
    }
    let input: Decide = serde_json::from_value(body).map_err(invalid)?;
    ensure_id(&input.request)?;
    if !store.owner_peer(&input.node) {
        return Err(error(
            403,
            "not_owner",
            "only your own devices can be approved from here",
        ));
    }
    if input.decision == ApprovalDecision::Approve && input.scope.is_none() {
        return Err(invalid("approval scope required"));
    }
    // 1. Only an identity with a recovery key can approve remotely.
    let Some(cert) = store.recovery_verifier().and(
        store
            .roster()
            .and_then(|r| r.current())
            .and_then(|r| r.owner_cert),
    ) else {
        return Err(error(
            409,
            "no_recovery",
            "create a recovery code first, under Identity",
        ));
    };
    let auth = if cert.recovery_salt.is_some() {
        "recovery_passphrase"
    } else {
        "recovery_code"
    };
    // 2. Fetch the request again and check it is exactly what the person saw.
    let fleet = agent_workspace_runtime::client::Fleet::new(peers.to_owned());
    let list: RemotePendingList = fleet
        .get(&input.node, "/api/v1/access/remote")
        .await
        .and_then(|v| serde_json::from_value(v).map_err(anyhow::Error::from))
        .map_err(|e| {
            let _ = store.record_remote(
                &input.node,
                None,
                &format!("rejected:{}", refusal_reason(&e)),
                auth,
                None,
            );
            error(502, "remote", format!("{e:#}"))
        })?;
    let pending = list
        .requests
        .into_iter()
        .find(|p| p.request.id == input.request)
        .filter(|_| list.node == input.node);
    let Some(pending) = pending else {
        let _ = store.record_remote(
            &input.node,
            None,
            "rejected:request_unavailable",
            auth,
            None,
        );
        return Err(error(
            409,
            "request_unavailable",
            "the request was already decided or has expired",
        ));
    };
    let hash = remote::op_hash(&pending.request);
    if hash != input.op_hash || hash != pending.op_hash {
        let _ = store.record_remote(
            &input.node,
            Some(&pending.request),
            "rejected:operation_mismatch",
            auth,
            None,
        );
        return Err(error(
            409,
            "operation_mismatch",
            "the request changed after it was shown",
        ));
    }
    let once = pending.once_only || remote::once_only(&pending.request);
    if input.decision == ApprovalDecision::Approve
        && once
        && input.scope != Some(DurationScope::Session)
    {
        return Err(error(
            400,
            "once_only",
            "this operation can only be allowed once",
        ));
    }
    // 3. Turn the code into the recovery key in memory, sign, and drop the key. The
    //    target checks the signature against the recovery public key it holds.
    let approver = Config::read(peers).map_err(invalid)?.node_id;
    let target = input.node.clone();
    let decision = (input.decision, input.scope);
    let code = zeroize::Zeroizing::new(input.auth.code);
    let assertion = tokio::task::spawn_blocking(move || {
        let recovery = crate::models::recovery_signing_key(code.trim(), Some(&cert))?;
        remote::sign(
            &recovery,
            &approver,
            &target,
            &pending,
            decision.0,
            decision.1,
            auth,
            now(),
        )
        .map(|a| (a, pending))
    })
    .await
    .map_err(invalid)?;
    let (assertion, pending) = match assertion {
        Ok(signed) => signed,
        Err(e) => {
            let _ = store.record_remote(&input.node, None, "rejected:not_a_code", auth, None);
            return Err(error(400, "not_a_code", format!("{e:#}")));
        }
    };
    match fleet
        .post(
            &input.node,
            "/api/v1/access/remote/decide",
            serde_json::to_value(&assertion).map_err(invalid)?,
        )
        .await
    {
        Ok(request) => {
            let result = if input.decision == ApprovalDecision::Approve {
                format!(
                    "approved:{}",
                    serde_json::to_value(input.scope)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_owned))
                        .unwrap_or_default()
                )
            } else {
                "denied".into()
            };
            let _ = store.record_remote(
                &input.node,
                Some(&pending.request),
                &result,
                auth,
                Some(&assertion.id),
            );
            Ok(json!({"request": request, "assertion": assertion}))
        }
        Err(e) => {
            let reason = refusal_reason(&e);
            let _ = store.record_remote(
                &input.node,
                Some(&pending.request),
                &format!("rejected:{reason}"),
                auth,
                Some(&assertion.id),
            );
            Err(refused(&e))
        }
    }
}
