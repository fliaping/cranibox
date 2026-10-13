use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const VERSION: u32 = 1;
pub const MAX_FRAME: usize = 64 * 1024;
pub const MAX_OUTPUT: usize = 64 * 1024;
pub const MAX_JOBS: usize = 128;
pub const MAX_PARALLEL: usize = 4;
pub const HEARTBEAT_SECS: u64 = 10;
pub const DEAD_SECS: u64 = 35;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecRequest {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}
fn default_timeout() -> u64 {
    60
}
impl ExecRequest {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.program.is_empty() && !self.program.contains('\0'),
            "invalid program"
        );
        anyhow::ensure!(
            (1..=3600).contains(&self.timeout_secs),
            "timeout must be 1..3600 seconds"
        );
        anyhow::ensure!(
            self.args.len() <= 256 && self.env.len() <= 64,
            "too many arguments or environment entries"
        );
        anyhow::ensure!(
            serde_json::to_vec(self)?.len() <= 32 * 1024,
            "command exceeds 32 KiB"
        );
        anyhow::ensure!(
            self.args.iter().all(|s| !s.contains('\0')),
            "NUL in argument"
        );
        anyhow::ensure!(
            self.cwd.as_ref().is_none_or(|s| !s.contains('\0')),
            "NUL in cwd"
        );
        anyhow::ensure!(
            self.env
                .iter()
                .all(|(k, v)| !k.is_empty() && !k.contains(['=', '\0']) && !v.contains('\0')),
            "invalid environment"
        );
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Wire {
    Hello {
        version: u32,
        name: String,
        os: String,
        arch: String,
        #[serde(default)]
        terminal: bool,
        #[serde(default)]
        terminal_sessions: bool,
    },
    Welcome {
        version: u32,
    },
    Heartbeat {},
    TerminalOpen {
        id: String,
        cols: u16,
        rows: u16,
        session: String,
        takeover: bool,
    },
    TerminalManage {
        id: String,
        request: serde_json::Value,
    },
    TerminalResult {
        id: String,
        result: serde_json::Value,
    },
    TerminalInput {
        id: String,
        data: String,
    },
    TerminalResize {
        id: String,
        cols: u16,
        rows: u16,
    },
    TerminalClose {
        id: String,
    },
    TerminalOutput {
        id: String,
        data: String,
    },
    TerminalExit {
        id: String,
        code: u32,
    },
    Execute {
        id: String,
        request: ExecRequest,
    },
    Cancel {
        id: String,
    },
    Output {
        id: String,
        stream: Stream,
        data: String,
    },
    Finished {
        id: String,
        exit_code: Option<i32>,
        outcome: Outcome,
        error: Option<String>,
    },
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stream {
    Stdout,
    Stderr,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Running,
    Exited,
    Failed,
    TimedOut,
    Cancelled,
    Disconnected,
}

pub fn valid_node_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reject_invalid_commands_and_versions() {
        let mut r: ExecRequest = serde_json::from_str(r#"{"program":"echo"}"#).unwrap();
        assert!(r.validate().is_ok());
        r.timeout_secs = 0;
        assert!(r.validate().is_err());
        assert!(!valid_node_id("../node"));
        assert!(serde_json::from_str::<Wire>(r#"{"type":"heartbeat","extra":1}"#).is_err());
    }
}
