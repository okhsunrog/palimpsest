use std::collections::HashMap;

use serde::Deserialize;

use super::common::{OutputVersion, PropertyMap};

// `zpool list -j` and `zpool get -j` produce structurally identical entries.
// Confirmed against captured fixtures from OpenZFS 2.4.1.

#[derive(Debug, Clone, Deserialize)]
pub struct ZpoolListOutput {
    pub output_version: OutputVersion,
    pub pools: HashMap<String, ZpoolListEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ZpoolListEntry {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub state: String,
    pub pool_guid: String,
    pub txg: String,
    pub spa_version: String,
    pub zpl_version: String,
    #[serde(default)]
    pub properties: PropertyMap,
}

pub type ZpoolGetEntry = ZpoolListEntry;
pub type ZpoolGetOutput = ZpoolListOutput;

// `zpool status -j` is a different shape — vdev tree, error counts, scrub
// progress.

#[derive(Debug, Clone, Deserialize)]
pub struct ZpoolStatusOutput {
    pub output_version: OutputVersion,
    pub pools: HashMap<String, ZpoolStatusEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ZpoolStatusEntry {
    pub name: String,
    pub state: String,
    pub pool_guid: String,
    pub txg: String,
    pub spa_version: String,
    pub zpl_version: String,
    pub error_count: String,
    /// The pool's root vdev tree. Keyed by vdev name (which equals the pool
    /// name for the synthetic root vdev).
    pub vdevs: HashMap<String, VdevStatus>,
    #[serde(default)]
    pub scan: Option<ScanStatus>,
}

/// Recursive vdev tree node. Top-level (root) vdevs have `vdev_type = "root"`;
/// leaves are typically `disk` or `file`. `mirror` / `raidz` are interior
/// nodes whose `vdevs` map contains their member disks.
#[derive(Debug, Clone, Deserialize)]
pub struct VdevStatus {
    pub name: String,
    pub vdev_type: String,
    pub guid: String,
    #[serde(default)]
    pub path: Option<String>,
    pub class: String,
    pub state: String,
    pub alloc_space: String,
    pub total_space: String,
    pub def_space: String,
    pub read_errors: String,
    pub write_errors: String,
    pub checksum_errors: String,
    #[serde(default)]
    pub rep_dev_size: Option<String>,
    #[serde(default)]
    pub phys_space: Option<String>,
    #[serde(default)]
    pub slow_ios: Option<String>,
    #[serde(default)]
    pub vdevs: HashMap<String, VdevStatus>,
}

/// Status of an in-flight scrub / resilver, when one is active or recently
/// completed. OpenZFS sometimes emits this and sometimes omits it; match by
/// `function` (`scrub` / `resilver` / `none`).
#[derive(Debug, Clone, Deserialize)]
pub struct ScanStatus {
    pub function: String,
    pub state: String,
    #[serde(default)]
    pub start_time: Option<String>,
    #[serde(default)]
    pub end_time: Option<String>,
}
