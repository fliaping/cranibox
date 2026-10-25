//! Terminal host protocol 1: each frame is a big-endian u32 length (1..=1 MiB),
//! followed by a kind byte: 0 = UTF-8 JSON, 1 = raw PTY bytes. The first frame
//! is JSON `{"token":"<64 hex digits>"}`, then `Hello { protocol: 1 }`.
//! JSON messages use a snake_case `type` tag (see Request/Response below).
//! One connection handles one Manage or one Attach after Hello. Manage returns
//! the native engine's JSON in Result, or Error. Attach returns Attached, raw
//! replay frames, Ready, then raw output and Exit. Clients send raw input,
//! Resize or Detach; EOF also detaches. Error contains the native error text.
//! Never change protocol-1 message meanings: an updated node must speak to an
//! older host. An incompatible Hello causes native fallback, never replacement
//! of a live host. No mutation is retried after transmission (Create may have
//! succeeded before a lost reply). Authentication is local to each connection;
//! neither the token nor IPC handles are passed to shells.
use crate::{Event, persistent, validate_size};
use anyhow::{Context, Result, bail, ensure};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::Duration,
};
use subtle::ConstantTimeEq;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{Mutex, mpsc},
    task::JoinSet,
    time::{Instant, timeout},
};

pub const PROTOCOL: u32 = 1;
const MAX_FRAME: usize = 1024 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const IDLE_TIMEOUT: Duration = Duration::from_secs(600);
const RECORD: &str = "terminal-host.json";

#[derive(Serialize, Deserialize)]
struct Record {
    protocol: u32,
    pid: u32,
    version: String,
    exe: PathBuf,
    endpoint: String,
    token: String,
}

#[derive(Serialize, Deserialize)]
struct Auth {
    token: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Request {
    Hello {
        protocol: u32,
    },
    Manage {
        command: persistent::Manage,
    },
    Attach {
        id: String,
        cols: u16,
        rows: u16,
        takeover: bool,
    },
    Resize {
        cols: u16,
        rows: u16,
    },
    Detach,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Response {
    Hello { protocol: u32 },
    Result { value: Value },
    Error { message: String },
    Attached,
    Ready,
    Exit { code: u32 },
}

#[derive(Debug)]
struct ProtocolMismatch;
impl std::fmt::Display for ProtocolMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("incompatible terminal host protocol")
    }
}
impl std::error::Error for ProtocolMismatch {}

enum Frame {
    Json(Vec<u8>),
    Bytes(Vec<u8>),
}
impl Frame {
    fn json<T: serde::de::DeserializeOwned>(self) -> Result<T> {
        match self {
            Self::Json(data) => Ok(serde_json::from_slice(&data)?),
            Self::Bytes(_) => bail!("expected terminal host control frame"),
        }
    }
}
async fn read_frame(reader: &mut (impl AsyncRead + Unpin)) -> Result<Frame> {
    let len = reader.read_u32().await? as usize;
    ensure!(
        (1..=MAX_FRAME).contains(&len),
        "invalid terminal host frame length"
    );
    let mut data = vec![0; len - 1];
    let kind = reader.read_u8().await?;
    reader.read_exact(&mut data).await?;
    match kind {
        0 => Ok(Frame::Json(data)),
        1 => Ok(Frame::Bytes(data)),
        _ => bail!("invalid terminal host frame kind"),
    }
}
async fn write_frame(writer: &mut (impl AsyncWrite + Unpin), kind: u8, data: &[u8]) -> Result<()> {
    ensure!(data.len() < MAX_FRAME, "terminal host frame too large");
    timeout(IO_TIMEOUT, async {
        writer.write_u32((data.len() + 1) as u32).await?;
        writer.write_u8(kind).await?;
        writer.write_all(data).await?;
        writer.flush().await
    })
    .await
    .context("terminal host write timed out")??;
    Ok(())
}
async fn send(writer: &mut (impl AsyncWrite + Unpin), value: &impl Serialize) -> Result<()> {
    write_frame(writer, 0, &serde_json::to_vec(value)?).await
}
async fn response(reader: &mut (impl AsyncRead + Unpin)) -> Result<Response> {
    let message = read_frame(reader).await?.json()?;
    if let Response::Error { message } = message {
        bail!("{message}");
    }
    Ok(message)
}

