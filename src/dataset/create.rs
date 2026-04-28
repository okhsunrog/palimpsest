use crate::error::{ZfsError, classify_stderr};
use crate::runner::{Cmd, CommandRunner};

/// Plan for creating a dataset. Carries only properties; the dataset's name
/// is supplied separately because at the engine layer it's derived from the
/// parent handle (`pool.create_dataset(rel_name, opts)`).
#[derive(Default, Clone, Debug)]
pub struct CreateOptions {
    /// Properties to apply via `-o name=value` at creation time.
    pub properties: Vec<(String, String)>,
}

impl CreateOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn property(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.properties.push((k.into(), v.into()));
        self
    }

    pub fn properties<I, K, V>(mut self, props: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        self.properties
            .extend(props.into_iter().map(|(k, v)| (k.into(), v.into())));
        self
    }

    pub fn build_args(&self, name: &str) -> Vec<String> {
        let mut args: Vec<String> = vec!["create".into()];
        for (k, v) in &self.properties {
            args.push("-o".into());
            args.push(format!("{k}={v}"));
        }
        args.push(name.into());
        args
    }
}

/// `zfs create <name>` with optional `-o key=value` properties.
/// Errors classified via `classify_stderr`.
pub async fn create(
    runner: &dyn CommandRunner,
    name: &str,
    opts: &CreateOptions,
) -> Result<(), ZfsError> {
    let output = runner
        .run(Cmd::new("zfs").args(opts.build_args(name)))
        .await?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(classify_stderr(&stderr, output.status.code()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_args_no_properties() {
        let opts = CreateOptions::new();
        assert_eq!(opts.build_args("tank/data"), vec!["create", "tank/data"]);
    }

    #[test]
    fn build_args_with_properties() {
        let opts = CreateOptions::new()
            .property("mountpoint", "/mnt/data")
            .property("compression", "lz4");
        assert_eq!(
            opts.build_args("tank/data"),
            vec![
                "create",
                "-o",
                "mountpoint=/mnt/data",
                "-o",
                "compression=lz4",
                "tank/data",
            ]
        );
    }
}
