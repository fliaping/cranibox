//! Shared PTY transport, used by the server and the independent node client.
use anyhow::{Result, ensure};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::{
    io::{Read, Write},
    path::PathBuf,
    sync::mpsc,
};
use tokio::sync::mpsc as async_mpsc;
pub mod herdr;

pub enum Event {
    Output(Vec<u8>),
    Exit(u32),
}
pub struct Session {
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    pid: Option<u32>,
    #[cfg(windows)]
    _job: Job,
    input: Option<mpsc::SyncSender<Vec<u8>>>,
    pub output: async_mpsc::Receiver<Event>,
}
pub fn validate_size(cols: u16, rows: u16) -> Result<()> {
    ensure!(
        (2..=500).contains(&cols) && (2..=300).contains(&rows),
        "terminal size out of range"
    );
    Ok(())
}
impl Session {
    pub fn open(cols: u16, rows: u16, cwd: Option<PathBuf>, home: Option<PathBuf>) -> Result<Self> {
        validate_size(cols, rows)?;
        #[cfg(windows)]
        let shell = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into());
        #[cfg(not(windows))]
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|s| std::path::Path::new(s).is_absolute() && std::path::Path::new(s).is_file())
            .unwrap_or_else(|| "/bin/sh".into());
        let mut command = CommandBuilder::new(shell);
        #[cfg(not(windows))]
        command.arg("-i");
        command.env("TERM", "xterm-256color");
        if let Some(home) = home {
            command.env("HOME", home);
        }
        if let Some(cwd) = cwd {
            ensure!(cwd.is_dir(), "working directory does not exist");
            command.cwd(cwd);
        }
        Self::command(cols, rows, command)
    }
    /// The command must attach to an independently started engine. Drop only
    /// terminates this client's PTY session / Windows Job, never that engine.
    pub fn command(cols: u16, rows: u16, command: CommandBuilder) -> Result<Self> {
        validate_size(cols, rows)?;
        let pair = native_pty_system().openpty(PtySize {
            cols,
            rows,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let mut reader = pair.master.try_clone_reader()?;
        // portable-pty's Unix writer sends newline + Ctrl+D when dropped.
        // An attachment must close without forwarding EOF into the persistent shell.
        #[cfg(unix)]
        let mut writer = {
            use std::os::fd::BorrowedFd;
            let fd = pair
                .master
                .as_raw_fd()
                .ok_or_else(|| anyhow::anyhow!("PTY has no file descriptor"))?;
            let owned = unsafe { BorrowedFd::borrow_raw(fd) }.try_clone_to_owned()?;
            std::fs::File::from(owned)
        };
        #[cfg(not(unix))]
        let mut writer = pair.master.take_writer()?;
        let mut child = pair.slave.spawn_command(command)?;
        #[cfg(windows)]
        let job = match Job::new(child.as_raw_handle()) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                return Err(error);
            }
        };
        drop(pair.slave);
        let killer = child.clone_killer();
        let pid = child.process_id();
        let (input, commands) = mpsc::sync_channel::<Vec<u8>>(32);
        let (sender, output) = async_mpsc::channel(32);
        std::thread::spawn(move || {
            for data in commands {
                if writer
                    .write_all(&data)
                    .and_then(|_| writer.flush())
                    .is_err()
                {
                    break;
                }
            }
        });
        std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        if sender
                            .blocking_send(Event::Output(buffer[..count].to_vec()))
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
            let code = child.wait().map(|s| s.exit_code()).unwrap_or(1);
            let _ = sender.blocking_send(Event::Exit(code));
        });
        Ok(Self {
            master: pair.master,
            killer,
            pid,
            #[cfg(windows)]
            _job: job,
            input: Some(input),
            output,
        })
    }
    pub fn write(&self, data: Vec<u8>) -> Result<()> {
        ensure!(data.len() <= 16 * 1024, "terminal input too large");
        self.input
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("terminal closed"))?
            .try_send(data)
            .map_err(|_| anyhow::anyhow!("terminal input queue full or closed"))
    }
    /// Wait until the external client has enabled raw input after its handshake.
    pub async fn ready(&mut self) -> Result<()> {
        #[cfg(unix)]
        if let Some(fd) = self.master.as_raw_fd() {
            for _ in 0..500 {
                let mut attributes = std::mem::MaybeUninit::<libc::termios>::uninit();
                let result = unsafe { libc::tcgetattr(fd, attributes.as_mut_ptr()) };
                ensure!(result == 0, "terminal client closed during startup");
                let attributes = unsafe { attributes.assume_init() };
                if attributes.c_lflag & (libc::ICANON | libc::ISIG) == 0 {
                    return Ok(());
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            anyhow::bail!("herdr attachment did not become ready");
        }
        Ok(())
    }
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        validate_size(cols, rows)?;
        self.master.resize(PtySize {
            cols,
            rows,
            pixel_width: 0,
            pixel_height: 0,
        })
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.input.take();
        self.output.close();
        #[cfg(unix)]
        if let Some(pid) = self.pid {
            kill_session(pid as i32);
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
        #[cfg(not(unix))]
        let _ = self.pid;
        let _ = self.killer.kill();
    }
}

// Interactive shells put foreground and background jobs in distinct process groups.
// End all members of their PTY session, including a foreground command at disconnect.
#[cfg(unix)]
fn kill_session(session: i32) {
    #[cfg(target_os = "linux")]
    let processes: Vec<i32> = std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
        .collect();
    #[cfg(not(target_os = "linux"))]
    let processes: Vec<i32> = std::process::Command::new("/bin/ps")
        .args(["-A", "-o", "pid="])
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .filter_map(|p| p.parse().ok())
                .collect()
        })
        .unwrap_or_default();
    for pid in processes {
        if pid != session && unsafe { libc::getsid(pid) } == session {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
}

#[cfg(windows)]
struct Job(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
unsafe impl Send for Job {}
#[cfg(windows)]
impl Job {
    fn new(process: Option<std::os::windows::io::RawHandle>) -> Result<Self> {
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            ensure!(!job.is_null(), "cannot create terminal Job Object");
            let job = Self(job);
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            ensure!(
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    std::mem::size_of_val(&info) as u32
                ) != 0,
                "cannot configure terminal Job Object"
            );
            ensure!(
                AssignProcessToJobObject(
                    job.0,
                    process.ok_or_else(|| anyhow::anyhow!("missing terminal process handle"))? as _
                ) != 0,
                "cannot assign terminal Job Object"
            );
            Ok(job)
        }
    }
}
#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
