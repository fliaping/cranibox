use anyhow::{Context, Result, ensure};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, process::Command};

/// Never collect unbounded output, inherit stdin, or leave a timed-out child alive.
pub(super) async fn output(program: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("Start network command")?;
    let stdout = child.stdout.take().context("Read network command")?;
    let result = tokio::time::timeout(Duration::from_secs(3), async {
        let mut bytes = Vec::new();
        stdout.take(256 * 1024 + 1).read_to_end(&mut bytes).await?;
        ensure!(
            bytes.len() <= 256 * 1024,
            "Network command output exceeded its limit."
        );
        let status = child.wait().await?;
        ensure!(
            status.success(),
            "Network command failed; check that its service is running and accessible."
        );
        Ok::<_, anyhow::Error>(bytes)
    })
    .await;
    match result {
        Ok(Ok(bytes)) => Ok(bytes),
        other => {
            let _ = child.kill().await;
            match other {
                Ok(Err(e)) => Err(e),
                Err(_) => anyhow::bail!("Network command timed out."),
                _ => unreachable!(),
            }
        }
    }
}
