//! Multiple explicitly authorized paths share one target grant. No operation replay.
use super::{
    Store,
    config::{Config, Endpoint, MAX_PATHS, Route},
};
use crate::{config, protocol::valid_node_id};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{path::PathBuf, time::Duration};

fn endpoint(
    url: &str,
    allow_insecure: bool,
    local_forward_lock: Option<PathBuf>,
) -> Result<Endpoint> {
    let origin = config::check_url(url.trim(), false, allow_insecure)?;
    ensure!(origin.path() == "/", "use an HTTP(S) origin without a path");
    let value = Endpoint {
        url: origin.origin().ascii_serialization(),
        allow_insecure,
        local_forward_lock,
    };
    value.validate()?;
    Ok(value)
}

fn check_token(route: &Route, token: &str) -> Result<()> {
    ensure!(
        token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()),
        "token must contain 64 hexadecimal characters"
    );
    ensure!(
        config::read_token(&route.token_file)? == token,
        "An additional path must use the same target access token. Edit the device connection to rotate its grant."
    );
    Ok(())
}
impl Store {
    pub(super) async fn add_path(
        &self,
        node: String,
        url: String,
        token: String,
        allow_private_http: bool,
    ) -> Result<Value> {
        let path = endpoint(&url, allow_private_http, None)?;
        let route = self.route(&node)?;
        check_token(&route, token.trim())?;
        ensure!(
            route
                .endpoints()
                .iter()
                .all(|p| p.url.trim_end_matches('/') != path.url),
            "this node path already exists"
        );
        ensure!(
            route.endpoints().len() < MAX_PATHS,
            "at most four paths per node"
        );
        // Validate the new address without running any target capability.
        let candidate = Route {
            url: path.url.clone(),
            token_file: route.token_file,
            allow_insecure: path.allow_insecure,
            local_forward_lock: None,
            alternatives: vec![],
            target_identity: None,
            config_path: None,
        };
        tokio::time::timeout(Duration::from_secs(3), candidate.api(&node)?.probe())
            .await
            .context(
                "The new path did not respond. Check the address and network before retrying.",
            )??;
        self.append_path(node, path.url, token, allow_private_http, None)
    }
    pub(super) fn append_path(
        &self,
        node: String,
        url: String,
        token: String,
        allow_private_http: bool,
        lease: Option<PathBuf>,
    ) -> Result<Value> {
        ensure!(valid_node_id(&node), "invalid node ID");
        let endpoint = endpoint(&url, allow_private_http, lease)?;
        Config::update(&self.path, |configuration| {
            let route = configuration
                .routes
                .get_mut(&node)
                .context("direct route is not configured")?;
            check_token(route, token.trim())?;
            if let Some(existing) = route
                .endpoints()
                .iter()
                .find(|p| p.url.trim_end_matches('/') == endpoint.url)
            {
                ensure!(
                    existing.local_forward_lock == endpoint.local_forward_lock
                        && existing.allow_insecure == endpoint.allow_insecure,
                    "this address already exists with different path settings"
                );
                return Ok(());
            }
            ensure!(
                route.endpoints().len() < MAX_PATHS,
                "at most four paths per node"
            );
            route.alternatives.push(endpoint);
            Ok(())
        })
        .map(|c| c.public_view())
    }
    pub(super) fn change_path(&self, node: String, url: String, prefer: bool) -> Result<Value> {
        ensure!(valid_node_id(&node), "invalid node ID");
        Config::update(&self.path, |configuration| {
            let route = configuration
                .routes
                .get_mut(&node)
                .context("direct route is not configured")?;
            let mut paths = route.endpoints();
            let index = paths
                .iter()
                .position(|p| p.url.trim_end_matches('/') == url.trim_end_matches('/'))
                .context("node path is not configured")?;
            if prefer {
                let selected = paths.remove(index);
                paths.insert(0, selected);
            } else {
                ensure!(
                    paths.len() > 1,
                    "Keep at least one path, or remove the device connection."
                );
                paths.remove(index);
            }
            if route.url != paths[0].url || route.allow_insecure != paths[0].allow_insecure {
                self.forget_activity_snapshot(&node)?;
            }
            route.set_endpoints(paths);
            Ok(())
        })
        .map(|c| c.public_view())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_share_a_grant_and_remain_bounded_and_private() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("peers.json"));
        let token = config::token();
        store
            .connect(
                "target".into(),
                "http://127.0.0.1:9200".into(),
                token.clone(),
                false,
            )
            .unwrap();
        let credential = store.route("target").unwrap().token_file;
        let before = std::fs::read(&store.path).unwrap();
        assert!(
            store
                .append_path(
                    "target".into(),
                    "http://127.0.0.1:9201".into(),
                    config::token(),
                    false,
                    None
                )
                .is_err()
        );
        assert_eq!(std::fs::read(&store.path).unwrap(), before);
        for port in 9201..=9203 {
            store
                .append_path(
                    "target".into(),
                    format!("http://127.0.0.1:{port}"),
                    token.clone(),
                    false,
                    None,
                )
                .unwrap();
        }
        let view = store.list().unwrap();
        assert_eq!(view["routes"][0]["paths"].as_array().unwrap().len(), 4);
        assert!(!view.to_string().contains(&token));
        assert!(!view.to_string().contains("token_file"));
        assert!(!view.to_string().contains("local_forward_lock"));
        assert_eq!(store.route("target").unwrap().token_file, credential);
        assert!(
            store
                .append_path(
                    "target".into(),
                    "http://127.0.0.1:9204".into(),
                    token.clone(),
                    false,
                    None
                )
                .is_err()
        );
        // Repeated provider discovery is idempotent and never rotates a grant.
        store
            .append_path(
                "target".into(),
                "http://127.0.0.1:9203/".into(),
                token,
                false,
                None,
            )
            .unwrap();
        assert_eq!(store.route("target").unwrap().endpoints().len(), 4);
    }

    #[test]
    fn path_order_removal_and_credential_rotation_are_explicit() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("peers.json"));
        let token = config::token();
        store
            .connect(
                "target".into(),
                "http://127.0.0.1:9200".into(),
                token.clone(),
                false,
            )
            .unwrap();
        store
            .append_path(
                "target".into(),
                "http://127.0.0.1:9201".into(),
                token,
                false,
                None,
            )
            .unwrap();
        store
            .change_path("target".into(), "http://127.0.0.1:9201/".into(), true)
            .unwrap();
        let route = store.route("target").unwrap();
        assert_eq!(route.url, "http://127.0.0.1:9201");
        assert_eq!(route.alternatives[0].url, "http://127.0.0.1:9200");
        store
            .connect("target".into(), route.url.clone(), String::new(), false)
            .unwrap();
        assert_eq!(store.route("target").unwrap().endpoints().len(), 2);
        store
            .change_path("target".into(), route.url, false)
            .unwrap();
        assert!(
            store
                .change_path("target".into(), "http://127.0.0.1:9200".into(), false)
                .is_err()
        );
        store
            .connect(
                "target".into(),
                "http://127.0.0.1:9202".into(),
                config::token(),
                false,
            )
            .unwrap();
        assert_eq!(store.route("target").unwrap().endpoints().len(), 1);
    }

    #[test]
    fn legacy_routes_load_and_duplicate_origins_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("peers.json"));
        store
            .connect(
                "target".into(),
                "http://127.0.0.1:9200".into(),
                config::token(),
                false,
            )
            .unwrap();
        let saved = std::fs::read(&store.path).unwrap();
        assert!(!String::from_utf8_lossy(&saved).contains("alternatives"));
        let mut configuration = Config::read(&store.path).unwrap();
        configuration
            .routes
            .get_mut("target")
            .unwrap()
            .alternatives
            .push(Endpoint {
                url: "http://127.0.0.1:9200/".into(),
                allow_insecure: false,
                local_forward_lock: None,
            });
        assert!(configuration.validate().is_err());
        assert_eq!(std::fs::read(&store.path).unwrap(), saved);
    }
}
