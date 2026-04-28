use crate::error::{ZfsError, classify_stderr};
use crate::pool::{ExportOptions, ImportOptions};
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

// Best-effort check whether a pool's root dataset has encryption enabled.
// The pool is imported ephemerally with `-fN` (force, no mount), the
// `encryption` property on the root dataset is read, and the pool is
// unload-keyed and exported. Cleanup steps are best-effort; only the
// initial import error is propagated.
//
// Returns `Ok(true)` when encryption is set to anything other than `off`,
// `Ok(false)` when it is `off` or the property is missing, and `Err(...)`
// when the pool cannot be imported (typical "no such pool available").
pub async fn is_pool_encrypted(
    runner: &dyn CommandRunner,
    pool_name: &str,
) -> Result<bool, ZfsError> {
    let import_opts = ImportOptions {
        force: true,
        no_mount: true,
        ..Default::default()
    };
    crate::pool::import(runner, pool_name, &import_opts).await?;

    let encrypted = match crate::dataset::get_property(runner, pool_name, "encryption").await {
        Ok(p) => p.value != "off" && !p.value.is_empty(),
        Err(_) => false,
    };

    let _ = unload_key(runner, pool_name).await;
    let _ = crate::pool::export(runner, pool_name, &ExportOptions::default()).await;

    Ok(encrypted)
}

// Best-effort verification of a pool's passphrase. Imports the pool
// ephemerally, attempts a `zfs load-key` with the passphrase fed via stdin,
// then unload-keys and exports. The passphrase never touches the filesystem.
//
// Returns `Ok(true)` when the key loads, `Ok(false)` when load-key is
// rejected (wrong passphrase), and `Err(...)` when the pool cannot be
// imported. A best-effort pre-clean `unload_key` is issued after import so
// that `load_key_with_passphrase` always starts from an unloaded state and
// its idempotency on "already loaded" doesn't mask a wrong passphrase.
pub async fn verify_pool_passphrase(
    runner: &dyn CommandRunner,
    pool_name: &str,
    passphrase: &[u8],
) -> Result<bool, ZfsError> {
    let import_opts = ImportOptions {
        force: true,
        no_mount: true,
        ..Default::default()
    };
    crate::pool::import(runner, pool_name, &import_opts).await?;

    let _ = unload_key(runner, pool_name).await;

    let verified = load_key_with_passphrase(runner, pool_name, passphrase)
        .await
        .is_ok();

    let _ = unload_key(runner, pool_name).await;
    let _ = crate::pool::export(runner, pool_name, &ExportOptions::default()).await;

    Ok(verified)
}
