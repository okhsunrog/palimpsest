use crate::error::{ZfsError, classify_stderr};
use crate::models::{ZpoolStatusEntry, ZpoolStatusOutput};
use crate::runner::{Cmd, CommandRunner};

/// `zpool status -j <pool>` — returns the status entry for a single pool.
pub async fn status(runner: &dyn CommandRunner, pool: &str) -> Result<ZpoolStatusEntry, ZfsError> {
    let output = runner
        .run(Cmd::new("zpool").args(["status", "-j", pool]))
        .await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(classify_stderr(&stderr, output.status.code()));
    }
    let parsed: ZpoolStatusOutput =
        serde_json::from_slice(&output.stdout).map_err(|e| ZfsError::Parse {
            command: "zpool status",
            message: e.to_string(),
        })?;
    parsed.output_version.validate("zpool status")?;
    parsed
        .pools
        .into_values()
        .find(|p| p.name == pool)
        .ok_or_else(|| ZfsError::Other {
            exit_code: None,
            stderr: format!("zpool status returned no entry for {pool}"),
        })
}

/// `zpool status -j` (no pool argument) — returns status for all pools.
pub async fn status_all(runner: &dyn CommandRunner) -> Result<Vec<ZpoolStatusEntry>, ZfsError> {
    let output = runner.run(Cmd::new("zpool").args(["status", "-j"])).await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(classify_stderr(&stderr, output.status.code()));
    }
    let parsed: ZpoolStatusOutput =
        serde_json::from_slice(&output.stdout).map_err(|e| ZfsError::Parse {
            command: "zpool status",
            message: e.to_string(),
        })?;
    parsed.output_version.validate("zpool status")?;
    let mut entries: Vec<ZpoolStatusEntry> = parsed.pools.into_values().collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}