#[cfg(unix)]
type Stream = tokio::net::UnixStream;
#[cfg(windows)]
type Stream = tokio::net::windows::named_pipe::NamedPipeClient;

async fn open(endpoint: &str) -> Result<Stream> {
    #[cfg(unix)]
    {
        Ok(Stream::connect(endpoint).await?)
    }
    #[cfg(windows)]
    {
        use tokio::net::windows::named_pipe::ClientOptions;
        // An instance can be busy while the host creates its next listener.
        loop {
            match ClientOptions::new().open(endpoint) {
                Ok(pipe) => return Ok(pipe),
                Err(error) if error.raw_os_error() == Some(231) => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}

#[derive(Clone)]
pub struct Client {
    state: PathBuf,
    home: PathBuf,
    starting: Arc<Mutex<()>>,
}
impl Client {
    pub async fn new(state: PathBuf, home: PathBuf) -> Result<Self> {
        let client = Self {
            state,
            home,
            starting: Arc::default(),
        };
        client.connection().await?;
        Ok(client)
    }
    async fn existing(&self) -> Result<Stream> {
        timeout(IO_TIMEOUT, async {
            let record: Record = serde_json::from_slice(&fs::read(self.state.join(RECORD))?)
                .context("read terminal host record")?;
            let mut stream = open(&record.endpoint).await?;
            send(
                &mut stream,
                &Auth {
                    token: record.token,
                },
            )
            .await?;
            send(&mut stream, &Request::Hello { protocol: PROTOCOL }).await?;
            match response(&mut stream).await? {
                Response::Hello { protocol }
                    if protocol == PROTOCOL && record.protocol == PROTOCOL =>
                {
                    Ok(stream)
                }
                Response::Hello { .. } => Err(ProtocolMismatch.into()),
                _ => bail!("invalid terminal host hello"),
            }
        })
        .await
        .context("terminal host hello timed out")?
    }
    async fn connection(&self) -> Result<Stream> {
        match self.existing().await {
            Ok(stream) => return Ok(stream),
            Err(error) if error.is::<ProtocolMismatch>() => return Err(error),
            Err(_) => {}
        }
        let _starting = self.starting.lock().await;
        match self.existing().await {
            Ok(stream) => return Ok(stream),
            Err(error) if error.is::<ProtocolMismatch>() => return Err(error),
            Err(_) => {}
        }
        spawn(&self.state, &self.home)?;
        let deadline = Instant::now() + IO_TIMEOUT;
        loop {
            tokio::time::sleep(Duration::from_millis(25)).await;
            match self.existing().await {
                Ok(stream) => return Ok(stream),
                Err(error) if error.is::<ProtocolMismatch>() || Instant::now() >= deadline => {
                    return Err(error);
                }
                Err(_) => {}
            }
        }
    }
    pub async fn manage(&self, command: persistent::Manage) -> Result<Value> {
        let mut stream = self.connection().await?;
        send(&mut stream, &Request::Manage { command }).await?;
        match timeout(IO_TIMEOUT, response(&mut stream))
            .await
            .context("terminal host manage timed out")??
        {
            Response::Result { value } => Ok(value),
            _ => bail!("invalid terminal host manage response"),
        }
    }
    pub async fn attach(
        &self,
        id: &str,
        cols: u16,
        rows: u16,
        takeover: bool,
    ) -> Result<Attachment> {
        validate_size(cols, rows)?;
        let mut stream = self.connection().await?;
        send(
            &mut stream,
            &Request::Attach {
                id: id.into(),
                cols,
                rows,
                takeover,
            },
        )
        .await?;
        let replay = timeout(IO_TIMEOUT, async {
            ensure!(
                matches!(response(&mut stream).await?, Response::Attached),
                "invalid terminal host attachment"
            );
            let mut replay = Vec::new();
            let mut bytes = 0;
            loop {
                match read_frame(&mut stream).await? {
                    Frame::Bytes(data) => {
                        bytes += data.len();
                        ensure!(bytes <= 256 * 1024, "terminal host replay too large");
                        replay.push(data);
                    }
                    frame => {
                        ensure!(
                            matches!(frame.json::<Response>()?, Response::Ready),
                            "invalid terminal host replay"
                        );
                        return Ok::<_, anyhow::Error>(replay);
                    }
                }
            }
        })
        .await
        .context("terminal host attach timed out")??;
        let (commands, mut input) = mpsc::channel::<Frame>(256);
        let (events, output) = mpsc::channel(32);
        let task = tokio::spawn(async move {
            let (mut reader, mut writer) = tokio::io::split(stream);
            // Preserve partially read frames across input/output selection.
            let mut incoming = Box::pin(read_frame(&mut reader));
            loop {
                tokio::select! {
                    frame = &mut incoming => {
                        let event = match frame {
                            Ok(Frame::Bytes(data)) => Event::Output(data),
                            Ok(frame) => match frame.json::<Response>() {
                                Ok(Response::Exit { code }) => {
                                    let _ = events.send(Event::Exit(code)).await;
                                    break;
                                }
                                _ => break,
                            },
                            Err(_) => break,
                        };
                        if events.send(event).await.is_err() { break; }
                        drop(incoming);
                        incoming = Box::pin(read_frame(&mut reader));
                    }
                    command = input.recv() => {
                        let result = match command {
                            Some(Frame::Bytes(data)) => write_frame(&mut writer, 1, &data).await,
                            Some(Frame::Json(data)) => write_frame(&mut writer, 0, &data).await,
                            None => {
                                let _ = send(&mut writer, &Request::Detach).await;
                                break;
                            }
                        };
                        if result.is_err() { break; }
                    }
                }
            }
        });
        Ok(Attachment {
            replay,
            output,
            commands,
            task,
        })
    }
}

pub struct Attachment {
    pub replay: Vec<Vec<u8>>,
    pub output: mpsc::Receiver<Event>,
    commands: mpsc::Sender<Frame>,
    task: tokio::task::JoinHandle<()>,
}
impl Attachment {
    pub fn write(&self, data: Vec<u8>) -> Result<()> {
        ensure!(data.len() <= 16 * 1024, "terminal input too large");
        self.queue(Frame::Bytes(data))
    }
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        validate_size(cols, rows)?;
        self.queue(Frame::Json(serde_json::to_vec(&Request::Resize {
            cols,
            rows,
        })?))
    }
    fn queue(&self, frame: Frame) -> Result<()> {
        self.commands
            .try_send(frame)
            .map_err(|_| anyhow::anyhow!("terminal control queue full or closed"))
    }
}
impl Drop for Attachment {
    fn drop(&mut self) {
        // Closing the socket detaches even when an output consumer is stalled.
        self.task.abort();
    }
}

fn spawn(state: &Path, home: &Path) -> Result<()> {
    let mut command = Command::new(std::env::current_exe()?);
    #[cfg(not(test))]
    command
        .arg("terminal-host")
        .arg("--state")
        .arg(state)
        .arg("--home")
        .arg(home);
    #[cfg(test)]
    command
        .args([
            "--exact",
            "hosted::tests::terminal_host_process",
            "--ignored",
        ])
        .env("CRANIBOX_TEST_TERMINAL_STATE", state)
        .env("CRANIBOX_TEST_TERMINAL_HOME", home);
    // Keep the host's own errors: it outlives the node whose log would show them.
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(state.join("terminal-host.log"))
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null());
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log);
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    #[cfg(windows)]
    let child = {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::{
            CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, DETACHED_PROCESS,
        };
        let flags = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW;
        command.creation_flags(flags | CREATE_BREAKAWAY_FROM_JOB);
        command.spawn().or_else(|_| {
            command.creation_flags(flags);
            command.spawn()
        })
    };
    #[cfg(not(windows))]
    let child = command.spawn();
    let mut child = child.context("spawn terminal host")?;
    // Reap if it exits while the node is alive; dropping the node never kills it.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

struct HostFiles {
    state: PathBuf,
    endpoint: String,
    _lock: File,
}
impl Drop for HostFiles {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.state.join(RECORD));
        #[cfg(unix)]
        let _ = fs::remove_file(&self.endpoint);
    }
}

