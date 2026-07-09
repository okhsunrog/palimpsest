use zfskit::dataset::set_property;
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
