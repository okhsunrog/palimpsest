pub mod args;
pub mod dry_run;

pub use args::{SendArgs, SendArgsError, SendFrom};
pub use dry_run::{DryRunSize, SendKind, dry_run};

use crate::error::ZfsError;
use crate::runner::{ChildHandle, Cmd, CommandRunner};
use tokio::io::{AsyncRead, AsyncReadExt};

/// Running `zfs send` process with stderr drained from the moment it starts.
pub struct SendProcess {
    child: ChildHandle,
    stderr: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
}

impl SendProcess {
    pub fn take_stdout(&mut self) -> Option<Box<dyn AsyncRead + Unpin + Send>> {
        self.child.stdout.take()
    }

    pub async fn finish(self) -> Result<(), ZfsError> {
        let status = self.child.wait().await.map_err(ZfsError::Spawn)?;
        let stderr = self.stderr.await.map_err(|error| ZfsError::Other {
            exit_code: status.code(),
            stderr: format!("send stderr task failed: {error}"),
        })??;
        if status.success() {
            Ok(())
        } else {
            Err(crate::error::classify_stderr(
                &String::from_utf8_lossy(&stderr),
                status.code(),
            ))
        }
    }

    pub async fn cancel(mut self) -> Result<(), ZfsError> {
        self.child.start_kill().map_err(ZfsError::Spawn)?;
        let _ = self.child.wait().await.map_err(ZfsError::Spawn)?;
        let _ = self.stderr.await;
        Ok(())
    }
}

/// `zfs send [flags] <snapshot>` — spawn a managed send process. Callers take
/// and consume stdout, then call [`SendProcess::finish`]; stderr is drained
/// concurrently and a non-zero exit becomes a classified [`ZfsError`].
///
/// Use [`SendArgs::resume`] for a resume token. OpenZFS rejects replication
/// and property-package flags in combination with `-t`; stream feature flags
/// are accepted and combined with the capabilities encoded in the token.
pub async fn send(runner: &dyn CommandRunner, args: &SendArgs) -> Result<SendProcess, ZfsError> {
    let cmd_args = args.build_args(false)?;
    let mut child = runner
        .spawn(Cmd::new("zfs").args(cmd_args))
        .await
        .map_err(ZfsError::Spawn)?;
    let mut stderr = child.stderr.take().ok_or_else(|| ZfsError::Other {
        exit_code: None,
        stderr: "zfs send runner did not provide stderr".to_string(),
    })?;
    let stderr = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).await?;
        Ok(bytes)
    });
    Ok(SendProcess { child, stderr })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::RecordingRunner;
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn send_full_returns_handle_with_stream() {
        let runner = RecordingRunner::new().record_spawn(
            Cmd::new("zfs").args(["send", "tank/data/home@snap1"]),
            b"fake-zfs-stream-bytes".to_vec(),
            vec![],
            0,
        );
        let args = SendArgs::new("tank/data/home@snap1");
        let mut handle = send(&runner, &args).await.expect("send spawns");
        let mut buf = Vec::new();
        let mut stdout = handle.take_stdout().unwrap();
        stdout.read_to_end(&mut buf).await.unwrap();
        drop(stdout);
        assert_eq!(buf, b"fake-zfs-stream-bytes");
        handle.finish().await.unwrap();
    }

    #[tokio::test]
    async fn send_incremental_spawns_correct_args() {
        let runner = RecordingRunner::new().record_spawn(
            Cmd::new("zfs").args(["send", "-i", "tank/data/home@snap1", "tank/data/home@snap2"]),
            vec![],
            vec![],
            0,
        );
        let args = SendArgs::new("tank/data/home@snap2").incremental("tank/data/home@snap1");
        send(&runner, &args).await.expect("incremental send spawns");
    }

    #[tokio::test]
    async fn send_resume_token_spawns_correct_args() {
        let token = "1-abc123deadbeef";
        let runner = RecordingRunner::new().record_spawn(
            Cmd::new("zfs").args(["send", "-t", token]),
            vec![],
            vec![],
            0,
        );
        let args = SendArgs::resume(token);
        send(&runner, &args)
            .await
            .expect("resume token send spawns");
    }

    #[tokio::test]
    async fn send_replication_flags_passed_through() {
        let runner = RecordingRunner::new().record_spawn(
            Cmd::new("zfs").args(["send", "-R", "-w", "-p", "tank/data@snap1"]),
            vec![],
            vec![],
            0,
        );
        let args = SendArgs::new("tank/data@snap1")
            .replicate()
            .raw()
            .properties();
        send(&runner, &args).await.expect("flagged send spawns");
    }
}
