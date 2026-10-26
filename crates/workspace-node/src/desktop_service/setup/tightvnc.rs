//! The pinned vendor download is never redistributed or executed before verification.
use serde_json::{Value, json};

pub(super) const URL: &str =
    "https://www.tightvnc.com/download/2.8.90/tightvnc-2.8.90-gpl-setup-64bit.msi";
const SIZE: u64 = 2543616;
const SHA256: &str = "9693cb4cdc2e7e482e8e3f5890415b57ef87de3369fa5c337db516218a1170ce";

fn arguments(msi: &str, password: &str, control: &str) -> String {
    // Both passwords are generated alphanumeric strings; the path is a Windows
    // filename (no double quotes) and always ends in .msi, never a backslash.
    format!(
        "/i \"{msi}\" /quiet /norestart ADDLOCAL=Server SERVER_REGISTER_AS_SERVICE=1 SERVER_ADD_FIREWALL_EXCEPTION=0 SERVER_ALLOW_SAS=1 SET_USEVNCAUTHENTICATION=1 VALUE_OF_USEVNCAUTHENTICATION=1 SET_PASSWORD=1 VALUE_OF_PASSWORD={password} SET_USECONTROLAUTHENTICATION=1 VALUE_OF_USECONTROLAUTHENTICATION=1 SET_CONTROLPASSWORD=1 VALUE_OF_CONTROLPASSWORD={control} SET_ALLOWLOOPBACK=1 VALUE_OF_ALLOWLOOPBACK=1 SET_LOOPBACKONLY=1 VALUE_OF_LOOPBACKONLY=1 SET_ACCEPTHTTPCONNECTIONS=1 VALUE_OF_ACCEPTHTTPCONNECTIONS=0 SET_RFBPORT=1 VALUE_OF_RFBPORT=5900"
    )
}
pub(crate) fn review() -> Value {
    json!({
        "commands":format!("msiexec.exe {}", arguments("<verified installer in node state directory>", "********", "********")),
        "administrator":true,"package":"TightVNC Server","source":"https://www.tightvnc.com/",
        "license":"GPL-2.0","sha256":SHA256,"size":SIZE,"url":URL,
        "summary":"Download the pinned official TightVNC 2.8.90 installer and verify its size and SHA-256. Windows asks for administrator approval on this device. Installs the auto-start TightVNC service listening on loopback only with a generated password, without a firewall exception or HTTP listener. This does not enable sharing."
    })
}

#[cfg(any(windows, test))]
mod transaction {
    use anyhow::{Result, ensure};
    use sha2::{Digest, Sha256};
    use std::{
        future::Future,
        io::Read,
        path::{Path, PathBuf},
    };

