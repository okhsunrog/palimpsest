use std::path::PathBuf;

use palimpsest::pool::{PoolCreateOptions, RaidZLevel, Vdev, create};
use palimpsest::{Cmd, RecordingRunner};

#[tokio::test]
async fn create_minimal_single_disk_succeeds() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zpool").args(["create", "tank", "/dev/sda"]),
        vec![],
        vec![],
        0,
    );
    let opts = PoolCreateOptions::new("tank").vdev(Vdev::Stripe(vec![PathBuf::from("/dev/sda")]));
    create(&runner, &opts).await.expect("create succeeds");
}

#[tokio::test]
async fn create_archinstall_shape_succeeds() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zpool").args([
            "create",
            "-f",
            "-o",
            "ashift=12",
            "-O",
            "acltype=posixacl",
            "-O",
            "compression=lz4",
            "-m",
            "none",
            "-R",
            "/mnt",
            "tank",
            "/dev/disk/by-id/test-part2",
        ]),
        vec![],
        vec![],
        0,
    );
    let opts = PoolCreateOptions::new("tank")
        .force()
        .pool_property("ashift", "12")
        .fs_property("acltype", "posixacl")
        .fs_property("compression", "lz4")
        .mountpoint("none")
        .altroot("/mnt")
        .vdev(Vdev::Stripe(vec![PathBuf::from(
            "/dev/disk/by-id/test-part2",
        )]));
    create(&runner, &opts).await.expect("create succeeds");
}

#[tokio::test]
async fn create_raidz_pool_succeeds() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zpool").args([
            "create", "tank", "raidz1", "/dev/sda", "/dev/sdb", "/dev/sdc",
        ]),
        vec![],
        vec![],
        0,
    );
    let opts = PoolCreateOptions::new("tank").vdev(Vdev::RaidZ(
        RaidZLevel::One,
        vec![
            PathBuf::from("/dev/sda"),
            PathBuf::from("/dev/sdb"),
            PathBuf::from("/dev/sdc"),
        ],
    ));
    create(&runner, &opts).await.expect("raidz create succeeds");
}
