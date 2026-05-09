use crate::error::{ZfsError, classify_stderr};
use crate::runner::{Cmd, CommandRunner};

/// Plan for `zfs rollback`. Rolls a dataset back to a snapshot, discarding
/// any state newer than that snapshot. The intermediate-snapshot policy is
/// expressed by the `-r`/`-R` flags below.
#[derive(Default, Clone, Debug)]
pub struct RollbackOptions {
    /// `-r`: destroy intermediate snapshots newer than the target.
    pub destroy_newer: bool,
    /// `-R`: also destroy newer bookmarks and clones. Implies `-r`. Use with
    /// care — clones will be unmounted and destroyed without further prompt.
    pub destroy_newer_with_clones: bool,
    /// `-f`: force-unmount any dependent clones if `-R` is in effect.
    pub force_unmount_clones: bool,
}

impl RollbackOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn destroy_newer(mut self) -> Self {
        self.destroy_newer = true;
        self
    }

    pub fn destroy_newer_with_clones(mut self) -> Self {
        self.destroy_newer_with_clones = true;
        self
    }

    pub fn force_unmount_clones(mut self) -> Self {
        self.force_unmount_clones = true;
        self
    }

    pub fn build_args(&self, full_snapshot: &str) -> Vec<String> {
        let mut args: Vec<String> = vec!["rollback".into()];
        if self.destroy_newer_with_clones {
            args.push("-R".into());
        } else if self.destroy_newer {
            args.push("-r".into());
        }
        if self.force_unmount_clones {
            args.push("-f".into());
        }
        args.push(full_snapshot.to_string());
        args
    }
}

/// `zfs rollback [flags] <full_snapshot>`. Errors classified via
/// `classify_stderr`.
pub async fn rollback(
    runner: &dyn CommandRunner,
    full_snapshot: &str,
    opts: &RollbackOptions,
) -> Result<(), ZfsError> {
    let output = runner
        .run(Cmd::new("zfs").args(opts.build_args(full_snapshot)))
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
    use crate::runner::RecordingRunner;

    #[test]
    fn build_args_default() {
        assert_eq!(
            RollbackOptions::new().build_args("tank/data@snap1"),
            vec!["rollback", "tank/data@snap1"]
        );
    }

    #[test]
    fn build_args_destroy_newer() {
        assert_eq!(
            RollbackOptions::new()
                .destroy_newer()
                .build_args("tank/data@snap1"),
            vec!["rollback", "-r", "tank/data@snap1"]
        );
    }

    #[test]
    fn build_args_destroy_newer_with_clones_supersedes_r() {
        let opts = RollbackOptions::new()
            .destroy_newer()
            .destroy_newer_with_clones()
            .force_unmount_clones();
        assert_eq!(
            opts.build_args("tank/data@snap1"),
            vec!["rollback", "-R", "-f", "tank/data@snap1"]
        );
    }

    #[tokio::test]
    async fn rollback_success() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["rollback", "tank/data@snap1"]),
            vec![],
            vec![],
            0,
        );
        rollback(&runner, "tank/data@snap1", &RollbackOptions::new())
            .await
            .expect("rollback succeeds");
    }

    #[tokio::test]
    async fn rollback_dataset_not_found() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["rollback", "tank/data@nope"]),
            vec![],
            b"cannot open 'tank/data@nope': dataset does not exist\n".to_vec(),
            1,
        );
        let err = rollback(&runner, "tank/data@nope", &RollbackOptions::new())
            .await
            .expect_err("missing snapshot must error");
        assert!(matches!(err, ZfsError::DatasetNotFound { .. }));
    }
}
