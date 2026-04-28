//! High-level entity handles over `CommandRunner`.
//!
//! `Zfs` is the entry point. It hands out `Pool` and `Dataset` handles bound to
//! a specific name; methods on those handles delegate to the free-function
//! operations under `crate::dataset`, `crate::pool`, `crate::encryption`. The
//! handle layer exists for ergonomics — consumers carry a single `Zfs` value
//! instead of threading a `&dyn CommandRunner` through every call. Tests can
//! still target the free functions directly when parser/error-classification
//! detail matters.
use std::sync::Arc;

use crate::dataset::{GetOptions, ListOptions, ZfsGetEntry, ZfsListEntry};
use crate::error::ZfsError;
use crate::models::PropertyValue;
use crate::pool::{ExportOptions, ImportOptions};
use crate::runner::{CommandRunner, RealRunner};

/// Top-level handle. Construct once with [`Zfs::new`] (uses the system's
/// `zfs(8)`/`zpool(8)`) or [`Zfs::with_runner`] (custom runner — typically
/// `RecordingRunner` in tests).
pub struct Zfs {
    runner: Arc<dyn CommandRunner>,
}

impl Zfs {
    pub fn new() -> Self {
        Self {
            runner: Arc::new(RealRunner),
        }
    }

    pub fn with_runner<R: CommandRunner + 'static>(runner: R) -> Self {
        Self {
            runner: Arc::new(runner),
        }
    }

    /// Handle bound to a specific dataset (filesystem, volume, snapshot, or
    /// bookmark) by name. The name is not validated on construction; ZFS will
    /// surface naming errors when an operation is invoked.
    pub fn dataset(&self, name: impl Into<String>) -> Dataset {
        Dataset {
            runner: self.runner.clone(),
            name: name.into(),
        }
    }

    /// Handle bound to a specific zpool by name.
    pub fn pool(&self, name: impl Into<String>) -> Pool {
        Pool {
            runner: self.runner.clone(),
            name: name.into(),
        }
    }

    /// List datasets across the system. Use a `Dataset` handle for
    /// single-dataset queries.
    pub async fn list_datasets(&self, opts: &ListOptions) -> Result<Vec<ZfsListEntry>, ZfsError> {
        crate::dataset::list(&*self.runner, opts).await
    }
}

impl Default for Zfs {
    fn default() -> Self {
        Self::new()
    }
}

/// Handle bound to a specific zpool.
pub struct Pool {
    runner: Arc<dyn CommandRunner>,
    name: String,
}

impl Pool {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub async fn import(&self, opts: &ImportOptions) -> Result<(), ZfsError> {
        crate::pool::import(&*self.runner, &self.name, opts).await
    }

    pub async fn export(&self, opts: &ExportOptions) -> Result<(), ZfsError> {
        crate::pool::export(&*self.runner, &self.name, opts).await
    }
}

/// Handle bound to a specific dataset.
pub struct Dataset {
    runner: Arc<dyn CommandRunner>,
    name: String,
}

impl Dataset {
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Read multiple properties via `zfs get -j`. The `datasets` field of
    /// `opts` is overridden with this handle's name.
    pub async fn get(&self, opts: &GetOptions) -> Result<Vec<ZfsGetEntry>, ZfsError> {
        let mut opts = opts.clone();
        opts.datasets = vec![self.name.clone()];
        crate::dataset::get(&*self.runner, &opts).await
    }

    /// Read a single property.
    pub async fn get_property(&self, property: &str) -> Result<PropertyValue, ZfsError> {
        crate::dataset::get_property(&*self.runner, &self.name, property).await
    }

    /// `zfs load-key <dataset>`. Reads the key from the dataset's
    /// `keylocation` (file://, prompt, ...). Idempotent on already-loaded.
    pub async fn load_key(&self) -> Result<(), ZfsError> {
        crate::encryption::load_key(&*self.runner, &self.name).await
    }

    /// `zfs load-key <dataset>` with the passphrase delivered via stdin —
    /// no terminal prompt, no temp keyfile. Idempotent on already-loaded.
    pub async fn load_key_with_passphrase(&self, passphrase: &[u8]) -> Result<(), ZfsError> {
        crate::encryption::load_key_with_passphrase(&*self.runner, &self.name, passphrase).await
    }

    /// `zfs unload-key <dataset>`. Idempotent on already-unloaded.
    pub async fn unload_key(&self) -> Result<(), ZfsError> {
        crate::encryption::unload_key(&*self.runner, &self.name).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{Cmd, RecordingRunner};

    fn get_property_json(ds: &str, value: &str, source_kind: &str) -> Vec<u8> {
        format!(
            "{{\"output_version\":{{\"command\":\"zfs get\",\"vers_major\":0,\"vers_minor\":1}},\
             \"datasets\":{{\"{ds}\":{{\"name\":\"{ds}\",\"type\":\"FILESYSTEM\",\
             \"pool\":\"{ds}\",\"createtxg\":\"1\",\"properties\":{{\"encryption\":\
             {{\"value\":\"{value}\",\"source\":{{\"type\":\"{source_kind}\",\"data\":\"-\"}}}}}}}}}}}}"
        )
        .into_bytes()
    }

    #[tokio::test]
    async fn dataset_get_property_via_handle() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["get", "-j", "-p", "encryption", "tank"]),
            get_property_json("tank", "off", "DEFAULT"),
            vec![],
            0,
        );
        let zfs = Zfs::with_runner(runner);
        let prop = zfs
            .dataset("tank")
            .get_property("encryption")
            .await
            .unwrap();
        assert_eq!(prop.value, "off");
    }

    #[tokio::test]
    async fn pool_import_export_via_handle() {
        let runner = RecordingRunner::new()
            .record(
                Cmd::new("zpool").args(["import", "-f", "-N", "tank"]),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zpool").args(["export", "tank"]),
                vec![],
                vec![],
                0,
            );
        let zfs = Zfs::with_runner(runner);
        let pool = zfs.pool("tank");
        let opts = ImportOptions {
            force: true,
            no_mount: true,
            ..Default::default()
        };
        pool.import(&opts).await.expect("import succeeds");
        pool.export(&ExportOptions::default())
            .await
            .expect("export succeeds");
    }

    #[tokio::test]
    async fn dataset_load_key_with_passphrase_via_handle() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs")
                .args(["load-key", "tank/encrypted"])
                .stdin_secret(b"correct".to_vec()),
            vec![],
            vec![],
            0,
        );
        let zfs = Zfs::with_runner(runner);
        zfs.dataset("tank/encrypted")
            .load_key_with_passphrase(b"correct")
            .await
            .expect("load_key_with_passphrase succeeds");
    }
}
