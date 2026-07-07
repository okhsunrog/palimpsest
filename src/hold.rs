use crate::error::{ZfsError, classify_stderr};
use crate::runner::{Cmd, CommandRunner};

const TAG_ALREADY_EXISTS: &str = "tag already exists on this dataset";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hold {
    pub dataset: String,
    pub tag: String,
    pub timestamp: u64,
}

/// `zfs hold <tag> <snapshot>` — places a user hold. Idempotent: returns Ok
/// when the tag already exists on the snapshot.
pub async fn hold(runner: &dyn CommandRunner, snapshot: &str, tag: &str) -> Result<(), ZfsError> {
    let output = runner
        .run(Cmd::new("zfs").args(["hold", tag, snapshot]))
        .await?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains(TAG_ALREADY_EXISTS) {
        return Ok(());
    }
    Err(classify_stderr(&stderr, output.status.code()))
}

/// `zfs release <tag> <snapshot>` — releases a user hold. Not idempotent:
/// releasing a non-existent hold is an error.
pub async fn release(
    runner: &dyn CommandRunner,
    snapshot: &str,
    tag: &str,
) -> Result<(), ZfsError> {
    let output = runner
        .run(Cmd::new("zfs").args(["release", tag, snapshot]))
        .await?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(classify_stderr(&stderr, output.status.code()))
}

/// `zfs holds -p -H <snapshot>` — lists user holds. Parses the
/// tab-separated output (NAME, TAG, TIMESTAMP columns; `-H` suppresses
/// the header row; `-p` forces the timestamp to print as unix seconds,
/// without which `zfs holds` emits human-readable dates ("Thu May 14
/// 13:09 2026") that no parser should try to interpret.
pub async fn list_holds(runner: &dyn CommandRunner, snapshot: &str) -> Result<Vec<Hold>, ZfsError> {
    let output = runner
        .run(Cmd::new("zfs").args(["holds", "-p", "-H", snapshot]))
        .await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(classify_stderr(&stderr, output.status.code()));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    parse_holds_text(&text)
}

/// `zfs holds -p -H <snap> <snap>...` — list user holds across many
/// snapshots in one invocation. Callers sweeping a dataset for stale
/// hold tags want this over N per-snapshot `list_holds` calls.
///
/// Best-effort on partial failure: if some of the named snapshots have
/// vanished between the caller's `zfs list` and this call, `zfs holds`
/// exits non-zero but still prints rows for the survivors — those rows
/// are returned. A non-zero exit with no parseable rows is a real error.
pub async fn list_holds_many(
    runner: &dyn CommandRunner,
    snapshots: &[&str],
) -> Result<Vec<Hold>, ZfsError> {
    if snapshots.is_empty() {
        return Ok(Vec::new());
    }
    let output = runner
        .run(
            Cmd::new("zfs")
                .args(["holds", "-p", "-H"])
                .args(snapshots.iter().copied()),
        )
        .await?;
    let text = String::from_utf8_lossy(&output.stdout);
    let holds = parse_holds_text(&text)?;
    if !output.status.success() && holds.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(classify_stderr(&stderr, output.status.code()));
    }
    Ok(holds)
}

fn parse_holds_text(text: &str) -> Result<Vec<Hold>, ZfsError> {
    let mut holds = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        if parts.len() != 3 {
            return Err(ZfsError::Other {
                exit_code: None,
                stderr: format!("unexpected holds line: {line:?}"),
            });
        }
        let timestamp = parts[2].parse::<u64>().map_err(|_| ZfsError::Other {
            exit_code: None,
            stderr: format!("invalid timestamp in holds output: {}", parts[2]),
        })?;
        holds.push(Hold {
            dataset: parts[0].to_string(),
            tag: parts[1].to_string(),
            timestamp,
        });
    }
    Ok(holds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::RecordingRunner;

    #[test]
    fn parse_single_hold() {
        let holds = parse_holds_text("tank/data/home@snap1\tmytag\t1777313280\n").unwrap();
        assert_eq!(holds.len(), 1);
        assert_eq!(holds[0].dataset, "tank/data/home@snap1");
        assert_eq!(holds[0].tag, "mytag");
        assert_eq!(holds[0].timestamp, 1_777_313_280);
    }

    #[test]
    fn parse_empty_output() {
        let holds = parse_holds_text("").unwrap();
        assert!(holds.is_empty());
    }

    #[test]
    fn parse_rejects_malformed_line() {
        let err = parse_holds_text("tank/data@snap1\tmytag\n").unwrap_err();
        assert!(matches!(err, ZfsError::Other { .. }));
    }

    #[tokio::test]
    async fn list_holds_parses_fixture() {
        let fixture = std::fs::read(format!(
            "{}/tests/fixtures/holds.txt",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["holds", "-p", "-H", "tank/data/home@snap1"]),
            fixture,
            vec![],
            0,
        );
        let holds = list_holds(&runner, "tank/data/home@snap1").await.unwrap();
        assert_eq!(holds.len(), 1);
        assert_eq!(holds[0].dataset, "tank/data/home@snap1");
        assert_eq!(holds[0].tag, "mytag");
        assert_eq!(holds[0].timestamp, 1_777_313_280);
    }

    #[tokio::test]
    async fn list_holds_many_empty_input_skips_spawn() {
        // No fixture recorded — any spawn would error NotFound.
        let runner = RecordingRunner::new();
        let holds = list_holds_many(&runner, &[]).await.unwrap();
        assert!(holds.is_empty());
    }

    #[tokio::test]
    async fn list_holds_many_parses_multiple_snapshots() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["holds", "-p", "-H", "tank/d@s1", "tank/d@s2"]),
            b"tank/d@s1\ttag_a\t1777313280\ntank/d@s2\ttag_b\t1777313281\n".to_vec(),
            vec![],
            0,
        );
        let holds = list_holds_many(&runner, &["tank/d@s1", "tank/d@s2"])
            .await
            .unwrap();
        assert_eq!(holds.len(), 2);
        assert_eq!(holds[0].dataset, "tank/d@s1");
        assert_eq!(holds[1].tag, "tag_b");
    }

    #[tokio::test]
    async fn list_holds_many_partial_failure_returns_survivors() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["holds", "-p", "-H", "tank/d@s1", "tank/d@gone"]),
            b"tank/d@s1\ttag_a\t1777313280\n".to_vec(),
            b"cannot open 'tank/d@gone': dataset does not exist\n".to_vec(),
            1,
        );
        let holds = list_holds_many(&runner, &["tank/d@s1", "tank/d@gone"])
            .await
            .unwrap();
        assert_eq!(holds.len(), 1);
        assert_eq!(holds[0].tag, "tag_a");
    }

    #[tokio::test]
    async fn list_holds_many_total_failure_errors() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["holds", "-p", "-H", "tank/d@gone"]),
            vec![],
            b"cannot open 'tank/d@gone': dataset does not exist\n".to_vec(),
            1,
        );
        let err = list_holds_many(&runner, &["tank/d@gone"]).await.unwrap_err();
        assert!(matches!(err, ZfsError::DatasetNotFound { .. }));
    }

    #[tokio::test]
    async fn hold_is_idempotent_on_tag_already_exists() {
        let err_text = std::fs::read(format!(
            "{}/tests/fixtures/err_hold_already_exists.txt",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["hold", "mytag", "tank/data/home@snap1"]),
            vec![],
            err_text,
            1,
        );
        hold(&runner, "tank/data/home@snap1", "mytag")
            .await
            .expect("idempotent hold should return Ok");
    }
}
