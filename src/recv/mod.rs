use std::collections::BTreeMap;

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
    /// `-o property=value` overrides emitted in deterministic key order so
    /// the wire arglist is reproducible (helps fixture-based tests and
    /// makes diffs of recorded commands meaningful).
    pub properties_override: BTreeMap<String, String>,
    /// `-x property` — drop the named property from the incoming stream so
    /// the receiver inherits its parent's value.
    pub properties_inherit: Vec<String>,
    /// `-s` — leave the partially-received state on disk if the receive
    /// is interrupted, so the next sender can pick up via
    /// `zfs send -t <token>`. Without this flag, an interrupted recv
    /// destroys the partial and forces a restart from the beginning.
    /// Required for any caller that wants `receive_resume_token` to be
    /// populated on failure.
    pub resumable: bool,
}

impl RecvArgs {
    pub fn new(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            force_rollback: false,
            unmounted: false,
            discard_first_component: false,
            exclude_first_component: false,
            properties_override: BTreeMap::new(),
            properties_inherit: Vec::new(),
            resumable: false,
        }
    }

    pub fn resumable(mut self) -> Self {
        self.resumable = true;
        self
    }

    pub fn force_rollback(mut self) -> Self {
        self.force_rollback = true;
        self
    }
    pub fn unmounted(mut self) -> Self {
        self.unmounted = true;
        self
    }
    pub fn property_override(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.properties_override.insert(key.into(), value.into());
        self
    }
    pub fn property_inherit(mut self, key: impl Into<String>) -> Self {
        self.properties_inherit.push(key.into());
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
        if self.resumable {
            args.push("-s".to_string());
        }
        for (k, v) in &self.properties_override {
            args.push("-o".to_string());
            args.push(format!("{k}={v}"));
        }
        for k in &self.properties_inherit {
            args.push("-x".to_string());
            args.push(k.clone());
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

/// Probe whether `dataset` has a partial-receive in flight. Returns the
/// resume token if there is one (ready to feed into
/// [`crate::send::SendArgs::resume_token`]), `Ok(None)` if the dataset has
/// no pending receive, or an error if the dataset can't be queried.
///
/// Backed by `zfs get -H -o value receive_resume_token <dataset>`, where
/// ZFS returns a literal `-` for "no token". Lets callers decide
/// resume-vs-restart without speculatively invoking `zfs recv`.
pub async fn receive_resume_token(
    runner: &dyn CommandRunner,
    dataset: &str,
) -> Result<Option<String>, ZfsError> {
    let prop = crate::dataset::get_property(runner, dataset, "receive_resume_token").await?;
    if prop.value == "-" || prop.value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(prop.value))
    }
}

/// `zfs recv -A <dataset>` — discard the partially-received state from
/// a prior interrupted resumable receive so the dataset can accept a
/// fresh full or incremental stream. Verified in OpenZFS 2.4.1: a
/// brand-new full send into a dataset that still carries
/// `receive_resume_token` fails with "destination contains
/// partially-complete state from \"zfs receive -s\"" even with `-F`.
/// The only path to recover is `-A` (or wait for the original sender's
/// resume stream).
///
/// Idempotent at the call-site's level of intent: if there is no partial,
/// `zfs recv -A` exits non-zero with "no partial state to abort". The
/// caller should treat that as success — there was nothing to clear.
pub async fn abort_partial(runner: &dyn CommandRunner, dataset: &str) -> Result<(), ZfsError> {
    let out = runner
        .run(Cmd::new("zfs").args(["recv", "-A", dataset]))
        .await
        .map_err(ZfsError::Spawn)?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    // No-op success: nothing to abort. ZFS's exact wording varies a
    // little across versions; key invariant we want is "the dataset
    // has no partial state when this returns Ok".
    if stderr.contains("no partial") || stderr.contains("does not have any resumable") {
        return Ok(());
    }
    Err(crate::error::classify_stderr(&stderr, out.status.code()))
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

    fn property_json(ds: &str, prop: &str, value: &str) -> Vec<u8> {
        format!(
            "{{\"output_version\":{{\"command\":\"zfs get\",\"vers_major\":0,\"vers_minor\":1}},\
             \"datasets\":{{\"{ds}\":{{\"name\":\"{ds}\",\"type\":\"FILESYSTEM\",\
             \"pool\":\"{ds}\",\"createtxg\":\"1\",\"properties\":{{\"{prop}\":\
             {{\"value\":\"{value}\",\"source\":{{\"type\":\"DEFAULT\",\"data\":\"-\"}}}}}}}}}}}}"
        )
        .into_bytes()
    }

    #[tokio::test]
    async fn receive_resume_token_returns_none_for_dash() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["get", "-j", "-p", "receive_resume_token", "tank/replica"]),
            property_json("tank/replica", "receive_resume_token", "-"),
            vec![],
            0,
        );
        let token = receive_resume_token(&runner, "tank/replica").await.unwrap();
        assert_eq!(token, None);
    }

    #[tokio::test]
    async fn receive_resume_token_returns_some_for_real_token() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["get", "-j", "-p", "receive_resume_token", "tank/replica"]),
            property_json("tank/replica", "receive_resume_token", "1-abc123deadbeef"),
            vec![],
            0,
        );
        let token = receive_resume_token(&runner, "tank/replica").await.unwrap();
        assert_eq!(token.as_deref(), Some("1-abc123deadbeef"));
    }

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
        assert!(
            token.starts_with("1-"),
            "token should start with '1-', got: {token}"
        );
        assert!(
            token.len() > 10,
            "token should be long, got len={}",
            token.len()
        );
    }

    #[test]
    fn build_args_defaults() {
        let args = RecvArgs::new("tank/replica");
        assert_eq!(args.build_args(), vec!["recv", "tank/replica"]);
    }

    #[test]
    fn build_args_flags() {
        let args = RecvArgs::new("tank/replica").force_rollback().unmounted();
        assert_eq!(args.build_args(), vec!["recv", "-F", "-u", "tank/replica"]);
    }

    #[test]
    fn build_args_resumable_flag() {
        let args = RecvArgs::new("tank/replica").unmounted().resumable();
        assert_eq!(args.build_args(), vec!["recv", "-u", "-s", "tank/replica"]);
    }

    #[tokio::test]
    async fn abort_partial_runs_recv_dash_a() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["recv", "-A", "tank/replica"]),
            vec![],
            vec![],
            0,
        );
        abort_partial(&runner, "tank/replica").await.unwrap();
    }

    #[tokio::test]
    async fn abort_partial_treats_no_partial_as_success() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["recv", "-A", "tank/replica"]),
            vec![],
            b"cannot abort: no partial recv to abort\n".to_vec(),
            1,
        );
        abort_partial(&runner, "tank/replica").await.unwrap();
    }

    #[test]
    fn build_args_property_override_and_inherit() {
        // Inserted out of alphabetical order to confirm BTreeMap sort.
        let args = RecvArgs::new("tank/replica")
            .unmounted()
            .property_override("readonly", "on")
            .property_override("canmount", "off")
            .property_inherit("mountpoint");
        assert_eq!(
            args.build_args(),
            vec![
                "recv",
                "-u",
                "-o",
                "canmount=off",
                "-o",
                "readonly=on",
                "-x",
                "mountpoint",
                "tank/replica"
            ]
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
