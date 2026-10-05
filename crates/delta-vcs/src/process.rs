//! Running git with bounded output.

use std::process::{ExitStatus, Stdio};

use delta_core::{DeltaError, Result};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

/// Output of [`output_capped`].
pub(crate) struct CappedOutput {
    pub stdout: Vec<u8>,
    /// The command wrote more than was kept, and was killed.
    pub truncated: bool,
    pub status: ExitStatus,
    /// The start of stderr, for logging.
    pub stderr: String,
}

/// Run `command`, keeping at most `max_bytes` of its stdout, so commands
/// whose output depends on repository content (a huge file, diff or commit
/// message) can't exhaust memory.
pub(crate) async fn output_capped(command: &mut Command, max_bytes: usize) -> Result<CappedOutput> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| DeltaError::Storage(format!("failed to run git: {e}")))?;
    let (Some(stdout), Some(mut stderr)) = (child.stdout.take(), child.stderr.take()) else {
        return Err(DeltaError::Storage("git pipes unavailable".into()));
    };

    // Drain stderr separately so git never blocks writing it.
    let stderr_task = tokio::spawn(async move {
        let mut kept = Vec::new();
        let _ = (&mut stderr).take(64 * 1024).read_to_end(&mut kept).await;
        let _ = tokio::io::copy(&mut stderr, &mut tokio::io::sink()).await;
        String::from_utf8_lossy(&kept).into_owned()
    });

    let mut out = Vec::new();
    stdout
        .take(max_bytes as u64 + 1)
        .read_to_end(&mut out)
        .await
        .map_err(|e| DeltaError::Storage(format!("failed to read git output: {e}")))?;
    let truncated = out.len() > max_bytes;
    if truncated {
        out.truncate(max_bytes);
        let _ = child.start_kill();
    }
    let status = child
        .wait()
        .await
        .map_err(|e| DeltaError::Storage(format!("failed to wait for git: {e}")))?;
    let stderr = stderr_task.await.unwrap_or_default();
    Ok(CappedOutput {
        stdout: out,
        truncated,
        status,
        stderr,
    })
}
