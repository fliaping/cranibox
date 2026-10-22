//! Explicit Computer Use access. The target owner alone chooses the loopback provider.
use super::Store;
use super::config::Config;
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{net::SocketAddr, time::Duration};
use uuid::Uuid;

pub const MAX_RESPONSE: usize = 8 * 1024 * 1024;
pub const MAX_LEASE_SECS: u64 = 300;
fn lease_default() -> u64 {
    60
}

pub mod cua;

/// Which local program serves Computer Use.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// The loopback `agent-desktop-bridge` HTTP protocol (`POST /v1/action`).
    #[default]
    Http,
    /// cua-driver over MCP stdio (macOS, Linux X11, Windows).
    Cua,
}
impl ProviderKind {
    fn is_http(&self) -> bool {
        *self == Self::Http
    }
}
/// Configurations written before `kind` existed are HTTP bridges and keep their bytes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    #[serde(default, skip_serializing_if = "ProviderKind::is_http")]
    pub kind: ProviderKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<SocketAddr>,
}
impl Provider {
    pub fn http(address: SocketAddr) -> Self {
        Self {
            kind: ProviderKind::Http,
            address: Some(address),
        }
    }
    pub fn cua() -> Self {
        Self {
            kind: ProviderKind::Cua,
            address: None,
        }
    }
    pub fn validate(&self) -> Result<()> {
        match (self.kind, self.address) {
            (ProviderKind::Http, Some(address)) => ensure!(
                address.ip().is_loopback() && address.port() != 0,
                "Computer Use provider must use a loopback address and a nonzero port"
            ),
            (ProviderKind::Http, None) => anyhow::bail!("Computer Use bridge address is required"),
            (ProviderKind::Cua, address) => ensure!(
                address.is_none(),
                "the cua-driver provider has no network address"
            ),
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum OwnerCommand {
    Status {},
    EmergencyStop {},
    Resume {},
}

/// The node's persistent stop fence also protects against bridge outages and restarts.
pub async fn owner(path: &std::path::Path, command: OwnerCommand) -> Result<Value> {
    let configuration = if matches!(command, OwnerCommand::EmergencyStop {}) {
        Config::update(path, |c| {
            c.computer_paused = true;
            c.computer_epoch = c
                .computer_epoch
                .checked_add(1)
                .context("computer generation exhausted")?;
            Ok(())
        })?
    } else {
        Config::read(path)?
    };
    let provider = configuration
        .computer
        .clone()
        .context("Computer Use sharing is disabled")?;
    let mut result = match bridge(&provider, serde_json::to_value(&command)?, "local-owner").await {
        Ok(value) => value,
        Err(_)
            if matches!(
                command,
                OwnerCommand::EmergencyStop {} | OwnerCommand::Status {}
            ) =>
        {
            json!({"state":"backend-unavailable","backend_available":false,"paused":configuration.computer_paused})
        }
        Err(error) => return Err(error),
    };
    if matches!(command, OwnerCommand::Resume {}) {
        ensure!(result["paused"] == false, "provider did not confirm resume");
        Config::update(path, |c| {
            ensure!(
                c.computer_epoch == configuration.computer_epoch
                    && c.computer == configuration.computer,
                "Computer Use sharing changed during resume"
            );
            c.computer_paused = false;
            Ok(())
        })?;
    } else if configuration.computer_paused {
        result["paused"] = json!(true);
        result["state"] = json!("paused");
    }
    Ok(result)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Status {},
    Acquire {
        #[serde(default = "lease_default")]
        ttl_secs: u64,
    },
    Renew {
        lease: Uuid,
        #[serde(default = "lease_default")]
        ttl_secs: u64,
    },
    Release {
        lease: Uuid,
    },
    Action {
        #[serde(default)]
        lease: Option<Uuid>,
        action: Action,
    },
    EmergencyStop {},
}
impl Request {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Acquire { ttl_secs } | Self::Renew { ttl_secs, .. } => {
                ensure!(
                    (1..=MAX_LEASE_SECS).contains(ttl_secs),
                    "lease duration must be 1–300 seconds"
                );
            }
            Self::Action { action, .. } => action.validate()?,
            _ => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Coordinates {
    #[default]
    Pixels,
    Normalized,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Screenshot {},
    Zoom {
        region: [f64; 4],
    },
    CursorPosition {},
    WindowList {},
    MouseMove {
        coordinate: [f64; 2],
        #[serde(default)]
        coordinate_space: Coordinates,
    },
    LeftClick {
        #[serde(default)]
        coordinate: Option<[f64; 2]>,
        #[serde(default)]
        coordinate_space: Coordinates,
    },
    RightClick {
        #[serde(default)]
        coordinate: Option<[f64; 2]>,
        #[serde(default)]
        coordinate_space: Coordinates,
    },
    MiddleClick {
        #[serde(default)]
        coordinate: Option<[f64; 2]>,
        #[serde(default)]
        coordinate_space: Coordinates,
    },
    DoubleClick {
        #[serde(default)]
        coordinate: Option<[f64; 2]>,
        #[serde(default)]
        coordinate_space: Coordinates,
    },
    LeftClickDrag {
        start_coordinate: [f64; 2],
        coordinate: [f64; 2],
        #[serde(default)]
        coordinate_space: Coordinates,
    },
    Scroll {
        scroll_direction: Direction,
        scroll_amount: u16,
        #[serde(default)]
        coordinate: Option<[f64; 2]>,
        #[serde(default)]
        coordinate_space: Coordinates,
    },
    Type {
        text: String,
    },
    Key {
        text: String,
    },
    WindowFocus {
        window_id: String,
    },
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}
fn coordinate(value: &[f64], space: Coordinates) -> Result<()> {
    let max = match space {
        Coordinates::Pixels => 65535.,
        Coordinates::Normalized => 1.,
    };
    ensure!(
        value.iter().all(|v| v.is_finite() && *v >= 0. && *v <= max),
        "coordinate is outside the allowed range"
    );
    Ok(())
}
impl Action {
    pub fn mutates(&self) -> bool {
        !matches!(
            self,
            Self::Screenshot {} | Self::Zoom { .. } | Self::CursorPosition {} | Self::WindowList {}
        )
    }
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Zoom { region } => {
                coordinate(region, Coordinates::Pixels)?;
                ensure!(
                    region[0] < region[2] && region[1] < region[3],
                    "invalid screenshot region"
                );
            }
            Self::MouseMove {
                coordinate: point,
                coordinate_space,
            } => coordinate(point, *coordinate_space)?,
            Self::LeftClickDrag {
                start_coordinate,
                coordinate: end,
                coordinate_space,
            } => {
                coordinate(start_coordinate, *coordinate_space)?;
                coordinate(end, *coordinate_space)?;
            }
            Self::Type { text } => ensure!(
                !text.is_empty() && text.len() <= 16 * 1024 && !text.contains('\0'),
                "text must contain 1–16384 bytes without NUL"
            ),
            Self::Key { text } => ensure!(
                !text.is_empty() && text.len() <= 128 && !text.chars().any(char::is_control),
                "invalid key combination"
            ),
            Self::WindowFocus { window_id } => ensure!(
                !window_id.is_empty()
                    && window_id.len() <= 20
                    && window_id.bytes().all(|v| v.is_ascii_digit()),
                "window ID must come from window_list"
            ),
            Self::Scroll {
                scroll_amount,
                coordinate: point,
                coordinate_space,
                ..
            } => {
                ensure!(
                    (1..=100).contains(scroll_amount),
                    "scroll amount must be 1–100"
                );
                if let Some(point) = point {
                    coordinate(point, *coordinate_space)?;
                }
            }
            Self::LeftClick {
                coordinate: point,
                coordinate_space,
            }
            | Self::RightClick {
                coordinate: point,
                coordinate_space,
            }
            | Self::MiddleClick {
                coordinate: point,
                coordinate_space,
            }
            | Self::DoubleClick {
                coordinate: point,
                coordinate_space,
            } => {
                if let Some(point) = point {
                    coordinate(point, *coordinate_space)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub fn payload(&self) -> Result<Value> {
        self.validate()?;
        let mut value = serde_json::to_value(self)?;
        if value["coordinate_space"] != "normalized" {
            for key in ["coordinate", "start_coordinate", "region"] {
                if let Some(points) = value.get_mut(key).and_then(Value::as_array_mut) {
                    for point in points {
                        *point = json!(point.as_f64().unwrap().round() as u64);
                    }
                }
            }
        }
        // The bridge distinguishes an omitted coordinate from JSON null.
        if value.get("coordinate").is_some_and(Value::is_null) {
            value.as_object_mut().unwrap().remove("coordinate");
        }
        Ok(value)
    }
}

/// No proxy, redirects or retry: an ambiguous input must never be replayed.
pub async fn bridge(provider: &Provider, payload: Value, session: &str) -> Result<Value> {
    provider.validate()?;
    if provider.kind == ProviderKind::Cua {
        let action = payload["action"].clone();
        let value = cua::driver().call(payload).await?;
        if action == "screenshot" || action == "zoom" {
            validate_image(&value)?;
        }
        return Ok(value);
    }
    let address = provider
        .address
        .context("Computer Use bridge address is required")?;
    let response: Result<Value> = async {
        let mut response = reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10)).build()?
            .post(format!("http://{address}/v1/action"))
            .header("X-Agent-Session", session).json(&payload).send().await?;
        let status = response.status();
        ensure!(status.is_success(), if status.as_u16() == 423 { "Computer Use is paused; resume on the target device" } else { "Computer Use provider rejected the action" });
        let mut bytes = vec![];
        while let Some(chunk) = response.chunk().await? {
            ensure!(bytes.len() + chunk.len() <= MAX_RESPONSE, "Computer Use response exceeds 8 MiB");
            bytes.extend_from_slice(&chunk);
        }
        let mut value: Value = serde_json::from_slice(&bytes)?;
        ensure!(value.is_object(), "invalid Computer Use response");
        ensure!(value.get("error").is_none(), "Computer Use provider reported an action error");
        if payload["action"] == "status" || payload["action"] == "emergency_stop" || payload["action"] == "resume" {
            value = json!({"state":value["state"],"paused":value["paused"],"backend_available":value["backend_available"],"resolution":value["resolution"]});
        }
        if payload["action"] == "screenshot" || payload["action"] == "zoom" {
            validate_image(&value)?;
        }
        Ok(value)
    }.await;
    // Transport errors contain local URLs. Return an actionable, bounded public error.
    response.map_err(|e| {
        if e.downcast_ref::<reqwest::Error>().is_some() {
            anyhow::anyhow!(
                "Computer Use provider unavailable or timed out; the action was not retried"
            )
        } else {
            e
        }
    })
}
pub fn validate_image(value: &Value) -> Result<()> {
    let data = value["data"]
        .as_str()
        .context("provider did not return a PNG screenshot")?;
    ensure!(data.len() <= MAX_RESPONSE - 1024, "screenshot is too large");
    let bytes = STANDARD
        .decode(data)
        .context("invalid screenshot encoding")?;
    ensure!(
        bytes.len() >= 24 && &bytes[..8] == b"\x89PNG\r\n\x1a\n" && &bytes[12..16] == b"IHDR",
        "provider did not return PNG data"
    );
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    ensure!(
        width > 0
            && height > 0
            && width <= 16384
            && height <= 16384
            && u64::from(width) * u64::from(height) <= 64_000_000,
        "screenshot dimensions exceed limits"
    );
    Ok(())
}
impl Store {
    pub async fn computer(&self, node: &str, request: Request) -> Result<Value> {
        request.validate()?;
        let value = self
            .route(node)?
            .api(node)?
            .computer_request(
                &format!("/api/v1/nodes/{node}/computer"),
                serde_json::to_value(&request)?,
            )
            .await?;
        ensure!(
            value["node"] == node,
            "Computer Use response target mismatch"
        );
        if matches!(
            request,
            Request::Action {
                action: Action::Screenshot {} | Action::Zoom { .. },
                ..
            }
        ) {
            validate_image(&value["result"])?;
        }
        Ok(value)
    }
}

/// This MCP process is bound to one explicit target and has no remote resume operation.
/// A screen the Agent may drive: the device's real desktop or a virtual space's.
#[derive(Clone, Debug)]
pub struct Target {
    pub node: String,
    pub label: String,
    /// The Agent's own computer (a virtual space), not the person's real screen.
    pub virtual_space: bool,
}
pub struct Mcp {
    pub store: Store,
    pub targets: Vec<Target>,
}
impl Mcp {
    async fn available(&self, node: &str) -> bool {
        let Ok(api) = self.store.route(node).and_then(|r| r.api(node)) else {
            return false;
        };
        api.probe().await.is_ok_and(|v| {
            v["nodes"].as_array().is_some_and(|nodes| {
                nodes
                    .iter()
                    .any(|n| n["id"] == node && n["computer"] == true)
            })
        })
    }
    async fn live(&self) -> Vec<&Target> {
        let checks = self.targets.iter().map(|t| self.available(&t.node));
        let up = futures_util::future::join_all(checks).await;
        self.targets
            .iter()
            .zip(up)
            .filter_map(|(t, up)| up.then_some(t))
            .collect()
    }
    pub async fn tools(&self) -> Value {
        let live = self.live().await;
        if live.is_empty() {
            return json!([]);
        }
        let mut properties = json!({"operation":{"enum":["status","acquire","renew","release","action","emergency_stop"]},"ttl_secs":{"type":"integer","minimum":1,"maximum":300},"lease":{"type":"string","format":"uuid"},"action":{"type":"object","description":"Typed bridge action: screenshot; zoom {region:[x1,y1,x2,y2]}; cursor_position; window_list; mouse_move/left_click/right_click/middle_click/double_click {coordinate:[x,y],coordinate_space:pixels|normalized}; left_click_drag {start_coordinate,coordinate}; scroll {scroll_direction:up|down|left|right,scroll_amount:1..100,coordinate?}; type/key {text}; window_focus {window_id}. No held inputs, arbitrary URLs or remote resume."}});
        let mut required = vec!["operation"];
        let place = if let [only] = live.as_slice() {
            only.label.clone()
        } else {
            let list = live
                .iter()
                .map(|t| {
                    format!(
                        "{} = {}{}",
                        t.node,
                        t.label,
                        if t.virtual_space {
                            " (your own virtual computer)"
                        } else {
                            " (the person's real screen)"
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            properties["target"] = json!({"enum":live.iter().map(|t| &t.node).collect::<Vec<_>>(),
                "description":format!("Which screen to use: {list}. Prefer your own virtual computer for unattended work; use the real screen only when the task needs the person's apps or sessions.")});
            required.push("target");
            "the selected target".into()
        };
        json!([{"name":"device_computer","description":format!("Computer Use on {place}. Separate grant required. Start with status. acquire returns a short lease required for input; renew or release it explicitly. emergency_stop can only be resumed locally. Input is never retried."),
            "inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}}])
    }
    pub async fn call(&self, parameters: &Value) -> Result<Value> {
        ensure!(parameters["name"] == "device_computer", "unknown tool");
        let mut arguments = parameters["arguments"].clone();
        let target = arguments
            .as_object_mut()
            .and_then(|a| a.remove("target"))
            .map(|t| t.as_str().map(str::to_owned).context("invalid target"))
            .transpose()?;
        let node = match target {
            Some(node) => {
                &self
                    .targets
                    .iter()
                    .find(|t| t.node == node)
                    .context("unknown Computer Use target")?
                    .node
            }
            None => {
                ensure!(self.targets.len() == 1, "choose a Computer Use target");
                &self.targets[0].node
            }
        };
        let request: Request =
            serde_json::from_value(arguments).context("invalid Computer Use command")?;
        let mut value = self.store.computer(node, request).await?;
        let image = value["result"]
            .as_object_mut()
            .and_then(|v| v.remove("data"));
        let mut content = vec![json!({"type":"text","text":serde_json::to_string(&value)?})];
        if let Some(data) = image {
            let screenshot = json!({"data":data});
            validate_image(&screenshot)?;
            content.push(json!({"type":"image","data":screenshot["data"],"mimeType":"image/png"}));
        }
        Ok(json!({"content":content}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commands_are_bounded_and_never_accept_remote_resume_or_destinations() {
        for value in [
            json!({"operation":"resume"}),
            json!({"operation":"status","url":"http://other"}),
            json!({"operation":"action","action":{"action":"hold_key","text":"ctrl"}}),
        ] {
            assert!(serde_json::from_value::<Request>(value).is_err());
        }
        assert!(Request::Acquire { ttl_secs: 301 }.validate().is_err());
        assert!(
            Action::MouseMove {
                coordinate: [1.1, 0.],
                coordinate_space: Coordinates::Normalized
            }
            .validate()
            .is_err()
        );
        assert!(
            Action::Type {
                text: "a".repeat(16385)
            }
            .validate()
            .is_err()
        );
        assert!(
            Provider::http("192.0.2.1:8765".parse().unwrap())
                .validate()
                .is_err()
        );
        assert!(validate_image(&json!({"data":STANDARD.encode(b"not-png")})).is_err());
    }
    #[tokio::test]
    async fn the_agent_names_one_of_the_offered_targets() {
        let dir = tempfile::tempdir().unwrap();
        let target = |node: &str, virtual_space| Target {
            node: node.into(),
            label: node.into(),
            virtual_space,
        };
        let mcp = Mcp {
            store: Store::new(dir.path().join("peers.json")),
            targets: vec![target("desk", false), target("space", true)],
        };
        let call = |arguments: Value| json!({"name":"device_computer","arguments":arguments});
        let missing = mcp.call(&call(json!({"operation":"status"}))).await;
        assert!(format!("{:#}", missing.unwrap_err()).contains("choose"));
        let unknown = mcp
            .call(&call(json!({"operation":"status","target":"elsewhere"})))
            .await;
        assert!(format!("{:#}", unknown.unwrap_err()).contains("unknown Computer Use target"));
        // A known target gets as far as routing (no route in this empty store).
        let routed = mcp
            .call(&call(json!({"operation":"status","target":"space"})))
            .await;
        assert!(!format!("{:#}", routed.unwrap_err()).contains("target"));
        // Nothing reachable: no tool is offered.
        assert_eq!(mcp.tools().await, json!([]));
    }
    #[test]
    fn provider_kind_defaults_to_the_http_bridge_and_keeps_old_bytes() {
        let old = json!({"address":"127.0.0.1:8765"});
        let provider: Provider = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(provider, Provider::http("127.0.0.1:8765".parse().unwrap()));
        assert_eq!(serde_json::to_value(&provider).unwrap(), old);
        let cua: Provider = serde_json::from_value(json!({"kind":"cua"})).unwrap();
        assert_eq!(cua, Provider::cua());
        cua.validate().unwrap();
        assert_eq!(serde_json::to_value(&cua).unwrap(), json!({"kind":"cua"}));
        for invalid in [
            json!({"kind":"cua","address":"127.0.0.1:8765"}),
            json!({"kind":"http"}),
        ] {
            assert!(
                serde_json::from_value::<Provider>(invalid)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
        assert!(serde_json::from_value::<Provider>(json!({"kind":"other"})).is_err());
    }
}
