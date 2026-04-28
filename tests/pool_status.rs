use palimpsest::pool::status;
use palimpsest::{Cmd, RecordingRunner};

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("loading fixture {name}: {e}"))
}

#[tokio::test]
async fn status_returns_typed_entry_with_vdev_tree() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zpool").args(["status", "-j", "tank"]),
        fixture("pool_status.json"),
        vec![],
        0,
    );
    let entry = status(&runner, "tank").await.expect("status succeeds");
    assert_eq!(entry.name, "tank");
    assert_eq!(entry.state, "ONLINE");
    assert_eq!(entry.error_count, "0");

    // Root vdev keyed by pool name in the fixture
    let root = entry.vdevs.get("tank").expect("root vdev present");
    assert_eq!(root.vdev_type, "root");
    assert_eq!(root.state, "ONLINE");

    // Child vdev (the file backing the test pool)
    let child = root.vdevs.get("/tmp/tank.img").expect("child vdev present");
    assert_eq!(child.vdev_type, "file");
    assert_eq!(child.state, "ONLINE");
    assert_eq!(child.read_errors, "0");
    assert_eq!(child.checksum_errors, "0");
}
