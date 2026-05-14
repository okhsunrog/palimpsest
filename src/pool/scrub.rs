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
    /// `zpool scrub -p <pool>` again on a paused scrub resumes it. ZFS
    /// itself toggles pause/resume with the same flag.
    Resume,
    /// `zpool scrub -s <pool>` — cancel and discard scrub progress.
    Stop,
}

impl ScrubAction {
    fn flag(self) -> Option<&'static str> {
        match self {
            ScrubAction::Start => None,
            ScrubAction::Pause | ScrubAction::Resume => Some("-p"),
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
