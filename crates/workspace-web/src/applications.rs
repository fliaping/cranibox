//! Applications run on a dedicated origin, never on the privileged console origin.
use crate::server::State;
use anyhow::{Context, Result, ensure};
use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
};
use cranibox_node::spaces::{
    Store,
    applications::{self, Application},
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    id: String,
    application: Application,
}
pub(crate) async fn issue(state: &State, owner: String, body: Value) -> Result<Value> {
    let options: Options = serde_json::from_value(body)?;
    let origin = url::Url::parse(&state.config.applications_url)?;
    state.config.validate()?;
    ensure!(
        !state.console_origin(&origin.origin().ascii_serialization()),
        "Configure a separate applicationsUrl origin"
    );
    let store = Store::new(state.root.join(".config/cranibox-node/peers.json"));
    let invitation = state
        .applications
        .issue(&store, owner, options.id.parse()?, options.application)
        .await?;
    let mut url = origin.join(&invitation.prefix)?;
    url.query_pairs_mut()
        .append_pair("open", &invitation.ticket);
    Ok(json!({"url":url.as_str()}))
}
/// A shared desktop's own web page (Selkies, or noVNC for VNC) on the application origin.
pub(crate) async fn issue_desktop(state: &State, owner: String, body: Value) -> Result<Value> {
    let origin = url::Url::parse(&state.config.applications_url)?;
    state.config.validate()?;
    ensure!(
        !state.console_origin(&origin.origin().ascii_serialization()),
        "Configure a separate applicationsUrl origin"
    );
    let page = state
        .services
        .desktop_page(body, state.console_origins())
        .await?;
    let protocol = page.protocol();
    let invitation = state.applications.issue_desktop(owner, page).await?;
    let mut url = origin.join(&invitation.prefix)?;
    url.query_pairs_mut()
        .append_pair("open", &invitation.ticket);
    Ok(json!({"url": url.as_str(), "protocol": protocol}))
}
pub(crate) async fn handle(state: State, request: Request<Body>) -> Response {
    let run = async {
        let origin = url::Url::parse(&state.config.applications_url)?;
        let origin_text = origin.origin().ascii_serialization();
        ensure!(
            !state.console_origin(&origin_text),
            "Application origin is not isolated"
        );
        let host = &origin[url::Position::BeforeHost..url::Position::AfterPort];
        ensure!(
            request.headers().get("host").and_then(|v| v.to_str().ok()) == Some(host),
            "Open applications through the configured application gateway"
        );
        let prefix = applications::prefix(request.uri().path())
            .context("Invalid application path")?
            .to_owned();
        if let Some(query) = request.uri().query() {
            let parameters: Vec<_> = url::form_urlencoded::parse(query.as_bytes()).collect();
            if parameters.iter().any(|(key, _)| key == "open") {
                ensure!(
                    request.method() == "GET" && parameters.len() == 1,
                    "Invalid application link"
                );
                let (cookie, access) = state.applications.claim(&parameters[0].1, &prefix).await?;
                ensure!(
                    state.has_session(&access.owner).await,
                    "Workspace session expired"
                );
                return Ok(applications::claimed_response(
                    &prefix,
                    &cookie,
                    origin.scheme() == "https",
                ));
            }
        }
        let request_origin = request
            .headers()
            .get("origin")
            .and_then(|v| v.to_str().ok());
        ensure!(
            request_origin.is_none_or(|v| v == origin_text),
            "Application origin rejected"
        );
        ensure!(
            request
                .headers()
                .get("sec-fetch-site")
                .is_none_or(|v| v != "cross-site"),
            "Cross-site application request rejected"
        );
        ensure!(
            request.method() == "GET"
                || request.method() == "HEAD"
                || request_origin == Some(origin_text.as_str()),
            "Application origin required"
        );
        let access = state
            .applications
            .access(
                applications::cookie(request.headers()).context("Reopen this application")?,
                &prefix,
            )
            .await?;
        ensure!(
            state.has_session(&access.owner).await && !*state.closing.borrow(),
            "Workspace session expired"
        );
        access.proxy.forward(request, &prefix, access.cancel).await
    }
    .await;
    let mut response = match run {
        Ok(response) => response,
        Err(error) => (StatusCode::FORBIDDEN, error.to_string()).into_response(),
    };
    response.headers_mut().insert(
        "referrer-policy",
        axum::http::HeaderValue::from_static("no-referrer"),
    );
    response.headers_mut().insert(
        "x-content-type-options",
        axum::http::HeaderValue::from_static("nosniff"),
    );
    response
}
