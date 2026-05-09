use crate::error::{ZfsError, classify_stderr};
use crate::runner::{Cmd, CommandRunner};

/// Plan for `zfs destroy`. Operates on filesystems, volumes, snapshots, and
/// bookmarks; the target's syntax (`pool/ds`, `pool/ds@snap`, `pool/ds#mark`)
/// determines the type. Bookmark destruction also goes through this op.
#[derive(Default, Clone, Debug)]
pub struct DestroyOptions {
    /// `-r`: recursively destroy descendants. For snapshots, destroys the
    /// snapshot on every descendant; for filesystems, destroys child datasets.
    pub recursive: bool,
    /// `-R`: like `-r` but also destroys clones. Implies `-r`. Dangerous —
    /// clones go too.
    pub recursive_with_clones: bool,
    /// `-f`: force-unmount any mounted filesystems before destroying.
    pub force_unmount: bool,
    /// `-d`: defer-destroy a held snapshot. The snapshot is marked for
    /// deletion and is destroyed when the last hold is released.
    pub defer_holds: bool,
}

impl DestroyOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn recursive(mut self) -> Self {
        self.recursive = true;
        self
    }

    pub fn recursive_with_clones(mut self) -> Self {
        self.recursive_with_clones = true;
        self
    }

    pub fn force_unmount(mut self) -> Self {
        self.force_unmount = true;
        self
    }

    pub fn defer_holds(mut self) -> Self {
        self.defer_holds = true;
        self
    }

    pub fn build_args(&self, target: &str) -> Vec<String> {
        let mut args: Vec<String> = vec!["destroy".into()];
        if self.recursive_with_clones {
            args.push("-R".into());
        } else if self.recursive {
            args.push("-r".into());
        }
        if self.force_unmount {
            args.push("-f".into());
        }
        if self.defer_holds {
            args.push("-d".into());
        }
        args.push(target.to_string());
        args
    }
}

/// `zfs destroy [flags] <target>`. `target` may be a filesystem
/// (`pool/ds`), volume, snapshot (`pool/ds@snap`), or bookmark
/// (`pool/ds#mark`). Held-snapshot failures surface as
/// [`ZfsError::SnapshotHeld`]; other errors are classified normally.
pub async fn destroy(
    runner: &dyn CommandRunner,
    target: &str,
    opts: &DestroyOptions,
) -> Result<(), ZfsError> {
    let output = runner
        .run(Cmd::new("zfs").args(opts.build_args(target)))
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
            DestroyOptions::new().build_args("tank/data"),
            vec!["destroy", "tank/data"]
        );
    }

    #[test]
    fn build_args_recursive_force() {
        let opts = DestroyOptions::new().recursive().force_unmount();
        assert_eq!(
            opts.build_args("tank/data"),
            vec!["destroy", "-r", "-f", "tank/data"]
        );
    }

    #[test]
    fn build_args_recursive_with_clones_supersedes_r() {
        let opts = DestroyOptions::new().recursive().recursive_with_clones();
        assert_eq!(
            opts.build_args("tank/data"),
            vec!["destroy", "-R", "tank/data"]
        );
    }

    #[test]
    fn build_args_defer_holds() {
        assert_eq!(
            DestroyOptions::new()
                .defer_holds()
                .build_args("tank/data@snap1"),
            vec!["destroy", "-d", "tank/data@snap1"]
        );
    }

    #[tokio::test]
    async fn destroy_success() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["destroy", "tank/data@snap1"]),
            vec![],
            vec![],
            0,
        );
        destroy(&runner, "tank/data@snap1", &DestroyOptions::new())
            .await
            .expect("destroy succeeds");
    }

    #[tokio::test]
    async fn destroy_held_snapshot_returns_typed_error() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["destroy", "tank/data/home@snap1"]),
            vec![],
            b"cannot destroy snapshot tank/data/home@snap1: it's being held. \
              Run 'zfs holds -r tank/data/home@snap1' to see holders.\n"
                .to_vec(),
            1,
        );
        let err = destroy(&runner, "tank/data/home@snap1", &DestroyOptions::new())
            .await
            .expect_err("held snapshot must error");
        let ZfsError::SnapshotHeld { name } = err else {
            panic!("expected SnapshotHeld, got {err:?}");
        };
        assert_eq!(name, "tank/data/home@snap1");
    }

    #[tokio::test]
    async fn destroy_dataset_busy_routes_to_busy() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["destroy", "tank/data"]),
            vec![],
            b"cannot destroy 'tank/data': dataset is busy\n".to_vec(),
            1,
        );
        let err = destroy(&runner, "tank/data", &DestroyOptions::new())
            .await
            .expect_err("busy dataset must error");
        let ZfsError::Busy { name } = err else {
            panic!("expected Busy, got {err:?}");
        };
        assert_eq!(name, "tank/data");
    }
}
