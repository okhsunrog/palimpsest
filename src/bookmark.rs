use crate::error::{ZfsError, classify_stderr};
use crate::names::{BookmarkName, SnapshotName};
use crate::runner::{Cmd, CommandRunner};

const BOOKMARK_EXISTS: &str = "bookmark exists";

/// `zfs bookmark <snapshot> <bookmark>` — creates a bookmark. Idempotent:
/// returns Ok when a bookmark with the same name already exists (covers the
/// arctern pattern of re-creating the same bookmark after a successful send).
///
/// If the name already exists, snapshot and bookmark GUIDs are compared so a
/// same-name bookmark pointing at a different snapshot is reported as a
/// conflict rather than silently accepted.
pub async fn create(
    runner: &dyn CommandRunner,
    snapshot: &str,
    bookmark: &str,
) -> Result<(), ZfsError> {
    let snapshot_name = SnapshotName::parse(snapshot)?;
    let bookmark_name = BookmarkName::parse(bookmark)?;
    if snapshot_name.dataset() != bookmark_name.dataset() {
        return Err(ZfsError::InvalidInput {
            message: "snapshot and bookmark must belong to the same dataset".to_string(),
        });
    }
    let output = runner
        .run(Cmd::new("zfs").args(["bookmark", snapshot_name.as_str(), bookmark_name.as_str()]))
        .await?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains(BOOKMARK_EXISTS) {
        let snapshot_guid =
            crate::dataset::get_property(runner, snapshot_name.as_str(), "guid").await?;
        let bookmark_guid =
            crate::dataset::get_property(runner, bookmark_name.as_str(), "guid").await?;
        if snapshot_guid.value == bookmark_guid.value {
            return Ok(());
        }
        return Err(ZfsError::BookmarkConflict {
            bookmark: bookmark_name.to_string(),
        });
    }
    Err(classify_stderr(&stderr, output.status.code()))
}

/// `zfs destroy <bookmark>` — destroys a bookmark. Not idempotent: destroying
/// a non-existent bookmark is an error.
pub async fn destroy(runner: &dyn CommandRunner, bookmark: &str) -> Result<(), ZfsError> {
    let output = runner
        .run(Cmd::new("zfs").args(["destroy", bookmark]))
        .await?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(classify_stderr(&stderr, output.status.code()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::RecordingRunner;

    fn guid_json(name: &str, kind: &str, guid: &str) -> Vec<u8> {
        format!(
            "{{\"output_version\":{{\"command\":\"zfs get\",\"vers_major\":0,\"vers_minor\":1}},\
             \"datasets\":{{\"{name}\":{{\"name\":\"{name}\",\"type\":\"{kind}\",\
             \"pool\":\"tank\",\"createtxg\":\"1\",\"properties\":{{\"guid\":\
             {{\"value\":\"{guid}\",\"source\":{{\"type\":\"NONE\",\"data\":\"-\"}}}}}}}}}}}}"
        )
        .into_bytes()
    }

    #[tokio::test]
    async fn create_succeeds() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["bookmark", "tank/data/home@snap1", "tank/data/home#bm1"]),
            vec![],
            vec![],
            0,
        );
        create(&runner, "tank/data/home@snap1", "tank/data/home#bm1")
            .await
            .expect("create should succeed");
    }

    #[tokio::test]
    async fn create_is_idempotent_on_bookmark_exists() {
        let err_text = std::fs::read(format!(
            "{}/tests/fixtures/err_bookmark_exists.txt",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let runner = RecordingRunner::new()
            .record(
                Cmd::new("zfs").args(["bookmark", "tank/data/home@snap1", "tank/data/home#bm1"]),
                vec![],
                err_text,
                1,
            )
            .record(
                Cmd::new("zfs").args(["get", "-j", "-p", "guid", "tank/data/home@snap1"]),
                guid_json("tank/data/home@snap1", "SNAPSHOT", "42"),
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args(["get", "-j", "-p", "guid", "tank/data/home#bm1"]),
                guid_json("tank/data/home#bm1", "BOOKMARK", "42"),
                vec![],
                0,
            );
        create(&runner, "tank/data/home@snap1", "tank/data/home#bm1")
            .await
            .expect("idempotent create should return Ok");
    }

    #[tokio::test]
    async fn create_rejects_existing_bookmark_for_other_snapshot() {
        let runner = RecordingRunner::new()
            .record(
                Cmd::new("zfs").args(["bookmark", "tank/data@snap1", "tank/data#cursor"]),
                vec![],
                b"cannot create bookmark 'tank/data#cursor': bookmark exists\n".to_vec(),
                1,
            )
            .record(
                Cmd::new("zfs").args(["get", "-j", "-p", "guid", "tank/data@snap1"]),
                guid_json("tank/data@snap1", "SNAPSHOT", "42"),
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args(["get", "-j", "-p", "guid", "tank/data#cursor"]),
                guid_json("tank/data#cursor", "BOOKMARK", "99"),
                vec![],
                0,
            );
        let error = create(&runner, "tank/data@snap1", "tank/data#cursor")
            .await
            .unwrap_err();
        assert!(matches!(error, ZfsError::BookmarkConflict { .. }));
    }

    #[tokio::test]
    async fn destroy_bookmark_succeeds() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["destroy", "tank/data/home#bm1"]),
            vec![],
            vec![],
            0,
        );
        destroy(&runner, "tank/data/home#bm1")
            .await
            .expect("destroy should succeed");
    }

    #[tokio::test]
    async fn destroy_not_found_returns_error() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["destroy", "tank/data/home#bm_missing"]),
            vec![],
            b"cannot open 'tank/data/home#bm_missing': dataset does not exist\n".to_vec(),
            1,
        );
        let err = destroy(&runner, "tank/data/home#bm_missing")
            .await
            .expect_err("should fail for missing bookmark");
        assert!(matches!(err, ZfsError::DatasetNotFound { .. }));
    }
}
