use crate::error::{ZfsError, classify_stderr};
use crate::runner::{Cmd, CommandRunner};

/// Plan for `zfs snapshot`. Multiple snapshot names can be created in a
/// single atomic transaction; ZFS guarantees that all snapshots in one
/// invocation share the same txg.
#[derive(Default, Clone, Debug)]
pub struct SnapshotOptions {
    /// `-r`: recursively snapshot all descendants. The snapshot tag is the
    /// same for every descendant; ZFS itself walks the tree.
    pub recursive: bool,
    /// `-o key=value` properties applied at snapshot time.
    pub properties: Vec<(String, String)>,
}

impl SnapshotOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn recursive(mut self) -> Self {
        self.recursive = true;
        self
    }

    pub fn property(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.properties.push((k.into(), v.into()));
        self
    }

    pub fn build_args(&self, snapshots: &[&str]) -> Vec<String> {
        let mut args: Vec<String> = vec!["snapshot".into()];
        if self.recursive {
            args.push("-r".into());
        }
        for (k, v) in &self.properties {
            args.push("-o".into());
            args.push(format!("{k}={v}"));
        }
        for s in snapshots {
            args.push((*s).to_string());
        }
        args
    }
}

/// `zfs snapshot [-r] [-o k=v]... <full_snapshot>` where `full_snapshot` is
/// of the form `pool/ds@name`. Errors classified via `classify_stderr`.
pub async fn snapshot(
    runner: &dyn CommandRunner,
    full_snapshot: &str,
    opts: &SnapshotOptions,
) -> Result<(), ZfsError> {
    snapshot_many(runner, &[full_snapshot], opts).await
}

/// `zfs snapshot ... <s1> <s2> ...` — atomic multi-snapshot in one txg.
pub async fn snapshot_many(
    runner: &dyn CommandRunner,
    snapshots: &[&str],
    opts: &SnapshotOptions,
) -> Result<(), ZfsError> {
    let output = runner
        .run(Cmd::new("zfs").args(opts.build_args(snapshots)))
        .await?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(classify_stderr(&stderr, output.status.code()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{Cmd, RecordingRunner};

    #[test]
    fn build_args_basic() {
        let opts = SnapshotOptions::new();
        assert_eq!(
            opts.build_args(&["tank/data@snap1"]),
            vec!["snapshot", "tank/data@snap1"]
        );
    }

    #[test]
    fn build_args_recursive_with_properties() {
        let opts = SnapshotOptions::new()
            .recursive()
            .property("com.sun:auto-snapshot", "true");
        assert_eq!(
            opts.build_args(&["tank/data@snap1"]),
            vec![
                "snapshot",
                "-r",
                "-o",
                "com.sun:auto-snapshot=true",
                "tank/data@snap1",
            ]
        );
    }

    #[test]
    fn build_args_atomic_many() {
        let opts = SnapshotOptions::new();
        assert_eq!(
            opts.build_args(&["tank/a@s", "tank/b@s"]),
            vec!["snapshot", "tank/a@s", "tank/b@s"]
        );
    }

    #[tokio::test]
    async fn snapshot_success() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["snapshot", "tank/data@snap1"]),
            vec![],
            vec![],
            0,
        );
        snapshot(&runner, "tank/data@snap1", &SnapshotOptions::new())
            .await
            .expect("snapshot succeeds");
    }

    #[tokio::test]
    async fn snapshot_dataset_not_found() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["snapshot", "tank/missing@snap1"]),
            vec![],
            b"cannot open 'tank/missing': dataset does not exist\n".to_vec(),
            1,
        );
        let err = snapshot(&runner, "tank/missing@snap1", &SnapshotOptions::new())
            .await
            .expect_err("missing dataset must error");
        let ZfsError::DatasetNotFound { name } = err else {
            panic!("expected DatasetNotFound, got {err:?}");
        };
        assert_eq!(name, "tank/missing");
    }

    #[tokio::test]
    async fn snapshot_recursive_propagates_args() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["snapshot", "-r", "tank/data@snap1"]),
            vec![],
            vec![],
            0,
        );
        snapshot(
            &runner,
            "tank/data@snap1",
            &SnapshotOptions::new().recursive(),
        )
        .await
        .expect("recursive snapshot succeeds");
    }
}
