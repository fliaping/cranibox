//! Conversions between the contract [`IrohAddr`] and iroh's `EndpointAddr`.
use anyhow::{Context, Result};
use cranibox_contracts::net::IrohAddr;
use iroh::{EndpointAddr, EndpointId, RelayUrl, TransportAddr};

/// Parse an EndpointId written as 64 hex characters or z-base-32.
pub fn parse_id(value: &str) -> Result<EndpointId> {
    if value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        let mut bytes = [0u8; 32];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16)?;
        }
        return EndpointId::from_bytes(&bytes).context("invalid EndpointId");
    }
    value.parse().context("invalid EndpointId")
}

pub fn hex_id(id: &EndpointId) -> String {
    id.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

/// Relay URLs are stored as origins; iroh always dials `<origin>/relay`.
pub fn normalize_relay(url: &str) -> Result<RelayUrl> {
    let mut parsed = url::Url::parse(url).context("invalid relay URL")?;
    anyhow::ensure!(
        matches!(parsed.scheme(), "https" | "http") && parsed.host_str().is_some(),
        "relay URL must be http(s)://host[:port]"
    );
    anyhow::ensure!(
        parsed.username().is_empty() && parsed.password().is_none(),
        "relay URL must not carry credentials"
    );
    parsed.set_path("/");
    parsed.set_query(None);
    parsed.set_fragment(None);
    Ok(RelayUrl::from(parsed))
}

pub fn endpoint_addr(addr: &IrohAddr) -> Result<EndpointAddr> {
    let mut out = EndpointAddr::new(parse_id(&addr.endpoint_id)?);
    if let Some(relay) = &addr.relay_url {
        out = out.with_relay_url(normalize_relay(relay)?);
    }
    for direct in &addr.direct {
        out = out.with_ip_addr(direct.parse().context("invalid direct address")?);
    }
    Ok(out)
}

pub fn iroh_addr(addr: &EndpointAddr) -> IrohAddr {
    IrohAddr {
        endpoint_id: hex_id(&addr.id),
        relay_url: addr.addrs.iter().find_map(|a| match a {
            TransportAddr::Relay(url) => Some(url.to_string()),
            _ => None,
        }),
        direct: addr
            .addrs
            .iter()
            .filter_map(|a| match a {
                TransportAddr::Ip(ip) => Some(ip.to_string()),
                _ => None,
            })
            .take(8)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn addresses_round_trip_and_reject_bad_input() {
        let key = iroh::SecretKey::from_bytes(&[7u8; 32]).public();
        let addr = IrohAddr {
            endpoint_id: hex_id(&key),
            relay_url: Some("https://hub.example.com/relay".into()),
            direct: vec!["192.0.2.1:4433".into()],
        };
        let parsed = endpoint_addr(&addr).unwrap();
        assert_eq!(parsed.id, key);
        let back = iroh_addr(&parsed);
        assert_eq!(back.endpoint_id, addr.endpoint_id);
        assert_eq!(back.relay_url.as_deref(), Some("https://hub.example.com/"));
        assert_eq!(back.direct, addr.direct);
        assert_eq!(parse_id(&key.to_string()).unwrap(), key);
        assert!(parse_id("zz").is_err());
        assert!(normalize_relay("ftp://x").is_err());
        assert!(normalize_relay("https://u:p@x/").is_err());
    }
}
