use crate::error::{ZfsError, classify_stderr};
use crate::runner::{Cmd, CommandRunner};

/// `zpool set <name>=<value> <pool>`.
pub async fn set_property(
    runner: &dyn CommandRunner,
    pool: &str,
    property: &str,
    value: &str,
) -> Result<(), ZfsError> {
    let kv = format!("{property}={value}");
    let output = runner
        .run(Cmd::new("zpool").args(["set", &kv, pool]))
        .await?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(classify_stderr(&stderr, output.status.code()))
}
