use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use crate::error::ZfsError;
use crate::runner::{ChildHandle, Cmd, CommandRunner};
use tokio::io::{AsyncReadExt, AsyncWrite};

#[derive(Error, Debug)]
pub enum RecvError {
    /// `zfs recv` was interrupted mid-stream. The embedded `token` can be
    /// passed to `SendArgs::resume()` to generate a resuming stream
    /// on the sender. The partially received snapshot is preserved.
    #[error("receive interrupted; resume with: zfs send -t {token}")]
    NeedsResumeToken { token: String },

    #[error(transparent)]
    Zfs(#[from] ZfsError),
}

/// Running `zfs recv` process with stderr drained concurrently.
pub struct RecvProcess {
    child: ChildHandle,
    stderr: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
}

impl RecvProcess {
    pub fn take_stdin(&mut self) -> Option<Box<dyn AsyncWrite + Unpin + Send>> {
        self.child.stdin.take()
    }

    pub async fn finish(self) -> Result<(), RecvError> {
        let status = self.child.wait().await.map_err(ZfsError::Spawn)?;
        let stderr = self
            .stderr
            .await
            .map_err(|error| ZfsError::Other {
                exit_code: status.code(),
                stderr: format!("recv stderr task failed: {error}"),
            })?
            .map_err(ZfsError::Spawn)?;
        if status.success() {
            return Ok(());
        }
        let text = String::from_utf8_lossy(&stderr);
        if let Some(token) = resume_token_from_stderr(&text) {
            return Err(RecvError::NeedsResumeToken { token });
        }
        Err(RecvError::Zfs(crate::error::classify_stderr(
            &text,
            status.code(),
        )))
    }

    pub async fn cancel(mut self) -> Result<(), RecvError> {
        self.child.start_kill().map_err(ZfsError::Spawn)?;
        let _ = self.child.wait().await.map_err(ZfsError::Spawn)?;
        let _ = self.stderr.await;
        Ok(())
    }
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
    pub discard_except_last_component: bool,
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
            discard_except_last_component: false,
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
    pub fn discard_first_component(mut self) -> Self {
        self.discard_first_component = true;
        self
    }
    pub fn discard_except_last_component(mut self) -> Self {
        self.discard_except_last_component = true;
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

    pub fn build_args(&self) -> Result<Vec<String>, ZfsError> {
        if self.target.contains('@') {
            crate::names::SnapshotName::parse(&self.target)?;
        } else {
            crate::names::DatasetName::parse(&self.target)?;
        }
        if self.discard_first_component && self.discard_except_last_component {
            return Err(ZfsError::InvalidInput {
                message: "RecvArgs cannot enable both -d and -e".to_string(),
            });
        }
        if self.target.contains('@')
            && (self.discard_first_component || self.discard_except_last_component)
        {
            return Err(ZfsError::InvalidInput {
                message: "zfs recv -d/-e requires a filesystem target, not a snapshot".to_string(),
            });
        }
        let mut seen = BTreeSet::new();
        if let Some(property) = self
            .properties_inherit
            .iter()
            .find(|property| !seen.insert(property.as_str()))
        {
            return Err(ZfsError::InvalidInput {
                message: format!("receive property {property:?} is inherited more than once"),
            });
        }
        if let Some(property) = self
            .properties_inherit
            .iter()
            .find(|property| self.properties_override.contains_key(*property))
        {
            return Err(ZfsError::InvalidInput {
                message: format!(
                    "receive property {property:?} cannot be both overridden and inherited"
                ),
            });
        }
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
        if self.discard_except_last_component {
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
        Ok(args)
    }
}

/// `zfs recv [flags] <target>` — spawn a managed receive process. Callers take
/// its stdin, write and close the stream, then call [`RecvProcess::finish`].
/// Stderr is drained concurrently and non-zero exits are classified there.
pub async fn recv(runner: &dyn CommandRunner, args: &RecvArgs) -> Result<RecvProcess, RecvError> {
    let command_args = args.build_args()?;
    let mut child = runner
        .spawn(Cmd::new("zfs").args(command_args))
        .await
        .map_err(|e| RecvError::Zfs(ZfsError::Spawn(e)))?;
    let mut stderr = child.stderr.take().ok_or_else(|| {
        RecvError::Zfs(ZfsError::Other {
            exit_code: None,
            stderr: "zfs recv runner did not provide stderr".to_string(),
        })
    })?;
    let stderr = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).await?;
        Ok(bytes)
    });
    Ok(RecvProcess { child, stderr })
}

