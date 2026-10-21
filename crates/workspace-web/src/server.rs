use agent_workspace_runtime::journal::Journal;
use anyhow::{Context, Result, ensure};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::{
    Json, Router,
    body::to_bytes,
    extract::{Query, Request, State as ExtractState, WebSocketUpgrade},
    http::{HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::any,
};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;
use tokio::{
    io::AsyncReadExt,
    process::Command,
    sync::{Mutex, oneshot, watch},
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub port: u16,
    pub public_url: String,
    pub workspace_url: String,
    pub desktop_url: String,
    pub applications_url: String,
    pub runtime_url: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            port: 9080,
            public_url: String::new(),
            workspace_url: "http://localhost:8443".into(),
            desktop_url: "https://localhost:3001".into(),
            applications_url: "https://localhost:3002".into(),
            runtime_url: "http://127.0.0.1:9090".into(),
        }
    }
}
impl Config {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join(".config/agent-workspace/web.json");
        if path.exists() {
            Ok(serde_json::from_slice(&std::fs::read(path)?)?)
        } else {
            Ok(Self::default())
        }
    }
    pub fn validate(&self) -> Result<()> {
        let runtime = url::Url::parse(&self.runtime_url)?;
        ensure!(
            runtime.scheme() == "http"
                && runtime.host_str().is_some_and(|h| h == "localhost"
                    || h.parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback()))
                && runtime.username().is_empty()
                && runtime.password().is_none()
                && runtime.path() == "/"
                && runtime.query().is_none()
                && runtime.fragment().is_none(),
            "runtimeUrl must be a loopback HTTP origin"
        );
        for (text, public) in [
            (&self.public_url, true),
            (&self.workspace_url, false),
            (&self.desktop_url, false),
            (&self.applications_url, true),
        ] {
            if public && text.is_empty() {
                continue;
            }
            let url = url::Url::parse(text)?;
            ensure!(
                matches!(url.scheme(), "http" | "https")
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none(),
                "Use HTTP(S) URLs without credentials"
            );
            ensure!(
                url.query().is_none() && url.fragment().is_none(),
                "URLs must not contain query or fragment"
            );
            if public {
                ensure!(url.path() == "/", "publicUrl must use a dedicated origin");
            }
        }
        let application = url::Url::parse(&self.applications_url)?;
        for text in [&self.public_url, &self.workspace_url, &self.desktop_url] {
            if !text.is_empty() {
                ensure!(
                    url::Url::parse(text)?.origin() != application.origin(),
                    "applicationsUrl must use a separate origin from the console, editor and host desktop"
                );
            }
        }
        Ok(())
    }
}
pub fn token() -> String {
    let bytes: [u8; 32] = rand::rng().random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub type SessionRef = Arc<Mutex<Session>>;
pub struct Prompt {
    pub view: Value,
    pub validation: Validation,
    pub sender: oneshot::Sender<Option<Value>>,
}
pub enum Validation {
    Text { kind: TextKind },
    Pick { items: Vec<Value>, many: bool },
    Confirm { choices: Vec<String> },
}
#[derive(Clone, Copy)]
pub enum TextKind {
    Name,
    Source,
    Target,
    Domain,
    Url,
}
impl Validation {
    fn validate(&self, value: Value) -> Result<Value> {
        match self {
            Self::Text { kind } => {
                let text = value.as_str().context("Enter text")?.trim();
                ensure!(
                    !text.is_empty() && !text.starts_with('-'),
                    "Enter a valid value"
                );
                match kind {
                    TextKind::Name => ensure!(
                        crate::actions::valid_name(text),
                        "Use letters, numbers, dot, underscore, @ or dash"
                    ),
                    TextKind::Source => {}
                    TextKind::Target => {
                        let (host, port) = text.rsplit_once(':').context("Use host:port")?;
                        ensure!(
                            !host.is_empty()
                                && !host.contains(char::is_whitespace)
                                && !host.contains(':')
                                && port.parse::<u16>().is_ok_and(|p| p > 0),
                            "Use host:port with a valid port"
                        );
                    }
                    TextKind::Domain => {
                        ensure!(crate::actions::valid_domain(text), "Enter a valid domain")
                    }
                    TextKind::Url => {
                        let url = url::Url::parse(text)?;
                        ensure!(
                            matches!(url.scheme(), "http" | "https")
                                && url.host_str().is_some()
                                && url.username().is_empty()
                                && url.password().is_none(),
                            "Enter an HTTP(S) URL without credentials"
                        );
                    }
                }
                Ok(json!(text))
            }
            Self::Pick { items, many } => {
                if *many {
                    let values = value.as_array().context("Choose valid items")?;
                    let mut selected = Vec::new();
                    let mut seen = HashSet::new();
                    for v in values {
                        let i = v.as_u64().context("Choose valid items")? as usize;
                        ensure!(i < items.len(), "Choose valid items");
                        if seen.insert(i) {
                            selected.push(items[i].clone());
                        }
                    }
                    Ok(json!(selected))
                } else {
                    let i = value.as_u64().context("Choose an item")? as usize;
                    Ok(items.get(i).context("Choose an item")?.clone())
                }
            }
            Self::Confirm { choices } => {
                ensure!(
                    value
                        .as_str()
                        .is_some_and(|v| choices.iter().any(|c| c == v)),
                    "Invalid choice"
                );
                Ok(value)
            }
        }
    }
}
pub struct Session {
    pub id: String,
    pub csrf: String,
    pub seq: u64,
    pub events: Vec<Value>,
    pub output: String,
    pub output_end: u64,
    pub changed: watch::Sender<u64>,
    pub streams: Arc<std::sync::atomic::AtomicUsize>,
    pub seen: Instant,
    pub busy: bool,
    pub force_welcome: bool,
    pub prompt: Option<Prompt>,
    pub cancel: watch::Sender<bool>,
}
impl Session {
    fn new() -> Self {
        let (cancel, _) = watch::channel(false);
        Self {
            id: token(),
            csrf: token(),
            seq: 0,
            events: Vec::new(),
            output: String::new(),
            output_end: 0,
            changed: watch::channel(0).0,
            streams: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            seen: Instant::now(),
            busy: false,
            force_welcome: false,
            prompt: None,
            cancel,
        }
    }
    pub fn touch(&self) {
        self.changed.send_modify(|v| *v += 1);
    }
    pub fn emit(&mut self, mut value: Value) {
        self.seq += 1;
        value["seq"] = json!(self.seq);
        self.events.push(value);
        self.touch();
        if self.events.len() > 256 {
            self.events.remove(0);
        }
    }
    pub fn append(&mut self, text: &str) {
        self.output.push_str(text);
        self.output_end += text.len() as u64;
        self.touch();
        if self.output.len() > 128 * 1024 {
            let mut offset = self.output.len() - 128 * 1024;
            while !self.output.is_char_boundary(offset) {
                offset += 1;
            }
            self.output.drain(..offset);
        }
    }
}
/// Workspace browser sessions as seen by the shared device routes.
struct WebSessions(Arc<Mutex<HashMap<String, SessionRef>>>);
#[async_trait::async_trait]
impl cranibox_node::web::SessionStore for WebSessions {
    async fn session(&self, headers: &HeaderMap) -> Option<String> {
        let cookie = headers.get("cookie")?.to_str().ok()?;
        let id = cookie
            .split(';')
            .map(str::trim)
            .find_map(|s| s.strip_prefix("aw_session="))?;
        let session = self.0.lock().await.get(id).cloned()?;
        session.lock().await.seen = Instant::now();
        Some(id.to_owned())
    }
    async fn csrf(&self, session: &str) -> Option<String> {
        let session = self.0.lock().await.get(session).cloned()?;
        Some(session.lock().await.csrf.clone())
    }
    async fn alive(&self, session: &str) -> bool {
        self.0.lock().await.contains_key(session)
    }
}
#[derive(Default)]
struct Snapshot {
    value: Option<Value>,
    at: u64,
    refreshing: bool,
    error: String,
}
#[derive(Clone)]
pub struct State {
    pub root: PathBuf,
    pub command: PathBuf,
    pub config: Config,
    pub remote_client: reqwest::Client,
    pub gateway: Option<agent_workspace_runtime::server::Gateway>,
    pub local: Option<Arc<cranibox_node::local::LocalNode>>,
    sessions: Arc<Mutex<HashMap<String, SessionRef>>>,
    snapshot: Arc<Mutex<Snapshot>>,
    remote_refresh: Arc<Mutex<()>>,
    hosts: HashSet<String>,
    origins: HashSet<String>,
    pub(crate) closing: watch::Sender<bool>,
    /// Device routes shared with the CLI node and desktop window.
    pub(crate) services: cranibox_node::web::Services,
    pub(crate) applications: cranibox_node::spaces::applications::Hub,
    history: Arc<Mutex<Journal>>,
    metrics: Arc<Mutex<crate::metrics::Metrics>>,
    changed: watch::Sender<u64>,
}
impl State {
    pub(crate) fn console_origin(&self, origin: &str) -> bool {
        self.origins.contains(origin)
    }
    pub fn new(root: PathBuf, command: PathBuf, config: Config) -> Result<Self> {
        let history = Journal::open(root.join(".local/state/cranibox/web-history"))?;
        let mut origins = HashSet::from([
            format!("http://127.0.0.1:{}", config.port),
            format!("http://localhost:{}", config.port),
            format!("http://[::1]:{}", config.port),
        ]);
        if !config.public_url.is_empty() {
            origins.insert(
                url::Url::parse(&config.public_url)
                    .unwrap()
                    .origin()
                    .ascii_serialization(),
            );
        }
        let mut hosts = HashSet::new();
        for origin in &origins {
            let u = url::Url::parse(origin).unwrap();
            hosts.insert(u[url::Position::BeforeHost..url::Position::AfterPort].to_owned());
        }
        if !config.public_url.is_empty() {
            hosts.insert(
                url::Url::parse(&config.public_url)
                    .unwrap()
                    .host_str()
                    .unwrap()
                    .to_owned(),
            );
        }
        let (closing, _) = watch::channel(false);
        let sessions: Arc<Mutex<HashMap<String, SessionRef>>> = Arc::default();
        let services = cranibox_node::web::Services::new(
            None,
            root.join(".config/cranibox-node/peers.json"),
            Arc::new(WebSessions(sessions.clone())),
            closing.subscribe(),
        );
        Ok(Self {
            gateway: None,
            local: None,
            services,
            applications: Default::default(),
            remote_client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(4))
                .build()?,
            history: Arc::new(Mutex::new(history)),
            metrics: Arc::default(),
            changed: watch::channel(0).0,
            root,
            command,
            config,
            sessions,
            snapshot: Arc::default(),
            remote_refresh: Arc::default(),
            hosts,
            origins,
            closing,
        })
    }
    /// Route device operations to the started node and, if enrolled, the legacy gateway.
    pub fn attach_node(&mut self) {
        let mut services = cranibox_node::web::Services::new(
            None,
            self.root.join(".config/cranibox-node/peers.json"),
            Arc::new(WebSessions(self.sessions.clone())),
            self.closing.subscribe(),
        )
        .with_relay(Arc::new(crate::gateway_relay::GatewayRelay {
            gateway: self.gateway.clone(),
            runtime_url: self.config.runtime_url.clone(),
            root: self.root.clone(),
        }));
        if let Some(local) = &self.local {
            services.set_local(local.clone());
        }
        self.services = services;
    }
    pub(crate) async fn session(&self, headers: &HeaderMap) -> Option<SessionRef> {
        let cookie = headers.get("cookie")?.to_str().ok()?;
        let id = cookie
            .split(';')
            .map(str::trim)
            .find_map(|s| s.strip_prefix("aw_session="))?;
        let s = self.sessions.lock().await.get(id).cloned()?;
        s.lock().await.seen = Instant::now();
        Some(s)
    }
    pub(crate) async fn has_session(&self, id: &str) -> bool {
        self.sessions.lock().await.contains_key(id)
    }
    async fn new_session(&self) -> Result<SessionRef> {
        let mut sessions = self.sessions.lock().await;
        if sessions.len() >= 64 {
            // Closed browsers must not exhaust the bounded session table. A live
            // request, event stream or WebSocket retains its SessionRef; pending
            // operations and prompts must also survive a browser disconnect.
            let mut oldest = None;
            for (id, session) in sessions.iter() {
                if Arc::strong_count(session) != 1 {
                    continue;
                }
                let session = session.lock().await;
                if !session.busy
                    && session.prompt.is_none()
                    && session.streams.load(std::sync::atomic::Ordering::Relaxed) == 0
                    && oldest.as_ref().is_none_or(|(_, seen)| session.seen < *seen)
                {
                    oldest = Some((id.clone(), session.seen));
                }
            }
            if let Some((id, _)) = oldest {
                sessions.remove(&id);
            }
        }
        ensure!(sessions.len() < 64, "Too many sessions");
        let s = Session::new();
        let id = s.id.clone();
        let s = Arc::new(Mutex::new(s));
        sessions.insert(id, s.clone());
        Ok(s)
    }
    pub fn allow_loopback_port(&mut self, port: u16) {
        for host in ["127.0.0.1", "localhost", "[::1]"] {
            self.hosts.insert(format!("{host}:{port}"));
            self.origins.insert(format!("http://{host}:{port}"));
        }
    }
    pub async fn maintenance(&self) {
        if let Some(gateway) = &self.gateway {
            gateway.maintenance().await;
        }
        if self.history.lock().await.flush().is_err() {
            eprintln!("operation history flush failed");
        }
        self.cleanup().await;
        let owners = self.sessions.lock().await.keys().cloned().collect();
        self.applications.retain(&owners).await;
    }
    pub async fn cleanup(&self) {
        let mut sessions = self.sessions.lock().await;
        let mut expired = Vec::new();
        for (id, s) in sessions.iter() {
            if Arc::strong_count(s) != 1 {
                continue;
            }
            let s = s.lock().await;
            if !s.busy
                && s.prompt.is_none()
                && s.streams.load(std::sync::atomic::Ordering::Relaxed) == 0
                && s.seen.elapsed() > Duration::from_secs(12 * 3600)
            {
                expired.push(id.clone());
            }
        }
        for id in expired {
            sessions.remove(&id);
        }
    }
    pub async fn shutdown(&self) {
        self.closing.send_replace(true);
        self.applications.shutdown();
        if let Some(gateway) = &self.gateway {
            gateway.shutdown().await;
        }
        if let Some(local) = &self.local {
            let _ = local.shutdown().await;
        }
        for s in self.sessions.lock().await.values() {
            let mut s = s.lock().await;
            s.cancel.send_replace(true);
            s.touch();
            if let Some(prompt) = s.prompt.take() {
                let _ = prompt.sender.send(None);
            }
        }
    }
    pub async fn snapshot(&self) -> Option<Value> {
        self.snapshot.lock().await.value.clone()
    }
    pub(crate) fn track_nodes(&self) -> Option<tokio::task::JoinHandle<()>> {
        let mut changes = self.gateway.as_ref()?.subscribe_nodes();
        let state = self.clone();
        let mut closing = self.closing.subscribe();
        Some(tokio::spawn(async move {
            while !*closing.borrow() {
                tokio::select! {
                    result = changes.changed() => { if result.is_err() { break; } }
                    _ = closing.changed() => { break; }
                }
                // Serialize with full collections so a slower status command cannot
                // overwrite a device connection update with an older snapshot.
                let _update = state.remote_refresh.lock().await;
                let remote = crate::remote::snapshot(&state).await;
                let mut cache = state.snapshot.lock().await;
                if let Some(value) = &mut cache.value
                    && value["remote_nodes"] != remote
                {
                    value["remote_nodes"] = remote;
                    cache.at = millis().max(cache.at.saturating_add(1));
                    state.changed.send_modify(|revision| *revision += 1);
                }
            }
        }))
    }
    async fn refresh(&self, force: bool) {
        let mut cache = self.snapshot.lock().await;
        if cache.refreshing
            || (!force && cache.value.is_some() && millis().saturating_sub(cache.at) < 15000)
        {
            self.metrics.lock().await.cache_hits += 1;
            return;
        }
        cache.refreshing = true;
        self.changed.send_modify(|v| *v += 1);
        drop(cache);
        let state = self.clone();
        tokio::spawn(async move {
            let started = Instant::now();
            let result = state.collect().await;
            let _update = state.remote_refresh.lock().await;
            let remote = crate::remote::snapshot(&state).await;
            let mut metrics = state.metrics.lock().await;
            metrics.collections += 1;
            metrics.collection_ms = started.elapsed().as_millis() as u64;
            metrics.maximum_ms = metrics.maximum_ms.max(metrics.collection_ms);
            let sample = metrics.sample(state.gateway.is_some());
            drop(metrics);
            let mut cache = state.snapshot.lock().await;
            cache.refreshing = false;
            match result {
                Ok(mut value) => {
                    value["remote_nodes"] = remote;
                    if let Some(local) = &state.local {
                        value["local_node"] =
                            json!({"id":local.id,"online":true,"terminal_sessions":true});
                    }
                    value["metrics"] = sample;
                    cache.value = Some(value);
                    cache.at = millis().max(cache.at.saturating_add(1));
                    cache.error.clear();
                }
                Err(error) => cache.error = format!("{error:#}"),
            }
            state.changed.send_modify(|v| *v += 1);
        });
    }
    fn child(&self, args: &[String]) -> Command {
        let mut command = Command::new(&self.command);
        command
            .args(args)
            .env("HOME", &self.root)
            .env("AGENT_WORKSPACE_CONFIG_ROOT", &self.root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        command
    }
    async fn collect(&self) -> Result<Value> {
        let mut child = self.child(&["status".into(), "--json".into()]).spawn()?;
        let pid = child.id();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let mut out = tokio::spawn(async move {
            let mut buffer = Vec::new();
            stdout
                .take(8 * 1024 * 1024 + 1)
                .read_to_end(&mut buffer)
                .await?;
            Ok::<_, std::io::Error>(buffer)
        });
        let mut err = tokio::spawn(async move {
            let mut buffer = Vec::new();
            stderr.take(8192).read_to_end(&mut buffer).await?;
            Ok::<_, std::io::Error>(buffer)
        });
        let mut closing = self.closing.subscribe();
        let status = tokio::select! {status=child.wait()=>Some(status?),_=tokio::time::sleep(Duration::from_secs(35))=>None,_=closing.changed()=>None};
        if status.is_none() {
            kill_group(pid);
            let _ = child.kill().await;
            out.abort();
            err.abort();
            anyhow::bail!("Status collection timed out or server stopped");
        }
        let streams = tokio::time::timeout(Duration::from_secs(2), async {
            Ok::<_, anyhow::Error>(((&mut out).await??, (&mut err).await??))
        })
        .await;
        let (output, error) = match streams {
            Ok(result) => result?,
            Err(_) => {
                kill_group(pid);
                out.abort();
                err.abort();
                anyhow::bail!("Status output did not close");
            }
        };
        ensure!(
            status.unwrap().success(),
            "{}",
            String::from_utf8_lossy(&error)
        );
        ensure!(
            output.len() <= 8 * 1024 * 1024,
            "Status snapshot exceeds 8 MiB"
        );
        let value: Value = serde_json::from_slice(&output)?;
        ensure!(
            value["schema_version"] == 1 && value["summary"].is_object(),
            "Invalid workspace snapshot"
        );
        Ok(value)
    }
    pub async fn operation(
        &self,
        session: &SessionRef,
        title: &str,
        args: Vec<String>,
        timeout: u64,
    ) -> Result<bool> {
        let id = token();
        self.history.lock().await.insert(json!({"id":id,"title":title,"kind":"local","created_at":millis(),"outcome":"running","output":""}))?;
        let result = self.run_operation(session, title, args, timeout, &id).await;
        let mut history = self.history.lock().await;
        history.update(&id, |v| {
            v["finished_at"] = json!(millis());
            if v["outcome"] == "running" {
                v["outcome"] = json!(match &result {
                    Ok(true) => "exited",
                    Ok(false) => "cancelled",
                    Err(_) => "failed",
                });
            }
            if let Err(e) = &result {
                v["error"] = json!(e.to_string());
            }
        });
        history.flush()?;
        self.changed.send_modify(|v| *v += 1);
        result
    }
    async fn run_operation(
        &self,
        session: &SessionRef,
        title: &str,
        args: Vec<String>,
        timeout: u64,
        id: &str,
    ) -> Result<bool> {
        let heading = format!("\n==> {title}\n$ workspacectl {}\n", args.join(" "));
        session.lock().await.append(&heading);
        self.history
            .lock()
            .await
            .update(id, |v| v["output"] = json!(heading));
        session.lock().await.emit(json!({"type":"output"}));
        let (_emergency_sender, emergency_cancel) = watch::channel(false);
        let mut cancel = if args == ["desktop", "emergency-stop"] {
            emergency_cancel
        } else {
            session.lock().await.cancel.subscribe()
        };
        let mut closing = self.closing.subscribe();
        if *cancel.borrow() || *closing.borrow() {
            return Ok(false);
        }
        let mut child = self.child(&args).spawn()?;
        let pid = child.id();
        let mut out = stream(
            child.stdout.take().unwrap(),
            session.clone(),
            self.history.clone(),
            id.into(),
        );
        let mut err = stream(
            child.stderr.take().unwrap(),
            session.clone(),
            self.history.clone(),
            id.into(),
        );
        let mut outcome = "exited";
        let status = tokio::select! {
            status=child.wait()=>Some(status?),
            _=tokio::time::sleep(Duration::from_secs(timeout))=>{outcome="timed_out";None},
            _=cancel.changed()=>{outcome="cancelled";None},
            _=closing.changed()=>{outcome="interrupted";None}
        };
        if status.is_none() {
            kill_group(pid);
            let _ = child.kill().await;
            let note = format!("Operation {outcome}.\n");
            session.lock().await.append(&note);
            self.history
                .lock()
                .await
                .update(id, |v| append_record(v, &note));
        }
        if tokio::time::timeout(Duration::from_secs(2), async {
            let _ = (&mut out).await;
            let _ = (&mut err).await;
        })
        .await
        .is_err()
        {
            kill_group(pid);
            out.abort();
            err.abort();
        }
        let ok = status.is_some_and(|s| s.success());
        self.history.lock().await.update(id, |v| {
            v["outcome"] = json!(if status.is_some() && !ok {
                "failed"
            } else {
                outcome
            });
            v["exit_code"] = json!(status.and_then(|s| s.code()));
        });
        session.lock().await.emit(json!({"type":"notice","level":if ok{"info"}else{"error"},"message":format!("{title}: {}",if ok{"✓"}else{"failed — see output"})}));
        self.refresh(true).await;
        Ok(ok)
    }
    pub async fn ask(
        &self,
        session: &SessionRef,
        mut view: Value,
        validation: Validation,
    ) -> Result<Option<Value>> {
        let id = token();
        view["id"] = json!(id);
        let (sender, receiver) = oneshot::channel();
        let mut s = session.lock().await;
        let mut cancel = s.cancel.subscribe();
        s.prompt = Some(Prompt {
            view,
            validation,
            sender,
        });
        s.touch();
        drop(s);
        let value = tokio::select! {value=receiver=>value.ok().flatten(),_=tokio::time::sleep(Duration::from_secs(300))=>None,_=cancel.changed()=>None};
        let mut s = session.lock().await;
        if s.prompt.as_ref().is_some_and(|p| p.view["id"] == id) {
            s.prompt = None;
            s.touch();
        }
        Ok(value)
    }
    pub fn locale(&self) -> &'static str {
        let text = std::fs::read_to_string(self.root.join(".local/state/agent-workspace/locale"))
            .unwrap_or_default();
        if text.trim() == "zh-cn" {
            "zh-cn"
        } else {
            "en"
        }
    }
}
fn kill_group(pid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pid) = pid {
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    let _ = pid;
}
fn append_record(value: &mut Value, text: &str) {
    let mut output = value["output"].as_str().unwrap_or_default().to_owned();
    output.push_str(text);
    let mut offset = output.len().saturating_sub(128 * 1024);
    while !output.is_char_boundary(offset) {
        offset += 1;
    }
    if offset > 0 {
        output.drain(..offset);
        value["truncated"] = json!(true);
        value["output_start"] = json!(value["output_start"].as_u64().unwrap_or(0) + offset as u64);
    }
    value["output"] = json!(output);
}
fn stream(
    mut reader: impl tokio::io::AsyncRead + Unpin + Send + 'static,
    session: SessionRef,
    history: Arc<Mutex<Journal>>,
    id: String,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut buffer = [0; 4096];
        let mut pending = Vec::new();
        loop {
            let n = reader.read(&mut buffer).await.unwrap_or(0);
            pending.extend_from_slice(&buffer[..n]);
            // Keep incomplete UTF-8 sequences until the next read.
            let end = match std::str::from_utf8(&pending) {
                Ok(_) => pending.len(),
                Err(e) if e.error_len().is_none() && n != 0 => e.valid_up_to(),
                Err(_) => pending.len(),
            };
            if end > 0 {
                let text = String::from_utf8_lossy(&pending[..end]).into_owned();
                pending.drain(..end);
                session.lock().await.append(&text);
                history
                    .lock()
                    .await
                    .update(&id, |v| append_record(v, &text));
            }
            if n == 0 {
                break;
            }
        }
    })
}
fn error(status: StatusCode, message: impl ToString) -> Response {
    (status, Json(json!({"error":message.to_string()}))).into_response()
}
fn header(headers: &HeaderMap, key: &str) -> String {
    headers
        .get(key)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .into()
}
async fn guard(ExtractState(state): ExtractState<State>, request: Request, next: Next) -> Response {
    if cranibox_node::spaces::applications::prefix(request.uri().path()).is_some() {
        return crate::applications::handle(state, request).await;
    }
    let headers = request.headers();
    let host = header(headers, "host");
    let origin = header(headers, "origin");
    let navigation = request.method() == "GET"
        && header(headers, "sec-fetch-mode") == "navigate"
        && header(headers, "sec-fetch-dest") == "document";
    let mut response = if !state.hosts.contains(&host) {
        error(
            StatusCode::FORBIDDEN,
            "Host not allowed; configure workspacectl web first",
        )
    } else if (header(headers, "sec-fetch-site") == "cross-site" && !navigation)
        || (!origin.is_empty() && !state.origins.contains(&origin))
    {
        error(StatusCode::FORBIDDEN, "Cross-origin request rejected")
    } else {
        next.run(request).await
    };
    for (key, value) in [
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        ("referrer-policy", "no-referrer"),
    ] {
        response
            .headers_mut()
            .insert(key, HeaderValue::from_static(value));
    }
    response
}
pub fn router(state: State) -> Router {
    Router::new()
        .route("/api/desktop/ws", axum::routing::get(desktop_ws))
        .route("/api/terminal/ws", axum::routing::get(terminal_ws))
        // App API (WS7) stays behind the browser guard.
        .route("/api/v1/app/{*rest}", any(app_api))
        .fallback(any(handle))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        // The embedded iroh relay (WS1) authenticates peers itself, outside the guard.
        .merge(crate::relay::router())
        .with_state(state)
}
async fn app_api(ExtractState(state): ExtractState<State>, request: Request) -> Response {
    cranibox_node::web::app_api::handle(state.services, request).await
}
async fn desktop_ws(
    ExtractState(state): ExtractState<State>,
    headers: HeaderMap,
    Query(query): Query<cranibox_node::web::desktop::Connection>,
    ws: WebSocketUpgrade,
) -> Response {
    cranibox_node::web::desktop::upgrade(state.services, headers, query, ws).await
}
async fn terminal_ws(
    ExtractState(state): ExtractState<State>,
    headers: HeaderMap,
    Query(query): Query<cranibox_node::web::terminal::Connection>,
    ws: WebSocketUpgrade,
) -> Response {
    cranibox_node::web::terminal::upgrade(state.services, headers, query, ws).await
}
async fn handle(ExtractState(state): ExtractState<State>, request: Request) -> Response {
    let path = request.uri().path().to_owned();
    let method = request.method().clone();
    let headers = request.headers().clone();
    // Enrollment authenticates only with a short-lived invitation. Keep all other network
    // operations behind the normal browser session, CSRF and origin checks.
    if path == "/api/network/bootstrap" {
        return cranibox_node::web::bootstrap(&state.services, request).await;
    }
    if method == "GET" {
        match path.as_str() {
            "/healthz" => return Json(json!({"ok":true,"implementation":"rust"})).into_response(),
            "/web.js" => {
                return (
                    [("content-type", "text/javascript; charset=utf-8")],
                    cranibox_node::web::WEB_JS,
                )
                    .into_response();
            }
            "/web.css" => {
                return (
                    [("content-type", "text/css; charset=utf-8")],
                    cranibox_node::web::WEB_CSS,
                )
                    .into_response();
            }
            "/" => {
                let session = if let Some(session) = state.session(&headers).await {
                    session
                } else {
                    match state.new_session().await {
                        Ok(s) => s,
                        Err(e) => return error(StatusCode::SERVICE_UNAVAILABLE, e),
                    }
                };
                let locale = state
                    .snapshot()
                    .await
                    .and_then(|v| v["locale"]["current"].as_str().map(str::to_owned))
                    .unwrap_or_else(|| state.locale().into());
                let s = session.lock().await;
                let config = json!({"csrf":s.csrf,"workspaceUrl":state.config.workspace_url,"desktopUrl":state.config.desktop_url,"language":locale,"cursor":s.seq,"edition":"workspace","node":state.local.as_ref().map(|l| l.id.clone())});
                let page = cranibox_node::web::page(&locale, &config);
                let mut response =
                    ([("content-type", "text/html; charset=utf-8")], page).into_response();
                let secure = state.config.public_url.starts_with("https:")
                    && url::Url::parse(&state.config.public_url)
                        .ok()
                        .is_some_and(|u| {
                            header(&headers, "host")
                                == u[url::Position::BeforeHost..url::Position::AfterPort]
                                || header(&headers, "host") == u.host_str().unwrap_or_default()
                        });
                response.headers_mut().insert(
                    "set-cookie",
                    HeaderValue::from_str(&format!(
                        "aw_session={}; HttpOnly; SameSite=Strict; Path=/{}",
                        s.id,
                        if secure { "; Secure" } else { "" }
                    ))
                    .unwrap(),
                );
                drop(s);
                state.refresh(false).await;
                return response;
            }
            _ => {}
        }
    }
    let Some(session) = state.session(&headers).await else {
        return error(
            StatusCode::UNAUTHORIZED,
            "Session expired. Reload this page",
        );
    };
    let csrf = header(&headers, "x-workspace-csrf");
    if !bool::from(session.lock().await.csrf.as_bytes().ct_eq(csrf.as_bytes())) {
        return error(
            StatusCode::UNAUTHORIZED,
            "Session expired. Reload this page",
        );
    }
    if method != "POST" {
        return error(StatusCode::METHOD_NOT_ALLOWED, "POST required");
    }
    if header(&headers, "content-type") != "application/json" {
        return error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "JSON required");
    }
    let body_limit = cranibox_node::web::Services::limit(&path).unwrap_or(16384);
    let bytes = match tokio::time::timeout(
        Duration::from_secs(10),
        to_bytes(request.into_body(), body_limit),
    )
    .await
    {
        Ok(Ok(b)) => b,
        Ok(Err(_)) => return error(StatusCode::PAYLOAD_TOO_LARGE, "Request too large"),
        Err(_) => return error(StatusCode::REQUEST_TIMEOUT, "Request timed out"),
    };
    let body: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "Invalid JSON"),
    };
    if !body.is_object() {
        return error(StatusCode::BAD_REQUEST, "Object required");
    }
    if cranibox_node::web::Services::limit(&path).is_some() {
        let session_id = session.lock().await.id.clone();
        return match state.services.api(&session_id, &path, body).await {
            Some(Ok(value)) => Json(value).into_response(),
            Some(Err(e)) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
            None => error(StatusCode::NOT_FOUND, "Not found"),
        };
    }
    match path.as_str() {
        "/api/spaces/application" => {
            let owner = session.lock().await.id.clone();
            match crate::applications::issue(&state, owner, body).await {
                Ok(value) => Json(value).into_response(),
                Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
            }
        }
        "/api/projects" => match crate::projects::manage(&state, body).await {
            Ok(value) => Json(value).into_response(),
            Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
        },
        "/api/events" => events(state, session).await,
        "/api/history/list" => {
            let records: Vec<_> = state
                .history
                .lock()
                .await
                .list()
                .into_iter()
                .map(|mut v| {
                    v.as_object_mut().unwrap().remove("output");
                    v
                })
                .collect();
            Json(json!({"records":records})).into_response()
        }
        "/api/history/get" => {
            match state
                .history
                .lock()
                .await
                .get(body["id"].as_str().unwrap_or_default())
            {
                Some(mut value) => {
                    incremental(&mut value, "output", body["outputCursor"].as_u64());
                    Json(value).into_response()
                }
                None => error(StatusCode::NOT_FOUND, "Operation unavailable"),
            }
        }
        "/api/nodes/group"
        | "/api/nodes/enroll"
        | "/api/nodes/revoke"
        | "/api/nodes/identity"
        | "/api/nodes/exec"
        | "/api/nodes/jobs"
        | "/api/nodes/job"
        | "/api/nodes/cancel" => {
            match crate::remote::manage(&state, path.rsplit('/').next().unwrap(), &body).await {
                Ok(mut value) => {
                    if path.ends_with("/job") {
                        incremental(&mut value, "stdout", body["stdoutCursor"].as_u64());
                        incremental(&mut value, "stderr", body["stderrCursor"].as_u64());
                    }
                    if !path.ends_with("/job") && !path.ends_with("/jobs") {
                        state.refresh(true).await;
                    }
                    Json(value).into_response()
                }
                Err(e) => error(StatusCode::BAD_REQUEST, e),
            }
        }
        "/api/metrics" => {
            Json(state.metrics.lock().await.sample(state.gateway.is_some())).into_response()
        }
        "/api/poll" => {
            state.refresh(body["refresh"] == true).await;
            let cache = state.snapshot.lock().await;
            let s = session.lock().await;
            Json(json!({"snapshot":cache.value,"snapshotAt":cache.at,"refreshing":cache.refreshing,"error":cache.error,"busy":s.busy,"forceWelcome":s.force_welcome,"prompt":s.prompt.as_ref().map(|p|&p.view),"output":s.output,"cursor":s.seq,"events":s.events.iter().filter(|e|e["seq"].as_u64().unwrap_or(0)>body["cursor"].as_u64().unwrap_or(0)).collect::<Vec<_>>()})).into_response()
        }
        "/api/answer" => {
            let mut s = session.lock().await;
            let Some(p) = s.prompt.as_ref() else {
                return error(StatusCode::CONFLICT, "Prompt expired");
            };
            if body["id"] != p.view["id"] {
                return error(StatusCode::CONFLICT, "Prompt expired");
            }
            let value = if body["cancel"] == true {
                None
            } else {
                match p.validation.validate(body["value"].clone()) {
                    Ok(v) => Some(v),
                    Err(e) => return error(StatusCode::BAD_REQUEST, e),
                }
            };
            let prompt = s.prompt.take().unwrap();
            s.touch();
            let _ = prompt.sender.send(value);
            Json(json!({"ok":true})).into_response()
        }
        "/api/cancel" => {
            let mut s = session.lock().await;
            s.cancel.send_replace(true);
            s.touch();
            if let Some(p) = s.prompt.take() {
                let _ = p.sender.send(None);
            }
            Json(json!({"ok":true})).into_response()
        }
        "/api/action" => {
            if let Err(e) = crate::actions::validate(&body) {
                return error(StatusCode::BAD_REQUEST, e);
            }
            let emergency = body["type"] == "desktopEmergencyStop";
            if !emergency {
                let mut s = session.lock().await;
                if s.busy {
                    return error(
                        StatusCode::CONFLICT,
                        "An operation is already running. See output",
                    );
                }
                s.busy = true;
                s.touch();
                s.cancel.send_replace(false);
            }
            let job_session = session.clone();
            let job_state = state.clone();
            tokio::spawn(async move {
                if let Err(e) = crate::actions::execute(&job_state, &job_session, body).await {
                    job_session
                        .lock()
                        .await
                        .emit(json!({"type":"notice","level":"error","message":e.to_string()}));
                }
                if !emergency {
                    let mut s = job_session.lock().await;
                    s.busy = false;
                    s.touch();
                }
            });
            (StatusCode::ACCEPTED, Json(json!({"ok":true}))).into_response()
        }
        _ => error(StatusCode::NOT_FOUND, "Not found"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prompt_validation() {
        assert!(
            Validation::Text {
                kind: TextKind::Name
            }
            .validate(json!("--help"))
            .is_err()
        );
        assert!(
            Validation::Text {
                kind: TextKind::Target
            }
            .validate(json!("localhost:99999"))
            .is_err()
        );
        assert!(
            Validation::Pick {
                items: vec![json!({"id":"a"})],
                many: false
            }
            .validate(json!(1))
            .is_err()
        );
    }
    #[test]
    fn bounded_utf8_output() {
        let mut s = Session::new();
        s.append(&"中文".repeat(50000));
        assert!(s.output.len() <= 128 * 1024);
    }
    #[test]
    fn dedicated_origin() {
        let c = Config {
            public_url: "https://example.test/path".into(),
            ..Config::default()
        };
        assert!(c.validate().is_err());
    }
}

struct StreamLease(Arc<std::sync::atomic::AtomicUsize>);
impl Drop for StreamLease {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}
struct EventStream {
    state: State,
    session: SessionRef,
    _lease: StreamLease,
    session_changed: watch::Receiver<u64>,
    state_changed: watch::Receiver<u64>,
    closing: watch::Receiver<bool>,
    tick: tokio::time::Interval,
    first: bool,
    cursor: u64,
    output_cursor: u64,
    snapshot_at: Option<u64>,
}
async fn events(state: State, session: SessionRef) -> Response {
    let s = session.lock().await;
    let count = s.streams.clone();
    if count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 4 {
        count.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "Maximum four live tabs per session",
        );
    }
    let context = EventStream {
        session_changed: s.changed.subscribe(),
        state_changed: state.changed.subscribe(),
        closing: state.closing.subscribe(),
        state: state.clone(),
        session: session.clone(),
        _lease: StreamLease(count),
        tick: tokio::time::interval(Duration::from_secs(15)),
        first: true,
        cursor: s.seq,
        output_cursor: 0,
        snapshot_at: None,
    };
    drop(s);
    let stream = futures_util::stream::unfold(context, |mut ctx| async move {
        if *ctx.closing.borrow() {
            return None;
        }
        if !ctx.first {
            tokio::select! {
                result = ctx.session_changed.changed() => { if result.is_err() { return None; } }
                result = ctx.state_changed.changed() => { if result.is_err() { return None; } }
                _ = ctx.tick.tick() => { ctx.state.refresh(false).await; }
                _ = ctx.closing.changed() => { return None; }
            }
        }
        let cache = ctx.state.snapshot.lock().await;
        let mut s = ctx.session.lock().await;
        s.seen = Instant::now();
        let start = s.output_end.saturating_sub(s.output.len() as u64);
        let offset = ctx.output_cursor.saturating_sub(start) as usize;
        let reset = ctx.first
            || ctx.output_cursor < start
            || ctx.output_cursor > s.output_end
            || !s.output.is_char_boundary(offset);
        let delta = if reset {
            s.output.clone()
        } else {
            s.output[offset..].to_owned()
        };
        let mut value = json!({"cursor":s.seq,"snapshotAt":cache.at,"refreshing":cache.refreshing,"error":cache.error,"busy":s.busy,"forceWelcome":s.force_welcome,"prompt":s.prompt.as_ref().map(|p| &p.view),"outputDelta":delta,"outputReset":reset,"events":s.events.iter().filter(|e| e["seq"].as_u64().unwrap_or(0)>ctx.cursor).collect::<Vec<_>>()});
        if ctx.snapshot_at != Some(cache.at) {
            value["snapshot"] = cache.value.clone().unwrap_or(Value::Null);
        }
        ctx.snapshot_at = Some(cache.at);
        ctx.cursor = s.seq;
        ctx.output_cursor = s.output_end;
        ctx.first = false;
        drop(s);
        drop(cache);
        let data = value.to_string();
        let mut metrics = ctx.state.metrics.lock().await;
        metrics.stream_messages += 1;
        metrics.stream_bytes += data.len() as u64;
        drop(metrics);
        Some((
            Ok::<_, std::convert::Infallible>(Event::default().event("state").data(data)),
            ctx,
        ))
    });
    let mut response = Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response();
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
}

fn incremental(value: &mut Value, field: &str, cursor: Option<u64>) {
    let Some(cursor) = cursor else {
        return;
    };
    let output = value[field].as_str().unwrap_or_default().to_owned();
    let start = if field == "output" {
        value["output_start"].as_u64().unwrap_or(0)
    } else {
        0
    };
    let offset = usize::try_from(cursor.saturating_sub(start)).unwrap_or(usize::MAX);
    let reset = cursor < start || offset > output.len() || !output.is_char_boundary(offset);
    value[format!("{field}Delta")] = json!(if reset { &output } else { &output[offset..] });
    value[format!("{field}Reset")] = json!(reset);
    value[format!("{field}Cursor")] = json!(start + output.len() as u64);
    value.as_object_mut().unwrap().remove(field);
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod operation_tests;
