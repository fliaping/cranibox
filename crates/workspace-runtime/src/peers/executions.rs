//! Explicit, durable launches. Remote callers see only runs submitted under their grant.
use super::{Store as Peers, config::Config, messages::now};
use crate::config;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
use uuid::Uuid;
pub(crate) mod mcp;

pub const CAPACITY: usize = 128;
pub const PARALLEL: usize = 8;
pub const LOG_LIMIT: usize = 1024 * 1024;
pub const INPUT_LIMIT: usize = 16 * 1024;
const RETENTION: u64 = 7 * 24 * 3600;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Launch {
    pub name: String,
    pub harness: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin: Option<String>,
}
impl Launch {
    pub(super) fn validate(&self) -> Result<()> {
        for (value, limit) in [(&self.name, 120), (&self.harness, 80)] {
            ensure!(
                !value.trim().is_empty()
                    && value.len() <= limit
                    && !value.chars().any(char::is_control),
                "run labels must be bounded plain text"
            );
        }
        ensure!(
            !self.program.trim().is_empty()
                && self.program.len() <= 4096
                && !self.program.contains('\0'),
            "invalid program"
        );
        ensure!(
            self.args.len() <= 128
                && self.args.iter().all(|arg| !arg.contains('\0'))
                && self.args.iter().map(String::len).sum::<usize>() <= 8192,
            "arguments exceed the 8192-byte limit"
        );
        ensure!(
            self.cwd.is_absolute() && self.cwd.as_os_str().len() <= 4096,
            "working directory must be an absolute path"
        );
        if let Some(input) = &self.stdin {
            ensure!(
                input.len() <= INPUT_LIMIT && !input.contains('\0'),
                "standard input exceeds 16384 bytes or contains NUL"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Adapter {
    Codex,
    ClaudeCode,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    ReadOnly,
    WorkspaceWrite,
    EditFiles,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentLaunch {
    pub name: String,
    pub adapter: Adapter,
    pub prompt: String,
    pub cwd: PathBuf,
    pub access: Access,
}
impl AgentLaunch {
    fn launch(self) -> Result<Launch> {
        ensure!(
            !self.prompt.trim().is_empty(),
            "task instructions are required"
        );
        let (harness, program, args) = match (self.adapter, self.access) {
            (Adapter::Codex, Access::ReadOnly | Access::WorkspaceWrite) => (
                "codex",
                "codex",
                vec![
                    "--ask-for-approval",
                    "never",
                    "exec",
                    "--sandbox",
                    if self.access == Access::ReadOnly {
                        "read-only"
                    } else {
                        "workspace-write"
                    },
                    "--color",
                    "never",
                    "-",
                ],
            ),
            (Adapter::ClaudeCode, Access::ReadOnly | Access::EditFiles) => {
                let tools = if self.access == Access::ReadOnly {
                    "Read,Glob,Grep"
                } else {
                    "Read,Glob,Grep,Edit,Write"
                };
                (
                    "claude-code",
                    "claude",
                    vec![
                        "--print",
                        "--output-format",
                        "text",
                        "--permission-mode",
                        "dontAsk",
                        "--tools",
                        tools,
                        "--allowedTools",
                        tools,
                        "--disallowedTools",
                        "mcp__*",
                        "--strict-mcp-config",
                        "--mcp-config",
                        r#"{"mcpServers":{}}"#,
                    ],
                )
            }
            _ => anyhow::bail!("access mode is not supported by this Agent"),
        };
        let launch = Launch {
            name: self.name,
            harness: harness.into(),
            program: program.into(),
            args: args.into_iter().map(String::from).collect(),
            cwd: self.cwd,
            stdin: Some(self.prompt),
        };
        launch.validate()?;
        Ok(launch)
    }
}

/// Resolve only absolute PATH entries, never a program in the task directory.
fn executable(program: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .filter(|path| path.is_absolute())
        .find_map(|path| {
            #[cfg(windows)]
            let names = [
                format!("{program}.exe"),
                format!("{program}.cmd"),
                format!("{program}.bat"),
            ];
            #[cfg(not(windows))]
            let names = [program.to_string()];
            names
                .into_iter()
                .map(|name| path.join(name))
                .find(|candidate| {
                    let Ok(metadata) = candidate.metadata() else {
                        return false;
                    };
                    if !metadata.is_file() {
                        return false;
                    }
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        metadata.permissions().mode() & 0o111 != 0
                    }
                    #[cfg(not(unix))]
                    {
                        true
                    }
                })
        })
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Queued,
    Starting,
    Running,
    Stopping,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}
impl State {
    pub fn active(self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Starting | Self::Running | Self::Stopping
        )
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Requester {
    node: String,
    credential_hash: String,
}
impl Requester {
    /// `credential` is the target grant's stored digest, never the bearer secret.
    pub fn new(node: String, credential: &str) -> Self {
        Self {
            node,
            credential_hash: credential.to_owned(),
        }
    }
    fn matches(&self, other: &Self) -> bool {
        self.node == other.node && config::matches(&self.credential_hash, &other.credential_hash)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub id: Uuid,
    pub session_id: Uuid,
    pub launch: Launch,
    pub state: State,
    pub created_at: u64,
    pub updated_at: u64,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub archived: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requester: Option<Requester>,
    /// Locally approved task worker, bound to the sender's messages grant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automatic_sender: Option<Requester>,
}
impl Record {
    /// Fail closed if an execution grant is removed, rotated, or cannot be read.
    pub fn permitted(&self, peers: &Path, owner: &str) -> bool {
        let permitted = |requester: &Requester, messages: bool| {
            Config::read(peers).is_ok_and(|configuration| {
                configuration.node_id == owner
                    && configuration
                        .grants
                        .get(&requester.node)
                        .is_some_and(|grant| {
                            (if messages {
                                grant.permissions.messages
                            } else {
                                grant.permissions.exec
                            }) && requester
                                .matches(&Requester::new(requester.node.clone(), &grant.credential))
                        })
            })
        };
        self.requester
            .as_ref()
            .is_none_or(|requester| permitted(requester, false))
            && self
                .automatic_sender
                .as_ref()
                .is_none_or(|requester| permitted(requester, true))
    }
    fn summary(&self) -> Value {
        json!({"id":self.id,"session_id":self.session_id,"name":self.launch.name,"harness":self.launch.harness,
            "requester":self.requester.as_ref().map(|requester| &requester.node),
            "automatic_sender":self.automatic_sender.as_ref().map(|requester| &requester.node),
            "state":self.state,"created_at":self.created_at,"updated_at":self.updated_at,"pid":self.pid,
            "exit_code":self.exit_code,"error":self.error,"stdout_truncated":self.stdout_truncated,"stderr_truncated":self.stderr_truncated})
    }
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Ledger {
    owner: String,
    records: BTreeMap<Uuid, Record>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    List {},
    PrepareAgent {
        agent: AgentLaunch,
    },
    Start {
        id: Uuid,
        launch: Launch,
    },
    Get {
        id: Uuid,
        #[serde(default)]
        stdout_offset: u64,
        #[serde(default)]
        stderr_offset: u64,
    },
    Stop {
        id: Uuid,
    },
    Archive {
        id: Uuid,
    },
}
#[derive(Clone)]
pub struct Store {
    path: PathBuf,
    owner: String,
    requester: Option<Requester>,
    automatic_sender: Option<Requester>,
}
impl Store {
    pub fn new(peers: &Path, owner: String) -> Self {
        Self {
            path: peers.with_file_name("agent-executions.json"),
            owner,
            requester: None,
            automatic_sender: None,
        }
    }
    pub fn for_requester(mut self, requester: Requester) -> Self {
        self.requester = Some(requester);
        self
    }
    pub(super) fn for_automatic_sender(mut self, sender: Requester) -> Self {
        self.automatic_sender = Some(sender);
        self
    }
    fn visible(&self, record: &Record) -> bool {
        self.requester.as_ref().is_none_or(|requester| {
            record
                .requester
                .as_ref()
                .is_some_and(|saved| saved.matches(requester))
        })
    }
    fn read(&self) -> Result<Ledger> {
        let file = match fs::File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Ledger {
                    owner: self.owner.clone(),
                    records: BTreeMap::new(),
                });
            }
            Err(error) => return Err(error.into()),
        };
        ensure!(
            file.metadata()?.len() <= 4 * 1024 * 1024,
            "run ledger is too large"
        );
        let ledger: Ledger = serde_json::from_reader(file)
            .context("read run ledger; existing data was preserved")?;
        ensure!(
            ledger.owner == self.owner && ledger.records.len() <= CAPACITY,
            "run ledger owner or capacity mismatch"
        );
        for (id, record) in &ledger.records {
            ensure!(
                *id == record.id && !id.is_nil() && !record.session_id.is_nil(),
                "invalid run identity"
            );
            record.launch.validate()?;
            for requester in record
                .requester
                .iter()
                .chain(record.automatic_sender.iter())
            {
                ensure!(
                    crate::protocol::valid_node_id(&requester.node)
                        && requester.credential_hash.len() == 64
                        && requester
                            .credential_hash
                            .bytes()
                            .all(|byte| byte.is_ascii_hexdigit()),
                    "invalid run requester"
                );
            }
        }
        Ok(ledger)
    }
    fn update<T>(&self, change: impl FnOnce(&mut Ledger) -> Result<T>) -> Result<T> {
        let _lock = config::lock_credentials(&self.path)?;
        let mut ledger = self.read()?;
        let result = change(&mut ledger)?;
        let bytes = serde_json::to_vec(&ledger)?;
        ensure!(bytes.len() <= 4 * 1024 * 1024, "run ledger is too large");
        config::write_private(&self.path, &bytes, true)?;
        Ok(result)
    }
    pub fn records(&self) -> Result<Vec<Record>> {
        Ok(self.read()?.records.into_values().collect())
    }
    pub fn record(&self, id: Uuid) -> Result<Record> {
        self.read()?
            .records
            .remove(&id)
            .filter(|record| self.visible(record))
            .context("run unavailable")
    }
    pub fn change(&self, id: Uuid, change: impl FnOnce(&mut Record) -> Result<()>) -> Result<()> {
        self.update(|ledger| {
            let record = ledger.records.get_mut(&id).context("run unavailable")?;
            ensure!(self.visible(record), "run unavailable");
            change(record)?;
            record.updated_at = now();
            Ok(())
        })
    }
    /// A new node process never replays an ambiguous launch or trusts a saved PID.
    pub fn recover(&self) -> Result<Vec<Uuid>> {
        self.update(|ledger| {
            let mut interrupted = Vec::new();
            for record in ledger.records.values_mut().filter(|record| record.state.active()) {
                record.state = State::Interrupted;
                record.pid = None;
                record.updated_at = now();
                record.error = Some("Node restarted; the previous execution outcome is unknown. Review before starting a new run.".into());
                interrupted.push(record.session_id);
            }
            Ok(interrupted)
        })
    }
    fn log_dir(&self) -> PathBuf {
        self.path.with_file_name("agent-logs")
    }
    pub fn log_path(&self, id: Uuid, stderr: bool) -> PathBuf {
        self.log_dir().join(format!(
            "{id}-{}.log",
            if stderr { "stderr" } else { "stdout" }
        ))
    }
    pub fn create_log(&self, id: Uuid, stderr: bool) -> Result<fs::File> {
        fs::create_dir_all(self.log_dir())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(self.log_dir(), fs::Permissions::from_mode(0o700))?;
        }
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        Ok(options.open(self.log_path(id, stderr))?)
    }
    fn output(&self, id: Uuid, stderr: bool, offset: u64, finished: bool) -> Result<Value> {
        ensure!(offset <= LOG_LIMIT as u64, "invalid output offset");
        let mut file = match fs::File::open(self.log_path(id, stderr)) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(json!({"text":"","next_offset":0}));
            }
            Err(error) => return Err(error.into()),
        };
        let length = file.metadata()?.len();
        ensure!(
            length <= LOG_LIMIT as u64 && offset <= length,
            "output changed; reload from offset zero"
        );
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = vec![0; (length - offset).min(65536) as usize];
        file.read_exact(&mut bytes)?;
        // Keep a UTF-8 code point together across pages and incremental writes.
        // Invalid bytes remain visible as replacement characters. A final incomplete
        // code point is also rendered once the stream can no longer grow.
        let more = offset + (bytes.len() as u64) < length || !finished;
        let mut checked = 0;
        while let Err(error) = std::str::from_utf8(&bytes[checked..]) {
            checked += error.valid_up_to();
            if let Some(length) = error.error_len() {
                checked += length;
            } else {
                if more {
                    bytes.truncate(checked);
                }
                break;
            }
        }
        Ok(json!({"text":String::from_utf8_lossy(&bytes),"next_offset":offset+bytes.len() as u64}))
    }
    fn catalog(&self) -> Result<Value> {
        let ledger = self.read()?;
        let visible = ledger
            .records
            .values()
            .filter(|record| self.visible(record))
            .collect::<Vec<_>>();
        Ok(
            json!({"owner":self.owner,"capacity":CAPACITY,"parallel":PARALLEL,"log_limit":LOG_LIMIT,"used":visible.len(),
            "executions":visible.into_iter().filter(|record| !record.archived).map(Record::summary).collect::<Vec<_>>()}),
        )
    }
    pub fn apply(&self, command: Command) -> Result<Value> {
        match command {
            Command::List {} => {}
            Command::PrepareAgent { agent } => {
                let adapter = agent.adapter;
                let access = agent.access;
                let mut launch = agent.launch()?;
                ensure!(
                    launch.cwd.is_dir(),
                    "working directory must be an existing absolute directory"
                );
                let program = executable(&launch.program);
                let available = program.is_some();
                if let Some(program) = program {
                    launch.program = program.to_string_lossy().into_owned();
                }
                return Ok(
                    json!({"launch":launch,"available":available,"adapter":adapter,"access":access}),
                );
            }
            Command::Get {
                id,
                stdout_offset,
                stderr_offset,
            } => {
                let record = self.record(id)?;
                return Ok(
                    json!({"owner":self.owner,"execution":record.summary(),"launch":record.launch,
                    "stdout":self.output(id,false,stdout_offset,!record.state.active())?,"stderr":self.output(id,true,stderr_offset,!record.state.active())?}),
                );
            }
            Command::Start { id, launch } => {
                self.start_with_session(id, None, launch)?;
            }
            Command::Stop { id } => self.change(id, |record| {
                record.state = match record.state {
                    State::Queued => State::Cancelled,
                    state if state.active() => State::Stopping,
                    state => state,
                };
                Ok(())
            })?,
            Command::Archive { id } => {
                self.change(id, |record| {
                    ensure!(!record.state.active(), "stop the run before archiving");
                    record.archived = true;
                    Ok(())
                })?;
                for stderr in [false, true] {
                    match fs::remove_file(self.log_path(id, stderr)) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error.into()),
                    }
                }
            }
        }
        self.catalog()
    }
    /// A reviewed task persists its claim before enqueueing, then reuses both identities.
    pub(super) fn start_with_session(
        &self,
        id: Uuid,
        session: Option<Uuid>,
        launch: Launch,
    ) -> Result<()> {
        ensure!(
            !id.is_nil() && session.is_none_or(|id| !id.is_nil()),
            "run requires nonzero UUIDs"
        );
        launch.validate()?;
        self.update(|ledger| {
            if let Some(record) = ledger.records.get(&id) {
                ensure!(self.visible(record), "run unavailable");
                if let Some(sender) = &self.automatic_sender {
                    ensure!(
                        record.requester.is_none()
                            && record
                                .automatic_sender
                                .as_ref()
                                .is_some_and(|saved| saved.matches(sender)),
                        "automatic worker identity changed"
                    );
                }
                ensure!(
                    record.launch == launch && session.is_none_or(|id| id == record.session_id),
                    "run UUID already used for another launch or session"
                );
                return Ok(());
            }
            ensure!(
                launch.cwd.is_dir(),
                "working directory must be an existing absolute directory"
            );
            ledger.records.retain(|_, record| {
                !(record.archived && record.updated_at.saturating_add(RETENTION) < now())
            });
            ensure!(
                ledger.records.len() < CAPACITY,
                "run history is full; archive old runs (deduplication is retained for seven days)"
            );
            ensure!(
                ledger
                    .records
                    .values()
                    .filter(|record| record.state.active())
                    .count()
                    < PARALLEL,
                "eight runs are already pending or active"
            );
            ledger.records.insert(
                id,
                Record {
                    id,
                    session_id: session.unwrap_or_else(Uuid::new_v4),
                    launch,
                    state: State::Queued,
                    created_at: now(),
                    updated_at: now(),
                    pid: None,
                    exit_code: None,
                    error: None,
                    stdout_truncated: false,
                    stderr_truncated: false,
                    archived: false,
                    requester: self.requester.clone(),
                    automatic_sender: self.automatic_sender.clone(),
                },
            );
            Ok(())
        })
    }
}
impl Peers {
    pub async fn node_executions(&self, node: Option<String>, command: Command) -> Result<Value> {
        if let Some(node) = node {
            ensure!(
                crate::protocol::valid_node_id(&node),
                "invalid target node ID"
            );
            return self
                .route(&node)?
                .api(&node)?
                .post(
                    &format!("/api/v1/nodes/{node}/executions"),
                    serde_json::to_value(command)?,
                )
                .await;
        }
        self.executions(command).await
    }
    pub async fn executions(&self, command: Command) -> Result<Value> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            let configuration = Config::read(&path)?;
            Store::new(&path, configuration.node_id).apply(command)
        })
        .await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_runs_are_scoped_to_the_requester_and_credential_generation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("peers.json");
        let owner = Store::new(&path, "owner".into());
        let a = owner
            .clone()
            .for_requester(Requester::new("caller-a".into(), &"a".repeat(64)));
        let b = owner
            .clone()
            .for_requester(Requester::new("caller-b".into(), &"b".repeat(64)));
        let rotated = owner
            .clone()
            .for_requester(Requester::new("caller-a".into(), &"c".repeat(64)));
        let local_id = Uuid::new_v4();
        owner
            .apply(Command::Start {
                id: local_id,
                launch: launch(root.path()),
            })
            .unwrap();
        let id = Uuid::new_v4();
        a.apply(Command::Start {
            id,
            launch: launch(root.path()),
        })
        .unwrap();
        a.apply(Command::Start {
            id,
            launch: launch(root.path()),
        })
        .unwrap();
        assert_eq!(a.catalog().unwrap()["used"], 1);
        assert_eq!(owner.catalog().unwrap()["used"], 2);
        for other in [&b, &rotated] {
            assert_eq!(other.catalog().unwrap()["used"], 0);
            for command in [
                Command::Get {
                    id,
                    stdout_offset: 0,
                    stderr_offset: 0,
                },
                Command::Stop { id },
                Command::Archive { id },
                Command::Start {
                    id,
                    launch: launch(root.path()),
                },
            ] {
                assert!(other.apply(command).is_err());
            }
        }
        assert!(a.record(local_id).is_err());
        assert_eq!(a.record(id).unwrap().state, State::Queued);
        assert!(
            !serde_json::to_string(&a.catalog().unwrap())
                .unwrap()
                .contains("credential_hash")
        );
        owner.apply(Command::Stop { id }).unwrap();
        a.apply(Command::Archive { id }).unwrap();
        a.apply(Command::Start {
            id,
            launch: launch(root.path()),
        })
        .unwrap();
        assert_eq!(owner.record(id).unwrap().state, State::Cancelled);
    }
    #[test]
    fn remote_execution_permission_follows_the_saved_grant_and_identity() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("peers.json");
        let config = Config::update(&path, |configuration| {
            configuration.grants.insert(
                "caller".into(),
                super::super::config::Grant::issue(
                    &"a".repeat(64),
                    super::super::config::Permissions {
                        exec: true,
                        ..Default::default()
                    },
                ),
            );
            Ok(())
        })
        .unwrap();
        let store = Store::new(&path, config.node_id.clone()).for_requester(Requester::new(
            "caller".into(),
            &config.grants["caller"].credential,
        ));
        let id = Uuid::new_v4();
        store
            .apply(Command::Start {
                id,
                launch: launch(root.path()),
            })
            .unwrap();
        let record = store.record(id).unwrap();
        assert!(record.permitted(&path, &config.node_id));
        assert!(!record.permitted(&path, "other-node"));
        Config::update(&path, |configuration| {
            let grant = configuration.grants.get_mut("caller").unwrap();
            *grant = super::super::config::Grant::issue(&"b".repeat(64), grant.permissions.clone());
            Ok(())
        })
        .unwrap();
        assert!(!record.permitted(&path, &config.node_id));
        fs::remove_file(&path).unwrap();
        assert!(!record.permitted(&path, &config.node_id));
    }
    fn launch(root: &Path) -> Launch {
        Launch {
            name: "Review".into(),
            harness: "custom".into(),
            program: "agent".into(),
            args: vec!["--noninteractive".into()],
            cwd: fs::canonicalize(root).unwrap(),
            stdin: None,
        }
    }
    #[test]
    fn agent_launches_keep_tasks_out_of_arguments_and_validate_access() {
        let root = tempfile::tempdir().unwrap();
        let prompt = "审查项目\n$(do-not-execute) --dangerously-skip-permissions";
        for (adapter, access, sandbox) in [
            (Adapter::Codex, Access::ReadOnly, Some("read-only")),
            (
                Adapter::Codex,
                Access::WorkspaceWrite,
                Some("workspace-write"),
            ),
            (Adapter::ClaudeCode, Access::ReadOnly, None),
            (Adapter::ClaudeCode, Access::EditFiles, None),
        ] {
            let launch = AgentLaunch {
                name: "Review".into(),
                adapter,
                access,
                prompt: prompt.into(),
                cwd: root.path().into(),
            }
            .launch()
            .unwrap();
            assert_eq!(launch.stdin.as_deref(), Some(prompt));
            assert!(!launch.args.iter().any(|argument| argument.contains(prompt)));
            if let Some(sandbox) = sandbox {
                assert!(
                    launch
                        .args
                        .windows(2)
                        .any(|args| args == ["--sandbox", sandbox])
                );
                assert!(
                    launch
                        .args
                        .windows(2)
                        .any(|args| args == ["--ask-for-approval", "never"])
                );
                assert_eq!(launch.args.last().unwrap(), "-");
            } else {
                assert!(
                    launch
                        .args
                        .windows(2)
                        .any(|args| args == ["--permission-mode", "dontAsk"])
                );
                assert!(!launch.args.iter().any(|arg| arg.contains("Bash")));
                let tools =
                    &launch.args[launch.args.iter().position(|arg| arg == "--tools").unwrap() + 1];
                assert_eq!(tools.contains("Edit"), access == Access::EditFiles);
            }
        }
        for (adapter, access, prompt) in [
            (Adapter::Codex, Access::EditFiles, "review".into()),
            (Adapter::ClaudeCode, Access::WorkspaceWrite, "review".into()),
            (Adapter::Codex, Access::ReadOnly, " ".into()),
            (
                Adapter::Codex,
                Access::ReadOnly,
                "x".repeat(INPUT_LIMIT + 1),
            ),
            (Adapter::Codex, Access::ReadOnly, "task\0text".into()),
        ] {
            assert!(
                AgentLaunch {
                    name: "Review".into(),
                    adapter,
                    access,
                    prompt,
                    cwd: root.path().into()
                }
                .launch()
                .is_err()
            );
        }
    }
    #[test]
    fn preparing_an_agent_does_not_enqueue_or_write_a_ledger() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(&root.path().join("peers.json"), "owner".into());
        let value = store
            .apply(Command::PrepareAgent {
                agent: AgentLaunch {
                    name: "Review".into(),
                    adapter: Adapter::Codex,
                    access: Access::ReadOnly,
                    prompt: "Review only".into(),
                    cwd: root.path().into(),
                },
            })
            .unwrap();
        assert!(value["available"].is_boolean());
        assert_eq!(value["launch"]["stdin"], "Review only");
        assert!(!store.path.exists());
        assert!(!store.log_dir().exists());
        assert!(store.records().unwrap().is_empty());
    }
    #[test]
    fn legacy_launches_keep_their_deduplication_identity() {
        let root = tempfile::tempdir().unwrap();
        let original = launch(root.path());
        let encoded = serde_json::to_value(&original).unwrap();
        assert!(encoded.get("stdin").is_none());
        let legacy: Launch = serde_json::from_value(encoded).unwrap();
        assert_eq!(legacy, original);
        let mut invalid = legacy;
        invalid.stdin = Some("x".repeat(INPUT_LIMIT + 1));
        assert!(invalid.validate().is_err());
    }
    #[test]
    fn submission_deduplicates_and_restart_never_reexecutes_saved_pids() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(&root.path().join("peers.json"), "owner".into());
        let id = Uuid::new_v4();
        let command = Command::Start {
            id,
            launch: launch(root.path()),
        };
        store.apply(command.clone()).unwrap();
        let session = store.record(id).unwrap().session_id;
        store.apply(command).unwrap();
        assert_eq!(store.records().unwrap().len(), 1);
        store
            .change(id, |record| {
                record.pid = Some(123);
                record.state = State::Running;
                Ok(())
            })
            .unwrap();
        assert_eq!(store.recover().unwrap(), vec![session]);
        assert_eq!(store.record(id).unwrap().state, State::Interrupted);
        assert!(store.record(id).unwrap().pid.is_none());
        store
            .apply(Command::Start {
                id,
                launch: launch(root.path()),
            })
            .unwrap();
        assert_eq!(store.record(id).unwrap().state, State::Interrupted);
        let mut changed = launch(root.path());
        changed.program = "different".into();
        assert!(
            store
                .apply(Command::Start {
                    id,
                    launch: changed
                })
                .is_err()
        );
    }
    #[test]
    fn stop_before_launch_and_archive_preserve_deduplication() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(&root.path().join("peers.json"), "owner".into());
        let id = Uuid::new_v4();
        let command = Command::Start {
            id,
            launch: launch(root.path()),
        };
        store.apply(command.clone()).unwrap();
        assert!(store.apply(Command::Archive { id }).is_err());
        store.apply(Command::Stop { id }).unwrap();
        assert_eq!(store.record(id).unwrap().state, State::Cancelled);
        store.apply(Command::Archive { id }).unwrap();
        store.apply(command).unwrap();
        assert!(
            store.apply(Command::List {}).unwrap()["executions"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn capacity_labels_and_owner_are_checked_without_overwriting_corruption() {
        let root = tempfile::tempdir().unwrap();
        let peers = root.path().join("peers.json");
        let store = Store::new(&peers, "owner".into());
        let mut invalid = launch(root.path());
        invalid.name = "secret\nvalue".into();
        assert!(
            store
                .apply(Command::Start {
                    id: Uuid::new_v4(),
                    launch: invalid
                })
                .is_err()
        );
        for _ in 0..PARALLEL {
            store
                .apply(Command::Start {
                    id: Uuid::new_v4(),
                    launch: launch(root.path()),
                })
                .unwrap();
        }
        assert!(
            store
                .apply(Command::Start {
                    id: Uuid::new_v4(),
                    launch: launch(root.path())
                })
                .is_err()
        );
        assert!(Store::new(&peers, "other".into()).records().is_err());
        fs::write(&store.path, b"broken").unwrap();
        assert!(store.recover().is_err());
        assert_eq!(fs::read(&store.path).unwrap(), b"broken");
    }
}
