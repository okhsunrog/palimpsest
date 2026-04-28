use crate::error::{ZfsError, classify_stderr};
use crate::runner::{Cmd, CommandRunner};

// Idempotency markers captured from real OpenZFS 2.4.1 stderr.
// See tests/fixtures/err_unload_key_not_loaded.stderr and
// tests/fixtures/err_load_key_already.stderr.
const ALREADY_UNLOADED: &str = "Key already unloaded";
const ALREADY_LOADED: &str = "Key already loaded";

// `zfs unload-key <dataset>` removes an in-memory encryption key. Idempotent:
// returns Ok(()) on success and on the "Key already unloaded" stderr.
// Other failures route through classify_stderr.
pub async fn unload_key(runner: &dyn CommandRunner, dataset: &str) -> Result<(), ZfsError> {
    let output = runner
        .run(Cmd::new("zfs").args(["unload-key", dataset]))
        .await?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains(ALREADY_UNLOADED) {
        return Ok(());
    }
    Err(classify_stderr(&stderr, output.status.code()))
}

// `zfs load-key <dataset>` loads an encryption key. Idempotent on the
// "Key already loaded" stderr. Reads the key from the dataset's `keylocation`
// property (file://, prompt, https://, http://).
pub async fn load_key(runner: &dyn CommandRunner, dataset: &str) -> Result<(), ZfsError> {
    let output = runner
        .run(Cmd::new("zfs").args(["load-key", dataset]))
        .await?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains(ALREADY_LOADED) {
        return Ok(());
    }
    Err(classify_stderr(&stderr, output.status.code()))
}

// `zfs load-key <dataset>` with the passphrase delivered via stdin. Equivalent
// to invoking `zfs load-key -L prompt <dataset>` and typing the passphrase, but
// piped — no terminal interaction required. Idempotent on "Key already loaded".
//
// The passphrase is marked secret on the Cmd so it is redacted from any
// Display/Debug output (e.g., RecordingRunner's "no fixture" error).
pub async fn load_key_with_passphrase(
    runner: &dyn CommandRunner,
    dataset: &str,
    passphrase: &[u8],
) -> Result<(), ZfsError> {
    let output = runner
        .run(
            Cmd::new("zfs")
                .args(["load-key", dataset])
                .stdin_secret(passphrase.to_vec()),
        )
        .await?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains(ALREADY_LOADED) {
        return Ok(());
    }
    Err(classify_stderr(&stderr, output.status.code()))
}
