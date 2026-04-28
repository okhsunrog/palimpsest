use crate::error::{ZfsError, classify_stderr};
use crate::runner::{Cmd, CommandRunner};

// Idempotency markers from OpenZFS stderr. mount on already-mounted prints
// "filesystem already mounted"; unmount on not-mounted prints "not currently
// mounted". Treating both as success matches archinstall_zfs's prior
// best-effort cleanup pattern and our load_key/unload_key precedent.
const ALREADY_MOUNTED: &str = "filesystem already mounted";
const NOT_MOUNTED: &str = "not currently mounted";

#[derive(Default, Clone, Debug)]
pub struct MountOptions {
    /// `-R`: recursively mount this dataset and its descendants.
    pub recursive: bool,
}

impl MountOptions {
    pub fn build_args(&self, dataset: &str) -> Vec<String> {
        let mut args: Vec<String> = vec!["mount".into()];
        if self.recursive {
            args.push("-R".into());
        }
        args.push(dataset.into());
        args
    }
}

#[derive(Default, Clone, Debug)]
pub struct UnmountOptions {
    /// `-f`: force unmount.
    pub force: bool,
}

impl UnmountOptions {
    pub fn build_args(&self, dataset: &str) -> Vec<String> {
        let mut args: Vec<String> = vec!["umount".into()];
        if self.force {
            args.push("-f".into());
        }
        args.push(dataset.into());
        args
    }
}

/// `zfs mount [-R] <dataset>`. Idempotent on already-mounted.
pub async fn mount(
    runner: &dyn CommandRunner,
    dataset: &str,
    opts: &MountOptions,
) -> Result<(), ZfsError> {
    let output = runner
        .run(Cmd::new("zfs").args(opts.build_args(dataset)))
        .await?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains(ALREADY_MOUNTED) {
        return Ok(());
    }
    Err(classify_stderr(&stderr, output.status.code()))
}

/// `zfs umount [-f] <dataset>`. Idempotent on not-currently-mounted.
pub async fn unmount(
    runner: &dyn CommandRunner,
    dataset: &str,
    opts: &UnmountOptions,
) -> Result<(), ZfsError> {
    let output = runner
        .run(Cmd::new("zfs").args(opts.build_args(dataset)))
        .await?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains(NOT_MOUNTED) {
        return Ok(());
    }
    Err(classify_stderr(&stderr, output.status.code()))
}

/// `zfs mount -a` — mount all importable filesystems.
pub async fn mount_all(runner: &dyn CommandRunner) -> Result<(), ZfsError> {
    let output = runner.run(Cmd::new("zfs").args(["mount", "-a"])).await?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(classify_stderr(&stderr, output.status.code()))
}

/// `zfs umount -a [-f]` — unmount all mounted filesystems.
pub async fn unmount_all(runner: &dyn CommandRunner, force: bool) -> Result<(), ZfsError> {
    let mut args: Vec<&str> = vec!["umount", "-a"];
    if force {
        args.push("-f");
    }
    let output = runner.run(Cmd::new("zfs").args(args)).await?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(classify_stderr(&stderr, output.status.code()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_args_default() {
        assert_eq!(
            MountOptions::default().build_args("tank/data"),
            vec!["mount", "tank/data"]
        );
    }

    #[test]
    fn mount_args_recursive() {
        let opts = MountOptions { recursive: true };
        assert_eq!(
            opts.build_args("tank/data"),
            vec!["mount", "-R", "tank/data"]
        );
    }

    #[test]
    fn unmount_args_default() {
        assert_eq!(
            UnmountOptions::default().build_args("tank/data"),
            vec!["umount", "tank/data"]
        );
    }

    #[test]
    fn unmount_args_force() {
        let opts = UnmountOptions { force: true };
        assert_eq!(
            opts.build_args("tank/data"),
            vec!["umount", "-f", "tank/data"]
        );
    }
}
