use crate::error::{ZfsError, classify_stderr};
use crate::runner::CommandRunner;

// `zfs unload-key <dataset>` removes an in-memory encryption key. Returns Ok(())
// when the key is unloaded successfully or was already unloaded; classifies
// other failures via classify_stderr.
//
// TODO: idempotency on "key not currently loaded" requires a captured stderr
// fixture from real ZFS. Until then, we suppress only the success exit code;
// a future slice will tighten this.
pub async fn unload_key(runner: &dyn CommandRunner, dataset: &str) -> Result<(), ZfsError> {
    let output = runner.run("zfs", &["unload-key", dataset]).await?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(classify_stderr(&stderr, output.status.code()))
}

// `zfs load-key <dataset>` loads an encryption key. The key source comes from
// the dataset's `keylocation` property (file://, prompt, https://, http://).
// For the prompt case, callers must arrange stdin themselves; this function
// only invokes the bare CLI.
pub async fn load_key(runner: &dyn CommandRunner, dataset: &str) -> Result<(), ZfsError> {
    let output = runner.run("zfs", &["load-key", dataset]).await?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(classify_stderr(&stderr, output.status.code()))
}
