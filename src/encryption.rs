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
                .args(["load-key", "-L", "prompt", dataset])
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

// `zfs load-key -L <keylocation> <dataset>` — load a key with an explicit
// keylocation override. Useful when the dataset's stored `keylocation` is
// `prompt` but we have the key in a known file path (e.g., the install
// pipeline writes the key file to /etc/zfs/zroot.key and loads from there).
//
// `keylocation` is a ZFS-format string: `file:///path`, `prompt`,
// `https://...`, `http://...`. Idempotent on "Key already loaded".
pub async fn load_key_with_keylocation(
    runner: &dyn CommandRunner,
    dataset: &str,
    keylocation: &str,
) -> Result<(), ZfsError> {
    let output = runner
        .run(Cmd::new("zfs").args(["load-key", "-L", keylocation, dataset]))
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

/// Verify a passphrase without changing key state.
///
/// The dataset must already be imported. Only OpenZFS's explicit incorrect-key
/// response maps to `Ok(false)`; transport, permission, and other failures are
/// returned to the caller.
pub async fn verify_passphrase(
    runner: &dyn CommandRunner,
    dataset: &str,
    passphrase: &[u8],
) -> Result<bool, ZfsError> {
    let output = runner
        .run(
            Cmd::new("zfs")
                .args(["load-key", "-n", "-L", "prompt", dataset])
                .stdin_secret(passphrase.to_vec()),
        )
        .await?;
    if output.status.success() {
        return Ok(true);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("Incorrect key provided") {
        return Ok(false);
    }
    Err(classify_stderr(&stderr, output.status.code()))
}
