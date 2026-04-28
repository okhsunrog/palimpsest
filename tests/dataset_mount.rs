use palimpsest::dataset::{MountOptions, UnmountOptions, mount, mount_all, unmount, unmount_all};
use palimpsest::{Cmd, RecordingRunner};

#[tokio::test]
async fn mount_basic_succeeds() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args(["mount", "tank/data"]),
        vec![],
        vec![],
        0,
    );
    mount(&runner, "tank/data", &MountOptions::default())
        .await
        .expect("mount succeeds");
}

#[tokio::test]
async fn mount_recursive_passes_dash_r() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args(["mount", "-R", "tank/data"]),
        vec![],
        vec![],
        0,
    );
    let opts = MountOptions { recursive: true };
    mount(&runner, "tank/data", &opts)
        .await
        .expect("mount -R succeeds");
}

#[tokio::test]
async fn mount_idempotent_when_already_mounted() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args(["mount", "tank/data"]),
        vec![],
        b"cannot mount 'tank/data': filesystem already mounted\n".to_vec(),
        1,
    );
    mount(&runner, "tank/data", &MountOptions::default())
        .await
        .expect("idempotent on already-mounted");
}

#[tokio::test]
async fn unmount_basic_succeeds() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args(["umount", "tank/data"]),
        vec![],
        vec![],
        0,
    );
    unmount(&runner, "tank/data", &UnmountOptions::default())
        .await
        .expect("unmount succeeds");
}

#[tokio::test]
async fn unmount_force_passes_dash_f() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args(["umount", "-f", "tank/data"]),
        vec![],
        vec![],
        0,
    );
    let opts = UnmountOptions { force: true };
    unmount(&runner, "tank/data", &opts)
        .await
        .expect("unmount -f succeeds");
}

#[tokio::test]
async fn unmount_idempotent_when_not_mounted() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args(["umount", "tank/data"]),
        vec![],
        b"cannot unmount 'tank/data': not currently mounted\n".to_vec(),
        1,
    );
    unmount(&runner, "tank/data", &UnmountOptions::default())
        .await
        .expect("idempotent on not-mounted");
}

#[tokio::test]
async fn mount_all_succeeds() {
    let runner =
        RecordingRunner::new().record(Cmd::new("zfs").args(["mount", "-a"]), vec![], vec![], 0);
    mount_all(&runner).await.expect("mount -a succeeds");
}

#[tokio::test]
async fn unmount_all_force_succeeds() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args(["umount", "-a", "-f"]),
        vec![],
        vec![],
        0,
    );
    unmount_all(&runner, true)
        .await
        .expect("umount -af succeeds");
}

#[tokio::test]
async fn unmount_all_no_force() {
    let runner =
        RecordingRunner::new().record(Cmd::new("zfs").args(["umount", "-a"]), vec![], vec![], 0);
    unmount_all(&runner, false)
        .await
        .expect("umount -a succeeds");
}
