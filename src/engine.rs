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
use crate::models::{PropertyValue, ZpoolGetEntry, ZpoolListEntry, ZpoolStatusEntry};
use crate::pool::{
    DestroyOptions, DiscoveredPool, ExportOptions, GetOptions as PoolGetOptions, ImportOptions,
    ListOptions as PoolListOptions, PoolCreateOptions,
};
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

    /// List pools imported on the system.
    pub async fn list_pools(
        &self,
        opts: &PoolListOptions,
    ) -> Result<Vec<ZpoolListEntry>, ZfsError> {
        crate::pool::list(&*self.runner, opts).await
    }

    /// `zpool status -j` for all pools.
    pub async fn pool_status_all(&self) -> Result<Vec<ZpoolStatusEntry>, ZfsError> {
        crate::pool::status_all(&*self.runner).await
    }

    /// `zpool import` (no args) — discover pools available for import.
    /// Returns an empty Vec when no pools are visible.
    pub async fn discover_importable_pools(&self) -> Result<Vec<DiscoveredPool>, ZfsError> {
        crate::pool::discover(&*self.runner).await
    }

    /// Create a new pool. Returns a [`Pool`] handle to the just-created pool.
    pub async fn create_pool(&self, opts: &PoolCreateOptions) -> Result<Pool, ZfsError> {
        crate::pool::create(&*self.runner, opts).await?;
        Ok(Pool {
            runner: self.runner.clone(),
            name: opts.name.clone(),
        })
    }

    /// Create a dataset by absolute name (e.g., `"tank/data/home"`). Convenience
    /// for callers that have full paths and don't want to navigate via a
    /// `Pool` or `Dataset` handle. Equivalent to looking up the parent and
    /// calling `pool.create_dataset(rel)` / `dataset.create_dataset(rel)`,
    /// but lets the caller skip the split.
    pub async fn create_dataset(
        &self,
        abs_name: impl Into<String>,
        opts: &CreateOptions,
    ) -> Result<Dataset, ZfsError> {
        let name = abs_name.into();
        crate::dataset::create(&*self.runner, &name, opts).await?;
        Ok(Dataset {
            runner: self.runner.clone(),
            name,
        })
    }

    /// `zfs mount -a` — mount all importable filesystems on the system.
    pub async fn mount_all(&self) -> Result<(), ZfsError> {
        crate::dataset::mount_all(&*self.runner).await
    }

    /// `zfs umount -a [-f]` — unmount all mounted filesystems on the system.
    pub async fn unmount_all(&self, force: bool) -> Result<(), ZfsError> {
        crate::dataset::unmount_all(&*self.runner, force).await
    }

    /// `zfs destroy <bookmark>` — destroy a bookmark by its full name
    /// (e.g., `"tank/data#bm1"`).
    pub async fn destroy_bookmark(&self, bookmark_name: &str) -> Result<(), ZfsError> {
        crate::bookmark::destroy(&*self.runner, bookmark_name).await
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

    /// `zpool destroy [-f] <pool>`. Permanent.
    pub async fn destroy(&self, opts: &DestroyOptions) -> Result<(), ZfsError> {
        crate::pool::destroy(&*self.runner, &self.name, opts).await
    }

    /// Read multiple properties via `zpool get -j`. The `pools` field of
    /// `opts` is overridden with this handle's name.
    pub async fn get(&self, opts: &PoolGetOptions) -> Result<Vec<ZpoolGetEntry>, ZfsError> {
        let mut opts = opts.clone();
        opts.pools = vec![self.name.clone()];
        crate::pool::get(&*self.runner, &opts).await
    }

    /// Read a single pool property.
    pub async fn get_property(&self, property: &str) -> Result<PropertyValue, ZfsError> {
        crate::pool::get_property(&*self.runner, &self.name, property).await
    }

    /// `zpool set <name>=<value> <pool>`.
    pub async fn set_property(&self, property: &str, value: &str) -> Result<(), ZfsError> {
        crate::pool::set_property(&*self.runner, &self.name, property, value).await
    }

    /// `zpool status -j <pool>`.
    pub async fn status(&self) -> Result<ZpoolStatusEntry, ZfsError> {
        crate::pool::status(&*self.runner, &self.name).await
    }

    /// Returns true if `zpool list` of this pool succeeds with at least
    /// one entry. Errors collapse to false, matching the common
    /// "is this pool imported and visible?" use.
    pub async fn exists(&self) -> bool {
        let opts = PoolListOptions {
            pools: vec![self.name.clone()],
            ..Default::default()
        };
        crate::pool::list(&*self.runner, &opts)
            .await
            .map(|entries| !entries.is_empty())
            .unwrap_or(false)
    }

    /// Best-effort encryption check via an ephemeral `import -fN` on the
    /// pool's root dataset. See [`crate::encryption::is_pool_encrypted`].
    pub async fn is_encrypted(&self) -> Result<bool, ZfsError> {
        crate::encryption::is_pool_encrypted(&*self.runner, &self.name).await
    }

    /// Best-effort passphrase verification via an ephemeral import +
    /// `load-key` with the passphrase piped on stdin. See
    /// [`crate::encryption::verify_pool_passphrase`].
    pub async fn verify_passphrase(&self, passphrase: &[u8]) -> Result<bool, ZfsError> {
        crate::encryption::verify_pool_passphrase(&*self.runner, &self.name, passphrase).await
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

    /// `zfs load-key -L <keylocation> <dataset>` — load with an explicit
    /// keylocation override (e.g., `file:///path` when the key file lives at
    /// a known location but the dataset's `keylocation` property is `prompt`).
    /// Idempotent on already-loaded.
    pub async fn load_key_with_keylocation(&self, keylocation: &str) -> Result<(), ZfsError> {
        crate::encryption::load_key_with_keylocation(&*self.runner, &self.name, keylocation).await
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

    /// `zfs hold <tag> <snapshot>`. Idempotent on already-held.
    pub async fn hold(&self, tag: &str) -> Result<(), ZfsError> {
        crate::hold::hold(&*self.runner, &self.name, tag).await
    }

    /// `zfs release <tag> <snapshot>`. Not idempotent.
    pub async fn release(&self, tag: &str) -> Result<(), ZfsError> {
        crate::hold::release(&*self.runner, &self.name, tag).await
    }

    /// `zfs holds -H <snapshot>` — list user holds on this snapshot.
    pub async fn list_holds(&self) -> Result<Vec<crate::hold::Hold>, ZfsError> {
        crate::hold::list_holds(&*self.runner, &self.name).await
    }

    /// `zfs bookmark <self> <bookmark_name>` — create a bookmark from this
    /// snapshot. `bookmark_name` is the full ZFS bookmark name
    /// (e.g., `"tank/data#bm1"`). Idempotent on already-existing bookmark.
    pub async fn bookmark(&self, bookmark_name: &str) -> Result<(), ZfsError> {
        crate::bookmark::create(&*self.runner, &self.name, bookmark_name).await
    }

    /// `zfs snapshot [-r] <self>@<tag>`. Returns a `Dataset` handle bound to
    /// the new snapshot (`<self>@<tag>`), which can then be held, bookmarked,
    /// or rolled back to via the same handle API.
    pub async fn snapshot(
        &self,
        tag: &str,
        opts: &crate::dataset::SnapshotOptions,
    ) -> Result<Dataset, ZfsError> {
        let full = format!("{}@{tag}", self.name);
        crate::dataset::snapshot(&*self.runner, &full, opts).await?;
        Ok(Dataset {
            runner: self.runner.clone(),
            name: full,
        })
    }

    /// Construct a `Dataset` handle for an existing snapshot of this dataset
    /// without taking the snapshot. Useful when handling a snapshot that
    /// already exists (e.g., received from replication).
    pub fn snapshot_handle(&self, tag: &str) -> Dataset {
        Dataset {
            runner: self.runner.clone(),
            name: format!("{}@{tag}", self.name),
        }
    }

    /// `zfs rollback [flags] <self>@<tag>`.
    pub async fn rollback(
        &self,
        tag: &str,
        opts: &crate::dataset::RollbackOptions,
    ) -> Result<(), ZfsError> {
        let full = format!("{}@{tag}", self.name);
        crate::dataset::rollback(&*self.runner, &full, opts).await
    }

    /// `zfs destroy [flags] <self>` — destroy this dataset. For destroying a
    /// snapshot or bookmark of this dataset, use [`Self::destroy_snapshot`]
    /// or [`Self::destroy_bookmark`].
    pub async fn destroy(&self, opts: &crate::dataset::DestroyOptions) -> Result<(), ZfsError> {
        crate::dataset::destroy(&*self.runner, &self.name, opts).await
    }

    /// `zfs destroy [flags] <self>@<tag>`.
    pub async fn destroy_snapshot(
        &self,
        tag: &str,
        opts: &crate::dataset::DestroyOptions,
    ) -> Result<(), ZfsError> {
        let full = format!("{}@{tag}", self.name);
        crate::dataset::destroy(&*self.runner, &full, opts).await
    }

    /// `zfs destroy [flags] <self>#<mark>`.
    pub async fn destroy_bookmark(
        &self,
        mark: &str,
        opts: &crate::dataset::DestroyOptions,
    ) -> Result<(), ZfsError> {
        let full = format!("{}#{mark}", self.name);
        crate::dataset::destroy(&*self.runner, &full, opts).await
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
    async fn zfs_create_dataset_takes_absolute_name() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zfs").args(["create", "-o", "compression=lz4", "tank/data/home"]),
            vec![],
            vec![],
            0,
        );
        let zfs = Zfs::with_runner(runner);
        let ds = zfs
            .create_dataset(
                "tank/data/home",
                &CreateOptions::new().property("compression", "lz4"),
            )
            .await
            .expect("create_dataset by abs name succeeds");
        assert_eq!(ds.name(), "tank/data/home");
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
    async fn dataset_snapshot_rollback_destroy_via_handle() {
        let runner = RecordingRunner::new()
            .record(
                Cmd::new("zfs").args(["snapshot", "tank/data@snap1"]),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args(["rollback", "-r", "tank/data@snap1"]),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args(["destroy", "tank/data@snap1"]),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args(["destroy", "-r", "-f", "tank/data"]),
                vec![],
                vec![],
                0,
            );
        let zfs = Zfs::with_runner(runner);
        let ds = zfs.dataset("tank/data");
        let snap = ds
            .snapshot("snap1", &crate::dataset::SnapshotOptions::new())
            .await
            .expect("snapshot succeeds");
        assert_eq!(snap.name(), "tank/data@snap1");
        ds.rollback(
            "snap1",
            &crate::dataset::RollbackOptions::new().destroy_newer(),
        )
        .await
        .expect("rollback succeeds");
        ds.destroy_snapshot("snap1", &crate::dataset::DestroyOptions::new())
            .await
            .expect("destroy_snapshot succeeds");
        ds.destroy(
            &crate::dataset::DestroyOptions::new()
                .recursive()
                .force_unmount(),
        )
        .await
        .expect("destroy succeeds");
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
    async fn pool_is_encrypted_true() {
        let runner = RecordingRunner::new()
            .record(
                Cmd::new("zpool").args(["import", "-f", "-N", "tank"]),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args(["get", "-j", "-p", "encryption", "tank"]),
                get_property_json("tank", "aes-256-gcm", "LOCAL"),
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args(["unload-key", "tank"]),
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
        assert!(zfs.pool("tank").is_encrypted().await.unwrap());
    }

    #[tokio::test]
    async fn pool_is_encrypted_false() {
        let runner = RecordingRunner::new()
            .record(
                Cmd::new("zpool").args(["import", "-f", "-N", "tank"]),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args(["get", "-j", "-p", "encryption", "tank"]),
                get_property_json("tank", "off", "DEFAULT"),
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args(["unload-key", "tank"]),
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
        assert!(!zfs.pool("tank").is_encrypted().await.unwrap());
    }

    #[tokio::test]
    async fn pool_is_encrypted_import_error_propagates() {
        let runner = RecordingRunner::new().record(
            Cmd::new("zpool").args(["import", "-f", "-N", "missing"]),
            vec![],
            b"cannot import 'missing': no such pool available\n".to_vec(),
            1,
        );
        let zfs = Zfs::with_runner(runner);
        let err = zfs.pool("missing").is_encrypted().await.unwrap_err();
        assert!(matches!(err, ZfsError::PoolNotFound { .. }));
    }

    #[tokio::test]
    async fn pool_verify_passphrase_correct() {
        let runner = RecordingRunner::new()
            .record(
                Cmd::new("zpool").args(["import", "-f", "-N", "tank"]),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args(["unload-key", "tank"]),
                vec![],
                b"Key unload error: Key already unloaded for 'tank'.\n".to_vec(),
                255,
            )
            .record(
                Cmd::new("zfs")
                    .args(["load-key", "tank"])
                    .stdin_secret(b"correct".to_vec()),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args(["unload-key", "tank"]),
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
        assert!(
            zfs.pool("tank")
                .verify_passphrase(b"correct")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn pool_verify_passphrase_wrong() {
        let runner = RecordingRunner::new()
            .record(
                Cmd::new("zpool").args(["import", "-f", "-N", "tank"]),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args(["unload-key", "tank"]),
                vec![],
                b"Key unload error: Key already unloaded for 'tank'.\n".to_vec(),
                255,
            )
            .record(
                Cmd::new("zfs")
                    .args(["load-key", "tank"])
                    .stdin_secret(b"wrong".to_vec()),
                vec![],
                b"Key load error: Incorrect key provided for 'tank'.\n".to_vec(),
                1,
            )
            .record(
                Cmd::new("zfs").args(["unload-key", "tank"]),
                vec![],
                b"Key unload error: Key already unloaded for 'tank'.\n".to_vec(),
                255,
            )
            .record(
                Cmd::new("zpool").args(["export", "tank"]),
                vec![],
                vec![],
                0,
            );
        let zfs = Zfs::with_runner(runner);
        assert!(!zfs.pool("tank").verify_passphrase(b"wrong").await.unwrap());
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