fn private_directory(state: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(state)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(state, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Only the dedicated process changes signal dispositions. PTY children reset them.
pub async fn run(state: PathBuf, home: PathBuf, version: String) -> Result<()> {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
        libc::signal(libc::SIGINT, libc::SIG_IGN);
    }
    serve(state, home, version, IDLE_TIMEOUT).await
}

async fn serve(state: PathBuf, home: PathBuf, version: String, idle: Duration) -> Result<()> {
    private_directory(&state)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(state.join("terminal-host.lock"))?;
    match fs2::FileExt::try_lock_exclusive(&lock) {
        Ok(()) => {}
        Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
            return Ok(());
        }
        Err(error) => return Err(error).context("lock terminal host"),
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::{
            JobObjects::IsProcessInJob, Threading::GetCurrentProcess,
        };
        let mut in_job = 0;
        ensure!(
            IsProcessInJob(GetCurrentProcess(), std::ptr::null_mut(), &mut in_job) != 0,
            "check terminal host Job Object"
        );
        ensure!(
            in_job == 0,
            "terminal host could not escape parent Job Object"
        );
    }
    #[cfg(unix)]
    let endpoint = state
        .join("terminal-host.sock")
        .to_str()
        .context("terminal host path is not UTF-8")?
        .to_owned();
    #[cfg(windows)]
    let endpoint = format!(r"\\.\pipe\cranibox-terminal-{}", uuid::Uuid::new_v4());
    let files = HostFiles {
        state: state.clone(),
        endpoint: endpoint.clone(),
        _lock: lock,
    };
    #[cfg(unix)]
    let listener = {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::remove_file(&endpoint);
        let listener = tokio::net::UnixListener::bind(&endpoint)?;
        fs::set_permissions(&endpoint, fs::Permissions::from_mode(0o600))?;
        listener
    };
    #[cfg(windows)]
    let mut listener = tokio::net::windows::named_pipe::ServerOptions::new()
        .first_pipe_instance(true)
        .reject_remote_clients(true)
        .create(&endpoint)?;
    let mut random = [0u8; 32];
    rand::rng().fill_bytes(&mut random);
    let token: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let record = Record {
        protocol: PROTOCOL,
        pid: std::process::id(),
        version,
        exe: std::env::current_exe()?,
        endpoint,
        token: token.clone(),
    };
    let mut temporary = tempfile::NamedTempFile::new_in(&state)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    temporary.write_all(&serde_json::to_vec(&record)?)?;
    temporary.as_file().sync_all()?;
    // Close before replacing, including Windows readers holding the old record.
    let source = temporary.into_temp_path().keep()?;
    if let Err(error) = fs::rename(&source, state.join(RECORD)) {
        let _ = fs::remove_file(source);
        return Err(error).context("publish terminal host record");
    }
    let engine = persistent::Engine::new(home);
    let mut clients = JoinSet::new();
    let mut empty_since = Some(Instant::now());
    let mut tick =
        tokio::time::interval((idle / 10).clamp(Duration::from_millis(1), Duration::from_secs(1)));
    loop {
        #[cfg(unix)]
        let accept = async { listener.accept().await.map(|(stream, _)| stream) };
        #[cfg(windows)]
        let accept = async {
            listener.connect().await?;
            let next = tokio::net::windows::named_pipe::ServerOptions::new()
                .reject_remote_clients(true)
                .create(&files.endpoint)?;
            Ok::<_, std::io::Error>(std::mem::replace(&mut listener, next))
        };
        tokio::select! {
            stream = accept => {
                let stream = stream?;
                empty_since = None;
                let engine = engine.clone();
                let token = token.clone();
                clients.spawn(async move { let _ = connection(stream, engine, &token).await; });
            }
            _ = clients.join_next(), if !clients.is_empty() => {}
            _ = tick.tick() => {
                let list = engine.manage(persistent::Manage::List).await?;
                if clients.is_empty() && list["sessions"].as_array().is_some_and(Vec::is_empty) {
                    if empty_since.get_or_insert_with(Instant::now).elapsed() >= idle { break; }
                } else { empty_since = None; }
            }
        }
    }
    drop(files);
    Ok(())
}

