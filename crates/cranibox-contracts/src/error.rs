//! C8 — errors and status codes (`docs/design/WORKPLAN.md` §1 C8).
//!
//! When the llm proxy (local `:3424` or mesh `/llm/*`) cannot serve a request
//! it answers HTTP 503 with [`UnavailableBody`]. The UI and agents choose
//! their text by [`ErrorCode`], never by `message`.
use crate::NodeId;
use serde::{Deserialize, Serialize};

pub const UNAVAILABLE_STATUS: u16 = 503;
pub const UNAVAILABLE_TYPE: &str = "cranibox_unavailable";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    HubOffline,
    ClaudeNodeOffline,
    AccountDisabledHere,
    HomeOffline,
    AllAccountsExhausted,
    NeedsSignIn,
}
impl ErrorCode {
    pub const ALL: [ErrorCode; 6] = [
        Self::HubOffline,
        Self::ClaudeNodeOffline,
        Self::AccountDisabledHere,
        Self::HomeOffline,
        Self::AllAccountsExhausted,
        Self::NeedsSignIn,
    ];
    /// i18n key the UI uses for this code.
    pub fn text_key(self) -> String {
        format!("error.{}", self.as_str())
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HubOffline => "hub_offline",
            Self::ClaudeNodeOffline => "claude_node_offline",
            Self::AccountDisabledHere => "account_disabled_here",
            Self::HomeOffline => "home_offline",
            Self::AllAccountsExhausted => "all_accounts_exhausted",
            Self::NeedsSignIn => "needs_sign_in",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnavailableError {
    /// Always [`UNAVAILABLE_TYPE`].
    #[serde(rename = "type")]
    pub kind: String,
    pub code: ErrorCode,
    pub message: String,
    pub node: Option<NodeId>,
}

/// `{"error":{"type":"cranibox_unavailable","code":"…","message":"…","node":"…"}}`
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnavailableBody {
    pub error: UnavailableError,
}
impl UnavailableBody {
    pub fn new(code: ErrorCode, message: impl Into<String>, node: Option<NodeId>) -> Self {
        Self {
            error: UnavailableError {
                kind: UNAVAILABLE_TYPE.into(),
                code,
                message: message.into(),
                node,
            },
        }
    }
}
