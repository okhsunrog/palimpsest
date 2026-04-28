use palimpsest::encryption::{load_key, load_key_with_passphrase, unload_key};
use palimpsest::{Cmd, RecordingRunner, ZfsError};

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("loading fixture {name}: {e}"))
}

#[tokio::test]
async fn unload_key_returns_ok_on_success() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args(["unload-key", "tank/encrypted"]),
        vec![],
        vec![],
        0,
    );
    unload_key(&runner, "tank/encrypted")
        .await
        .expect("unload_key succeeds");
}

#[tokio::test]
async fn unload_key_is_idempotent_when_already_unloaded() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args(["unload-key", "tank/encrypted"]),
        vec![],
        fixture("err_unload_key_not_loaded.stderr"),
        255,
    );
    unload_key(&runner, "tank/encrypted")
        .await
        .expect("idempotent unload_key returns Ok");
}

#[tokio::test]
async fn unload_key_classifies_dataset_not_found() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args(["unload-key", "tank"]),
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
async fn unload_key_propagates_unencrypted_error() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args(["unload-key", "tank"]),
        vec![],
        fixture("err_unload_key_unencrypted.stderr"),
        255,
    );
    let err = unload_key(&runner, "tank")
        .await
        .expect_err("unload_key on unencrypted dataset should fail");
    let ZfsError::Other { stderr, .. } = err else {
        panic!("expected Other, got {err:?}");
    };
    assert!(stderr.contains("not encrypted"));
}

#[tokio::test]
async fn load_key_returns_ok_on_success() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args(["load-key", "tank/encrypted"]),
        vec![],
        vec![],
        0,
    );
    load_key(&runner, "tank/encrypted")
        .await
        .expect("load_key succeeds");
}

#[tokio::test]
async fn load_key_is_idempotent_when_already_loaded() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args(["load-key", "tank/encrypted"]),
        vec![],
        fixture("err_load_key_already.stderr"),
        255,
    );
    load_key(&runner, "tank/encrypted")
        .await
        .expect("idempotent load_key returns Ok");
}

#[tokio::test]
async fn load_key_with_passphrase_correct() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs")
            .args(["load-key", "tank/encrypted"])
            .stdin_secret(b"correct".to_vec()),
        vec![],
        vec![],
        0,
    );
    load_key_with_passphrase(&runner, "tank/encrypted", b"correct")
        .await
        .expect("load_key_with_passphrase succeeds");
}

#[tokio::test]
async fn load_key_with_passphrase_wrong_returns_error() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs")
            .args(["load-key", "tank/encrypted"])
            .stdin_secret(b"wrong".to_vec()),
        vec![],
        b"Key load error: Incorrect key provided for 'tank/encrypted'.\n".to_vec(),
        1,
    );
    let err = load_key_with_passphrase(&runner, "tank/encrypted", b"wrong")
        .await
        .expect_err("wrong passphrase should fail");
    assert!(matches!(err, ZfsError::Other { .. }));
}

#[tokio::test]
async fn load_key_with_passphrase_idempotent_when_already_loaded() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs")
            .args(["load-key", "tank/encrypted"])
            .stdin_secret(b"correct".to_vec()),
        vec![],
        fixture("err_load_key_already.stderr"),
        255,
    );
    load_key_with_passphrase(&runner, "tank/encrypted", b"correct")
        .await
        .expect("idempotent load_key_with_passphrase returns Ok");
}
