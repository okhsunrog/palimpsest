//! End-to-end exercises against real ZFS via [`SshCommandRunner`]. Gated by
//! the `integration` cargo feature; see `tests/common/mod.rs` for the
//! pool-isolation strategy and `justfile` for the VM lifecycle.

#![cfg(feature = "integration")]

mod common;

use zfskit::ZfsError;
use zfskit::dataset::{DestroyOptions, ListOptions, RollbackOptions, SnapshotOptions};
use zfskit::models::DatasetType;

use common::{LoopbackPool, ssh_runner_from_env};

#[tokio::test]
async fn snapshot_rollback_destroy_roundtrip() {
    let runner = ssh_runner_from_env();
    let pool = LoopbackPool::create(runner).await.expect("pool create");
    let zfs = pool.zfs();
    let root = zfs.dataset(pool.name()).expect("valid pool dataset name");

    let data = root
        .create_dataset("data", &Default::default())
        .await
        .expect("create child dataset");

    let snap1 = data
        .snapshot("snap1", &SnapshotOptions::new())
        .await
        .expect("snapshot snap1");
    assert!(snap1.exists().await.expect("snapshot exists probe"));

    data.snapshot("snap2", &SnapshotOptions::new())
        .await
        .expect("snapshot snap2");

    snap1
        .rollback(&RollbackOptions::new().destroy_newer())
        .await
        .expect("rollback to snap1");

    snap1
        .destroy(&DestroyOptions::new())
        .await
        .expect("destroy snap1");

    pool.destroy().await.expect("pool destroy");
}

#[tokio::test]
async fn list_recursive_with_empty_roots_returns_descendants() {
    let runner = ssh_runner_from_env();
    let pool = LoopbackPool::create(runner.clone()).await.expect("pool");
    let zfs = pool.zfs();
    let root = zfs.dataset(pool.name()).expect("valid pool dataset name");
    let a = root
        .create_dataset("a", &Default::default())
        .await
        .expect("create a");
    a.create_dataset("b", &Default::default())
        .await
        .expect("create a/b");

    let entries = zfskit::dataset::list(
        &runner,
        &ListOptions {
            recursive: true,
            types: vec![DatasetType::Filesystem, DatasetType::Volume],
            ..ListOptions::default()
        },
    )
    .await
    .expect("list with empty roots + recursive");

    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    let pname = pool.name();
    let a_full = format!("{pname}/a");
    let b_full = format!("{pname}/a/b");
    assert!(names.contains(&pname), "missing pool root in {names:?}");
    assert!(names.contains(&a_full.as_str()), "missing a in {names:?}");
    assert!(names.contains(&b_full.as_str()), "missing a/b in {names:?}");

    pool.destroy().await.expect("pool destroy");
}

#[tokio::test]
async fn destroy_held_snapshot_returns_typed_error() {
    let runner = ssh_runner_from_env();
    let pool = LoopbackPool::create(runner).await.expect("pool create");
    let zfs = pool.zfs();
    let data = zfs
        .dataset(pool.name())
        .expect("valid pool dataset name")
        .create_dataset("held", &Default::default())
        .await
        .expect("create dataset");

    let snap = data
        .snapshot("s1", &SnapshotOptions::new())
        .await
        .expect("snapshot");
    snap.hold("hold-tag").await.expect("hold");

    let err = snap
        .destroy(&DestroyOptions::new())
        .await
        .expect_err("held snapshot must error");
    assert!(
        matches!(err, ZfsError::SnapshotHeld { .. }),
        "expected SnapshotHeld, got {err:?}"
    );

    snap.release("hold-tag").await.expect("release");
    snap.destroy(&DestroyOptions::new())
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
        .expect("valid pool dataset name")
        .create_dataset("defer", &Default::default())
        .await
        .expect("create dataset");

    let snap = data
        .snapshot("s1", &SnapshotOptions::new())
        .await
        .expect("snapshot");
    snap.hold("h").await.expect("hold");

    snap.destroy(&DestroyOptions::new().defer_holds())
        .await
        .expect("defer-destroy");

    snap.release("h").await.expect("release triggers gc");

    pool.destroy().await.expect("pool destroy");
}
