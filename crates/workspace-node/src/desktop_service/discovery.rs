//! Bounded, owner-requested discovery. Reading protocol greetings never enables
//! sharing, authenticates a viewer, or sends Selkies SETTINGS.
use super::{Desktop, DesktopProtocol};
use anyhow::Result;
use serde_json::{Value, json};
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    time::Duration,
};

pub(super) async fn discover(current: Option<Desktop>, managed: Option<Desktop>) -> Result<Value> {
    let mut candidates = Vec::with_capacity(6);
    for desktop in current.iter().chain(managed.iter()) {
        desktop.validate()?;
        if !candidates.contains(desktop) {
            candidates.push(desktop.clone());
        }
    }
    for (protocol, port) in [
        (DesktopProtocol::Vnc, 5900),
        (DesktopProtocol::Selkies, 8082),
    ] {
        for ip in [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
        ] {
            let desktop = Desktop {
                address: (ip, port).into(),
                protocol,
            };
            if !candidates.contains(&desktop) {
                candidates.push(desktop);
            }
        }
    }
    let checked = candidates.len();
    // At most six loopback probes run together. A silent or unrelated service
    // cannot stall local service management for a complete network timeout.
    let services = futures_util::future::join_all(candidates.into_iter().map(|desktop| {
        let configured = current.as_ref() == Some(&desktop);
        async move {
            let ready = tokio::time::timeout(Duration::from_secs(3), async {
                match desktop.protocol {
                    DesktopProtocol::Vnc => super::rfb(desktop.address).await,
                    DesktopProtocol::Selkies => {
                        agent_workspace_runtime::peers::desktop::selkies::check(desktop.address).await
                    }
                }
            }).await.is_ok_and(|result| result.is_ok());
            ready.then(|| json!({"address":desktop.address,"protocol":desktop.protocol,"configured":configured}))
        }
    })).await.into_iter().flatten().collect::<Vec<_>>();
    Ok(json!({"services":services,"checked":checked}))
}
