use crate::error::{ZfsError, classify_stderr};
use crate::models::{PropertyValue, ZpoolGetEntry, ZpoolGetOutput};
use crate::runner::{Cmd, CommandRunner};

#[derive(Default, Clone, Debug)]
pub struct GetOptions {
    pub pools: Vec<String>,
    /// MUST contain at least one entry. Use the special token "all" for every property.
    pub properties: Vec<String>,
}

impl GetOptions {
    pub fn build_args(&self) -> Result<Vec<String>, ZfsError> {
        if self.properties.is_empty() {
            return Err(ZfsError::InvalidInput {
                message:
                    "GetOptions::properties must not be empty (use vec![\"all\".into()] for all)"
                        .to_string(),
            });
        }
        let mut args: Vec<String> = vec!["get".into(), "-j".into(), "-p".into()];
        args.push(self.properties.join(","));
        for p in &self.pools {
            args.push(p.clone());
        }
        Ok(args)
    }
}

pub async fn get(
    runner: &dyn CommandRunner,
    opts: &GetOptions,
) -> Result<Vec<ZpoolGetEntry>, ZfsError> {
    let args = opts.build_args()?;
    let output = runner.run(Cmd::new("zpool").args(args)).await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(classify_stderr(&stderr, output.status.code()));
    }
    let parsed: ZpoolGetOutput =
        serde_json::from_slice(&output.stdout).map_err(|e| ZfsError::Parse {
            command: "zpool get",
            message: e.to_string(),
        })?;
    parsed.output_version.validate("zpool get")?;
    let mut entries: Vec<ZpoolGetEntry> = parsed.pools.into_values().collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

pub async fn get_property(
    runner: &dyn CommandRunner,
    pool: &str,
    property: &str,
) -> Result<PropertyValue, ZfsError> {
    let opts = GetOptions {
        pools: vec![pool.to_string()],
        properties: vec![property.to_string()],
    };
    let entries = get(runner, &opts).await?;
    let entry = entries.into_iter().next().ok_or_else(|| ZfsError::Other {
        exit_code: None,
        stderr: format!("zpool get returned no entries for {pool}"),
    })?;
    entry
        .properties
        .into_iter()
        .find(|(k, _)| k == property)
        .map(|(_, v)| v)
        .ok_or_else(|| ZfsError::Other {
            exit_code: None,
            stderr: format!("property '{property}' not in response for {pool}"),
        })
}
