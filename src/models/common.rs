use std::collections::HashMap;

use serde::{Deserialize, Deserializer};

use crate::error::ZfsError;

#[derive(Debug, Clone, Deserialize)]
pub struct OutputVersion {
    pub command: String,
    pub vers_major: u32,
    pub vers_minor: u32,
}

impl OutputVersion {
    pub fn validate(&self, command: &'static str) -> Result<(), ZfsError> {
        if self.vers_major == 0 {
            Ok(())
        } else {
            Err(ZfsError::IncompatibleOutput {
                command,
                major: self.vers_major,
                minor: self.vers_minor,
            })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DatasetType {
    Filesystem,
    Volume,
    Snapshot,
    Bookmark,
    Unknown(String),
}

impl DatasetType {
    pub fn cli_name(&self) -> &str {
        match self {
            Self::Filesystem => "filesystem",
            Self::Volume => "volume",
            Self::Snapshot => "snapshot",
            Self::Bookmark => "bookmark",
            Self::Unknown(value) => value,
        }
    }
}

impl<'de> Deserialize<'de> for DatasetType {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "FILESYSTEM" => Self::Filesystem,
            "VOLUME" => Self::Volume,
            "SNAPSHOT" => Self::Snapshot,
            "BOOKMARK" => Self::Bookmark,
            _ => Self::Unknown(value),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PropertySourceKind {
    None,
    Local,
    Default,
    Inherited,
    Received,
    Temporary,
    Unknown(String),
}

impl PropertySourceKind {
    pub fn cli_name(&self) -> &str {
        match self {
            Self::None => "none",
            Self::Local => "local",
            Self::Default => "default",
            Self::Inherited => "inherited",
            Self::Received => "received",
            Self::Temporary => "temporary",
            Self::Unknown(value) => value,
        }
    }
}

impl<'de> Deserialize<'de> for PropertySourceKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "NONE" => Self::None,
            "LOCAL" => Self::Local,
            "DEFAULT" => Self::Default,
            "INHERITED" => Self::Inherited,
            "RECEIVED" => Self::Received,
            "TEMPORARY" => Self::Temporary,
            _ => Self::Unknown(value),
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_wire_enums_are_preserved() {
        let dataset: DatasetType = serde_json::from_str("\"FUTURE_KIND\"").unwrap();
        assert_eq!(dataset, DatasetType::Unknown("FUTURE_KIND".to_string()));
        let source: PropertySourceKind = serde_json::from_str("\"FUTURE_SOURCE\"").unwrap();
        assert_eq!(
            source,
            PropertySourceKind::Unknown("FUTURE_SOURCE".to_string())
        );
    }

    #[test]
    fn unsupported_output_major_is_explicit() {
        let error = OutputVersion {
            command: "zfs list".to_string(),
            vers_major: 1,
            vers_minor: 0,
        }
        .validate("zfs list")
        .unwrap_err();
        assert!(matches!(
            error,
            ZfsError::IncompatibleOutput { major: 1, .. }
        ));
    }
}
