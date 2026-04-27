use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct OutputVersion {
    pub command: String,
    pub vers_major: u32,
    pub vers_minor: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum DatasetType {
    Filesystem,
    Volume,
    Snapshot,
    Bookmark,
}

impl DatasetType {
    pub fn cli_name(self) -> &'static str {
        match self {
            Self::Filesystem => "filesystem",
            Self::Volume => "volume",
            Self::Snapshot => "snapshot",
            Self::Bookmark => "bookmark",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum PropertySourceKind {
    None,
    Local,
    Default,
    Inherited,
    Received,
    Temporary,
}

impl PropertySourceKind {
    pub fn cli_name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Local => "local",
            Self::Default => "default",
            Self::Inherited => "inherited",
            Self::Received => "received",
            Self::Temporary => "temporary",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PropertySource {
    #[serde(rename = "type")]
    pub kind: PropertySourceKind,
    // For INHERITED, this carries the dataset name from which the property was inherited.
    // For other kinds it is "-" or empty.
    pub data: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PropertyValue {
    pub value: String,
    pub source: PropertySource,
}

pub type PropertyMap = HashMap<String, PropertyValue>;
