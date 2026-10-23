//! Computer Use through cua-driver (<https://cua.ai/docs/cua-driver>), an MCP stdio
//! server. In the macOS app the driver is embedded (see [`embedded`]) and uses the
//! app's own grants; elsewhere an installed driver holds the macOS Screen Recording
//! and Accessibility grants. One long-lived session keeps cua-driver's screenshot-to-input coordinate
//! mapping; it is respawned after an exit, but an interrupted input is never replayed.
pub mod embedded;

use super::{MAX_RESPONSE, validate_image};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::Stdio,
    sync::LazyLock,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{Mutex, watch},
};

pub const BINARY: &str = if cfg!(windows) {
    "cua-driver.exe"
} else {
    "cua-driver"
};
/// The reviewed official installer (macOS and Linux).
pub const INSTALL_URL: &str =
    "https://raw.githubusercontent.com/trycua/cua/main/libs/cua-driver/scripts/install.sh";
pub const MAC_APP: &str = "/Applications/CuaDriver.app";
/// Public run label repeated on every call, so screenshot coordinates map to input.
const SESSION: &str = "cranibox";
/// Long-edge cap for screenshots, keeping a PNG within the 8 MiB response limit.
const MAX_IMAGE_DIMENSION: u32 = 1920;
const START_TIMEOUT: Duration = Duration::from_secs(20);
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// One JSON-RPC line may carry a base64 PNG plus its metadata.
const MAX_LINE: usize = MAX_RESPONSE + MAX_RESPONSE / 2;

