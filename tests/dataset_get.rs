use palimpsest::dataset::{GetOptions, get, get_property};
use palimpsest::models::PropertySourceKind;
use palimpsest::{Cmd, RecordingRunner, ZfsError};

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("loading fixture {name}: {e}"))
}

#[tokio::test]
async fn get_property_returns_off_for_unencrypted_pool() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args(["get", "-j", "-p", "encryption", "tank"]),
        fixture("dataset_get_encryption_off.json"),
        vec![],
        0,
    );
    let prop = get_property(&runner, "tank", "encryption")
        .await
        .expect("get_property succeeds");
    assert_eq!(prop.value, "off");
    assert_eq!(prop.source.kind, PropertySourceKind::Default);
}

#[tokio::test]
async fn get_property_returns_aes_for_encrypted_dataset() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args(["get", "-j", "-p", "encryption", "tank/encrypted"]),
        fixture("dataset_get_encryption_on.json"),
        vec![],
        0,
    );
    let prop = get_property(&runner, "tank/encrypted", "encryption")
        .await
        .expect("get_property succeeds");
    assert_eq!(prop.value, "aes-256-gcm");
}

#[tokio::test]
async fn get_batch_returns_all_requested_properties() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args([
            "get",
            "-j",
            "-p",
            "encryption,keystatus,keyformat,keylocation",
            "tank/encrypted",
        ]),
        fixture("dataset_get_encryption_batch.json"),
        vec![],
        0,
    );
    let opts = GetOptions {
        datasets: vec!["tank/encrypted".into()],
        properties: vec![
            "encryption".into(),
            "keystatus".into(),
            "keyformat".into(),
            "keylocation".into(),
        ],
        ..Default::default()
    };
    let entries = get(&runner, &opts).await.expect("get succeeds");
    assert_eq!(entries.len(), 1);
    let props = &entries[0].properties;
    assert_eq!(props["encryption"].value, "aes-256-gcm");
    assert_eq!(props["keystatus"].value, "available");
    assert_eq!(props["keyformat"].value, "passphrase");
    assert_eq!(props["keylocation"].value, "file:///tmp/keyfile");
}

#[tokio::test]
async fn get_recursive_returns_multiple_entries() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args(["get", "-j", "-p", "-r", "mountpoint,used", "tank"]),
        fixture("dataset_get_recursive.json"),
        vec![],
        0,
    );
    let opts = GetOptions {
        recursive: true,
        datasets: vec!["tank".into()],
        properties: vec!["mountpoint".into(), "used".into()],
        ..Default::default()
    };
    let entries = get(&runner, &opts).await.expect("get succeeds");
    assert!(
        entries.len() > 1,
        "recursive get should return multiple entries, got {}",
        entries.len()
    );
    for entry in &entries {
        assert!(
            entry.properties.contains_key("mountpoint") || entry.properties.contains_key("used"),
            "entry {} missing requested properties",
            entry.name
        );
    }
}

#[tokio::test]
async fn get_source_filter_returns_only_local_properties() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args(["get", "-j", "-p", "-s", "local", "all", "tank/data"]),
        fixture("dataset_get_source_local.json"),
        vec![],
        0,
    );
    let opts = GetOptions {
        sources: vec![PropertySourceKind::Local],
        datasets: vec!["tank/data".into()],
        properties: vec!["all".into()],
        ..Default::default()
    };
    let entries = get(&runner, &opts).await.expect("get succeeds");
    assert_eq!(entries.len(), 1);
    let props = &entries[0].properties;
    for (name, prop) in props {
        assert_eq!(
            prop.source.kind,
            PropertySourceKind::Local,
            "property {name} should be LOCAL but was {:?}",
            prop.source.kind,
        );
    }
}

#[tokio::test]
async fn get_returns_typed_error_on_missing_dataset() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zfs").args(["get", "-j", "-p", "encryption", "tank/missing"]),
        vec![],
        b"cannot open 'tank/missing': dataset does not exist\n".to_vec(),
        1,
    );
    let err = get_property(&runner, "tank/missing", "encryption")
        .await
        .expect_err("should fail");
    let ZfsError::DatasetNotFound { name } = err else {
        panic!("expected DatasetNotFound, got {err:?}");
    };
    assert_eq!(name, "tank/missing");
}

#[tokio::test]
async fn get_returns_io_error_when_runner_has_no_fixture() {
    let runner = RecordingRunner::new();
    let err = get_property(&runner, "tank", "encryption")
        .await
        .expect_err("unmatched call should fail");
    assert!(matches!(err, ZfsError::Spawn(_)));
}
