use crate::error::{ZfsError, classify_stderr};
use crate::runner::{Cmd, CommandRunner};

/// Modifiers for `zfs set`.
#[derive(Default, Clone, Debug)]
pub struct SetOptions {
    /// `-u` — "update mountpoint, sharenfs, sharesmb property but do not mount
    /// or share the dataset"; every other property is unaffected by the flag.
    ///
    /// Matters most for `mountpoint`: a plain `zfs set mountpoint=…` remounts
    /// the dataset immediately when the previous value was `none` or `legacy`.
    /// Freshly created datasets under a container with `mountpoint=none`
    /// inherit `none`, so that condition holds throughout boot-environment
    /// setup — and the new mountpoint is typically a path the running system
    /// already occupies, such as `/home`.
    pub no_mount: bool,
}

impl SetOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn no_mount(mut self) -> Self {
        self.no_mount = true;
        self
    }

    pub fn build_args(&self, properties: &[(&str, &str)], dataset: &str) -> Vec<String> {
        let mut args: Vec<String> = vec!["set".into()];
        if self.no_mount {
            args.push("-u".into());
        }
        for (k, v) in properties {
            args.push(format!("{k}={v}"));
        }
        args.push(dataset.into());
        args
    }
}

/// `zfs set <name>=<value> <dataset>`.
pub async fn set_property(
    runner: &dyn CommandRunner,
    dataset: &str,
    property: &str,
    value: &str,
) -> Result<(), ZfsError> {
    set_properties(
        runner,
        dataset,
        &[(property, value)],
        &SetOptions::default(),
    )
    .await
}

/// `zfs set [-u] <name>=<value>… <dataset>` — sets any number of properties in
/// a single invocation.
///
/// An empty `properties` slice is a no-op: `zfs set` with no pairs is a usage
/// error, and there is nothing to apply.
pub async fn set_properties(
    runner: &dyn CommandRunner,
    dataset: &str,
    properties: &[(&str, &str)],
    opts: &SetOptions,
) -> Result<(), ZfsError> {
    if properties.is_empty() {
        return Ok(());
    }
    let output = runner
        .run(Cmd::new("zfs").args(opts.build_args(properties, dataset)))
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
    fn build_args_single_property() {
        let opts = SetOptions::default();
        assert_eq!(
            opts.build_args(&[("compression", "zstd")], "tank/data"),
            vec!["set", "compression=zstd", "tank/data"]
        );
    }

    #[test]
    fn build_args_no_mount() {
        let opts = SetOptions::new().no_mount();
        assert_eq!(
            opts.build_args(&[("mountpoint", "/home")], "tank/be0/data/home"),
            vec!["set", "-u", "mountpoint=/home", "tank/be0/data/home"]
        );
    }

    #[test]
    fn build_args_multiple_properties() {
        let opts = SetOptions::default();
        assert_eq!(
            opts.build_args(
                &[("mountpoint", "/home"), ("canmount", "on")],
                "tank/be0/data/home"
            ),
            vec![
                "set",
                "mountpoint=/home",
                "canmount=on",
                "tank/be0/data/home",
            ]
        );
    }
}