async fn connection(
    stream: impl AsyncRead + AsyncWrite + Unpin,
    engine: persistent::Engine,
    token: &str,
) -> Result<()> {
    let (mut reader, mut writer) = tokio::io::split(stream);
    let request = timeout(IO_TIMEOUT, async {
        let auth: Auth = read_frame(&mut reader).await?.json()?;
        ensure!(
            bool::from(auth.token.as_bytes().ct_eq(token.as_bytes())),
            "terminal host authentication failed"
        );
        let Request::Hello { protocol } = read_frame(&mut reader).await?.json()? else {
            bail!("expected terminal host hello")
        };
        send(&mut writer, &Response::Hello { protocol: PROTOCOL }).await?;
        ensure!(protocol == PROTOCOL, "incompatible terminal host protocol");
        read_frame(&mut reader).await?.json::<Request>()
    })
    .await
    .context("terminal host handshake timed out")??;
    let error = match request {
        Request::Manage { command } => match engine.manage(command).await {
            Ok(value) => return send(&mut writer, &Response::Result { value }).await,
            Err(error) => error,
        },
        Request::Attach {
            id,
            cols,
            rows,
            takeover,
        } => match engine.attach(&id, cols, rows, takeover).await {
            Ok(mut attachment) => {
                send(&mut writer, &Response::Attached).await?;
                for data in std::mem::take(&mut attachment.replay) {
                    write_frame(&mut writer, 1, &data).await?;
                }
                send(&mut writer, &Response::Ready).await?;
                let mut incoming = Box::pin(read_frame(&mut reader));
                loop {
                    tokio::select! {
                        frame = &mut incoming => {
                            match frame? {
                                Frame::Bytes(data) => attachment.write(data)?,
                                frame => match frame.json::<Request>()? {
                                    Request::Resize { cols, rows } => attachment.resize(cols, rows)?,
                                    Request::Detach => return Ok(()),
                                    _ => bail!("invalid terminal host attachment request"),
                                }
                            }
                            drop(incoming);
                            incoming = Box::pin(read_frame(&mut reader));
                        }
                        event = attachment.output.recv() => match event {
                            Some(Event::Output(data)) => write_frame(&mut writer, 1, &data).await?,
                            Some(Event::Exit(code)) => return send(&mut writer, &Response::Exit { code }).await,
                            None => return Ok(()),
                        }
                    }
                }
            }
            Err(error) => error,
        },
        _ => bail!("expected terminal host manage or attach"),
    };
    let message = if error.to_string() == "unknown session" {
        "session not found".into()
    } else {
        error.to_string()
    };
    send(&mut writer, &Response::Error { message }).await
}

