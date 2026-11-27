//! One-time pairing tickets: `cranibox://pair/<base32-nopad(postcard(ticket))>`,
//! the web form `https://<hub>/pair#<same>`, and the QR code of the app link.
use anyhow::{Context, Result, bail, ensure};
use cranibox_contracts::net::{PAIR_LINK_PREFIX, PAIR_TICKET_VERSION, PAIR_WEB_PATH, PairTicket};
use data_encoding::BASE32_NOPAD;

/// Upper bound for an encoded ticket (QR codes stay scannable well below this).
pub const MAX_ENCODED: usize = 1024;

pub fn encode(ticket: &PairTicket) -> Result<String> {
    let bytes = postcard::to_allocvec(ticket).context("encode pairing ticket")?;
    let text = BASE32_NOPAD.encode(&bytes);
    ensure!(text.len() <= MAX_ENCODED, "pairing ticket too large");
    Ok(text)
}

pub fn decode(text: &str) -> Result<PairTicket> {
    let text = text.trim();
    ensure!(
        !text.is_empty() && text.len() <= MAX_ENCODED,
        "This is not a Cranibox pairing link."
    );
    let bytes = BASE32_NOPAD
        .decode(text.to_ascii_uppercase().as_bytes())
        .context("This is not a Cranibox pairing link.")?;
    let ticket: PairTicket = postcard::from_bytes(&bytes).map_err(|_| undecodable(&bytes))?;
    ensure!(
        ticket.v == PAIR_TICKET_VERSION,
        "This pairing link needs a newer Cranibox."
    );
    crate::addr::parse_id(&ticket.inviter.endpoint_id)?;
    Ok(ticket)
}

/// The ticket layout with the purpose as its raw variant index, to recognise links
/// that only a newer Cranibox understands. Same wire format as [`PairTicket`].
#[derive(serde::Deserialize)]
#[allow(dead_code)]
struct RawTicket {
    v: u8,
    inviter: cranibox_contracts::net::IrohAddr,
    inviter_node_id: String,
    secret: [u8; 16],
    expires_at: u64,
    purpose: u32,
    hub: Option<cranibox_contracts::net::IrohAddr>,
}

/// Why valid base32 did not decode as a ticket, in words (never a serde message).
fn undecodable(bytes: &[u8]) -> anyhow::Error {
    const UPDATE: &str =
        "Update Cranibox on this device (`cranibox update apply latest`) and try again.";
    match bytes.first() {
        Some(&v) if v > PAIR_TICKET_VERSION => {
            anyhow::anyhow!("This pairing link needs a newer Cranibox (link version {v}). {UPDATE}")
        }
        Some(&v) if v == PAIR_TICKET_VERSION => match postcard::from_bytes::<RawTicket>(bytes) {
            Ok(raw) => anyhow::anyhow!(
                "This pairing link uses a pairing type this Cranibox does not know yet (type {}), such as a controlled worker. {UPDATE}",
                raw.purpose
            ),
            Err(_) => anyhow::anyhow!(
                "This pairing link is damaged or comes from a different Cranibox version. Copy the whole link again; if it still fails, update Cranibox on both devices."
            ),
        },
        _ => anyhow::anyhow!("This is not a Cranibox pairing link."),
    }
}

pub fn app_link(ticket: &PairTicket) -> Result<String> {
    Ok(format!("{PAIR_LINK_PREFIX}{}", encode(ticket)?))
}

/// `https://<hub>/pair#<ticket>`. The fragment never reaches the server.
pub fn web_link(public_url: &str, ticket: &PairTicket) -> Result<String> {
    let mut base = url::Url::parse(public_url).context("invalid public URL")?;
    base.set_path("");
    base.set_query(None);
    base.set_fragment(None);
    Ok(format!(
        "{}{PAIR_WEB_PATH}{}",
        base.as_str().trim_end_matches('/'),
        encode(ticket)?
    ))
}