/// The driver: the macOS app's embedded copy, then PATH, `~/.local/bin` and the
/// CuaDriver app bundle.
pub fn locate() -> Option<PathBuf> {
    if embedded::active()
        && let Some(binary) = embedded::binary().filter(|b| executable(b))
    {
        return Some(binary);
    }
    let home =
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|root| root.join(BINARY))
        .chain(home.map(|h| h.join(".local/bin").join(BINARY)))
        .chain(
            cfg!(target_os = "macos").then(|| Path::new(MAC_APP).join("Contents/MacOS/cua-driver")),
        )
        .find(|path| executable(path))
}
pub fn executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Run a short, bounded driver CLI command. Output is never replayed to a shell.
pub async fn cli(program: &Path, args: &[&str], timeout: Duration) -> Result<(bool, String)> {
    let output = tokio::time::timeout(
        timeout,
        Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("cua-driver did not answer in time")??;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&output.stderr).into_owned();
    }
    text.truncate(64 * 1024);
    Ok((output.status.success(), text))
}
pub async fn version(program: &Path) -> Option<String> {
    let (ok, text) = cli(program, &["--version"], Duration::from_secs(5))
        .await
        .ok()?;
    let line = text.lines().next()?.trim();
    (ok && !line.is_empty() && line.len() <= 128).then(|| {
        line.trim_start_matches("cua-driver")
            .trim()
            .trim_start_matches('v')
            .to_owned()
    })
}
/// Whether the daemon answers (`cua-driver status` exits successfully).
pub async fn daemon_running(program: &Path) -> bool {
    if embedded::active() {
        return embedded::daemon_running();
    }
    cli(program, &["status"], Duration::from_secs(5))
        .await
        .is_ok_and(|(ok, text)| ok && !text.to_ascii_lowercase().contains("not running"))
}
/// Start the daemon in the background. On macOS through LaunchServices, so the
/// Screen Recording and Accessibility grants stay attributed to CuaDriver.app.
pub async fn start_daemon(program: &Path) -> Result<()> {
    if embedded::active() {
        return embedded::start_daemon(program).await;
    }
    let mut command = if cfg!(target_os = "macos") && Path::new(MAC_APP).is_dir() {
        let mut command = Command::new("/usr/bin/open");
        command.args(["-n", "-g", "-a", "CuaDriver", "--args", "serve"]);
        command
    } else {
        let mut command = Command::new(program);
        command.arg("serve");
        command
    };
    // The daemon outlives this request and is owned by the user's session.
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(false);
    let mut child = command.spawn().context("start the cua-driver daemon")?;
    if cfg!(target_os = "macos") {
        child.wait().await?;
    } else {
        tokio::spawn(async move {
            let _ = child.wait().await;
        });
    }
    for _ in 0..20 {
        if daemon_running(program).await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    bail!("cua-driver did not start; run `cua-driver doctor` in your desktop session")
}
pub async fn stop_daemon(program: &Path) -> Result<()> {
    if embedded::active() {
        embedded::stop_daemon();
        return Ok(());
    }
    let (ok, text) = cli(program, &["stop"], Duration::from_secs(10)).await?;
    ensure!(
        ok || text.to_ascii_lowercase().contains("not running"),
        "cua-driver did not stop"
    );
    Ok(())
}
/// `Some((screen_recording, accessibility))` from the daemon; `None` when unknown.
pub async fn permissions(program: &Path) -> Option<(Option<bool>, Option<bool>)> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    if embedded::active() {
        return Some(embedded::permissions());
    }
    let (ok, text) = cli(
        program,
        &["permissions", "status", "--json"],
        Duration::from_secs(10),
    )
    .await
    .ok()?;
    ok.then(|| parse_permissions(&serde_json::from_str(text.trim()).ok()?))?
}
pub fn parse_permissions(value: &Value) -> Option<(Option<bool>, Option<bool>)> {
    let find = |key: &str| -> Option<bool> {
        let direct = value.get(key).or_else(|| value["permissions"].get(key))?;
        direct
            .as_bool()
            .or_else(|| direct["granted"].as_bool())
            .or_else(|| direct.as_str().map(|s| s == "granted"))
    };
    let result = (find("screen_recording"), find("accessibility"));
    (result != (None, None)).then_some(result)
}
/// Raise the macOS prompts, attributed to CuaDriver.app.
pub async fn request_permissions(program: &Path) -> Result<()> {
    ensure!(
        cfg!(target_os = "macos"),
        "Only macOS requires Computer Use permissions"
    );
    if embedded::active() {
        embedded::request_permissions();
        return Ok(());
    }
    // The command waits for a user decision; do not block the request on it.
    let mut child = Command::new(program)
        .args(["permissions", "grant"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(false)
        .spawn()
        .context("request cua-driver permissions")?;
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
    Ok(())
}

struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next: u64,
    tools: BTreeSet<String>,
}
impl Client {
    async fn spawn(program: &Path, args: &[String]) -> Result<Self> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("start cua-driver mcp")?;
        let mut client = Self {
            stdin: child.stdin.take().context("cua-driver stdin")?,
            stdout: BufReader::new(child.stdout.take().context("cua-driver stdout")?),
            child,
            next: 0,
            tools: BTreeSet::new(),
        };
        tokio::time::timeout(START_TIMEOUT, async {
            client
                .request(
                    "initialize",
                    json!({"protocolVersion":"2025-06-18","capabilities":{},
                        "clientInfo":{"name":"cranibox","version":env!("CARGO_PKG_VERSION")}}),
                )
                .await?;
            client
                .send(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
                .await?;
            let mut cursor = Value::Null;
            for _ in 0..16 {
                let params = if cursor.is_null() {
                    json!({})
                } else {
                    json!({"cursor":cursor})
                };
                let page = client.request("tools/list", params).await?;
                for tool in page["tools"].as_array().into_iter().flatten() {
                    if let Some(name) = tool["name"].as_str() {
                        client.tools.insert(name.to_owned());
                    }
                }
                cursor = page["nextCursor"].clone();
                if !cursor.is_string() {
                    break;
                }
            }
            ensure!(!client.tools.is_empty(), "cua-driver listed no tools");
            Ok(())
        })
        .await
        .context("cua-driver mcp did not start in time")??;
        Ok(client)
    }
    async fn send(&mut self, value: &Value) -> Result<()> {
        let mut line = serde_json::to_vec(value)?;
        line.push(b'\n');
        self.stdin.write_all(&line).await?;
        self.stdin.flush().await?;
        Ok(())
    }
    async fn read_line(&mut self) -> Result<Vec<u8>> {
        let mut line = Vec::new();
        loop {
            let available = self.stdout.fill_buf().await?;
            ensure!(!available.is_empty(), "cua-driver exited");
            if let Some(end) = available.iter().position(|b| *b == b'\n') {
                line.extend_from_slice(&available[..end]);
                self.stdout.consume(end + 1);
                return Ok(line);
            }
            let length = available.len();
            line.extend_from_slice(available);
            self.stdout.consume(length);
            ensure!(line.len() <= MAX_LINE, "cua-driver response is too large");
        }
    }
    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next += 1;
        let id = self.next;
        self.send(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await?;
        loop {
            let line = self.read_line().await?;
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let message: Value =
                serde_json::from_slice(&line).context("invalid cua-driver message")?;
            if message.get("method").is_some() {
                // Server requests (ping, sampling, …) get an answer; notifications none.
                if let Some(request) = message
                    .get("id")
                    .filter(|_| message.get("result").is_none())
                {
                    let reply = if message["method"] == "ping" {
                        json!({"jsonrpc":"2.0","id":request,"result":{}})
                    } else {
                        json!({"jsonrpc":"2.0","id":request,"error":{"code":-32601,"message":"not supported"}})
                    };
                    self.send(&reply).await?;
                }
                continue;
            }
            if message["id"] != id {
                continue;
            }
            if let Some(error) = message.get("error") {
                bail!("cua-driver: {}", bounded(&error["message"]));
            }
            return Ok(message["result"].clone());
        }
    }
}
fn bounded(value: &Value) -> String {
    let mut text = value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned);
    if text.len() > 300 {
        let mut end = 300;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

/// Screenshot space of the last capture, and the screen size in points.
#[derive(Clone, Copy, Debug, Default)]
struct Geometry {
    width: u32,
    height: u32,
    points: Option<(f64, f64)>,
}
#[derive(Default)]
struct State {
    client: Option<Client>,
    geometry: Option<Geometry>,
}
type Locator = Box<dyn Fn() -> Option<(PathBuf, Vec<String>)> + Send + Sync>;
pub struct Driver {
    locate: Locator,
    state: Mutex<State>,
    /// Bumped by emergency stop: interrupts an in-flight call and kills the session.
    abort: watch::Sender<u64>,
}
static DRIVER: LazyLock<Driver> = LazyLock::new(|| {
    Driver::new(Box::new(|| {
        let program = locate()?;
        let args = if embedded::active() {
            embedded::mcp_args()?
        } else {
            vec!["mcp".to_owned()]
        };
        Some((program, args))
    }))
});
/// The process-wide driver session for the installed cua-driver.
pub fn driver() -> &'static Driver {
    &DRIVER
}
impl Driver {
    pub fn new(locate: Locator) -> Self {
        Self {
            locate,
            state: Mutex::new(State::default()),
            abort: watch::channel(0).0,
        }
    }
    /// Drop the session at once, cancelling a pending call. The next call respawns.
    pub async fn stop(&self) {
        self.abort.send_modify(|generation| *generation += 1);
        *self.state.lock().await = State::default();
    }
    /// Run one bridge-protocol payload (see [`super::Action::payload`]).
    pub async fn call(&self, payload: Value) -> Result<Value> {
        let action = payload["action"].as_str().unwrap_or_default().to_owned();
        match action.as_str() {
            "emergency_stop" => {
                self.stop().await;
                return Ok(
                    json!({"state":"paused","paused":true,"backend_available":true,"resolution":null}),
                );
            }
            "status" | "resume" => {
                let mut state = self.state.lock().await;
                let available = self.session(&mut state).await.is_ok();
                let resolution = state
                    .geometry
                    .map(|g| json!({"width":g.width,"height":g.height}));
                return Ok(
                    json!({"state":if available {"ready"} else {"backend-unavailable"},
                    "paused":false,"backend_available":available,"resolution":resolution}),
                );
            }
            _ => {}
        }
        let mut abort = self.abort.subscribe();
        let mut state = self.state.lock().await;
        let outcome = tokio::select! {
            result = tokio::time::timeout(CALL_TIMEOUT, act(&mut state, self, &payload)) => Some(result),
            _ = abort.changed() => None,
        };
        // A timed-out or cancelled exchange leaves the stream unsynchronized.
        match outcome {
            Some(Ok(result)) => result,
            Some(Err(_)) => {
                *state = State::default();
                bail!("Computer Use provider timed out; the action was not retried")
            }
            None => {
                *state = State::default();
                bail!("Computer Use was stopped; the action may not have completed")
            }
        }
    }
    async fn session<'a>(&self, state: &'a mut State) -> Result<&'a mut Client> {
        if state
            .client
            .as_mut()
            .is_some_and(|c| c.child.try_wait().ok().flatten().is_some())
        {
            // A respawned driver has no screenshot context: drop the mapping too.
            *state = State::default();
        }
        if state.client.is_none() {
            let (program, args) = (self.locate)()
                .context("cua-driver is not installed; install it from the capabilities page")?;
            if embedded::active() {
                // The app's own daemon serves the embedded session.
                embedded::start_daemon(&program).await?;
            }
            state.client = Some(Client::spawn(&program, &args).await?);
            state.geometry = None;
        }
        Ok(state.client.as_mut().unwrap())
    }
}

