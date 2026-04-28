use crate::error::{ZfsError, classify_stderr};
use crate::runner::{Cmd, CommandRunner};

const BOOKMARK_EXISTS: &str = "bookmark exists";

/// `zfs bookmark <snapshot> <bookmark>` — creates a bookmark. Idempotent:
/// returns Ok when a bookmark with the same name already exists (covers the
/// arctern pattern of re-creating the same bookmark after a successful send).
///
/// Full GUID-equivalence checking (to distinguish a same-name bookmark from a
/// different snapshot) requires additional `zfs get guid` calls; that path is
/// not yet tested with fixtures and is deferred.
pub async fn create(
    runner: &dyn CommandRunner,
    snapshot: &str,
    bookmark: &str,
) -> Result<(), ZfsError> {
    let output = runner
        .run(Cmd::new("zfs").args(["bookmark", snapshot, bookmark]))
        .await?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains(BOOKMARK_EXISTS) {
        return Ok(());
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
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["bookmark", "tank/data/home@snap1", "tank/data/home#bm1"]),
            vec![],
            err_text,
            1,
        );
        create(&runner, "tank/data/home@snap1", "tank/data/home#bm1")
            .await
            .expect("idempotent create should return Ok");
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