/// Pruning needs only these fields, never the authentication token.
pub fn live_executable(state: &Path) -> Option<PathBuf> {
    #[derive(Deserialize)]
    struct Owner {
        pid: u32,
        exe: PathBuf,
    }
    let owner: Owner = serde_json::from_slice(&fs::read(state.join(RECORD)).ok()?).ok()?;
    if owner.pid == 0 {
        return None;
    }
    #[cfg(unix)]
    let alive = i32::try_from(owner.pid).ok().is_some_and(|pid| unsafe {
        libc::kill(pid, 0) == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    });
    #[cfg(windows)]
    let alive = unsafe {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, WAIT_TIMEOUT},
            System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
        };
        let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, owner.pid);
        if process.is_null() {
            false
        } else {
            let alive = WaitForSingleObject(process, 0) == WAIT_TIMEOUT;
            CloseHandle(process);
            alive
        }
    };
    alive.then_some(owner.exe)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::{engine::Engine, persistent::Manage};
    use std::os::unix::fs::PermissionsExt;

    #[test]
    #[ignore = "subprocess fixture, invoked by the hosted client"]
    fn terminal_host_process() {
        let state = std::env::var_os("CRANIBOX_TEST_TERMINAL_STATE")
            .unwrap()
            .into();
        let home = std::env::var_os("CRANIBOX_TEST_TERMINAL_HOME")
            .unwrap()
            .into();
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(run(state, home, "test-process".into()))
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn detached_host_survives_signals_and_restarts_after_death() {
        struct ProcessHost(tempfile::TempDir);
        impl ProcessHost {
            fn pid(&self) -> u32 {
                let record: Record =
                    serde_json::from_slice(&fs::read(self.0.path().join(RECORD)).unwrap()).unwrap();
                record.pid
            }
        }
        impl Drop for ProcessHost {
            fn drop(&mut self) {
                if let Ok(bytes) = fs::read(self.0.path().join(RECORD))
                    && let Ok(record) = serde_json::from_slice::<Record>(&bytes)
                {
                    unsafe {
                        libc::kill(record.pid as i32, libc::SIGKILL);
                    }
                }
            }
        }
        let host = ProcessHost(tempfile::tempdir().unwrap());
        let client = Client::new(host.0.path().into(), host.0.path().into())
            .await
            .unwrap();
        let id = created(&client).await;
        let pid = host.pid();
        assert_eq!(unsafe { libc::getsid(pid as i32) }, pid as i32);
        unsafe {
            libc::kill(pid as i32, libc::SIGHUP);
            libc::kill(pid as i32, libc::SIGINT);
        }
        let mut attachment = client.attach(&id, 80, 24, false).await.unwrap();
        attachment
            .write(b"printf 'signal-%s\n' survived\r".to_vec())
            .unwrap();
        expect(&mut attachment, "signal-survived").await;
        drop(attachment);
        drop(client);
        let client = Client::new(host.0.path().into(), host.0.path().into())
            .await
            .unwrap();
        assert_eq!(
            client.manage(Manage::List).await.unwrap()["sessions"][0]["id"],
            id
        );
        assert_eq!(host.pid(), pid);
        // A live Client must refresh the record and launch a new host after a crash.
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
        timeout(IO_TIMEOUT, async {
            while live_executable(host.0.path()).is_some() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(
            client.manage(Manage::List).await.unwrap()["sessions"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_ne!(host.pid(), pid);
        assert_eq!(
            client
                .attach(&id, 80, 24, false)
                .await
                .err()
                .unwrap()
                .to_string(),
            "session not found"
        );
    }

    struct Host {
        root: tempfile::TempDir,
        task: tokio::task::JoinHandle<Result<()>>,
    }
    impl Host {
        async fn start(idle: Duration) -> Self {
            let root = tempfile::tempdir().unwrap();
            let state = root.path().to_owned();
            let task = tokio::spawn(serve(state.clone(), state, "test".into(), idle));
            let host = Self { root, task };
            timeout(IO_TIMEOUT, async {
                while !host.root.path().join(RECORD).exists() {
                    assert!(
                        !host.task.is_finished(),
                        "host exited before publishing its endpoint"
                    );
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            host
        }
        async fn client(&self) -> Client {
            Client::new(self.root.path().into(), self.root.path().into())
                .await
                .unwrap()
        }
        fn record(&self) -> Record {
            serde_json::from_slice(&fs::read(self.root.path().join(RECORD)).unwrap()).unwrap()
        }
    }
    impl Drop for Host {
        fn drop(&mut self) {
            self.task.abort();
        }
    }
    async fn expect(attachment: &mut Attachment, marker: &str) {
        timeout(IO_TIMEOUT, async {
            let mut data = String::new();
            while let Some(event) = attachment.output.recv().await {
                if let Event::Output(bytes) = event {
                    data.push_str(&String::from_utf8_lossy(&bytes));
                    if data.contains(marker) {
                        return;
                    }
                }
            }
            panic!("attachment closed before marker");
        })
        .await
        .unwrap();
    }
    async fn created(client: &Client) -> String {
        client
            .manage(Manage::Create {
                name: "persistent".into(),
                cwd: None,
            })
            .await
            .unwrap()["session"]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn restart_replay_input_and_takeover_match_native() {
        let host = Host::start(Duration::from_secs(60)).await;
        let a = host.client().await;
        let id = created(&a).await;
        let mut first = a.attach(&id, 80, 24, false).await.unwrap();
        first.write(b"echo marker-1\r".to_vec()).unwrap();
        // Also prove the shell executed input, rather than only echoing it.
        first
            .write(b"export CB_HOST_VALUE=kept; printf 'marker-%s\n' done\r".to_vec())
            .unwrap();
        expect(&mut first, "marker-done").await;
        drop(first);
        drop(a);
        // Wait for EOF to pass through the host's detach actor.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let b = host.client().await;
        assert_eq!(
            b.manage(Manage::List).await.unwrap()["sessions"][0]["id"],
            id
        );
        let mut second = b.attach(&id, 100, 30, false).await.unwrap();
        let replay: Vec<_> = second.replay.iter().flatten().copied().collect();
        assert!(String::from_utf8_lossy(&replay).contains("marker-1"));
        second
            .write(b"printf 'value:%s\n' \"$CB_HOST_VALUE\"\r".to_vec())
            .unwrap();
        expect(&mut second, "value:kept").await;
        let error = b.attach(&id, 80, 24, false).await.err().unwrap();
        assert_eq!(
            error.to_string(),
            "session already attached; take over to reconnect"
        );
        let mut third = b.attach(&id, 80, 24, true).await.unwrap();
        timeout(IO_TIMEOUT, async {
            while second.output.recv().await.is_some() {}
        })
        .await
        .unwrap();
        third.resize(120, 40).unwrap();
        third
            .write(b"stty size; printf 'after-%s\n' takeover\r".to_vec())
            .unwrap();
        expect(&mut third, "40 120").await;
        b.manage(Manage::Rename {
            id: id.clone(),
            name: "renamed".into(),
        })
        .await
        .unwrap();
        assert_eq!(
            b.manage(Manage::List).await.unwrap()["sessions"][0]["name"],
            "renamed"
        );
        // Hosted shutdown drops no sessions.
        let engine = Engine::Hosted(b.clone());
        engine.shutdown().await;
        assert_eq!(
            engine.manage(Manage::List).await.unwrap()["sessions"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        b.manage(Manage::End { id }).await.unwrap();
        assert!(
            b.manage(Manage::List).await.unwrap()["sessions"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn auth_permissions_and_single_instance() {
        let host = Host::start(Duration::from_secs(60)).await;
        for (path, mode) in [
            (host.root.path().to_owned(), 0o700),
            (host.root.path().join(RECORD), 0o600),
            (host.root.path().join("terminal-host.sock"), 0o600),
        ] {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                mode
            );
        }
        let record = host.record();
        assert_eq!(record.token.len(), 64);
        let mut stream = open(&record.endpoint).await.unwrap();
        send(
            &mut stream,
            &Auth {
                token: "0".repeat(64),
            },
        )
        .await
        .unwrap();
        assert!(
            timeout(IO_TIMEOUT, read_frame(&mut stream))
                .await
                .unwrap()
                .is_err()
        );
        serve(
            host.root.path().into(),
            host.root.path().into(),
            "second".into(),
            Duration::ZERO,
        )
        .await
        .unwrap();
        // The losing host must not remove/replace the first host's record.
        assert_eq!(host.record().version, "test");
        host.client().await.manage(Manage::List).await.unwrap();
    }

    #[tokio::test]
    async fn incompatible_protocol_falls_back_without_replacing_host() {
        let host = Host::start(Duration::from_secs(60)).await;
        let record = host.record();
        let mut stream = open(&record.endpoint).await.unwrap();
        send(
            &mut stream,
            &Auth {
                token: record.token,
            },
        )
        .await
        .unwrap();
        send(
            &mut stream,
            &Request::Hello {
                protocol: PROTOCOL + 1,
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            response(&mut stream).await.unwrap(),
            Response::Hello { protocol: PROTOCOL }
        ));
        assert!(read_frame(&mut stream).await.is_err());
        // Model an old host record. The production engine chooses Native.
        let mut record = host.record();
        record.protocol += 1;
        fs::write(
            host.root.path().join(RECORD),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            Engine::hosted(host.root.path().into(), host.root.path().into()).await,
            Engine::Native(_)
        ));
        assert!(!host.task.is_finished());
    }

    #[tokio::test]
    async fn idle_exit_waits_for_both_sessions_and_clients() {
        let host = Host::start(Duration::from_millis(100)).await;
        let client = host.client().await;
        let id = created(&client).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!host.task.is_finished());
        let mut connection = client.existing().await.unwrap();
        client.manage(Manage::Remove { id }).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!host.task.is_finished());
        send(
            &mut connection,
            &Request::Manage {
                command: Manage::List,
            },
        )
        .await
        .unwrap();
        response(&mut connection).await.unwrap();
        drop(connection);
        timeout(IO_TIMEOUT, async {
            while !host.task.is_finished() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(!host.root.path().join(RECORD).exists());
        assert!(!host.root.path().join("terminal-host.sock").exists());
    }

    #[tokio::test]
    async fn reconnect_reads_new_record_and_old_ids_are_not_found() {
        let host = Host::start(Duration::from_secs(60)).await;
        let client = host.client().await;
        let id = created(&client).await;
        host.task.abort();
        while !host.task.is_finished() {
            tokio::task::yield_now().await;
        }
        let state = host.root.path().to_owned();
        let replacement = tokio::spawn(serve(
            state.clone(),
            state,
            "replacement".into(),
            Duration::from_secs(60),
        ));
        timeout(IO_TIMEOUT, async {
            while !host.root.path().join(RECORD).exists() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            client.manage(Manage::List).await.unwrap()["sessions"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            client
                .attach(&id, 80, 24, false)
                .await
                .err()
                .unwrap()
                .to_string(),
            "session not found"
        );
        replacement.abort();
        let _ = replacement.await;
    }

    #[tokio::test]
    async fn frames_are_bounded_and_fragmentation_survives_output_selection() {
        let (mut writer, mut reader) = tokio::io::duplex(64);
        writer.write_u32(MAX_FRAME as u32 + 1).await.unwrap();
        assert!(read_frame(&mut reader).await.is_err());
        writer.write_u32(0).await.unwrap();
        assert!(read_frame(&mut reader).await.is_err());
        // A partial frame remains pending while another branch is selected.
        writer.write_all(&[0, 0]).await.unwrap();
        let mut pending = Box::pin(read_frame(&mut reader));
        tokio::select! {
            _ = &mut pending => panic!("partial header must wait"),
            _ = tokio::time::sleep(Duration::from_millis(5)) => {}
        }
        writer.write_all(&[0, 3, 1, b'o', b'k']).await.unwrap();
        assert!(matches!(pending.await.unwrap(), Frame::Bytes(bytes) if bytes == b"ok"));
    }
}