/// Choose the first tool the driver lists, so renamed tools fail clearly.
fn pick<'a>(tools: &BTreeSet<String>, names: &[&'a str]) -> Result<&'a str> {
    names
        .iter()
        .find(|name| tools.contains(**name))
        .copied()
        .with_context(|| {
            format!(
                "cua-driver does not provide {}; update cua-driver",
                names[0]
            )
        })
}
async fn tool(
    state: &mut State,
    driver: &Driver,
    names: &[&str],
    mut args: Value,
) -> Result<Value> {
    let client = driver.session(state).await?;
    let name = pick(&client.tools, names)?;
    args["session"] = json!(SESSION);
    let result = match client
        .request("tools/call", json!({"name":name,"arguments":args}))
        .await
    {
        Ok(result) => result,
        Err(error) => {
            // A broken exchange (exit, bad frame) ends the session; nothing is resent.
            *state = State::default();
            return Err(error);
        }
    };
    if result["isError"] == true {
        let text = result["content"]
            .as_array()
            .into_iter()
            .flatten()
            .find_map(|c| c["text"].as_str())
            .map_or_else(
                || bounded(&result["structuredContent"]),
                |t| bounded(&json!(t)),
            );
        bail!("cua-driver {name}: {text}");
    }
    Ok(result)
}
/// Structured output, or the first JSON text block of older drivers.
fn structured(result: &Value) -> Value {
    if result["structuredContent"].is_object() {
        return result["structuredContent"].clone();
    }
    result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["text"].as_str())
        .find_map(|t| {
            serde_json::from_str::<Value>(t)
                .ok()
                .filter(Value::is_object)
        })
        .unwrap_or_else(|| json!({}))
}
fn png_size(bytes: &[u8]) -> Result<(u32, u32)> {
    ensure!(
        bytes.len() >= 24 && &bytes[..8] == b"\x89PNG\r\n\x1a\n" && &bytes[12..16] == b"IHDR",
        "cua-driver did not return PNG data"
    );
    Ok((
        u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
        u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
    ))
}
async fn screenshot(state: &mut State, driver: &Driver) -> Result<String> {
    let result = tool(
        state,
        driver,
        &["get_desktop_state"],
        json!({"max_image_dimension":MAX_IMAGE_DIMENSION}),
    )
    .await?;
    let data = result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|c| c["type"] == "image")
        .context("cua-driver returned no screenshot")?;
    ensure!(
        data["mimeType"].as_str().is_none_or(|m| m == "image/png"),
        "cua-driver returned a non-PNG screenshot"
    );
    let data = data["data"]
        .as_str()
        .context("cua-driver returned no screenshot")?;
    validate_image(&json!({"data":data}))?;
    let (width, height) = png_size(&STANDARD.decode(data)?)?;
    let points = state.geometry.and_then(|g| g.points);
    state.geometry = Some(Geometry {
        width,
        height,
        points,
    });
    Ok(data.to_owned())
}
/// The screenshot space; captures one first, so input coordinates always refer to a
/// screenshot of this driver session.
async fn geometry(state: &mut State, driver: &Driver) -> Result<Geometry> {
    if state.geometry.is_none() {
        screenshot(state, driver).await?;
    }
    Ok(state.geometry.unwrap())
}
/// Screenshot pixels per screen point, for cursor positions reported in points.
async fn scale(state: &mut State, driver: &Driver) -> Result<(f64, f64)> {
    let mut geometry = geometry(state, driver).await?;
    if geometry.points.is_none() {
        let size = structured(&tool(state, driver, &["get_screen_size"], json!({})).await?);
        let size = if size["width"].is_number() {
            size
        } else {
            size["size"].clone()
        };
        let (width, height) = (size["width"].as_f64(), size["height"].as_f64());
        geometry.points = Some(match (width, height) {
            (Some(w), Some(h)) if w > 0. && h > 0. => (w, h),
            _ => (f64::from(geometry.width), f64::from(geometry.height)),
        });
        state.geometry = Some(geometry);
    }
    let (w, h) = geometry.points.unwrap();
    Ok((
        f64::from(geometry.width) / w,
        f64::from(geometry.height) / h,
    ))
}
/// Map a payload point to screenshot pixels (normalized like the HTTP bridge).
fn point(value: &Value, normalized: bool, geometry: Geometry) -> Result<(f64, f64)> {
    let x = value[0].as_f64().context("invalid coordinate")?;
    let y = value[1].as_f64().context("invalid coordinate")?;
    let (x, y) = if normalized {
        (
            (x * f64::from(geometry.width.saturating_sub(1))).round(),
            (y * f64::from(geometry.height.saturating_sub(1))).round(),
        )
    } else {
        (x, y)
    };
    ensure!(
        x < f64::from(geometry.width) && y < f64::from(geometry.height),
        "coordinate is outside the screenshot ({}x{})",
        geometry.width,
        geometry.height
    );
    Ok((x, y))
}
/// `ctrl+shift+t` → hotkey keys, `Return` → press_key key.
pub fn keys(text: &str) -> Vec<String> {
    text.split('+')
        .filter(|k| !k.is_empty())
        .map(|key| {
            let lower = key.trim().to_ascii_lowercase();
            match lower.as_str() {
                "control" => "ctrl".into(),
                "super" | "meta" | "command" | "win" if cfg!(target_os = "macos") => "cmd".into(),
                "option" if !cfg!(target_os = "macos") => "alt".into(),
                "esc" => "escape".into(),
                "enter" => "return".into(),
                "page_up" | "prior" => "pageup".into(),
                "page_down" | "next" => "pagedown".into(),
                _ if key.chars().count() == 1 => key.into(),
                _ => lower,
            }
        })
        .collect()
}
fn ok(result: &Value) -> Value {
    let mut value = structured(result);
    value["result"] = json!("ok");
    value
}
async fn act(state: &mut State, driver: &Driver, payload: &Value) -> Result<Value> {
    let action = payload["action"].as_str().unwrap_or_default();
    let normalized = payload["coordinate_space"] == "normalized";
    let desktop = || json!({"scope":"desktop"});
    match action {
        "screenshot" => {
            let data = screenshot(state, driver).await?;
            let g = state.geometry.unwrap();
            Ok(json!({"result":"ok","data":data,"width":g.width,"height":g.height}))
        }
        "zoom" => {
            let data = screenshot(state, driver).await?;
            let region: Vec<u32> = payload["region"]
                .as_array()
                .context("invalid region")?
                .iter()
                .map(|v| v.as_f64().map(|v| v.round() as u32))
                .collect::<Option<_>>()
                .context("invalid region")?;
            let cropped = crop(&STANDARD.decode(data)?, &region)?;
            let data = STANDARD.encode(cropped);
            validate_image(&json!({"data":data}))?;
            Ok(json!({"result":"ok","data":data,"region":region}))
        }
        "cursor_position" => {
            let (sx, sy) = scale(state, driver).await?;
            let position =
                structured(&tool(state, driver, &["get_cursor_position"], json!({})).await?);
            let (x, y) = (
                position["x"]
                    .as_f64()
                    .context("cua-driver returned no cursor position")?,
                position["y"]
                    .as_f64()
                    .context("cua-driver returned no cursor position")?,
            );
            Ok(
                json!({"result":"ok","coordinate":[(x * sx).round() as i64, (y * sy).round() as i64]}),
            )
        }
        "window_list" => {
            let listed = structured(
                &tool(
                    state,
                    driver,
                    &["list_windows"],
                    json!({"on_screen_only":true}),
                )
                .await?,
            );
            let windows: Vec<Value> = listed["windows"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|w| {
                    let id = w["window_id"].as_u64()?;
                    Some(json!({"id":id.to_string(),"title":w["title"],"app":w["app_name"],"pid":w["pid"]}))
                })
                .take(512)
                .collect();
            Ok(json!({"result":"ok","windows":windows,"backend":"cua-driver"}))
        }
        "window_focus" => {
            let id: u64 = payload["window_id"]
                .as_str()
                .and_then(|v| v.parse().ok())
                .context("window ID must come from window_list")?;
            let listed = structured(&tool(state, driver, &["list_windows"], json!({})).await?);
            let pid = listed["windows"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|w| w["window_id"].as_u64() == Some(id))
                .and_then(|w| w["pid"].as_u64())
                .context("window is no longer listed; call window_list again")?;
            let result = tool(
                state,
                driver,
                &["bring_to_front"],
                json!({"pid":pid,"window_id":id}),
            )
            .await?;
            Ok(ok(&result))
        }
        "mouse_move" => {
            let g = geometry(state, driver).await?;
            let (x, y) = point(&payload["coordinate"], normalized, g)?;
            let mut args = desktop();
            args["x"] = json!(x);
            args["y"] = json!(y);
            // Moving the real pointer is the requested effect, not a fallback.
            args["delivery_mode"] = json!("foreground");
            Ok(ok(&tool(state, driver, &["move_cursor"], args).await?))
        }
        "left_click" | "right_click" | "middle_click" | "double_click" => {
            let g = geometry(state, driver).await?;
            let (x, y) = if payload.get("coordinate").is_some() {
                point(&payload["coordinate"], normalized, g)?
            } else {
                let (sx, sy) = scale(state, driver).await?;
                let position =
                    structured(&tool(state, driver, &["get_cursor_position"], json!({})).await?);
                (
                    (position["x"]
                        .as_f64()
                        .context("cursor position unavailable")?
                        * sx)
                        .round(),
                    (position["y"]
                        .as_f64()
                        .context("cursor position unavailable")?
                        * sy)
                        .round(),
                )
            };
            let mut args = desktop();
            args["x"] = json!(x);
            args["y"] = json!(y);
            let name: &[&str] = match action {
                "right_click" => &["right_click"],
                "double_click" => &["double_click"],
                _ => &["click"],
            };
            if action == "middle_click" {
                args["button"] = json!("middle");
            }
            Ok(ok(&tool(state, driver, name, args).await?))
        }
        "left_click_drag" => {
            let g = geometry(state, driver).await?;
            let (from_x, from_y) = point(&payload["start_coordinate"], normalized, g)?;
            let (to_x, to_y) = point(&payload["coordinate"], normalized, g)?;
            let mut args = desktop();
            for (key, value) in [
                ("from_x", from_x),
                ("from_y", from_y),
                ("to_x", to_x),
                ("to_y", to_y),
            ] {
                args[key] = json!(value);
            }
            Ok(ok(&tool(state, driver, &["drag"], args).await?))
        }
        "scroll" => {
            let mut args = desktop();
            args["direction"] = payload["scroll_direction"].clone();
            args["amount"] = json!(payload["scroll_amount"].as_u64().unwrap_or(3).clamp(1, 50));
            if payload.get("coordinate").is_some() {
                let g = geometry(state, driver).await?;
                let (x, y) = point(&payload["coordinate"], normalized, g)?;
                args["x"] = json!(x);
                args["y"] = json!(y);
            }
            Ok(ok(&tool(state, driver, &["scroll"], args).await?))
        }
        "type" => {
            let mut args = desktop();
            args["text"] = payload["text"].clone();
            Ok(ok(&tool(state, driver, &["type_text"], args).await?))
        }
        "key" => {
            let keys = keys(payload["text"].as_str().unwrap_or_default());
            ensure!(!keys.is_empty(), "invalid key combination");
            let mut args = desktop();
            let name: &[&str] = if keys.len() == 1 {
                args["key"] = json!(keys[0]);
                &["press_key"]
            } else {
                args["keys"] = json!(keys);
                &["hotkey"]
            };
            Ok(ok(&tool(state, driver, name, args).await?))
        }
        _ => bail!("unsupported Computer Use action"),
    }
}