/// Probe whether `dataset` has a partial-receive in flight. Returns the
/// resume token if there is one (ready to feed into
/// [`crate::send::SendArgs::resume`]), `Ok(None)` if the dataset has
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
fn resume_token_from_stderr(stderr: &str) -> Option<String> {
    if !stderr.contains("checksum mismatch or incomplete stream") {
        return None;
    }
    for line in stderr.lines() {
        // The hint line is always "    zfs send -t <token>" (4-space indent).
        if let Some(rest) = line.strip_prefix("    zfs send -t ") {
            let token = rest.trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }
    None
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
        assert_eq!(resume_token_from_stderr(""), None);
        assert_eq!(resume_token_from_stderr("receive complete.\n"), None);
    }

    #[test]
    fn check_recv_stderr_needs_resume_token() {
        let fixture = std::fs::read(format!(
            "{}/tests/fixtures/send_recv_interrupted.stderr",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let text = String::from_utf8_lossy(&fixture);
        let token = resume_token_from_stderr(&text).expect("resume token");
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
        assert_eq!(args.build_args().unwrap(), vec!["recv", "tank/replica"]);
    }

    #[test]
    fn build_args_flags() {
        let args = RecvArgs::new("tank/replica").force_rollback().unmounted();
        assert_eq!(
            args.build_args().unwrap(),
            vec!["recv", "-F", "-u", "tank/replica"]
        );
    }

    #[test]
    fn build_args_resumable_flag() {
        let args = RecvArgs::new("tank/replica").unmounted().resumable();
        assert_eq!(
            args.build_args().unwrap(),
            vec!["recv", "-u", "-s", "tank/replica"]
        );
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
            args.build_args().unwrap(),
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

    #[test]
    fn build_args_rejects_conflicting_shape_and_property_flags() {
        let mut shape = RecvArgs::new("tank/replica");
        shape.discard_first_component = true;
        shape.discard_except_last_component = true;
        assert!(matches!(
            shape.build_args(),
            Err(ZfsError::InvalidInput { .. })
        ));

        let properties = RecvArgs::new("tank/replica")
            .property_override("mountpoint", "/srv")
            .property_inherit("mountpoint");
        assert!(matches!(
            properties.build_args(),
            Err(ZfsError::InvalidInput { .. })
        ));

        let duplicate = RecvArgs::new("tank/replica")
            .property_inherit("mountpoint")
            .property_inherit("mountpoint");
        assert!(matches!(
            duplicate.build_args(),
            Err(ZfsError::InvalidInput { .. })
        ));
    }

    #[test]
    fn build_args_validates_target_shape() {
        assert!(matches!(
            RecvArgs::new("tank//replica").build_args(),
            Err(ZfsError::InvalidName(_))
        ));
        assert!(matches!(
            RecvArgs::new("tank/data@snap")
                .discard_first_component()
                .build_args(),
            Err(ZfsError::InvalidInput { .. })
        ));
        assert_eq!(
            RecvArgs::new("tank/root")
                .discard_except_last_component()
                .build_args()
                .unwrap(),
            vec!["recv", "-e", "tank/root"]
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
        let mut stdin = handle.take_stdin().unwrap();
        stdin.write_all(b"fake-stream").await.unwrap();
        drop(stdin);
        handle.finish().await.unwrap();
    }

    #[tokio::test]
    async fn recv_interrupted_stderr_yields_resume_token() {
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
        let handle = recv(&runner, &args).await.expect("recv spawns");

        let err = handle.finish().await.unwrap_err();
        assert!(matches!(err, RecvError::NeedsResumeToken { .. }));
    }
}
