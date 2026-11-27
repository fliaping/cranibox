//! Network diagnostics for pairing: readable dial failures and `cranibox peer pair doctor`.
//!
//! Field reports came from restricted networks (no UDP, a transparent TCP proxy whose
//! DNS answers every name with one fake IP). There a joiner reaches its own home relay
//! but may not reach the inviter's relay, while the reverse direction works because the
//! inviter dials *our* relay. These helpers say which of the three it is: this device's
//! relay, the other device's relay, or the link itself.
use crate::{IrohNode, addr, ticket};
use cranibox_contracts::net::PairTicket;
use iroh::{RelayUrl, Watcher};
use serde::Serialize;
use std::{
    net::IpAddr,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// `{error:#}` without the repeats that transparent error wrappers produce
/// (iroh's connect errors print `timed out: timed out: timed out`).
pub fn describe(error: &anyhow::Error) -> String {
    let mut parts: Vec<String> = Vec::new();
    for cause in error.chain() {
        let text = cause.to_string();
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        // A wrapper that already prints its source (`a: b`) followed by that source.
        if parts
            .last()
            .is_some_and(|last| last == text || last.ends_with(&format!(": {text}")))
        {
            continue;
        }
        parts.push(text.to_owned());
    }
    parts.join(": ")
}

/// The public n0 relays. They admit every endpoint, so a pairing token is useless there.
pub fn is_public_relay(url: &RelayUrl) -> bool {
    iroh::defaults::prod::default_relay_map()
        .urls::<Vec<_>>()
        .contains(url)
}

/// What this endpoint knows about its own network right now.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct Network {
    /// Relays this endpoint keeps a home connection to, and whether it is up.
    pub home_relays: Vec<(String, bool)>,
    /// QUIC address discovery over UDP worked (None: no network report yet).
    pub udp: Option<bool>,
    /// The relay with the lowest latency in the last report.
    pub preferred_relay: Option<String>,
    /// Public IPv4 address seen by the relays over UDP.
    pub global_v4: Option<String>,
    pub captive_portal: Option<bool>,
}
impl Network {
    pub fn of(node: &IrohNode) -> Self {
        let report = node.endpoint().net_report().get();
        Self {
            home_relays: node.relays_connected(),
            udp: report.as_ref().map(|r| r.udp_v4 || r.udp_v6),
            preferred_relay: report
                .as_ref()
                .and_then(|r| r.preferred_relay.as_ref().map(ToString::to_string)),
            global_v4: report
                .as_ref()
                .and_then(|r| r.global_v4.map(|a| a.to_string())),
            captive_portal: report.as_ref().and_then(|r| r.captive_portal),
        }
    }
    /// The connected home relay, else the first configured one.
    pub fn home_relay(&self) -> Option<&str> {
        self.home_relays
            .iter()
            .find(|(_, up)| *up)
            .or_else(|| self.home_relays.first())
            .map(|(url, _)| url.as_str())
    }
    fn home_connected(&self) -> bool {
        self.home_relays.iter().any(|(_, up)| *up)
    }
}

