use crate::error::{ZfsError, classify_stderr};
use crate::runner::{Cmd, CommandRunner};

/// `zfs set <name>=<value> <dataset>`.
pub async fn set_property(
    runner: &dyn CommandRunner,
    dataset: &str,
    property: &str,
    value: &str,
) -> Result<(), ZfsError> {
    let kv = format!("{property}={value}");
    let output = runner
        .run(Cmd::new("zfs").args(["set", &kv, dataset]))
        .await?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(classify_stderr(&stderr, output.status.code()))
}
