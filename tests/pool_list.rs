use palimpsest::pool::{ListOptions, list};
use palimpsest::{Cmd, RecordingRunner};

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("loading fixture {name}: {e}"))
}

#[tokio::test]
async fn list_returns_entries() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zpool").args(["list", "-j", "-p"]),
        fixture("pool_list.json"),
        vec![],
        0,
    );
    let entries = list(&runner, &ListOptions::default())
        .await
        .expect("list succeeds");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "tank");
    assert_eq!(entries[0].state, "ONLINE");
    assert!(entries[0].properties.contains_key("size"));
    assert!(entries[0].properties.contains_key("free"));
}

#[tokio::test]
async fn list_named_pools_passes_args() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zpool").args(["list", "-j", "-p", "tank"]),
        fixture("pool_list.json"),
        vec![],
        0,
    );
    let opts = ListOptions {
        pools: vec!["tank".into()],
        ..Default::default()
    };
    let entries = list(&runner, &opts).await.expect("list succeeds");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "tank");
}
