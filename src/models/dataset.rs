use std::collections::HashMap;

use serde::Deserialize;

use super::common::{DatasetType, OutputVersion, PropertyMap};

#[derive(Debug, Clone, Deserialize)]
pub struct ZfsListOutput {
    pub output_version: OutputVersion,
    pub datasets: HashMap<String, ZfsListEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ZfsListEntry {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: DatasetType,
    pub pool: String,
    pub createtxg: String,

    // Present only on SNAPSHOT entries: the parent filesystem and the part after `@`.
    #[serde(default)]
    pub dataset: Option<String>,
    #[serde(default)]
    pub snapshot_name: Option<String>,

    #[serde(default)]
    pub properties: PropertyMap,
}

// `zfs get -j` produces structurally identical entries to `zfs list -j`.
// Confirmed against captured fixtures from OpenZFS 2.4.1.
pub type ZfsGetEntry = ZfsListEntry;
pub type ZfsGetOutput = ZfsListOutput;
