use thiserror::Error;

use crate::error::ZfsError;
use crate::runner::{ChildHandle, Cmd, CommandRunner};

#[derive(Error, Debug)]
pub enum RecvError {
    /// `zfs recv` was interrupted mid-stream. The embedded `token` can be
    /// passed to `SendArgs::resume_token()` to generate a resuming stream
    /// on the sender. The partially received snapshot is preserved.
    #[error("receive interrupted; resume with: zfs send -t {token}")]
    NeedsResumeToken { token: String },

    #[error(transparent)]
    Zfs(#[from] ZfsError),
}

/// Arguments for a `zfs recv` invocation.
#[derive(Debug, Clone)]
pub struct RecvArgs {
    /// Destination dataset or snapshot.
    pub target: String,
    /// `-F` — force rollback of the target to the most recent snapshot if
    /// needed to receive the stream.
    pub force_rollback: bool,
    /// `-u` — do not mount the received filesystem after receive.
    pub unmounted: bool,
    /// `-d` — discard the first component of the sending dataset's name, using
    /// the target dataset as the root.
    pub discard_first_component: bool,
    /// `-e` — strip everything but the last component of the sending dataset's
    /// name, appending it to the target.
    pub exclude_first_component: bool,
}

impl RecvArgs {
    pub fn new(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            force_rollback: false,
            unmounted: false,
            discard_first_component: false,
            exclude_first_component: false,
        }
    }

    pub fn force_rollback(mut self) -> Self {
        self.force_rollback = true;
        self
    }
    pub fn unmounted(mut self) -> Self {
        self.unmounted = true;
        self
    }

    fn build_args(&self) -> Vec<String> {
        let mut args = vec!["recv".to_string()];
        if self.force_rollback {
            args.push("-F".to_string());
        }
        if self.unmounted {
            args.push("-u".to_string());
        }
        if self.discard_first_component {
            args.push("-d".to_string());
        }
        if self.exclude_first_component {
            args.push("-e".to_string());
        }
        args.push(self.target.clone());
        args
    }
}

/// `zfs recv [flags] <target>` — spawn the receive process and return a
/// [`ChildHandle`]. Callers write the byte stream to `child.stdin`, close it,
/// then read `child.stderr` and call `check_recv_stderr()` to detect resume
/// tokens before calling `child.wait()`.
pub async fn recv(runner: &dyn CommandRunner, args: &RecvArgs) -> Result<ChildHandle, RecvError> {
    runner
        .spawn(Cmd::new("zfs").args(args.build_args()))
        .await
        .map_err(|e| RecvError::Zfs(ZfsError::Spawn(e)))
}

/// Parse `zfs recv` stderr for a `NeedsResumeToken` condition. Call this
/// after recv exits with a non-zero status and you have collected all stderr
/// output. Returns `Ok(())` on clean exit, `Err(RecvError::NeedsResumeToken)`
/// when the interrupted-stream marker is found.
pub fn check_recv_stderr(stderr: &str) -> Result<(), RecvError> {
    if !stderr.contains("checksum mismatch or incomplete stream") {
        return Ok(());
    }
    for line in stderr.lines() {
        // The hint line is always "    zfs send -t <token>" (4-space indent).
        if let Some(rest) = line.strip_prefix("    zfs send -t ") {
            return Err(RecvError::NeedsResumeToken {
                token: rest.trim().to_string(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{Cmd, RecordingRunner};

    #[test]
    fn check_recv_stderr_clean() {
        assert!(check_recv_stderr("").is_ok());
        assert!(check_recv_stderr("receive complete.\n").is_ok());
    }

    #[test]
    fn check_recv_stderr_needs_resume_token() {
        let fixture = std::fs::read(format!(
            "{}/tests/fixtures/send_recv_interrupted.stderr",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let text = String::from_utf8_lossy(&fixture);
        let err = check_recv_stderr(&text).unwrap_err();
        let RecvError::NeedsResumeToken { token } = err else {
            panic!("expected NeedsResumeToken, got {err:?}");
        };
        // The token is the long hex string from the fixture
        assert!(token.starts_with("1-"), "token should start with '1-', got: {token}");
        assert!(token.len() > 10, "token should be long, got len={}", token.len());
    }

    #[test]
    fn build_args_defaults() {
        let args = RecvArgs::new("tank/replica");
        assert_eq!(args.build_args(), vec!["recv", "tank/replica"]);
    }

    #[test]
    fn build_args_flags() {
        let args = RecvArgs::new("tank/replica").force_rollback().unmounted();
        assert_eq!(
            args.build_args(),
            vec!["recv", "-F", "-u", "tank/replica"]
        );
    }

    #[tokio::test]
    async fn recv_spawns_correct_args() {
        use tokio::io::AsyncWriteExt;

        let runner = RecordingRunner::new().record_spawn(
            Cmd::new("zfs").args(["recv", "-F", "tank/replica"]),
            vec![],
            vec![],
            0,
        );
        let args = RecvArgs::new("tank/replica").force_rollback();
        let mut handle = recv(&runner, &args).await.expect("recv spawns");
        handle
            .stdin
            .as_mut()
            .unwrap()
            .write_all(b"fake-stream")
            .await
            .unwrap();
        assert!(handle.wait().await.unwrap().success());
    }

    #[tokio::test]
    async fn recv_interrupted_stderr_yields_resume_token() {
        use tokio::io::AsyncReadExt;

        let fixture = std::fs::read(format!(
            "{}/tests/fixtures/send_recv_interrupted.stderr",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let runner = RecordingRunner::new().record_spawn(
            Cmd::new("zfs").args(["recv", "tank/data"]),
            vec![],
            fixture,
            1,
        );
        let args = RecvArgs::new("tank/data");
        let mut handle = recv(&runner, &args).await.expect("recv spawns");

        let mut stderr_buf = Vec::new();
        handle
            .stderr
            .as_mut()
            .unwrap()
            .read_to_end(&mut stderr_buf)
            .await
            .unwrap();
        let _status = handle.wait().await.unwrap();

        let err = check_recv_stderr(&String::from_utf8_lossy(&stderr_buf)).unwrap_err();
        assert!(matches!(err, RecvError::NeedsResumeToken { .. }));
    }
}
