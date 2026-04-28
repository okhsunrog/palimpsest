use palimpsest::dataset::{CreateOptions, create};
use palimpsest::{Cmd, RecordingRunner, ZfsError};

#[tokio::test]
async fn create_no_props_succeeds() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args(["create", "tank/data"]),
        vec![],
        vec![],
        0,
    );
    create(&runner, "tank/data", &CreateOptions::new())
        .await
        .expect("create succeeds");
}

#[tokio::test]
async fn create_with_properties_emits_dash_o_pairs() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args([
            "create",
            "-o",
            "mountpoint=/mnt/data",
            "-o",
            "compression=lz4",
            "tank/data",
        ]),
        vec![],
        vec![],
        0,
    );
    let opts = CreateOptions::new()
        .property("mountpoint", "/mnt/data")
        .property("compression", "lz4");
    create(&runner, "tank/data", &opts)
        .await
        .expect("create with props succeeds");
}

#[tokio::test]
async fn create_propagates_other_error_for_existing() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args(["create", "tank/data"]),
        vec![],
        b"cannot create 'tank/data': dataset already exists\n".to_vec(),
        1,
    );
    let err = create(&runner, "tank/data", &CreateOptions::new())
        .await
        .expect_err("already-exists should error");
    let ZfsError::Other { stderr, .. } = err else {
        panic!("expected Other, got {err:?}");
    };
    assert!(stderr.contains("already exists"));
}
