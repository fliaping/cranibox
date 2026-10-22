//! "Add a device by server address": a device with a public relay address accepts
//! pairing requests; its owner approves them in the inbox, then ordinary iroh pairing
//! completes with the server side confirming automatically for the approved device.
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Mutex};

/// How long a request waits for the owner, and how long an approval stays usable.
pub const REQUEST_TTL_SECS: u64 = 600;
const MAX_PENDING: usize = 16;

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn valid_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A device asking to join, as it described itself.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub node_id: String,
    pub name: String,
    pub fingerprint: String,
    pub endpoint_id: String,
    /// SHA-256 (hex) of the secret the requester polls with.
    pub secret_sha256: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Pending,
    Approved { link: String },
    Rejected,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: String,
    pub request: Request,
    pub created_at: u64,
    pub state: State,
}

/// Server side: requests received on the public listener.
#[derive(Default)]
pub struct Requests {
    entries: Mutex<BTreeMap<String, Entry>>,
}
impl Requests {
    fn prune(entries: &mut BTreeMap<String, Entry>) {
        let now = now();
        entries.retain(|_, e| e.created_at + REQUEST_TTL_SECS > now);
    }
    pub fn create(&self, request: Request) -> Result<String> {
        ensure!(
            agent_workspace_runtime::protocol::valid_node_id(&request.node_id),
            "invalid node"
        );
        ensure!(valid_hex(&request.endpoint_id, 64), "invalid endpoint");
        ensure!(valid_hex(&request.secret_sha256, 64), "invalid secret");
        ensure!(
            !request.name.trim().is_empty() && request.name.chars().count() <= 64,
            "invalid name"
        );
        ensure!(request.fingerprint.len() <= 64, "invalid fingerprint");
        let mut entries = self.entries.lock().unwrap();
        Self::prune(&mut entries);
        // One open request per device; a new one replaces it.
        entries.retain(|_, e| e.request.node_id != request.node_id || e.state != State::Pending);
        ensure!(
            entries
                .values()
                .filter(|e| e.state == State::Pending)
                .count()
                < MAX_PENDING,
            "too many pairing requests; try again later"
        );
        let id = agent_workspace_runtime::config::token()[..32].to_owned();
        entries.insert(
            id.clone(),
            Entry {
                id: id.clone(),
                request,
                created_at: now(),
                state: State::Pending,
            },
        );
        Ok(id)
    }
    /// What the requester may see, proven by the secret behind `secret_sha256`.
    pub fn status(&self, id: &str, secret: &str) -> Result<Value> {
        let mut entries = self.entries.lock().unwrap();
        Self::prune(&mut entries);
        let entry = entries.get(id).context("request expired")?;
        let digest = hex(&Sha256::digest(secret.as_bytes()));
        ensure!(
            bool::from(subtle::ConstantTimeEq::ct_eq(
                digest.as_bytes(),
                entry.request.secret_sha256.as_bytes()
            )),
            "request expired"
        );
        Ok(match &entry.state {
            State::Pending => json!({"state": "pending"}),
            State::Rejected => json!({"state": "rejected"}),
            State::Approved { link } => json!({"state": "approved", "link": link}),
        })
    }
    pub fn pending(&self) -> Vec<Entry> {
        let mut entries = self.entries.lock().unwrap();
        Self::prune(&mut entries);
        entries
            .values()
            .filter(|e| e.state == State::Pending)
            .cloned()
            .collect()
    }
    /// Record the owner's decision; returns the request if it was still pending.
    pub fn decide(&self, id: &str, state: State) -> Option<Request> {
        let mut entries = self.entries.lock().unwrap();
        Self::prune(&mut entries);
        let entry = entries.get_mut(id).filter(|e| e.state == State::Pending)?;
        entry.state = state;
        Some(entry.request.clone())
    }
}