/// Accept the app link, the web link or the bare ticket text (pasted or scanned).
pub fn parse_link(link: &str) -> Result<PairTicket> {
    let link = link.trim();
    if let Some(rest) = link.strip_prefix(PAIR_LINK_PREFIX) {
        return decode(rest.trim_end_matches('/'));
    }
    if link.starts_with("https://") || link.starts_with("http://") {
        let url = url::Url::parse(link).context("This is not a Cranibox pairing link.")?;
        ensure!(
            url.path() == "/pair",
            "This is not a Cranibox pairing link."
        );
        return decode(url.fragment().unwrap_or_default());
    }
    if link.contains("://") {
        bail!("This is not a Cranibox pairing link.");
    }
    decode(link)
}

/// QR code (SVG) of `text`, with a quiet zone, black on white.
pub fn qr_svg(text: &str) -> Result<String> {
    let code = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::M)
        .context("QR code")?;
    Ok(code
        .render::<qrcode::render::svg::Color<'_>>()
        .min_dimensions(256, 256)
        .quiet_zone(true)
        .build())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cranibox_contracts::net::{IrohAddr, PairPurpose};
    fn ticket() -> PairTicket {
        PairTicket {
            v: 1,
            inviter: IrohAddr {
                endpoint_id: crate::addr::hex_id(&iroh::SecretKey::from_bytes(&[3; 32]).public()),
                relay_url: Some("https://hub.example.com/".into()),
                direct: vec!["192.0.2.7:51000".into()],
            },
            inviter_node_id: "node-1f2e3d4c5b6a79880716253443526170".into(),
            secret: [9; 16],
            expires_at: 1_792_370_000,
            purpose: PairPurpose::OwnerDevice,
            hub: None,
        }
    }
    #[test]
    fn links_round_trip_in_every_form() {
        let t = ticket();
        let link = app_link(&t).unwrap();
        assert!(link.starts_with("cranibox://pair/"));
        assert_eq!(parse_link(&link).unwrap(), t);
        let web = web_link("https://hub.example.com/some/path?q=1", &t).unwrap();
        assert!(web.starts_with("https://hub.example.com/pair#"));
        assert_eq!(parse_link(&web).unwrap(), t);
        let bare = encode(&t).unwrap();
        assert_eq!(parse_link(&bare.to_ascii_lowercase()).unwrap(), t);
        let svg = qr_svg(&link).unwrap();
        assert!(svg.starts_with("<?xml") || svg.starts_with("<svg"));
    }
    #[test]
    fn foreign_or_damaged_links_are_rejected() {
        let t = ticket();
        let mut text = encode(&t).unwrap();
        text.truncate(text.len() - 9);
        assert!(parse_link(&text).is_err());
        assert!(parse_link("https://evil.example.com/other#AAAA").is_err());
        assert!(parse_link("ftp://x/pair#AAAA").is_err());
        assert!(parse_link("").is_err());
        let mut newer = t.clone();
        newer.v = 2;
        assert!(parse_link(&encode(&newer).unwrap()).is_err());
    }
    #[test]
    fn unknown_pairing_types_ask_for_an_update_instead_of_a_serde_error() {
        // A newer Cranibox adds a fourth purpose (variant index 3): replace the
        // trailing purpose and `hub: None` bytes of a known ticket.
        let mut bytes = postcard::to_allocvec(&ticket()).unwrap();
        let n = bytes.len();
        assert_eq!(&bytes[n - 2..], &[0, 0]);
        bytes[n - 2] = 3;
        let error = parse_link(&BASE32_NOPAD.encode(&bytes))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("pairing type this Cranibox does not know yet (type 3)"),
            "{error}"
        );
        assert!(error.contains("cranibox update apply latest"), "{error}");
        assert!(!error.to_lowercase().contains("serde"), "{error}");
        // Damaged bytes after a valid version byte.
        let error = parse_link(&BASE32_NOPAD.encode(&[1, 0xff, 0xff]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("damaged"), "{error}");
        // A newer ticket layout.
        let error = parse_link(&BASE32_NOPAD.encode(&[9, 1, 2]))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("needs a newer Cranibox (link version 9)"),
            "{error}"
        );
        // Worker links still round-trip unchanged.
        let mut worker = ticket();
        worker.purpose = PairPurpose::Worker;
        assert_eq!(parse_link(&app_link(&worker).unwrap()).unwrap(), worker);
    }
}
