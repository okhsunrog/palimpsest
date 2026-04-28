use palimpsest::pool::{GetOptions, get, get_property};
use palimpsest::{Cmd, RecordingRunner};

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("loading fixture {name}: {e}"))
}

#[tokio::test]
async fn get_property_ashift() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zpool").args(["get", "-j", "-p", "all", "tank"]),
        fixture("pool_get_all.json"),
        vec![],
        0,
    );
    let opts = GetOptions {
        pools: vec!["tank".into()],
        properties: vec!["all".into()],
    };
    let entries = get(&runner, &opts).await.expect("get succeeds");
    assert_eq!(entries.len(), 1);
    let pool = &entries[0];
    assert_eq!(pool.name, "tank");
    assert_eq!(pool.properties["ashift"].value, "12");
    assert_eq!(pool.properties["health"].value, "ONLINE");
}

#[tokio::test]
async fn get_property_single() {
    // Reuse the all-properties fixture; get_property scans for the requested key.
    let runner = RecordingRunner::new().record(
        Cmd::new("zpool").args(["get", "-j", "-p", "ashift", "tank"]),
        fixture("pool_get_all.json"),
        vec![],
        0,
    );
    let prop = get_property(&runner, "tank", "ashift")
        .await
        .expect("get_property succeeds");
    assert_eq!(prop.value, "12");
}
