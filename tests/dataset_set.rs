use zfskit::dataset::{SetOptions, set_properties, set_property};
use zfskit::{Cmd, RecordingRunner, ZfsError};

#[tokio::test]
async fn set_property_succeeds() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args(["set", "compression=lz4", "tank/data"]),
        vec![],
        vec![],
        0,
    );
    set_property(&runner, "tank/data", "compression", "lz4")
        .await
        .expect("set_property succeeds");
}

#[tokio::test]
async fn set_property_classifies_dataset_not_found() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args(["set", "compression=lz4", "tank/missing"]),
        vec![],
        b"cannot open 'tank/missing': dataset does not exist\n".to_vec(),
        1,
    );
    let err = set_property(&runner, "tank/missing", "compression", "lz4")
        .await
        .expect_err("missing dataset should error");
    assert!(matches!(err, ZfsError::DatasetNotFound { .. }));
}

#[tokio::test]
async fn set_properties_no_mount_passes_minus_u() {
    // Boot-environment case: point the dataset at a path the running system
    // already occupies, without mounting it there.
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args(["set", "-u", "mountpoint=/home", "tank/be0/data/home"]),
        vec![],
        vec![],
        0,
    );
    set_properties(
        &runner,
        "tank/be0/data/home",
        &[("mountpoint", "/home")],
        &SetOptions::new().no_mount(),
    )
    .await
    .expect("set_properties with -u succeeds");
}

#[tokio::test]
async fn set_properties_applies_several_pairs_in_one_call() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args(["set", "mountpoint=/home", "canmount=on", "tank/data"]),
        vec![],
        vec![],
        0,
    );
    set_properties(
        &runner,
        "tank/data",
        &[("mountpoint", "/home"), ("canmount", "on")],
        &SetOptions::default(),
    )
    .await
    .expect("multiple properties succeed");
}

#[tokio::test]
async fn set_properties_empty_never_runs_a_command() {
    // RecordingRunner errors on any unrecorded command, so reaching the runner
    // at all would fail this test.
    set_properties(
        &RecordingRunner::new(),
        "tank/data",
        &[],
        &SetOptions::default(),
    )
    .await
    .expect("empty property list is a no-op");
}
