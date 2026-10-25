//! C2 — transports, mesh routes and the pairing ticket
//! (`docs/design/WORKPLAN.md` §1 C2, ARCHITECTURE §4, §5.2).
//!
//! Implemented by WS1 (iroh) and WS0 (thin wrapper over the existing HTTP and
//! Noise paths in `workspace-runtime`). Used by WS6 and WS7.
use crate::{EndpointId, NodeId};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::{fmt, pin::Pin};

/// ALPN of the peer HTTP protocol: one HTTP/1.1 connection (with Upgrade) per
/// QUIC bidirectional stream, carrying the unchanged `/api/v1/mesh/*` router.
pub const ALPN_PEER_HTTP: &[u8] = b"cranibox/peer-http/1";
/// Reserved for a future bulk llm channel. Not used in this phase.
pub const ALPN_LLM_RESERVED: &[u8] = b"cranibox/llm/1";
/// Header naming the expected target node on every mesh request (existing behaviour).
pub const TARGET_HEADER: &str = "x-cranibox-target";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportKind {
    #[serde(rename = "iroh")]
    Iroh,
    /// Existing HTTP(S) routes, with Noise IK end-to-end frames when a key is pinned.
    #[serde(rename = "direct")]
    Direct,
    #[serde(rename = "tailscale")]
    Tailscale,
    #[serde(rename = "easytier")]
    EasyTier,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathKind {
    Direct,
    Relay,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathScope {
    Lan,
    Internet,
    Relay,
    Overlay,
}
impl PathScope {
    /// Classify the remote IP, including IPv4-mapped IPv6 and overlay ranges.
    pub fn for_ip(ip: std::net::IpAddr) -> Self {
        use std::net::IpAddr;
        match ip {
            IpAddr::V4(ip) => {
                let [a, b, _, _] = ip.octets();
                if a == 100 && (64..128).contains(&b) {
                    Self::Overlay
                } else if ip.is_private()
                    || ip.is_link_local()
                    || ip.is_loopback()
                    || ip.is_unspecified()
                {
                    Self::Lan
                } else {
                    Self::Internet
                }
            }
            IpAddr::V6(ip) => {
                if let Some(ip) = ip.to_ipv4_mapped() {
                    return Self::for_ip(ip.into());
                }
                let parts = ip.segments();
                if parts[..3] == [0xfd7a, 0x115c, 0xa1e0] {
                    Self::Overlay
                } else if ip.is_unique_local()
                    || ip.is_unicast_link_local()
                    || ip.is_loopback()
                    || ip.is_unspecified()
                {
                    Self::Lan
                } else {
                    Self::Internet
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathInfo {
    pub kind: PathKind,
    pub rtt_ms: Option<u32>,
    /// Relay host, direct IP class, or overlay transport name; never a full URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<PathScope>,
}
impl PathInfo {
    pub fn direct(ip: std::net::IpAddr) -> Self {
        let scope = PathScope::for_ip(ip);
        Self {
            kind: PathKind::Direct,
            rtt_ms: None,
            scope: Some(scope),
            via: Some(
                match scope {
                    PathScope::Lan => "private",
                    PathScope::Internet => "public",
                    _ => "overlay",
                }
                .into(),
            ),
        }
    }
}

/// Request extension every transport must insert **before** handing a request
/// to `mesh_router()`. The `auth` middleware trusts it instead of a bearer
/// token; it can only be inserted in-process, never by a remote client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authenticated {
    pub node_id: NodeId,
    /// Peer EndpointId taken from the TLS handshake (iroh), if any. Must equal
    /// the pinned `signing_key` of the caller's grant.
    pub endpoint_id: Option<EndpointId>,
    pub transport: TransportKind,
    /// Whether the transport itself is end-to-end encrypted.
    pub e2e: bool,
}

/// Streaming body error.
pub type BodyError = Box<dyn std::error::Error + Send + Sync + 'static>;
type ChunkStream = Pin<Box<dyn futures_core::Stream<Item = Result<Bytes, BodyError>> + Send>>;

/// Transport-neutral streaming body (SSE responses must stay streaming).
pub struct BoxBody(ChunkStream);

impl BoxBody {
    pub fn new(
        stream: impl futures_core::Stream<Item = Result<Bytes, BodyError>> + Send + 'static,
    ) -> Self {
        Self(Box::pin(stream))
    }
    pub fn empty() -> Self {
        Self::new(Once(None))
    }
    pub fn full(bytes: impl Into<Bytes>) -> Self {
        Self::new(Once(Some(bytes.into())))
    }
    pub fn into_stream(self) -> ChunkStream {
        self.0
    }
}
impl fmt::Debug for BoxBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BoxBody")
    }
}
struct Once(Option<Bytes>);
impl futures_core::Stream for Once {
    type Item = Result<Bytes, BodyError>;
    fn poll_next(
        mut self: Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::task::Poll::Ready(self.0.take().map(Ok))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    #[error("peer unreachable")]
    Unreachable,
    #[error("handshake failed")]
    Handshake,
    #[error("peer rejected our credential")]
    Unauthorized,
    #[error("timed out")]
    Timeout,
    #[error("{0}")]
    Other(String),
}

/// A way to reach a peer's `/api/v1/mesh/*` router.
#[async_trait::async_trait]
pub trait PeerTransport: Send + Sync {
    fn kind(&self) -> TransportKind;
    /// Send one HTTP request to the peer's `/api/v1/mesh/*`. The request URI is
    /// the full mesh path (for example `/api/v1/mesh/nodes/{node}/llm/v1/models`).
    /// The response body must support streaming (SSE).
    async fn request(
        &self,
        peer: &NodeId,
        req: http::Request<BoxBody>,
    ) -> Result<http::Response<BoxBody>, TransportError>;
    async fn path(&self, peer: &NodeId) -> Option<PathInfo>;
}

/// Mesh route constants. Nested mounts are relative to [`routes::MESH_PREFIX`].
pub mod routes {
    pub const MESH_PREFIX: &str = "/api/v1/mesh";
    /// Mount point of the llm sub-router (WS6 fills `server/llm.rs`).
    pub const LLM_MOUNT: &str = "/nodes/{node}/llm";
    /// Mount point of the vault sub-router (WS6 fills `server/vault.rs`).
    pub const VAULT_MOUNT: &str = "/nodes/{node}/vault";

    // Relative to LLM_MOUNT. Need `Permissions.llm && roster.is_owner_node(caller)`.
    pub const LLM_CHAT: &str = "/v1/chat/completions";
    pub const LLM_RESPONSES: &str = "/v1/responses";
    pub const LLM_MESSAGES: &str = "/v1/messages";
    pub const LLM_COUNT_TOKENS: &str = "/v1/messages/count_tokens";
    pub const LLM_MODELS: &str = "/v1/models";
    /// → [`crate::llm::LlmHealth`].
    pub const LLM_HEALTH: &str = "/health";

    // Relative to VAULT_MOUNT. Need `Permissions.vault && roster.is_owner_node(caller)`.
    pub const VAULT_PULL: &str = "/pull";
    pub const VAULT_PUSH: &str = "/push";
    pub const VAULT_REFRESH_REQUEST: &str = "/refresh-request";
    pub const VAULT_LEASE: &str = "/lease";

    /// Full mesh path of an llm sub-route on `node`.
    pub fn llm(node: &str, sub: &str) -> String {
        format!("{MESH_PREFIX}/nodes/{node}/llm{sub}")
    }
    /// Full mesh path of a vault sub-route on `node`.
    pub fn vault(node: &str, sub: &str) -> String {
        format!("{MESH_PREFIX}/nodes/{node}/vault{sub}")
    }
}

/// iroh address as stored in config and tickets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IrohAddr {
    /// EndpointId, lowercase hex (64 chars) or z-base-32.
    pub endpoint_id: String,
    pub relay_url: Option<String>,
    /// Direct socket addresses, `ip:port`.
    #[serde(default)]
    pub direct: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairPurpose {
    /// Joins the owner's roster (all capabilities, including llm and vault).
    OwnerDevice,
    /// Gets a grant only (default: messages). Never joins the roster.
    Guest,
}

/// Current ticket version.
pub const PAIR_TICKET_VERSION: u8 = 1;
/// Ticket lifetime (ARCHITECTURE §5.2).
pub const PAIR_TICKET_TTL_SECS: u64 = 15 * 60;
/// App link prefix. Full link: `cranibox://pair/<base32-nopad(postcard(ticket))>`.
pub const PAIR_LINK_PREFIX: &str = "cranibox://pair/";
/// Web link: `https://<hub>/pair#<same base32 string>` (fragment never reaches the server).
pub const PAIR_WEB_PATH: &str = "/pair#";

/// One-time pairing invitation. Binary encoding is `postcard` of this struct
/// (decided by coordinator), then RFC 4648 base32 without padding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairTicket {
    /// Always [`PAIR_TICKET_VERSION`].
    pub v: u8,
    pub inviter: IrohAddr,
    pub inviter_node_id: NodeId,
    pub secret: [u8; 16],
    pub expires_at: u64,
    pub purpose: PairPurpose,
    pub hub: Option<IrohAddr>,
}
