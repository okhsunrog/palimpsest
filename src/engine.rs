//! High-level entity handles over `CommandRunner`.
//!
//! `Zfs` is the entry point. It hands out `Pool` handles by name and an
//! absolute-name `Dataset` shortcut. Pool/Dataset handles in turn hand out
//! nested handles via relative names: `pool.dataset("data")` or
//! `pool.create_dataset("data", &opts)`. Methods on a handle delegate to the
//! free-function operations under `crate::dataset`, `crate::pool`, and
//! `crate::encryption`.
//!
//! The handle layer exists for ergonomics — consumers carry a single `Zfs`
//! value instead of threading a `&dyn CommandRunner` through every call,
//! and they navigate by relative name once a parent is in scope. Tests can
//! still target the free functions directly when parser/error-classification
//! detail matters.
use std::sync::Arc;

use crate::dataset::{
    CreateOptions, GetOptions, ListOptions, MountOptions, UnmountOptions, ZfsGetEntry, ZfsListEntry,
};
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

    /// Handle bound to a specific zpool by name.
    pub fn pool(&self, name: impl Into<String>) -> Pool {
        Pool {
            runner: self.runner.clone(),
            name: name.into(),
        }
    }

    /// Sugar: handle to any dataset by absolute name. Equivalent to
    /// `zfs.pool("a").dataset("b/c")` for `zfs.dataset("a/b/c")` — useful
    /// when you already have the full path and don't need a `Pool` handle.
    pub fn dataset(&self, abs_name: impl Into<String>) -> Dataset {
        Dataset {
            runner: self.runner.clone(),
            name: abs_name.into(),
        }
    }

    /// List datasets across the system.
    pub async fn list_datasets(&self, opts: &ListOptions) -> Result<Vec<ZfsListEntry>, ZfsError> {
        crate::dataset::list(&*self.runner, opts).await
    }

    /// `zfs mount -a` — mount all importable filesystems on the system.
    pub async fn mount_all(&self) -> Result<(), ZfsError> {
        crate::dataset::mount_all(&*self.runner).await
    }

    /// `zfs umount -a [-f]` — unmount all mounted filesystems on the system.
    pub async fn unmount_all(&self, force: bool) -> Result<(), ZfsError> {
        crate::dataset::unmount_all(&*self.runner, force).await
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

    /// Handle to the pool's root filesystem (the zfs dataset whose name
    /// matches the pool name). Encryption properties of a pool actually
    /// live on this dataset.
    pub fn root_dataset(&self) -> Dataset {
        Dataset {
            runner: self.runner.clone(),
            name: self.name.clone(),
        }
    }

    /// Lookup a nested dataset by relative name. The full ZFS name is
    /// `<pool>/<rel_name>`.
    pub fn dataset(&self, rel_name: &str) -> Dataset {
        Dataset {
            runner: self.runner.clone(),
            name: format!("{}/{rel_name}", self.name),
        }
    }

    /// Create a child dataset within this pool. The full ZFS name is
    /// `<pool>/<rel_name>`. Returns a `Dataset` handle to the just-created
    /// dataset on success.
    pub async fn create_dataset(
        &self,
        rel_name: &str,
        opts: &CreateOptions,
    ) -> Result<Dataset, ZfsError> {
        let full_name = format!("{}/{rel_name}", self.name);
        crate::dataset::create(&*self.runner, &full_name, opts).await?;
        Ok(Dataset {
            runner: self.runner.clone(),
            name: full_name,
        })
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

    /// `zfs set <property>=<value> <dataset>`.
    pub async fn set_property(&self, property: &str, value: &str) -> Result<(), ZfsError> {
        crate::dataset::set_property(&*self.runner, &self.name, property, value).await
    }

    /// `zfs mount [-R] <dataset>`. Idempotent on already-mounted.
    pub async fn mount(&self, opts: &MountOptions) -> Result<(), ZfsError> {
        crate::dataset::mount(&*self.runner, &self.name, opts).await
    }

    /// `zfs umount [-f] <dataset>`. Idempotent on not-currently-mounted.
    pub async fn unmount(&self, opts: &UnmountOptions) -> Result<(), ZfsError> {
        crate::dataset::unmount(&*self.runner, &self.name, opts).await
    }

    /// Returns true if a `zfs list` of this dataset succeeds with at least
    /// one entry. Errors (including DatasetNotFound) collapse to false,
    /// matching the common "best-effort existence check" use.
    pub async fn exists(&self) -> bool {
        let opts = ListOptions {
            roots: vec![self.name.clone()],
            ..Default::default()
        };
        crate::dataset::list(&*self.runner, &opts)
            .await
            .map(|entries| !entries.is_empty())
            .unwrap_or(false)
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

    /// Lookup a nested dataset by relative name. The full ZFS name is
    /// `<self>/<rel_name>`.
    pub fn dataset(&self, rel_name: &str) -> Dataset {
        Dataset {
            runner: self.runner.clone(),
            name: format!("{}/{rel_name}", self.name),
        }
    }

    /// Create a child dataset within this dataset. The full ZFS name is
    /// `<self>/<rel_name>`. Returns a `Dataset` handle to the just-created
    /// dataset on success.
    pub async fn create_dataset(
        &self,
        rel_name: &str,
        opts: &CreateOptions,
    ) -> Result<Dataset, ZfsError> {
        let full_name = format!("{}/{rel_name}", self.name);
        crate::dataset::create(&*self.runner, &full_name, opts).await?;
        Ok(Dataset {
            runner: self.runner.clone(),
            name: full_name,
        })
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
    async fn pool_create_dataset_returns_handle_with_full_name() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["create", "-o", "compression=lz4", "tank/data"]),
            vec![],
            vec![],
            0,
        );
        let zfs = Zfs::with_runner(runner);
        let pool = zfs.pool("tank");
        let ds = pool
            .create_dataset("data", &CreateOptions::new().property("compression", "lz4"))
            .await
            .expect("create_dataset succeeds");
        assert_eq!(ds.name(), "tank/data");
    }

    #[tokio::test]
    async fn nested_create_dataset_joins_path() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["create", "tank/data/home"]),
            vec![],
            vec![],
            0,
        );
        let zfs = Zfs::with_runner(runner);
        let parent = zfs.dataset("tank/data");
        let child = parent
            .create_dataset("home", &CreateOptions::new())
            .await
            .expect("nested create_dataset succeeds");
        assert_eq!(child.name(), "tank/data/home");
    }

    #[tokio::test]
    async fn pool_root_dataset_uses_pool_name() {
        let zfs = Zfs::with_runner(RecordingRunner::new());
        let root = zfs.pool("tank").root_dataset();
        assert_eq!(root.name(), "tank");
    }

    #[tokio::test]
    async fn pool_dataset_lookup_joins_path() {
        let zfs = Zfs::with_runner(RecordingRunner::new());
        let ds = zfs.pool("tank").dataset("data/home");
        assert_eq!(ds.name(), "tank/data/home");
    }

    #[tokio::test]
    async fn dataset_set_property_via_handle() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["set", "compression=zstd", "tank/data"]),
            vec![],
            vec![],
            0,
        );
        let zfs = Zfs::with_runner(runner);
        zfs.dataset("tank/data")
            .set_property("compression", "zstd")
            .await
            .expect("set_property succeeds");
    }

    #[tokio::test]
    async fn dataset_mount_unmount_via_handle() {
        let runner = RecordingRunner::new()
            .record(
                Cmd::new("zfs").args(["mount", "tank/data"]),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args(["umount", "-f", "tank/data"]),
                vec![],
                vec![],
                0,
            );
        let zfs = Zfs::with_runner(runner);
        let ds = zfs.dataset("tank/data");
        ds.mount(&MountOptions::default())
            .await
            .expect("mount succeeds");
        ds.unmount(&UnmountOptions { force: true })
            .await
            .expect("unmount succeeds");
    }

    #[tokio::test]
    async fn zfs_unmount_all_force() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["umount", "-a", "-f"]),
            vec![],
            vec![],
            0,
        );
        let zfs = Zfs::with_runner(runner);
        zfs.unmount_all(true)
            .await
            .expect("unmount_all force succeeds");
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
