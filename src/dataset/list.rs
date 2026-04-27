use crate::error::{ZfsError, classify_stderr};
use crate::models::{DatasetType, ZfsListEntry, ZfsListOutput};
use crate::runner::CommandRunner;

#[derive(Default, Clone, Debug)]
pub struct ListOptions {
    pub recursive: bool,
    pub depth: Option<u32>,
    pub types: Vec<DatasetType>,
    // Empty = list all datasets visible to the caller.
    pub roots: Vec<String>,
    // Empty = let zfs return its default property set. Order is preserved on the wire.
    pub properties: Vec<String>,
}

impl ListOptions {
    pub fn build_args(&self) -> Vec<String> {
        // Always -j for native JSON output and -p for raw numeric values.
        let mut args: Vec<String> = vec!["list".into(), "-j".into(), "-p".into()];
        if self.recursive {
            args.push("-r".into());
        }
        if let Some(d) = self.depth {
            args.push("-d".into());
            args.push(d.to_string());
        }
        if !self.types.is_empty() {
            args.push("-t".into());
            args.push(
                self.types
                    .iter()
                    .map(|t| t.cli_name())
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
        if !self.properties.is_empty() {
            args.push("-o".into());
            args.push(self.properties.join(","));
        }
        for r in &self.roots {
            args.push(r.clone());
        }
        args
    }
}

pub async fn list(
    runner: &dyn CommandRunner,
    opts: &ListOptions,
) -> Result<Vec<ZfsListEntry>, ZfsError> {
    let args = opts.build_args();
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = runner.run("zfs", &arg_refs).await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(classify_stderr(&stderr, output.status.code()));
    }
    let parsed: ZfsListOutput =
        serde_json::from_slice(&output.stdout).map_err(|e| ZfsError::Other {
            exit_code: output.status.code(),
            stderr: format!("failed to parse zfs list -j output: {e}"),
        })?;
    let mut entries: Vec<ZfsListEntry> = parsed.datasets.into_values().collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_args_default() {
        let opts = ListOptions::default();
        assert_eq!(opts.build_args(), vec!["list", "-j", "-p"]);
    }

    #[test]
    fn build_args_full() {
        let opts = ListOptions {
            recursive: true,
            depth: Some(2),
            types: vec![DatasetType::Filesystem, DatasetType::Snapshot],
            roots: vec!["tank".into(), "tank/sub".into()],
            properties: vec!["name".into(), "used".into()],
        };
        assert_eq!(
            opts.build_args(),
            vec![
                "list", "-j", "-p", "-r", "-d", "2", "-t", "filesystem,snapshot", "-o",
                "name,used", "tank", "tank/sub"
            ]
        );
    }
}
