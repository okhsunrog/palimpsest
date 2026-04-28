pub mod args;
pub mod dry_run;

pub use args::{SendArgs, SendArgsError, SendFrom};
pub use dry_run::{DryRunSize, SendKind, dry_run};

use crate::error::ZfsError;
use crate::runner::{ChildHandle, Cmd, CommandRunner};

/// `zfs send [flags] <snapshot>` — spawn the send process and return a
/// [`ChildHandle`]. Callers read the byte stream from `child.stdout` and pipe
/// it to the receiver. Call `child.wait()` after consuming all output.
///
/// When `args.from` is a `ResumeToken`, the snapshot field is ignored and
/// `-t <token>` is passed instead.
pub async fn send(runner: &dyn CommandRunner, args: &SendArgs) -> Result<ChildHandle, ZfsError> {
    let cmd_args = args.build_args(false).map_err(|e| ZfsError::Other {
        exit_code: None,
        stderr: e.to_string(),
    })?;
    runner
        .spawn(Cmd::new("zfs").args(cmd_args))
        .await
        .map_err(ZfsError::Spawn)
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
        handle
            .stdout
            .as_mut()
            .unwrap()
            .read_to_end(&mut buf)
            .await
            .unwrap();
        assert_eq!(buf, b"fake-zfs-stream-bytes");
        assert!(handle.wait().await.unwrap().success());
    }

    #[tokio::test]
    async fn send_incremental_spawns_correct_args() {
        let runner = RecordingRunner::new().record_spawn(
            Cmd::new("zfs").args([
                "send",
                "-i",
                "tank/data/home@snap1",
                "tank/data/home@snap2",
            ]),
            vec![],
            vec![],
            0,
        );
        let args =
            SendArgs::new("tank/data/home@snap2").incremental("tank/data/home@snap1");
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
        let args = SendArgs::new("ignored").resume_token(token);
        send(&runner, &args).await.expect("resume token send spawns");
    }

    #[tokio::test]
    async fn send_replication_flags_passed_through() {
        let runner = RecordingRunner::new().record_spawn(
            Cmd::new("zfs").args(["send", "-R", "-w", "-p", "tank/data@snap1"]),
            vec![],
            vec![],
            0,
        );
        let args = SendArgs::new("tank/data@snap1").replicate().raw().properties();
        send(&runner, &args).await.expect("flagged send spawns");
    }
}
