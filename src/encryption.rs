use crate::error::{ZfsError, classify_stderr};
use crate::runner::CommandRunner;

// Idempotency markers captured from real OpenZFS 2.4.1 stderr.
// See tests/fixtures/err_unload_key_not_loaded.stderr and
// tests/fixtures/err_load_key_already.stderr.
const ALREADY_UNLOADED: &str = "Key already unloaded";
const ALREADY_LOADED: &str = "Key already loaded";

// `zfs unload-key <dataset>` removes an in-memory encryption key. Idempotent:
// returns Ok(()) on success and on the "Key already unloaded" stderr.
// Other failures route through classify_stderr.
pub async fn unload_key(runner: &dyn CommandRunner, dataset: &str) -> Result<(), ZfsError> {
    let output = runner.run("zfs", &["unload-key", dataset]).await?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains(ALREADY_UNLOADED) {
        return Ok(());
    }
    Err(classify_stderr(&stderr, output.status.code()))
}

// `zfs load-key <dataset>` loads an encryption key. Idempotent:
// returns Ok(()) on success and on the "Key already loaded" stderr.
// The key source comes from the dataset's `keylocation` property
// (file://, prompt, https://, http://). For the `prompt` case, callers must
// arrange stdin themselves via a future stdin-aware runner variant — this
// function only invokes the bare CLI.
pub async fn load_key(runner: &dyn CommandRunner, dataset: &str) -> Result<(), ZfsError> {
    let output = runner.run("zfs", &["load-key", dataset]).await?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains(ALREADY_LOADED) {
        return Ok(());
    }
    Err(classify_stderr(&stderr, output.status.code()))
}
