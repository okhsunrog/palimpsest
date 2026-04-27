use palimpsest::encryption::{load_key, unload_key};
use palimpsest::{RecordingRunner, ZfsError};

#[tokio::test]
async fn unload_key_returns_ok_on_success() {
    let runner = RecordingRunner::new().record(
        "zfs",
        &["unload-key", "tank"],
        vec![],
        vec![],
        0,
    );
    unload_key(&runner, "tank")
        .await
        .expect("unload_key succeeds");
}

#[tokio::test]
async fn unload_key_classifies_dataset_not_found() {
    let runner = RecordingRunner::new().record(
        "zfs",
        &["unload-key", "tank"],
        vec![],
        b"cannot open 'tank': dataset does not exist\n".to_vec(),
        1,
    );
    let err = unload_key(&runner, "tank")
        .await
        .expect_err("unload_key should fail");
    assert!(matches!(err, ZfsError::DatasetNotFound { .. }));
}

#[tokio::test]
async fn load_key_returns_ok_on_success() {
    let runner = RecordingRunner::new().record(
        "zfs",
        &["load-key", "tank"],
        vec![],
        vec![],
        0,
    );
    load_key(&runner, "tank").await.expect("load_key succeeds");
}