    pub(super) fn exit_code(code: Option<i32>) -> Result<bool> {
        match code {
            Some(0) => Ok(false),
            Some(3010) => Ok(true),
            Some(1223 | 1602) => {
                anyhow::bail!("Administrator approval was declined on this device.")
            }
            code => anyhow::bail!(
                "TightVNC msiexec failed (exit {}).",
                code.map_or("unknown".into(), |c| c.to_string())
            ),
        }
    }
    fn verify(path: &Path, size: u64, sha256: &str) -> Result<()> {
        let file = std::fs::File::open(path)?;
        ensure!(
            file.metadata()?.len() == size,
            "TightVNC installer size verification failed."
        );
        let mut hash = Sha256::new();
        // Bound reads too, in case the file changes after metadata was checked.
        let copied = std::io::copy(&mut file.take(size + 1), &mut hash)?;
        ensure!(
            copied == size,
            "TightVNC installer size verification failed."
        );
        ensure!(
            format!("{:x}", hash.finalize()) == sha256,
            "TightVNC installer SHA-256 verification failed."
        );
        Ok(())
    }
    pub(super) async fn verified_install<D, DF, R, RF>(
        state: &Path,
        size: u64,
        sha256: &str,
        download: D,
        run: R,
    ) -> Result<bool>
    where
        D: FnOnce(PathBuf) -> DF,
        DF: Future<Output = Result<()>>,
        R: FnOnce(PathBuf) -> RF,
        RF: Future<Output = Result<Option<i32>>>,
    {
        let file = tempfile::Builder::new()
            .prefix("desktop-tightvnc-")
            .suffix(".msi")
            .tempfile_in(state)?;
        let path = file.into_temp_path();
        download(path.to_path_buf()).await?;
        verify(&path, size, sha256)?;
        // TempPath deletes the download on success, error, or cancellation.
        exit_code(run(path.to_path_buf()).await?)
    }
    /// msiexec cannot open `\\?\` verbatim paths (exit 1619), which canonicalized
    /// Windows paths use.
    pub(super) fn plain_path(path: &str) -> String {
        if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{rest}")
        } else {
            path.strip_prefix(r"\\?\").unwrap_or(path).to_owned()
        }
    }
    /// The error lines of a verbose msiexec log, without the generated passwords.
    pub(super) fn log_errors(bytes: &[u8], secrets: &[&str]) -> String {
        let text = match bytes {
            [0xff, 0xfe, rest @ ..] => String::from_utf16_lossy(
                &rest
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_le_bytes(*c))
                    .collect::<Vec<_>>(),
            ),
            _ => String::from_utf8_lossy(bytes).into_owned(),
        };
        let redact = |line: &str| {
            secrets.iter().fold(line.trim().to_owned(), |line, secret| {
                line.replace(secret, "********")
            })
        };
        let mut lines: Vec<String> = text
            .lines()
            .filter(|line| {
                let line = line.to_ascii_lowercase();
                ["error", "note:", "return value 3", "returning 16"]
                    .iter()
                    .any(|needle| line.contains(needle))
            })
            .map(redact)
            .collect();
        if lines.is_empty() {
            // Unknown failure shape: the end of the log usually names it.
            lines = text
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(redact)
                .collect();
        }
        lines.dedup();
        let skip = lines.len().saturating_sub(6);
        lines.drain(..skip);
        lines.join(" | ").chars().take(1500).collect()
    }
    pub(super) fn script(msi: &str, log: &str, password: &str, control: &str) -> String {
        let args = format!(
            "{} /l*v \"{log}\"",
            super::arguments(msi, password, control)
        )
        .replace('\'', "''");
        // Start-Process joins ArgumentList itself: pass one correctly quoted
        // Windows command line, inside a literal PowerShell string. A BOM makes
        // Windows PowerShell 5 read Unicode state-directory paths as UTF-8.
        format!(
            "\u{feff}$ErrorActionPreference = 'Stop'\ntry {{\n  $arguments = '{args}'\n  $p = Start-Process -FilePath msiexec.exe -Verb RunAs -Wait -PassThru -ArgumentList $arguments\n  exit $p.ExitCode\n}} catch {{ exit 1223 }}\n"
        )
    }
}