fn same_relay(a: &str, b: &str) -> bool {
    match (addr::normalize_relay(a), addr::normalize_relay(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// The step of [`crate::pair::Pairing::accept`] that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// QUIC handshake with the inviter (over its relay or a direct address).
    Connect,
    /// Waiting for the inviter's answer to our hello.
    Answer,
}

/// Why the joiner could not reach the inviter, with the facts needed to tell a
/// local relay problem from a remote one, and what to try next.
pub fn dial_failure(
    stage: Stage,
    ticket: &PairTicket,
    network: &Network,
    waited: Duration,
    cause: Option<&anyhow::Error>,
) -> String {
    let target_relay = ticket.inviter.relay_url.as_deref();
    let what = match stage {
        Stage::Connect => "connecting to the inviting device",
        Stage::Answer => "waiting for the inviting device to answer",
    };
    let mut out = format!(
        "The other device did not answer (stalled after {}s while {what}).",
        waited.as_secs()
    );
    out.push_str(&format!("\n  other device: {}", ticket.inviter.endpoint_id));
    out.push_str(&format!(
        "\n  its relay: {}",
        target_relay.unwrap_or("(none in the link)")
    ));
    let home = match network.home_relay() {
        Some(url) if network.home_connected() => format!("{url} (connected)"),
        Some(url) => format!("{url} (NOT connected)"),
        None => "(none)".into(),
    };
    out.push_str(&format!("\n  this device's relay: {home}"));
    out.push_str(&format!(
        "\n  direct addresses in the link: {}; UDP here: {}",
        ticket.inviter.direct.len(),
        match network.udp {
            Some(true) => "works",
            Some(false) => "blocked",
            None => "unknown",
        }
    ));
    if let Some(cause) = cause {
        out.push_str(&format!("\n  cause: {}", describe(cause)));
    }
    let relays_differ = match (target_relay, network.home_relay()) {
        (Some(target), Some(home)) => !same_relay(target, home),
        _ => false,
    };
    out.push_str("\nMake sure the other device is online and Cranibox is running there.");
    if !network.home_connected() {
        out.push_str(
            "\nThis device is not connected to any relay: check that outbound HTTPS (443) works here.",
        );
    }
    if relays_differ || network.udp != Some(true) {
        out.push_str(
            "\nTry the other direction: run `cranibox peer pair invite` on this device and accept that link on the other device.",
        );
    }
    if stage == Stage::Answer {
        out.push_str("\nThe link may have been used or revoked: ask for a new one.");
    }
    out.push_str("\nRun `cranibox peer pair doctor <link>` to check both relays, DNS and UDP.");
    out
}

/// One line of the doctor report.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    /// `ok`, `warn`, `fail` or `unknown`.
    pub status: &'static str,
    pub detail: String,
}
fn check(name: impl Into<String>, status: &'static str, detail: impl Into<String>) -> Check {
    Check {
        name: name.into(),
        status,
        detail: detail.into(),
    }
}

/// Reachability of one relay from this device.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RelayProbe {
    pub url: String,
    pub host: String,
    /// What the system resolver answered (what a transparent proxy sees too).
    pub addresses: Vec<String>,
    pub dns_error: Option<String>,
    /// Every address is in 198.18.0.0/15, the range proxies use for fake-IP DNS.
    pub fake_ip: bool,
    /// `GET <relay>/ping` answered 2xx.
    pub https: bool,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
}

/// 198.18.0.0/15 (RFC 2544 benchmarking), used by Clash/sing-box/Surge fake-IP DNS.
pub fn is_fake_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            o[0] == 198 && (o[1] & 0xfe) == 18
        }
        IpAddr::V6(_) => false,
    }
}

pub async fn probe_relay(url: &str, extra_roots: &[Vec<u8>]) -> RelayProbe {
    let normalized = addr::normalize_relay(url);
    let (host, port) = match &normalized {
        Ok(u) => (
            u.host_str().unwrap_or_default().to_owned(),
            u.port_or_known_default().unwrap_or(443),
        ),
        Err(_) => (String::new(), 443),
    };
    let mut probe = RelayProbe {
        url: url.to_owned(),
        host: host.clone(),
        addresses: vec![],
        dns_error: None,
        fake_ip: false,
        https: false,
        latency_ms: None,
        error: None,
    };
    if let Err(error) = normalized {
        probe.error = Some(describe(&error));
        return probe;
    }
    let lookup = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host((host.trim_end_matches('.'), port)),
    )
    .await;
    match lookup {
        Ok(Ok(addrs)) => {
            let mut ips: Vec<IpAddr> = addrs.map(|a| a.ip()).collect();
            ips.sort();
            ips.dedup();
            probe.fake_ip = !ips.is_empty() && ips.iter().all(is_fake_ip);
            probe.addresses = ips.iter().map(ToString::to_string).collect();
        }
        Ok(Err(error)) => probe.dns_error = Some(error.to_string()),
        Err(_) => probe.dns_error = Some("DNS lookup timed out".into()),
    }
    let started = Instant::now();
    match crate::relay::ping_detail(url, extra_roots).await {
        Ok(()) => {
            probe.https = true;
            probe.latency_ms = Some(started.elapsed().as_millis() as u64);
        }
        Err(error) => probe.error = Some(describe(&error)),
    }
    probe
}

