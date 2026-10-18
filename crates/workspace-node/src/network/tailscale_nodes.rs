//! Explicit, credential-free discovery of one online device in the existing tailnet.
use super::tailscale;
use agent_workspace_runtime::protocol::valid_node_id;
use anyhow::{Context, Result, ensure};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

pub(super) fn origin(address: IpAddr, port: u16) -> String {
    format!("http://{}", SocketAddr::new(address, port))
}
pub(super) fn tailnet_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => u32::from(ip) & 0xffc00000 == 0x64400000,
        IpAddr::V6(ip) => ip.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0],
    }
}
fn check_member(state: &Value, address: IpAddr, port: u16) -> Result<()> {
    ensure!(port != 0, "Enter the target's node listener port.");
    ensure!(
        tailnet_address(address),
        "Choose a Tailscale IP address from the device list."
    );
    ensure!(
        state["state"] == "Running",
        "Connect the system Tailscale service first."
    );
    ensure!(
        state["devices"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|device| {
                device["online"] == true
                    && device["addresses"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|value| {
                            value.as_str().and_then(|ip| ip.parse::<IpAddr>().ok()) == Some(address)
                        })
            }),
        "This address is no longer an online Tailscale peer. Refresh the device list."
    );
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    version: u8,
    node_id: String,
}
pub(super) async fn resolve(address: IpAddr, port: u16, own: &str) -> Result<Value> {
    let url = origin(address, port);
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(2))
        .build()?;
    let response = client
        .get(format!("{url}/api/v1/mesh/identity"))
        .send()
        .await
        .context("Node not reachable. Check its listener port and tailnet access policy")?;
    ensure!(
        response.status() == reqwest::StatusCode::OK,
        "No compatible Cranibox identity at this port. Check the target node listener."
    );
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("Read node identity")?;
        ensure!(
            bytes.len() + chunk.len() <= 4096,
            "Node identity response is too large."
        );
        bytes.extend_from_slice(&chunk);
    }
    let identity: Identity =
        serde_json::from_slice(&bytes).context("Invalid Cranibox node identity")?;
    ensure!(
        identity.version == 1 && valid_node_id(&identity.node_id) && identity.node_id != own,
        "The target must identify a compatible remote Cranibox node."
    );
    Ok(json!({"node_id":identity.node_id,"address":address,"port":port,"url":url}))
}
pub(super) async fn discover(address: IpAddr, port: u16, own: &str) -> Result<Value> {
    let state = tailscale::status().await?;
    check_member(&state, address, port)?;
    // This explicit lookup sends no token and checks fresh system membership.
    resolve(address, port, own).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[test]
    fn membership_requires_online_tailnet_addresses_and_a_port() {
        let state = json!({"state":"Running","devices":[
            {"online":true,"addresses":["100.64.0.2","fd7a:115c:a1e0::2","127.0.0.1"]},
            {"online":false,"addresses":["100.64.0.3"]}]});
        for address in ["100.64.0.2", "fd7a:115c:a1e0::2"] {
            assert!(check_member(&state, address.parse().unwrap(), 1234).is_ok());
        }
        for address in [
            "100.64.0.3",
            "100.64.0.4",
            "127.0.0.1",
            "100.63.255.255",
            "100.128.0.0",
            "::ffff:100.64.0.2",
            "fd7a:115c:a1e1::2",
        ] {
            assert!(check_member(&state, address.parse().unwrap(), 1234).is_err());
        }
        assert!(check_member(&state, "100.64.0.2".parse().unwrap(), 0).is_err());
        assert!(
            check_member(
                &json!({"state":"Stopped"}),
                "100.64.0.2".parse().unwrap(),
                1234
            )
            .is_err()
        );
        assert_eq!(
            origin("fd7a:115c:a1e0::2".parse().unwrap(), 1234),
            "http://[fd7a:115c:a1e0::2]:1234"
        );
    }

    #[tokio::test]
    async fn identity_discovery_is_bounded_and_sends_no_credentials() {
        for (status, body, allowed) in [
            (
                "200 OK",
                json!({"version":1,"node_id":"remote-node"}).to_string(),
                true,
            ),
            (
                "200 OK",
                json!({"version":1,"node_id":"own-node"}).to_string(),
                false,
            ),
            (
                "200 OK",
                json!({"version":2,"node_id":"remote-node"}).to_string(),
                false,
            ),
            (
                "200 OK",
                json!({"version":1,"node_id":"../wrong"}).to_string(),
                false,
            ),
            ("200 OK", "x".repeat(4097), false),
            ("302 Found", String::new(), false),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![0; 4096];
                let len = socket.read(&mut request).await.unwrap();
                let request = String::from_utf8(request[..len].to_vec())
                    .unwrap()
                    .to_ascii_lowercase();
                assert!(request.starts_with("get /api/v1/mesh/identity http/1.1\r\n"));
                assert!(!request.contains("authorization") && !request.contains("cookie"));
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nLocation: http://127.0.0.1:1/never\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(header.as_bytes()).await.unwrap();
                let _ = socket.write_all(body.as_bytes()).await;
            });
            assert_eq!(
                resolve(address.ip(), address.port(), "own-node")
                    .await
                    .is_ok(),
                allowed
            );
            server.await.unwrap();
        }
    }
}