/// Crop an 8-bit PNG to `[x1, y1, x2, y2]` (exclusive end), clamped to the image.
fn crop(bytes: &[u8], region: &[u32]) -> Result<Vec<u8>> {
    ensure!(region.len() == 4, "invalid region");
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().context("invalid screenshot")?;
    let mut buffer = vec![
        0;
        reader
            .output_buffer_size()
            .context("screenshot too large")?
    ];
    let info = reader
        .next_frame(&mut buffer)
        .context("invalid screenshot")?;
    let channels = info.color_type.samples();
    let (x1, y1) = (region[0].min(info.width), region[1].min(info.height));
    let (x2, y2) = (region[2].min(info.width), region[3].min(info.height));
    ensure!(x1 < x2 && y1 < y2, "region is outside the screenshot");
    let (width, height) = (x2 - x1, y2 - y1);
    let mut pixels = Vec::with_capacity(width as usize * height as usize * channels);
    for row in y1..y2 {
        let start = row as usize * info.line_size + x1 as usize * channels;
        pixels.extend_from_slice(&buffer[start..start + width as usize * channels]);
    }
    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, width, height);
    encoder.set_color(info.color_type);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&pixels)?;
    Ok(output)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::peers::computer::Action;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut output = Vec::new();
        let mut encoder = png::Encoder::new(&mut output, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let pixels: Vec<u8> = (0..width * height * 3).map(|v| v as u8).collect();
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        output
    }
    /// A fake `cua-driver mcp`: answers the handshake, logs each tools/call line, and
    /// returns a fixed screenshot, cursor and window list. Tool `die` exits mid-call.
    fn fake(root: &Path) -> (PathBuf, PathBuf) {
        let log = root.join("calls.log");
        let image = STANDARD.encode(png(200, 100));
        let log_path = log.display();
        let script = root.join("fake-cua-driver");
        std::fs::write(&script, format!(r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*) printf '{{"jsonrpc":"2.0","id":%s,"result":{{"protocolVersion":"2025-06-18","capabilities":{{"tools":{{}}}},"serverInfo":{{"name":"fake","version":"1"}}}}}}\n' "$id";;
    *'"method":"notifications/initialized"'*) printf '{{"jsonrpc":"2.0","method":"notifications/message","params":{{}}}}\n';;
    *'"method":"tools/list"'*) printf '{{"jsonrpc":"2.0","id":%s,"result":{{"tools":[{{"name":"get_desktop_state"}},{{"name":"get_screen_size"}},{{"name":"get_cursor_position"}},{{"name":"list_windows"}},{{"name":"bring_to_front"}},{{"name":"move_cursor"}},{{"name":"click"}},{{"name":"double_click"}},{{"name":"right_click"}},{{"name":"drag"}},{{"name":"scroll"}},{{"name":"type_text"}},{{"name":"press_key"}},{{"name":"hotkey"}},{{"name":"die"}}]}}}}\n' "$id";;
    *'"method":"tools/call"'*)
      printf '%s\n' "$line" >> '{log_path}'
      case "$line" in
        *'"name":"die"'*) exit 3;;
        *'"name":"get_desktop_state"'*) printf '{{"jsonrpc":"2.0","id":%s,"result":{{"content":[{{"type":"image","mimeType":"image/png","data":"{image}"}}]}}}}\n' "$id";;
        *'"name":"get_screen_size"'*) printf '{{"jsonrpc":"2.0","id":%s,"result":{{"content":[],"structuredContent":{{"width":100,"height":50,"scale_factor":2.0}}}}}}\n' "$id";;
        *'"name":"get_cursor_position"'*) printf '{{"jsonrpc":"2.0","id":%s,"result":{{"content":[{{"type":"text","text":"{{\\"x\\":10,\\"y\\":20}}"}}]}}}}\n' "$id";;
        *'"name":"list_windows"'*) printf '{{"jsonrpc":"2.0","id":%s,"result":{{"content":[],"structuredContent":{{"windows":[{{"window_id":42,"pid":7,"app_name":"Notes","title":"Todo"}}]}}}}}}\n' "$id";;
        *'"text":"hang"'*) sleep 30;;
        *'"text":"fail"'*) printf '{{"jsonrpc":"2.0","id":%s,"result":{{"isError":true,"content":[{{"type":"text","text":"no focused field"}}]}}}}\n' "$id";;
        *) printf '{{"jsonrpc":"2.0","id":%s,"result":{{"content":[{{"type":"text","text":"done"}}],"structuredContent":{{"effect":"confirmed"}}}}}}\n' "$id";;
      esac;;
  esac