#[cfg(windows)]
pub(super) async fn install(journal: &std::path::Path) -> anyhow::Result<()> {
    use super::{Phase, Provider, Record};
    use agent_workspace_runtime::config as secrets;
    use anyhow::{Context, ensure};
    use std::{io::Write, process::Stdio, time::Duration};
    use tokio::process::Command;

    let state = journal.parent().context("Missing node state directory")?;
    let password_path = journal.with_file_name("desktop-vnc.password");
    let password = super::super::password(&password_path)?;
    secrets::write_private(&password_path, format!("{password}\n").as_bytes(), true)?;
    let control = super::super::new_password();
    let log = tempfile::Builder::new()
        .prefix("desktop-tightvnc-")
        .suffix(".log")
        .tempfile_in(state)?
        .into_temp_path();
    let log_path = transaction::plain_path(log.to_str().context("Invalid installer log path")?);
    let (password_log, control_log) = (password.clone(), control.clone());
    let log_argument = log_path.clone();
    let result = transaction::verified_install(state, SIZE, SHA256,
        |path| async move {
            let client = reqwest::Client::builder().https_only(true)
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(300)).build()?;
            let mut response = client.get(URL).send().await?.error_for_status()?;
            ensure!(response.content_length().is_none_or(|n| n == SIZE), "TightVNC installer size verification failed.");
            let mut file = std::fs::OpenOptions::new().write(true).truncate(true).open(path)?;
            let mut size = 0u64;
            while let Some(chunk) = response.chunk().await? {
                size += chunk.len() as u64;
                ensure!(size <= SIZE, "TightVNC installer exceeds the pinned size limit.");
                file.write_all(&chunk)?;
            }
            file.sync_all()?;
            Ok(())
        },
        |path| async move {
            let script = tempfile::Builder::new().prefix("desktop-tightvnc-").suffix(".ps1").tempfile_in(state)?.into_temp_path();
            let msi = transaction::plain_path(path.to_str().context("Invalid installer path")?);
            let contents = transaction::script(&msi, &log_argument, &password, &control);
            secrets::write_private(&script, contents.as_bytes(), true)?;
            let mut child = Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"]).arg(&script)
                .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
                .kill_on_drop(false).spawn().context("Open Windows administrator approval")?;
            // Never terminate an elevated installer, including on timeout/shutdown.
            // Start-Process has already consumed the script before monitoring it.
            let status = tokio::time::timeout(Duration::from_secs(900), child.wait()).await
                .context("TightVNC installation monitoring timed out. The Windows installer may still be running; check Windows Apps and Services before retrying.")??;
            Ok(status.code())
        }).await;
    // The verbose log may hold the passwords; keep only redacted error lines.
    let details = std::fs::read(&log)
        .map(|bytes| transaction::log_errors(&bytes, &[&password_log, &control_log]))
        .unwrap_or_default();
    drop(log);
    let reboot = match result {
        Err(error) => anyhow::bail!(
            "{error:#} Installer log {}: {}",
            log_path,
            if details.is_empty() {
                "empty or missing"
            } else {
                details.as_str()
            }
        ),
        result => result?,
    };
    let record = Record {
        phase: Phase::Running,
        provider: Some(Provider::Tightvnc),
        tightvnc_installed: Some(true),
        note: reboot.then(|| "restart Windows to finish".into()),
        error: None,
    };
    secrets::write_private(journal, &serde_json::to_vec(&record)?, true)?;
    ensure!(
        super::executable(Provider::Tightvnc).is_some(),
        "The Windows installer finished but tvnserver.exe is missing. Check Windows Apps settings."
    );
    super::super::wait_for_tightvnc("127.0.0.1:5900".parse()?, Duration::from_secs(30)).await
}

#[cfg(test)]
mod tests {
    use super::{transaction::*, *};
    use sha2::{Digest, Sha256};
    use std::{cell::RefCell, rc::Rc};

