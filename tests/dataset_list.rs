use palimpsest::dataset::{DatasetType, ListOptions, list};
use palimpsest::{RecordingRunner, ZfsError};

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("loading fixture {name}: {e}"))
}

#[tokio::test]
async fn list_simple_returns_one_filesystem() {
    let runner = RecordingRunner::new().record(
        "zfs",
        &["list", "-j", "-p", "tank"],
        fixture("dataset_list_simple.json"),
        vec![],
        0,
    );
    let opts = ListOptions {
        roots: vec!["tank".into()],
        ..Default::default()
    };
    let entries = list(&runner, &opts).await.expect("list succeeds");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "tank");
    assert_eq!(entries[0].kind, DatasetType::Filesystem);
    assert_eq!(entries[0].pool, "tank");
    assert!(entries[0].properties.contains_key("used"));
}

#[tokio::test]
async fn list_recursive_with_depth() {
    let runner = RecordingRunner::new().record(
        "zfs",
        &["list", "-j", "-p", "-r", "-d", "2", "tank"],
        fixture("dataset_list_recursive.json"),
        vec![],
        0,
    );
    let opts = ListOptions {
        recursive: true,
        depth: Some(2),
        roots: vec!["tank".into()],
        ..Default::default()
    };
    let entries = list(&runner, &opts).await.expect("list succeeds");
    // tank, tank/data, tank/data/home, tank/encrypted (4 — hierarchy capped at depth 2)
    assert!(entries.len() >= 3, "got {} entries", entries.len());
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"tank"));
    assert!(names.contains(&"tank/data"));
}

#[tokio::test]
async fn list_mixed_types_separates_kinds() {
    // Fixture was captured with `-t all`; our build_args emits the four types
    // comma-joined in declaration order. The RecordingRunner key matches the
    // emitted command line.
    let opts = ListOptions {
        recursive: true,
        depth: Some(2),
        types: vec![
            DatasetType::Filesystem,
            DatasetType::Volume,
            DatasetType::Snapshot,
            DatasetType::Bookmark,
        ],
        roots: vec!["tank/data/home".into()],
        ..Default::default()
    };
    let runner = RecordingRunner::new().record(
        "zfs",
        &[
            "list",
            "-j",
            "-p",
            "-r",
            "-d",
            "2",
            "-t",
            "filesystem,volume,snapshot,bookmark",
            "tank/data/home",
        ],
        fixture("dataset_list_mixed.json"),
        vec![],
        0,
    );

    let entries = list(&runner, &opts).await.expect("list succeeds");

    let fs_count = entries
        .iter()
        .filter(|e| e.kind == DatasetType::Filesystem)
        .count();
    let snap_count = entries
        .iter()
        .filter(|e| e.kind == DatasetType::Snapshot)
        .count();
    let bm_count = entries
        .iter()
        .filter(|e| e.kind == DatasetType::Bookmark)
        .count();

    assert_eq!(fs_count, 1);
    assert_eq!(snap_count, 2, "fixture has snap1 and snap2");
    assert_eq!(bm_count, 1);

    let snap = entries
        .iter()
        .find(|e| e.kind == DatasetType::Snapshot)
        .unwrap();
    assert!(snap.dataset.is_some(), "snapshot has parent dataset field");
    assert!(
        snap.snapshot_name.is_some(),
        "snapshot has snapshot_name field"
    );
    let bm = entries
        .iter()
        .find(|e| e.kind == DatasetType::Bookmark)
        .unwrap();
    assert!(bm.name.contains('#'), "bookmark name contains #");
}

#[tokio::test]
async fn list_returns_typed_error_on_missing_dataset() {
    let runner = RecordingRunner::new().record(
        "zfs",
        &["list", "-j", "-p", "tank/missing"],
        vec![],
        b"cannot open 'tank/missing': dataset does not exist\n".to_vec(),
        1,
    );
    let opts = ListOptions {
        roots: vec!["tank/missing".into()],
        ..Default::default()
    };
    let err = list(&runner, &opts).await.expect_err("should fail");
    let ZfsError::DatasetNotFound { name } = err else {
        panic!("expected DatasetNotFound, got {err:?}");
    };
    assert_eq!(name, "tank/missing");
}

#[tokio::test]
async fn list_returns_io_error_when_runner_has_no_fixture() {
    let runner = RecordingRunner::new();
    let opts = ListOptions::default();
    let err = list(&runner, &opts).await.expect_err("unmatched call should fail");
    assert!(matches!(err, ZfsError::Spawn(_)));
}
