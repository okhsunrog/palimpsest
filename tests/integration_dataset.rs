//! End-to-end exercises against real ZFS via [`SshCommandRunner`]. Gated by
//! the `integration` cargo feature; see `tests/common/mod.rs` for the
//! pool-isolation strategy and `justfile` for the VM lifecycle.

#![cfg(feature = "integration")]

mod common;

use palimpsest::ZfsError;
use palimpsest::dataset::{DestroyOptions, RollbackOptions, SnapshotOptions};

use common::{LoopbackPool, ssh_runner_from_env};

#[tokio::test]
async fn snapshot_rollback_destroy_roundtrip() {
    let runner = ssh_runner_from_env();
    let pool = LoopbackPool::create(runner).await.expect("pool create");
    let zfs = pool.zfs();
    let root = zfs.dataset(pool.name());

    let data = root
        .create_dataset("data", &Default::default())
        .await
        .expect("create child dataset");

    let snap1 = data
        .snapshot("snap1", &SnapshotOptions::new())
        .await
        .expect("snapshot snap1");
    assert!(snap1.exists().await);

    data.snapshot("snap2", &SnapshotOptions::new())
        .await
        .expect("snapshot snap2");

    data.rollback("snap1", &RollbackOptions::new().destroy_newer())
        .await
        .expect("rollback to snap1");

    data.destroy_snapshot("snap1", &DestroyOptions::new())
        .await
        .expect("destroy snap1");

    pool.destroy().await.expect("pool destroy");
}

#[tokio::test]
async fn destroy_held_snapshot_returns_typed_error() {
    let runner = ssh_runner_from_env();
    let pool = LoopbackPool::create(runner).await.expect("pool create");
    let zfs = pool.zfs();
    let data = zfs
        .dataset(pool.name())
        .create_dataset("held", &Default::default())
        .await
        .expect("create dataset");

    let snap = data
        .snapshot("s1", &SnapshotOptions::new())
        .await
        .expect("snapshot");
    snap.hold("hold-tag").await.expect("hold");

    let err = data
        .destroy_snapshot("s1", &DestroyOptions::new())
        .await
        .expect_err("held snapshot must error");
    assert!(
        matches!(err, ZfsError::SnapshotHeld { .. }),
        "expected SnapshotHeld, got {err:?}"
    );

    snap.release("hold-tag").await.expect("release");
    data.destroy_snapshot("s1", &DestroyOptions::new())
        .await
        .expect("destroy after release");

    pool.destroy().await.expect("pool destroy");
}

#[tokio::test]
async fn defer_destroy_marks_held_snapshot() {
    let runner = ssh_runner_from_env();
    let pool = LoopbackPool::create(runner).await.expect("pool create");
    let zfs = pool.zfs();
    let data = zfs
        .dataset(pool.name())
        .create_dataset("defer", &Default::default())
        .await
        .expect("create dataset");

    let snap = data
        .snapshot("s1", &SnapshotOptions::new())
        .await
        .expect("snapshot");
    snap.hold("h").await.expect("hold");

    data.destroy_snapshot("s1", &DestroyOptions::new().defer_holds())
        .await
        .expect("defer-destroy");

    snap.release("h").await.expect("release triggers gc");

    pool.destroy().await.expect("pool destroy");
}