/// Client side: normalise what the user pasted. `Ok(Some(link))` is a pairing link.
pub fn classify(input: &str) -> Result<(url::Url, Option<String>)> {
    let input = input.trim();
    let url = url::Url::parse(input)
        .context("Paste a server address such as https://cranibox.example.com")?;
    let loopback = url.host_str().is_some_and(|h| {
        h == "localhost"
            || h.parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && loopback),
        "Use an https:// address"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none() && url.query().is_none(),
        "The address must not contain credentials or a query"
    );
    if url.path() == "/pair" && url.fragment().is_some() {
        return Ok((url.clone(), Some(input.to_owned())));
    }
    ensure!(
        url.path() == "/" && url.fragment().is_none(),
        "Paste the server's address without a path"
    );
    Ok((url, None))
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(8))
        .build()?)
}
async fn json_response(response: reqwest::Response) -> Result<Value> {
    let status = response.status();
    let bytes = response.bytes().await?;
    ensure!(bytes.len() <= 16 * 1024, "server response too large");
    let value: Value = serde_json::from_slice(&bytes).context("not a Cranibox server")?;
    if !status.is_success() {
        bail!(
            "{}",
            value["error"]
                .as_str()
                .unwrap_or("the server refused the request")
        );
    }
    Ok(value)
}

/// The server's public identity (`/.well-known/cranibox`).
pub async fn server_identity(base: &url::Url) -> Result<Value> {
    let value = json_response(
        client()?
            .get(base.join(".well-known/cranibox")?)
            .send()
            .await?,
    )
    .await?;
    ensure!(
        value["protocol"] == 1
            && value["node_id"]
                .as_str()
                .is_some_and(agent_workspace_runtime::protocol::valid_node_id),
        "not a Cranibox server"
    );
    Ok(value)
}

/// Ask the server to pair; returns the request ID and the secret to poll with.
pub async fn request(base: &url::Url, me: Value) -> Result<(String, String)> {
    let secret = agent_workspace_runtime::config::token();
    let mut body = me;
    body["secret_sha256"] = json!(hex(&Sha256::digest(secret.as_bytes())));
    let value = json_response(
        client()?
            .post(base.join("pair/request")?)
            .json(&body)
            .send()
            .await?,
    )
    .await?;
    let id = value["request_id"]
        .as_str()
        .filter(|id| valid_hex(id, 32))
        .context("not a Cranibox server")?
        .to_owned();
    Ok((id, secret))
}

pub async fn poll(base: &url::Url, id: &str, secret: &str) -> Result<Value> {
    json_response(
        client()?
            .post(base.join(&format!("pair/request/{id}"))?)
            .json(&json!({"secret": secret}))
            .send()
            .await?,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(node: &str, secret: &str) -> Request {
        Request {
            node_id: node.into(),
            name: "Laptop".into(),
            fingerprint: "abcd-efgh".into(),
            endpoint_id: "a".repeat(64),
            secret_sha256: hex(&Sha256::digest(secret.as_bytes())),
        }
    }

    #[test]
    fn requests_need_the_secret_and_the_owner() {
        let store = Requests::default();
        let node = format!("node-{}", "1".repeat(32));
        let id = store.create(request(&node, "s3cret")).unwrap();
        assert_eq!(store.status(&id, "s3cret").unwrap()["state"], "pending");
        assert!(
            store.status(&id, "wrong").is_err(),
            "the secret proves the requester"
        );
        assert_eq!(store.pending().len(), 1);
        assert!(
            store
                .decide(
                    &id,
                    State::Approved {
                        link: "cranibox://pair/x".into()
                    }
                )
                .is_some()
        );
        assert!(store.decide(&id, State::Rejected).is_none(), "decided once");
        assert_eq!(
            store.status(&id, "s3cret").unwrap()["link"],
            "cranibox://pair/x"
        );
        assert!(store.pending().is_empty());
    }

    #[test]
    fn pending_requests_are_bounded_and_one_per_device() {
        let store = Requests::default();
        let node = format!("node-{}", "2".repeat(32));
        let first = store.create(request(&node, "a")).unwrap();
        store.create(request(&node, "b")).unwrap();
        assert!(
            store.status(&first, "a").is_err(),
            "a new request replaces the old one"
        );
        for i in 0..15 {
            store
                .create(request(&format!("node-{:032x}", i + 100), "x"))
                .unwrap();
        }
        assert!(
            store
                .create(request(&format!("node-{:032x}", 999), "x"))
                .is_err()
        );
    }

    #[test]
    fn classify_accepts_https_servers_and_links_only() {
        assert!(classify("https://c.example.com").unwrap().1.is_none());
        assert!(
            classify("https://c.example.com/pair#ABC")
                .unwrap()
                .1
                .is_some()
        );
        assert!(classify("http://c.example.com").is_err());
        assert!(classify("http://127.0.0.1:7378").is_ok());
        assert!(classify("https://c.example.com/other").is_err());
        assert!(classify("https://user:pw@c.example.com").is_err());
    }
}