/// `cranibox peer pair doctor [link]`.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Report {
    pub checks: Vec<Check>,
    pub relays: Vec<RelayProbe>,
    pub network: Option<Network>,
    /// What to do, most important first.
    pub hints: Vec<String>,
}
impl Report {
    pub fn ok(&self) -> bool {
        self.checks.iter().all(|c| c.status != "fail")
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Inputs gathered by the caller: the running node's view (if the service runs),
/// and the link to check (if any).
pub struct DoctorInput<'a> {
    pub network: Option<Network>,
    /// Own relays that should be probed even when the node is not running.
    pub configured_relays: Vec<String>,
    pub node_id: Option<&'a str>,
    pub link: Option<&'a str>,
    pub extra_roots: &'a [Vec<u8>],
}

pub async fn doctor(input: DoctorInput<'_>) -> Report {
    let mut checks = vec![];
    let mut hints = vec![];
    // 1. The link.
    let mut ticket: Option<PairTicket> = None;
    if let Some(link) = input.link {
        match ticket::parse_link(link) {
            Ok(t) => {
                let now = now();
                if t.expires_at <= now {
                    checks.push(check(
                        "link",
                        "fail",
                        format!("expired {} min ago", (now - t.expires_at) / 60 + 1),
                    ));
                    hints.push("The link has expired: ask the other device for a new one.".into());
                } else if input.node_id == Some(t.inviter_node_id.as_str()) {
                    checks.push(check("link", "fail", "it was created on this device"));
                    hints.push(
                        "Open the link on the other device, not on the one that created it.".into(),
                    );
                } else {
                    checks.push(check(
                        "link",
                        "ok",
                        format!(
                            "{:?} link from {}, valid for {} more min",
                            t.purpose,
                            t.inviter.endpoint_id,
                            (t.expires_at - now) / 60
                        ),
                    ));
                }
                ticket = Some(t);
            }
            Err(error) => {
                checks.push(check("link", "fail", describe(&error)));
                hints.push(
                    "Copy the whole link again, or update Cranibox if the message says so.".into(),
                );
            }
        }
    }
    // 2. This device's node service and relay.
    let network = input.network.clone();
    match &network {
        Some(net) if net.home_connected() => checks.push(check(
            "this device's relay",
            "ok",
            format!("connected to {}", net.home_relay().unwrap_or_default()),
        )),
        Some(net) => {
            checks.push(check(
                "this device's relay",
                "fail",
                match net.home_relay() {
                    Some(url) => format!("{url} is not connected"),
                    None => "no relay configured".into(),
                },
            ));
            hints.push(
                "This device has no working relay: check that outbound HTTPS (443) is allowed."
                    .into(),
            );
        }
        None => {
            checks.push(check(
                "node service",
                "warn",
                "not running; start it with `cranibox serve --daemon` (relay and UDP checks need it)",
            ));
        }
    }
    // 3. UDP / direct paths.
    match network.as_ref().and_then(|n| n.udp) {
        Some(true) => checks.push(check(
            "UDP",
            "ok",
            match network.as_ref().and_then(|n| n.global_v4.clone()) {
                Some(addr) => format!("works (public address {addr})"),
                None => "works".into(),
            },
        )),
        Some(false) => checks.push(check(
            "UDP",
            "warn",
            "blocked: only relayed connections are possible from this device",
        )),
        None if network.is_none() => checks.push(check(
            "UDP",
            "unknown",
            "needs the node service (`cranibox serve --daemon`)",
        )),
        None => checks.push(check("UDP", "unknown", "no network report yet")),
    }
    if network.as_ref().and_then(|n| n.captive_portal) == Some(true) {
        checks.push(check(
            "captive portal",
            "warn",
            "iroh's captive-portal probe was intercepted (a login page, or a transparent proxy)",
        ));
    }
    // 4. Relays: ours and the other device's.
    let mut urls: Vec<String> = vec![];
    let push = |url: &str, urls: &mut Vec<String>| {
        if !urls.iter().any(|u| same_relay(u, url)) {
            urls.push(url.to_owned());
        }
    };
    let home = network
        .as_ref()
        .and_then(|n| n.home_relay().map(str::to_owned));
    if let Some(home) = &home {
        push(home, &mut urls);
    }
    for url in &input.configured_relays {
        push(url, &mut urls);
    }
    let target = ticket.as_ref().and_then(|t| t.inviter.relay_url.clone());
    if let Some(target) = &target {
        push(target, &mut urls);
    }
    let mut relays = vec![];
    for url in &urls {
        relays.push(probe_relay(url, input.extra_roots).await);
    }
    for probe in &relays {
        let role = if target.as_deref().is_some_and(|t| same_relay(t, &probe.url)) {
            if home.as_deref().is_some_and(|h| same_relay(h, &probe.url)) {
                "relay (both devices)"
            } else {
                "other device's relay"
            }
        } else {
            "this device's relay (HTTPS)"
        };
        let mut detail = format!("{} ->", probe.url);
        if let Some(error) = &probe.dns_error {
            detail.push_str(&format!(" DNS failed ({error})"));
        } else {
            detail.push_str(&format!(" {}", probe.addresses.join(", ")));
            if probe.fake_ip {
                detail.push_str(" (proxy fake IP)");
            }
        }
        match (probe.https, probe.latency_ms) {
            (true, Some(ms)) => detail.push_str(&format!("; HTTPS ok in {ms} ms")),
            _ => detail.push_str(&format!(
                "; HTTPS failed ({})",
                probe.error.as_deref().unwrap_or("no answer")
            )),
        }
        checks.push(check(role, if probe.https { "ok" } else { "fail" }, detail));
        if !probe.https && role == "other device's relay" {
            hints.push(format!(
                "This device cannot reach the other device's relay {}: pair the other way round (run `cranibox peer pair invite` here, accept on the other device).",
                probe.url
            ));
        }
    }
    // Fake-IP DNS: different relay names, one address. Only TLS SNI tells them apart.
    let fakes: Vec<&RelayProbe> = relays.iter().filter(|p| p.fake_ip).collect();
    if fakes.len() >= 2 {
        let shared = fakes.iter().any(|a| {
            fakes
                .iter()
                .any(|b| a.host != b.host && a.addresses.iter().any(|ip| b.addresses.contains(ip)))
        });
        if shared {
            checks.push(check(
                "fake-IP DNS",
                "warn",
                "different relays resolve to the same proxy address; fine only if the proxy routes by TLS server name",
            ));
            hints.push(
                "Your proxy answers DNS with one fake IP for different relays. If it routes by that IP instead of sniffing the TLS server name, connections to the other relay land on the wrong server. Enable SNI sniffing, or exclude *.iroh.link (and your own relay) from fake-IP."
                    .into(),
            );
        }
    } else if relays.iter().any(|p| p.fake_ip) {
        checks.push(check(
            "fake-IP DNS",
            "warn",
            "relay names resolve to a proxy fake IP (198.18.0.0/15); relays work only through that proxy",
        ));
    }
    // Different relays without UDP: the joiner must reach the other relay itself.
    if let (Some(target), Some(home)) = (&target, &home)
        && !same_relay(target, home)
        && network.as_ref().and_then(|n| n.udp) != Some(true)
    {
        hints.push(format!(
            "The devices use different relays ({home} here, {target} there) and UDP is not confirmed here. If accepting times out, pair the other way round: the other device can reach this device's relay."
        ));
    }
    if hints.is_empty() && checks.iter().all(|c| c.status == "ok") {
        hints.push("Everything this device can check looks fine.".into());
    }
    Report {
        checks,
        relays,
        network,
        hints,
    }
}

/// Doctor for a node that runs in this process.
pub async fn doctor_node(node: &IrohNode, node_id: &str, link: Option<&str>) -> Report {
    doctor(DoctorInput {
        network: Some(Network::of(node)),
        configured_relays: node.own_relays().iter().map(ToString::to_string).collect(),
        node_id: Some(node_id),
        link,
        extra_roots: node.extra_roots(),
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use cranibox_contracts::net::{IrohAddr, PairPurpose};

    #[test]
    fn repeated_transparent_causes_print_once() {
        #[derive(Debug)]
        struct Inner;
        impl std::fmt::Display for Inner {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("timed out")
            }
        }
        impl std::error::Error for Inner {}
        #[derive(Debug)]
        struct Transparent(Inner);
        impl std::fmt::Display for Transparent {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
        impl std::error::Error for Transparent {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }
        let error = anyhow::Error::new(Transparent(Inner)).context("iroh connect");
        assert_eq!(format!("{error:#}"), "iroh connect: timed out: timed out");
        assert_eq!(describe(&error), "iroh connect: timed out");
        let plain = anyhow::anyhow!("a").context("b");
        assert_eq!(describe(&plain), "b: a");
    }

    #[test]
    fn fake_ip_range_is_198_18_slash_15() {
        for ip in ["198.18.0.1", "198.19.255.254"] {
            assert!(is_fake_ip(&ip.parse().unwrap()), "{ip}");
        }
        for ip in ["198.17.0.1", "198.20.0.1", "8.8.8.8", "::1"] {
            assert!(!is_fake_ip(&ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn n0_relays_are_public() {
        let n0 = iroh::defaults::prod::default_relay_map().urls::<Vec<_>>();
        assert!(is_public_relay(&n0[0]));
        assert!(!is_public_relay(
            &addr::normalize_relay("https://hub.example.com").unwrap()
        ));
    }

    fn ticket(relay: &str) -> PairTicket {
        PairTicket {
            v: 1,
            inviter: IrohAddr {
                endpoint_id: addr::hex_id(&iroh::SecretKey::from_bytes(&[3; 32]).public()),
                relay_url: Some(relay.into()),
                direct: vec!["192.0.2.7:51000".into()],
            },
            inviter_node_id: "node-x".into(),
            secret: [1; 16],
            expires_at: now() + 600,
            purpose: PairPurpose::Worker,
            hub: None,
        }
    }

    #[test]
    fn dial_failure_names_both_relays_and_suggests_the_reverse() {
        let network = Network {
            home_relays: vec![("https://use1-1.relay.n0.iroh.link./".into(), true)],
            udp: Some(false),
            ..Default::default()
        };
        let t = ticket("https://usw1-1.relay.n0.iroh.link./");
        let cause = anyhow::anyhow!("timed out").context("iroh connect");
        let text = dial_failure(
            Stage::Connect,
            &t,
            &network,
            Duration::from_secs(30),
            Some(&cause),
        );
        assert!(text.contains(&t.inviter.endpoint_id), "{text}");
        assert!(
            text.contains("its relay: https://usw1-1.relay.n0.iroh.link./"),
            "{text}"
        );
        assert!(
            text.contains("this device's relay: https://use1-1.relay.n0.iroh.link./ (connected)"),
            "{text}"
        );
        assert!(text.contains("UDP here: blocked"), "{text}");
        assert!(text.contains("cause: iroh connect: timed out"), "{text}");
        assert!(!text.contains("timed out: timed out"), "{text}");
        assert!(text.contains("cranibox peer pair invite"), "{text}");
        assert!(text.contains("cranibox peer pair doctor"), "{text}");
        // Same relay and working UDP: no reverse-pairing advice.
        let same = Network {
            home_relays: vec![("https://usw1-1.relay.n0.iroh.link./".into(), true)],
            udp: Some(true),
            ..Default::default()
        };
        let text = dial_failure(Stage::Connect, &t, &same, Duration::from_secs(30), None);
        assert!(!text.contains("other direction"), "{text}");
    }

    #[tokio::test]
    async fn doctor_flags_expired_links_and_missing_service() {
        let mut t = ticket("http://127.0.0.1:9/");
        t.expires_at = now() - 120;
        let link = ticket::app_link(&t).unwrap();
        let report = doctor(DoctorInput {
            network: None,
            configured_relays: vec![],
            node_id: None,
            link: Some(&link),
            extra_roots: &[],
        })
        .await;
        assert!(!report.ok());
        assert_eq!(report.checks[0].name, "link");
        assert_eq!(report.checks[0].status, "fail");
        assert!(report.checks[0].detail.starts_with("expired"));
        assert!(report.checks.iter().any(|c| c.name == "node service"));
        assert!(report.hints.iter().any(|h| h.contains("new one")));
        // The unreachable relay of the other device is reported with the reverse hint.
        assert!(
            report
                .checks
                .iter()
                .any(|c| c.name == "other device's relay" && c.status == "fail")
        );
    }

    #[tokio::test]
    async fn doctor_warns_when_relays_differ_without_udp() {
        let t = ticket("http://127.0.0.1:9/");
        let link = ticket::app_link(&t).unwrap();
        let report = doctor(DoctorInput {
            network: Some(Network {
                home_relays: vec![("http://127.0.0.2:9/".into(), true)],
                udp: Some(false),
                ..Default::default()
            }),
            configured_relays: vec![],
            node_id: Some("node-self"),
            link: Some(&link),
            extra_roots: &[],
        })
        .await;
        assert!(
            report
                .checks
                .iter()
                .any(|c| c.name == "UDP" && c.status == "warn")
        );
        assert!(report.hints.iter().any(|h| h.contains("different relays")));
        assert_eq!(report.relays.len(), 2);
    }
}
