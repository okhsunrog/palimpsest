use palimpsest::pool::{DiscoveredPool, discover};
use palimpsest::{Cmd, RecordingRunner};

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("loading fixture {name}: {e}"))
}

#[tokio::test]
async fn discover_two_pools_with_mixed_state() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zpool").arg("import"),
        fixture("pool_discover_two_pools.txt"),
        vec![],
        0,
    );
    let pools = discover(&runner).await.expect("discover succeeds");
    assert_eq!(pools.len(), 2);

    let tank = pools.iter().find(|p| p.name == "tank").unwrap();
    assert_eq!(tank.id, "9316430153991325696");
    assert_eq!(tank.state, "ONLINE");
    assert!(tank.status.is_none());

    let backup = pools.iter().find(|p| p.name == "backup").unwrap();
    assert_eq!(backup.state, "DEGRADED");
    assert!(
        backup
            .status
            .as_deref()
            .unwrap()
            .contains("One or more devices could not be opened")
    );
}

#[tokio::test]
async fn discover_empty_returns_empty_vec_even_on_nonzero_exit() {
    // Real zpool prints "no pools available to import" to stderr and exits 1.
    let runner = RecordingRunner::new().record(
        Cmd::new("zpool").arg("import"),
        vec![],
        fixture("pool_discover_empty.txt"),
        1,
    );
    let pools: Vec<DiscoveredPool> = discover(&runner).await.expect("discover succeeds");
    assert!(pools.is_empty());
}

#[tokio::test]
async fn discover_combines_stdout_and_stderr() {
    // OpenZFS sometimes routes the pool list to stderr instead of stdout.
    // The discover() free fn combines both streams to be defensive.
    let runner = RecordingRunner::new().record(
        Cmd::new("zpool").arg("import"),
        vec![],
        fixture("pool_discover_two_pools.txt"),
        0,
    );
    let pools = discover(&runner).await.expect("discover succeeds");
    assert_eq!(pools.len(), 2);
}
