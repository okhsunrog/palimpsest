use crate::error::{ZfsError, classify_stderr};
use crate::models::{DatasetType, PropertySourceKind, PropertyValue, ZfsGetEntry, ZfsGetOutput};
use crate::runner::{Cmd, CommandRunner};

#[derive(Default, Clone, Debug)]
pub struct GetOptions {
    pub recursive: bool,
    pub depth: Option<u32>,
    pub types: Vec<DatasetType>,
    pub sources: Vec<PropertySourceKind>,
    // Empty = the caller's default datasets (zfs(8) lists all visible).
    pub datasets: Vec<String>,
    // MUST contain at least one entry. Use the special token "all" to request every property.
    pub properties: Vec<String>,
}

impl GetOptions {
    pub fn build_args(&self) -> Result<Vec<String>, ZfsError> {
        if self.properties.is_empty() {
            return Err(ZfsError::Other {
                exit_code: None,
                stderr:
                    "GetOptions::properties must not be empty (use vec![\"all\".into()] for all)"
                        .to_string(),
            });
        }
        let mut args: Vec<String> = vec!["get".into(), "-j".into(), "-p".into()];
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
        if !self.sources.is_empty() {
            args.push("-s".into());
            args.push(
                self.sources
                    .iter()
                    .map(|s| s.cli_name())
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
        args.push(self.properties.join(","));
        for d in &self.datasets {
            args.push(d.clone());
        }
        Ok(args)
    }
}

pub async fn get(
    runner: &dyn CommandRunner,
    opts: &GetOptions,
) -> Result<Vec<ZfsGetEntry>, ZfsError> {
    let args = opts.build_args()?;
    let output = runner.run(Cmd::new("zfs").args(args)).await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(classify_stderr(&stderr, output.status.code()));
    }
    let parsed: ZfsGetOutput =
        serde_json::from_slice(&output.stdout).map_err(|e| ZfsError::Other {
            exit_code: output.status.code(),
            stderr: format!("failed to parse zfs get -j output: {e}"),
        })?;
    let mut entries: Vec<ZfsGetEntry> = parsed.datasets.into_values().collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

pub async fn get_property(
    runner: &dyn CommandRunner,
    dataset: &str,
    property: &str,
) -> Result<PropertyValue, ZfsError> {
    let opts = GetOptions {
        datasets: vec![dataset.to_string()],
        properties: vec![property.to_string()],
        ..Default::default()
    };
    let entries = get(runner, &opts).await?;
    let entry = entries.into_iter().next().ok_or_else(|| ZfsError::Other {
        exit_code: None,
        stderr: format!("zfs get returned no entries for {dataset}"),
    })?;
    entry
        .properties
        .into_iter()
        .find(|(k, _)| k == property)
        .map(|(_, v)| v)
        .ok_or_else(|| ZfsError::Other {
            exit_code: None,
            stderr: format!("property '{property}' not in response for {dataset}"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_args_minimal() {
        let opts = GetOptions {
            properties: vec!["encryption".into()],
            datasets: vec!["tank".into()],
            ..Default::default()
        };
        assert_eq!(
            opts.build_args().unwrap(),
            vec!["get", "-j", "-p", "encryption", "tank"]
        );
    }

    #[test]
    fn build_args_full() {
        let opts = GetOptions {
            recursive: true,
            depth: Some(2),
            types: vec![DatasetType::Filesystem],
            sources: vec![PropertySourceKind::Local, PropertySourceKind::Inherited],
            datasets: vec!["tank/data".into()],
            properties: vec!["all".into()],
        };
        assert_eq!(
            opts.build_args().unwrap(),
            vec![
                "get",
                "-j",
                "-p",
                "-r",
                "-d",
                "2",
                "-t",
                "filesystem",
                "-s",
                "local,inherited",
                "all",
                "tank/data",
            ]
        );
    }

    #[test]
    fn build_args_rejects_empty_properties() {
        let opts = GetOptions::default();
        let err = opts.build_args().expect_err("empty properties must error");
        let ZfsError::Other { stderr, .. } = err else {
            panic!("expected Other");
        };
        assert!(stderr.contains("must not be empty"));
    }
}