done
"#)).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        (script, log)
    }
    fn calls(log: &Path) -> Vec<(String, Value)> {
        std::fs::read_to_string(log)
            .unwrap_or_default()
            .lines()
            .map(|line| {
                let value: Value = serde_json::from_str(line).unwrap();
                (
                    value["params"]["name"].as_str().unwrap().to_owned(),
                    value["params"]["arguments"].clone(),
                )
            })
            .collect()
    }
    fn action(value: Value) -> Value {
        serde_json::from_value::<Action>(value)
            .unwrap()
            .payload()
            .unwrap()
    }

    #[tokio::test]
    async fn bridge_actions_map_to_cua_driver_tools() {
        let root = tempfile::tempdir().unwrap();
        let (script, log) = fake(root.path());
        let driver = Driver::new(Box::new(move || Some((script.clone(), vec![]))));
        let status = driver.call(json!({"action":"status"})).await.unwrap();
        assert_eq!(status["backend_available"], true);
        assert_eq!(status["paused"], false);

        // Input before any screenshot first captures one, defining the pixel space.
        let clicked = driver
            .call(action(json!({"action":"left_click","coordinate":[0.5,1.0],"coordinate_space":"normalized"})))
            .await
            .unwrap();
        assert_eq!(clicked["result"], "ok");
        let shot = driver.call(json!({"action":"screenshot"})).await.unwrap();
        validate_image(&shot).unwrap();
        assert_eq!(
            (shot["width"].as_u64(), shot["height"].as_u64()),
            (Some(200), Some(100))
        );
        let zoom = driver
            .call(action(json!({"action":"zoom","region":[10,10,60,30]})))
            .await
            .unwrap();
        assert_eq!(
            png_size(&STANDARD.decode(zoom["data"].as_str().unwrap()).unwrap()).unwrap(),
            (50, 20)
        );
        // Cursor positions in points scale to screenshot pixels (200 px / 100 pt).
        let cursor = driver
            .call(json!({"action":"cursor_position"}))
            .await
            .unwrap();
        assert_eq!(cursor["coordinate"], json!([20, 40]));
        for value in [
            json!({"action":"mouse_move","coordinate":[5,6]}),
            json!({"action":"right_click"}),
            json!({"action":"middle_click","coordinate":[1,2]}),
            json!({"action":"double_click","coordinate":[3,4]}),
            json!({"action":"left_click_drag","start_coordinate":[1,2],"coordinate":[30,40]}),
            json!({"action":"scroll","scroll_direction":"down","scroll_amount":80,"coordinate":[7,8]}),
            json!({"action":"type","text":"héllo"}),
            json!({"action":"key","text":"Return"}),
            json!({"action":"key","text":"ctrl+shift+t"}),
            json!({"action":"window_focus","window_id":"42"}),
        ] {
            driver.call(action(value)).await.unwrap();
        }
        let windows = driver.call(json!({"action":"window_list"})).await.unwrap();
        assert_eq!(windows["windows"][0]["id"], "42");
        // Coordinates outside the screenshot never reach the driver.
        assert!(
            driver
                .call(action(json!({"action":"left_click","coordinate":[200,5]})))
                .await
                .is_err()
        );
        // Driver tool errors are reported, not hidden.
        let error = driver
            .call(action(json!({"action":"type","text":"fail"})))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("no focused field"), "{error}");

        let calls = calls(&log);
        let names: Vec<&str> = calls.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                "get_desktop_state",
                "click",
                "get_desktop_state",
                "get_desktop_state",
                "get_screen_size",
                "get_cursor_position",
                "move_cursor",
                "get_cursor_position",
                "right_click",
                "click",
                "double_click",
                "drag",
                "scroll",
                "type_text",
                "press_key",
                "hotkey",
                "list_windows",
                "bring_to_front",
                "list_windows",
                "type_text",
            ]
        );
        let args = |i: usize| &calls[i].1;
        assert!(calls.iter().all(|(_, a)| a["session"] == SESSION));
        assert_eq!(args(0)["max_image_dimension"], MAX_IMAGE_DIMENSION);
        // Normalized [0.5, 1.0] of 200x100 → (100, 99), like the HTTP bridge.
        assert_eq!(
            (args(1)["x"].as_f64(), args(1)["y"].as_f64()),
            (Some(100.), Some(99.))
        );
        assert_eq!(args(1)["scope"], "desktop");
        assert_eq!(args(6)["delivery_mode"], "foreground");
        assert_eq!(
            (args(8)["x"].as_f64(), args(8)["y"].as_f64()),
            (Some(20.), Some(40.))
        );
        assert_eq!(args(9)["button"], "middle");
        assert_eq!(
            (args(11)["from_x"].as_f64(), args(11)["to_y"].as_f64()),
            (Some(1.), Some(40.))
        );
        assert_eq!(
            (args(12)["direction"].as_str(), args(12)["amount"].as_u64()),
            (Some("down"), Some(50))
        );
        assert_eq!(args(13)["text"], "héllo");
        assert_eq!(args(14)["key"], "return");
        assert_eq!(args(15)["keys"], json!(["ctrl", "shift", "t"]));
        assert_eq!(
            (args(17)["pid"].as_u64(), args(17)["window_id"].as_u64()),
            (Some(7), Some(42))
        );
    }

    #[tokio::test]
    async fn exits_respawn_without_replaying_input_and_stop_interrupts() {
        let root = tempfile::tempdir().unwrap();
        let (script, log) = fake(root.path());
        let driver = Driver::new(Box::new(move || Some((script.clone(), vec![]))));
        driver.call(json!({"action":"screenshot"})).await.unwrap();
        // `die` is not a bridge action; inject it directly to simulate a crash mid-input.
        let mut state = driver.state.lock().await;
        tool(&mut state, &driver, &["die"], json!({}))
            .await
            .unwrap_err();
        assert!(state.client.is_none() && state.geometry.is_none());
        drop(state);
        // The next input respawns and takes a new screenshot before clicking.
        driver
            .call(action(json!({"action":"left_click","coordinate":[1,1]})))
            .await
            .unwrap();
        let names: Vec<String> = calls(&log).into_iter().map(|(n, _)| n).collect();
        assert_eq!(
            names,
            ["get_desktop_state", "die", "get_desktop_state", "click"]
        );
        assert_eq!(names.iter().filter(|n| *n == "die").count(), 1);

        // Emergency stop interrupts an in-flight input at once.
        let started = std::time::Instant::now();
        let (hung, stopped) = tokio::join!(
            driver.call(action(json!({"action":"type","text":"hang"}))),
            async {
                tokio::time::sleep(Duration::from_millis(300)).await;
                driver.call(json!({"action":"emergency_stop"})).await
            }
        );
        assert!(hung.unwrap_err().to_string().contains("stopped"));
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(stopped.unwrap()["paused"], true);
        let stopped = driver
            .call(json!({"action":"emergency_stop"}))
            .await
            .unwrap();
        assert_eq!(stopped["paused"], true);
        assert!(driver.state.lock().await.client.is_none());
        let missing = Driver::new(Box::new(|| None));
        let status = missing.call(json!({"action":"status"})).await.unwrap();
        assert_eq!(status["backend_available"], false);
        assert!(missing.call(json!({"action":"screenshot"})).await.is_err());
    }

    #[test]
    fn keys_and_permissions_parse() {
        assert_eq!(keys("Return"), ["return"]);
        assert_eq!(keys("ctrl+C"), ["ctrl", "C"]);
        assert_eq!(keys("Control+Page_Down"), ["ctrl", "pagedown"]);
        assert_eq!(
            parse_permissions(&json!({"accessibility":true,"screen_recording":false})),
            Some((Some(false), Some(true)))
        );
        assert_eq!(
            parse_permissions(&json!({"permissions":{"accessibility":{"granted":true}}})),
            Some((None, Some(true)))
        );
        assert_eq!(parse_permissions(&json!({"other":1})), None);
    }
}
