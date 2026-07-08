use crate::error::{ZfsError, classify_stderr};
use crate::runner::{Cmd, CommandRunner};

/// What to do with the pool's current scrub.
#[derive(Debug, Clone, Copy)]
pub enum ScrubAction {
    /// `zpool scrub <pool>` — start a new scrub, or no-op if one's running.
    Start,
    /// `zpool scrub -p <pool>` — pause an in-progress scrub. ZFS persists
    /// the pause across reboots; call `Resume` to continue.
    Pause,
    /// `zpool scrub <pool>` on a paused scrub continues it from where it
    /// paused. There is no dedicated resume flag — `-p` on an already-
    /// paused scrub is an error ("scrub is paused; use 'zpool scrub' to
    /// resume"), so Resume maps to the bare command like Start; zpool
    /// distinguishes them by the pool's current scan state.
    Resume,
    /// `zpool scrub -s <pool>` — cancel and discard scrub progress.
    Stop,
}

impl ScrubAction {
    fn flag(self) -> Option<&'static str> {
        match self {
            ScrubAction::Start | ScrubAction::Resume => None,
            ScrubAction::Pause => Some("-p"),
            ScrubAction::Stop => Some("-s"),
        }
    }
}

/// `zpool scrub [-p | -s] <pool>` — start / pause-resume / stop a scrub.
/// Returns `Ok(())` when zpool exits 0; non-zero stderr is classified
/// through `classify_stderr` so callers can match on idempotent cases
/// (e.g., "scrub is already in progress").
pub async fn scrub(
    runner: &dyn CommandRunner,
    pool: &str,
    action: ScrubAction,
) -> Result<(), ZfsError> {
    let mut argv: Vec<&str> = vec!["scrub"];
    if let Some(f) = action.flag() {
        argv.push(f);
    }
    argv.push(pool);
    let output = runner.run(Cmd::new("zpool").args(argv)).await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(classify_stderr(&stderr, output.status.code()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::RecordingRunner;

    async fn assert_argv(action: ScrubAction, argv: &[&str]) {
        let runner = RecordingRunner::new().record(
            Cmd::new("zpool").args(argv.to_vec()),
            Vec::new(),
            Vec::new(),
            0,
        );
        scrub(&runner, "tank", action).await.unwrap();
    }

    #[tokio::test]
    async fn start_is_bare_scrub() {
        assert_argv(ScrubAction::Start, &["scrub", "tank"]).await;
    }

    #[tokio::test]
    async fn pause_uses_dash_p() {
        assert_argv(ScrubAction::Pause, &["scrub", "-p", "tank"]).await;
    }

    /// Regression: `-p` on an already-paused scrub errors with "use
    /// 'zpool scrub' to resume" — resume must be the bare command.
    #[tokio::test]
    async fn resume_is_bare_scrub_not_dash_p() {
        assert_argv(ScrubAction::Resume, &["scrub", "tank"]).await;
    }

    #[tokio::test]
    async fn stop_uses_dash_s() {
        assert_argv(ScrubAction::Stop, &["scrub", "-s", "tank"]).await;
    }
}