    #[test]
    fn arguments_review_and_powershell_quoting() {
        let args = arguments(
            r"C:\Users\O'Brien $x\桌面\setup.msi",
            "abc123XY",
            "other123",
        );
        assert!(args.contains(r#"/i "C:\Users\O'Brien $x\桌面\setup.msi" /quiet /norestart"#));
        for setting in [
            "ADDLOCAL=Server",
            "SERVER_REGISTER_AS_SERVICE=1",
            "SERVER_ADD_FIREWALL_EXCEPTION=0",
            "SERVER_ALLOW_SAS=1",
            "SET_PASSWORD=1 VALUE_OF_PASSWORD=abc123XY",
            "SET_CONTROLPASSWORD=1 VALUE_OF_CONTROLPASSWORD=other123",
            "SET_LOOPBACKONLY=1 VALUE_OF_LOOPBACKONLY=1",
            "SET_ALLOWLOOPBACK=1 VALUE_OF_ALLOWLOOPBACK=1",
            "SET_ACCEPTHTTPCONNECTIONS=1 VALUE_OF_ACCEPTHTTPCONNECTIONS=0",
            "SET_RFBPORT=1 VALUE_OF_RFBPORT=5900",
        ] {
            assert!(args.contains(setting), "{setting}");
        }
        let review = review();
        assert_eq!(review["license"], "GPL-2.0");
        assert_eq!(review["sha256"], SHA256);
        assert_eq!(review["administrator"], true);
        let commands = review["commands"].as_str().unwrap();
        assert_eq!(commands.matches("********").count(), 2);
        assert!(!commands.contains("abc123XY") && !commands.contains("other123"));
        let script = script(
            r"C:\O'Brien $x\setup.msi",
            r"C:\O'Brien $x\setup.log",
            "abc123XY",
            "other123",
        );
        assert!(script.contains("O''Brien $x"));
        assert!(script.contains(r#"/l*v "C:\O''Brien $x\setup.log"'"#));
        assert!(script.contains("-Verb RunAs -Wait -PassThru"));
        assert!(script.contains("catch { exit 1223 }"));
    }
    #[test]
    fn verbatim_paths_and_redacted_log_errors() {
        assert_eq!(plain_path(r"\\?\C:\Users\a\x.msi"), r"C:\Users\a\x.msi");
        assert_eq!(
            plain_path(r"\\?\UNC\host\share\x.msi"),
            r"\\host\share\x.msi"
        );
        assert_eq!(plain_path(r"C:\x.msi"), r"C:\x.msi");
        let log = "Action start\r\nProperty(C): VALUE_OF_PASSWORD = abc123XY\r\nNote: 1: 1309 2: 5\r\nMSI (c) Error 1619 opening abc123XY\r\n";
        let mut utf16 = vec![0xff, 0xfe];
        utf16.extend(log.encode_utf16().flat_map(u16::to_le_bytes));
        for bytes in [log.as_bytes().to_vec(), utf16] {
            let errors = log_errors(&bytes, &["abc123XY"]);
            assert_eq!(
                errors,
                "Note: 1: 1309 2: 5 | MSI (c) Error 1619 opening ********"
            );
        }
    }
    #[test]
    fn installer_exit_codes() {
        assert!(!exit_code(Some(0)).unwrap());
        assert!(exit_code(Some(3010)).unwrap());
        for code in [1223, 1602] {
            assert_eq!(
                exit_code(Some(code)).unwrap_err().to_string(),
                "Administrator approval was declined on this device."
            );
        }
        assert!(
            exit_code(Some(1603))
                .unwrap_err()
                .to_string()
                .contains("1603")
        );
        assert!(exit_code(None).is_err());
    }
    #[tokio::test]
    async fn verification_and_download_failures_delete_without_executing() {
        let root = tempfile::tempdir().unwrap();
        let digest = format!("{:x}", Sha256::digest(b"good"));
        for (contents, fail, expected) in [
            (b"good".as_slice(), false, true),
            (b"bad!", false, false),
            (b"bad", false, false),
            (b"extra", false, false),
            (b"good", true, false),
        ] {
            let path = Rc::new(RefCell::new(None));
            let downloaded = path.clone();
            let ran = Rc::new(RefCell::new(false));
            let marker = ran.clone();
            let result = verified_install(
                root.path(),
                4,
                &digest,
                |p| async move {
                    std::fs::write(&p, contents)?;
                    *downloaded.borrow_mut() = Some(p);
                    anyhow::ensure!(!fail, "download failed");
                    Ok(())
                },
                |_| async move {
                    *marker.borrow_mut() = true;
                    Ok(Some(0))
                },
            )
            .await;
            assert_eq!(result.is_ok(), expected);
            assert_eq!(*ran.borrow(), expected);
            assert!(!path.borrow().as_ref().unwrap().exists());
            assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        }
    }
}
