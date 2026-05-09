use crate::error::{ZfsError, classify_stderr};
use crate::runner::{Cmd, CommandRunner};
use crate::send::args::SendArgs;

/// The kind of send stream described by a dry-run size estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendKind {
    Full,
    Incremental,
}

/// Result of a `zfs send -nvP` dry-run size estimate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DryRunSize {
    pub kind: SendKind,
    /// The destination snapshot (the "to" end of the stream).
    pub snapshot: String,
    /// Total byte count for the stream. For a replicate send this covers all
    /// component streams; for a simple full or incremental it equals the
    /// per-stream size.
    pub total_bytes: u64,
}

/// `zfs send -nvP` — estimate the size of a send stream without producing it.
/// Returns a `DryRunSize` parsed from the machine-parseable stdout. Returns
/// `SendArgsError` if `args` has a resume token source (use
/// `resume_token::decode()` instead).
///
/// **Output goes to stdout** in OpenZFS ≥ 2.2 with `-P` (parseable mode).
pub async fn dry_run(runner: &dyn CommandRunner, args: &SendArgs) -> Result<DryRunSize, ZfsError> {
    let cmd_args = args.build_args(true).map_err(|e| ZfsError::Other {
        exit_code: None,
        stderr: e.to_string(),
    })?;
    let output = runner.run(Cmd::new("zfs").args(cmd_args)).await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(classify_stderr(&stderr, output.status.code()));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    parse_dry_run_output(&text, output.status.code())
}

/// Parse the machine-parseable output of `zfs send -nvP`.
///
/// Full send line: `full\t<snapshot>\t<bytes>`
/// Incremental line: `incremental\t<from_snap>\t<to_snap>\t<bytes>`
/// Size summary: `size\t<total_bytes>`
fn parse_dry_run_output(text: &str, exit_code: Option<i32>) -> Result<DryRunSize, ZfsError> {
    let mut kind: Option<SendKind> = None;
    let mut snapshot: Option<String> = None;
    let mut total_bytes: Option<u64> = None;

    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        match parts.as_slice() {
            ["full", snap, _size] => {
                kind = Some(SendKind::Full);
                snapshot = Some(snap.to_string());
            }
            ["incremental", _from, to_snap, _size] => {
                kind = Some(SendKind::Incremental);
                snapshot = Some(to_snap.to_string());
            }
            ["size", size_str] => {
                total_bytes = Some(size_str.parse::<u64>().map_err(|_| ZfsError::Other {
                    exit_code,
                    stderr: format!("invalid size value in dry-run output: {size_str:?}"),
                })?);
            }
            _ => {}
        }
    }

    Ok(DryRunSize {
        kind: kind.ok_or_else(|| ZfsError::Other {
            exit_code,
            stderr: "dry-run output missing send type line (full/incremental)".to_string(),
        })?,
        snapshot: snapshot.ok_or_else(|| ZfsError::Other {
            exit_code,
            stderr: "dry-run output missing snapshot name".to_string(),
        })?,
        total_bytes: total_bytes.ok_or_else(|| ZfsError::Other {
            exit_code,
            stderr: "dry-run output missing size line".to_string(),
        })?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_send() {
        let text = "full\ttank/data/home@snap1\t44144\nsize\t44144\n";
        let result = parse_dry_run_output(text, Some(0)).unwrap();
        assert_eq!(result.kind, SendKind::Full);
        assert_eq!(result.snapshot, "tank/data/home@snap1");
        assert_eq!(result.total_bytes, 44144);
    }

    #[test]
    fn parse_incremental_send() {
        let text = "incremental\ttank/data/home@snap1\ttank/data/home@snap2\t33904\nsize\t33904\n";
        let result = parse_dry_run_output(text, Some(0)).unwrap();
        assert_eq!(result.kind, SendKind::Incremental);
        assert_eq!(result.snapshot, "tank/data/home@snap2");
        assert_eq!(result.total_bytes, 33904);
    }

    #[test]
    fn parse_empty_fails() {
        let err = parse_dry_run_output("", Some(0)).unwrap_err();
        assert!(matches!(err, ZfsError::Other { .. }));
    }

    #[tokio::test]
    async fn dry_run_full_from_fixture() {
        let fixture = std::fs::read(format!(
            "{}/tests/fixtures/send_dry_run_full.txt",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let runner = crate::runner::RecordingRunner::new().record(
            Cmd::new("zfs").args(["send", "-n", "-v", "-P", "tank/data/home@snap1"]),
            fixture,
            vec![],
            0,
        );
        let args = SendArgs::new("tank/data/home@snap1");
        let result = dry_run(&runner, &args).await.unwrap();
        assert_eq!(result.kind, SendKind::Full);
        assert_eq!(result.snapshot, "tank/data/home@snap1");
        assert_eq!(result.total_bytes, 44144);
    }

    #[tokio::test]
    async fn dry_run_incremental_from_fixture() {
        let fixture = std::fs::read(format!(
            "{}/tests/fixtures/send_dry_run_incremental.txt",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let runner = crate::runner::RecordingRunner::new().record(
            Cmd::new("zfs").args([
                "send",
                "-n",
                "-v",
                "-P",
                "-i",
                "tank/data/home@snap1",
                "tank/data/home@snap2",
            ]),
            fixture,
            vec![],
            0,
        );
        let args = SendArgs::new("tank/data/home@snap2").incremental("tank/data/home@snap1");
        let result = dry_run(&runner, &args).await.unwrap();
        assert_eq!(result.kind, SendKind::Incremental);
        assert_eq!(result.snapshot, "tank/data/home@snap2");
        assert_eq!(result.total_bytes, 33904);
    }
}
