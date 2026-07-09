use zfskit::pool::{ScrubAction, scrub, status};
use zfskit::{Cmd, RecordingRunner};

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

#[tokio::test]
async fn status_parses_scan_stats_when_scrub_recorded() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zpool").args(["status", "-j", "novafs"]),
        fixture("pool_status_with_scrub.json"),
        vec![],
        0,
    );
    let entry = status(&runner, "novafs").await.expect("status succeeds");
    let scan = entry.scan.expect("scan_stats present in fixture");
    assert_eq!(scan.function, "SCRUB");
    assert_eq!(scan.state, "FINISHED");
    assert_eq!(scan.examined.as_deref(), Some("523G"));
    assert_eq!(scan.errors.as_deref(), Some("1"));
    assert_eq!(scan.pass_start.as_deref(), Some("1778518346"));
}

#[tokio::test]
async fn scrub_start_invokes_zpool_scrub() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zpool").args(["scrub", "tank"]),
        Vec::new(),
        Vec::new(),
        0,
    );
    scrub(&runner, "tank", ScrubAction::Start).await.unwrap();
}

#[tokio::test]
async fn scrub_stop_invokes_zpool_scrub_minus_s() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zpool").args(["scrub", "-s", "tank"]),
        Vec::new(),
        Vec::new(),
        0,
    );
    scrub(&runner, "tank", ScrubAction::Stop).await.unwrap();
}

#[tokio::test]
async fn scrub_pause_uses_minus_p_and_resume_is_bare() {
    // zpool has no resume flag: -p pauses, and -p on an already-paused
    // scrub errors with "use 'zpool scrub' to resume".
    let runner = RecordingRunner::new()
        .record(
            Cmd::new("zpool").args(["scrub", "-p", "tank"]),
            Vec::new(),
            Vec::new(),
            0,
        )
        .record(
            Cmd::new("zpool").args(["scrub", "tank"]),
            Vec::new(),
            Vec::new(),
            0,
        );
    scrub(&runner, "tank", ScrubAction::Pause).await.unwrap();
    scrub(&runner, "tank", ScrubAction::Resume).await.unwrap();
}
