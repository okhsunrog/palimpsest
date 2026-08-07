use crate::error::{ZfsError, classify_stderr};
use crate::runner::{Cmd, CommandRunner};

/// Plan for creating a dataset. Carries only properties; the dataset's name
/// is supplied separately because at the engine layer it's derived from the
/// parent handle (`pool.create_dataset(rel_name, opts)`).
#[derive(Default, Clone, Debug)]
pub struct CreateOptions {
    /// Properties to apply via `-o name=value` at creation time.
    pub properties: Vec<(String, String)>,
    /// `-p` — create any missing parent datasets, idempotent on existing
    /// parents. The leaf still errors if it already exists; combine with
    /// caller-side "already exists" tolerance for full idempotency.
    pub create_parents: bool,
    /// `-u` — do not mount the newly created dataset.
    ///
    /// Needed whenever a dataset is created with a `mountpoint` that is already
    /// occupied on the running system — for example when preparing a second
    /// boot environment on a pool the current OS is running from. Without it
    /// `zfs create -o mountpoint=/home` mounts immediately and shadows the live
    /// `/home`. An installer that imports the pool under an altroot does not
    /// need this.
    pub no_mount: bool,
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

    pub fn create_parents(mut self) -> Self {
        self.create_parents = true;
        self
    }

    pub fn no_mount(mut self) -> Self {
        self.no_mount = true;
        self
    }

    pub fn build_args(&self, name: &str) -> Vec<String> {
        let mut args: Vec<String> = vec!["create".into()];
        if self.create_parents {
            args.push("-p".into());
        }
        if self.no_mount {
            args.push("-u".into());
        }
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
    fn build_args_with_create_parents() {
        let opts = CreateOptions::new().create_parents();
        assert_eq!(
            opts.build_args("tank/a/b/c"),
            vec!["create", "-p", "tank/a/b/c"]
        );
    }

    #[test]
    fn build_args_with_no_mount() {
        let opts = CreateOptions::new().no_mount();
        assert_eq!(
            opts.build_args("tank/data"),
            vec!["create", "-u", "tank/data"]
        );
    }

    #[test]
    fn build_args_no_mount_with_mountpoint_property() {
        // The boot-environment case: the mountpoint is set at creation time but
        // the dataset must not be mounted over the running system's /home.
        let opts = CreateOptions::new()
            .create_parents()
            .no_mount()
            .property("mountpoint", "/home");
        assert_eq!(
            opts.build_args("tank/be0/data/home"),
            vec![
                "create",
                "-p",
                "-u",
                "-o",
                "mountpoint=/home",
                "tank/be0/data/home",
            ]
        );
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
