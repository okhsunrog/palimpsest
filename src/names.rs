use std::fmt;
use std::str::FromStr;

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid {kind} name {value:?}: {reason}")]
pub struct NameError {
    kind: &'static str,
    value: String,
    reason: &'static str,
}

impl NameError {
    pub fn kind(&self) -> &'static str {
        self.kind
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn reason(&self) -> &'static str {
        self.reason
    }
}

fn invalid(kind: &'static str, value: &str, reason: &'static str) -> NameError {
    NameError {
        kind,
        value: value.to_owned(),
        reason,
    }
}

const MAX_ENTITY_LEN: usize = 255;
// OpenZFS reserves room for `/$ORIGIN@$ORIGIN` when validating pool names.
const MAX_POOL_LEN: usize = 239;

fn valid_component_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | ' ' | '%')
}

fn validate_component(kind: &'static str, full: &str, component: &str) -> Result<(), NameError> {
    if component.is_empty() {
        return Err(invalid(kind, full, "path components cannot be empty"));
    }
    if matches!(component, "." | "..") {
        return Err(invalid(
            kind,
            full,
            "'.' and '..' components are not allowed",
        ));
    }
    if !component.chars().all(valid_component_char) {
        return Err(invalid(
            kind,
            full,
            "contains a character OpenZFS does not allow",
        ));
    }
    Ok(())
}

fn validate_dataset(value: &str) -> Result<(), NameError> {
    if value.is_empty() {
        return Err(invalid("dataset", value, "name is empty"));
    }
    if value.len() > MAX_ENTITY_LEN {
        return Err(invalid("dataset", value, "name exceeds 255 bytes"));
    }
    if value.contains(['@', '#']) {
        return Err(invalid(
            "dataset",
            value,
            "dataset names cannot contain '@' or '#'",
        ));
    }
    for component in value.split('/') {
        validate_component("dataset", value, component)?;
    }
    Ok(())
}

macro_rules! simple_name {
    ($name:ident, $kind:literal, $validate:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, NameError> {
                let value = value.into();
                ($validate)(&value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = NameError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = NameError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = NameError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }
    };
}

fn validate_pool(value: &str) -> Result<(), NameError> {
    if value.is_empty() {
        return Err(invalid("pool", value, "name is empty"));
    }
    if value.len() > MAX_POOL_LEN {
        return Err(invalid("pool", value, "name exceeds 239 bytes"));
    }
    if value.contains(['/', '@', '#', '%']) {
        return Err(invalid(
            "pool",
            value,
            "pool names cannot contain '/', '@', '#', or '%'",
        ));
    }
    if !value.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return Err(invalid(
            "pool",
            value,
            "pool names must begin with a letter",
        ));
    }
    if !value.chars().all(valid_component_char) {
        return Err(invalid(
            "pool",
            value,
            "contains a character OpenZFS does not allow",
        ));
    }
    if matches!(value, "mirror" | "raidz" | "draid") {
        return Err(invalid("pool", value, "name is reserved by OpenZFS"));
    }
    Ok(())
}

simple_name!(PoolName, "pool", validate_pool);
simple_name!(DatasetName, "dataset", validate_dataset);

impl PoolName {
    /// Parse a name for `zpool create` or `zpool import`. OpenZFS accepts
    /// legacy reserved-prefix names when opening an existing pool, but rejects
    /// them when creating or importing one.
    pub fn parse_for_create(value: impl Into<String>) -> Result<Self, NameError> {
        let value = value.into();
        validate_pool(&value)?;
        if ["mirror", "raidz", "draid", "spare"]
            .iter()
            .any(|prefix| value.starts_with(prefix))
            || value == "log"
        {
            return Err(invalid("pool", &value, "name is reserved by OpenZFS"));
        }
        Ok(Self(value))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SnapshotName {
    full: String,
    dataset: DatasetName,
    tag_start: usize,
}

impl SnapshotName {
    pub fn parse(value: impl Into<String>) -> Result<Self, NameError> {
        let full = value.into();
        if full.len() > MAX_ENTITY_LEN {
            return Err(invalid("snapshot", &full, "name exceeds 255 bytes"));
        }
        let Some((dataset, tag)) = full.split_once('@') else {
            return Err(invalid("snapshot", &full, "expected '<dataset>@<tag>'"));
        };
        if tag.contains(['/', '@', '#']) {
            return Err(invalid(
                "snapshot",
                &full,
                "snapshot tag is empty or malformed",
            ));
        }
        validate_component("snapshot", &full, tag)?;
        let dataset = DatasetName::parse(dataset.to_owned())?;
        let tag_start = dataset.as_str().len() + 1;
        Ok(Self {
            full,
            dataset,
            tag_start,
        })
    }

    pub fn new(dataset: DatasetName, tag: &str) -> Result<Self, NameError> {
        Self::parse(format!("{dataset}@{tag}"))
    }

    pub fn as_str(&self) -> &str {
        &self.full
    }
    pub fn dataset(&self) -> &DatasetName {
        &self.dataset
    }
    pub fn tag(&self) -> &str {
        &self.full[self.tag_start..]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BookmarkName {
    full: String,
    dataset: DatasetName,
    mark_start: usize,
}

impl BookmarkName {
    pub fn parse(value: impl Into<String>) -> Result<Self, NameError> {
        let full = value.into();
        if full.len() > MAX_ENTITY_LEN {
            return Err(invalid("bookmark", &full, "name exceeds 255 bytes"));
        }
        let Some((dataset, mark)) = full.split_once('#') else {
            return Err(invalid("bookmark", &full, "expected '<dataset>#<mark>'"));
        };
        if mark.contains(['/', '@', '#']) {
            return Err(invalid(
                "bookmark",
                &full,
                "bookmark name is empty or malformed",
            ));
        }
        validate_component("bookmark", &full, mark)?;
        let dataset = DatasetName::parse(dataset.to_owned())?;
        let mark_start = dataset.as_str().len() + 1;
        Ok(Self {
            full,
            dataset,
            mark_start,
        })
    }

    pub fn new(dataset: DatasetName, mark: &str) -> Result<Self, NameError> {
        Self::parse(format!("{dataset}#{mark}"))
    }

    pub fn as_str(&self) -> &str {
        &self.full
    }
    pub fn dataset(&self) -> &DatasetName {
        &self.dataset
    }
    pub fn mark(&self) -> &str {
        &self.full[self.mark_start..]
    }
}

macro_rules! composite_impls {
    ($name:ident) => {
        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
        impl FromStr for $name {
            type Err = NameError;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }
        impl TryFrom<String> for $name {
            type Error = NameError;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }
        impl TryFrom<&str> for $name {
            type Error = NameError;
            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }
    };
}

composite_impls!(SnapshotName);
composite_impls!(BookmarkName);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_distinguish_entity_kinds() {
        assert!(PoolName::parse("tank").is_ok());
        assert!(PoolName::parse("tank/data").is_err());
        assert!(DatasetName::parse("tank/data").is_ok());
        assert!(DatasetName::parse("tank/data@s1").is_err());
        let snapshot = SnapshotName::parse("tank/data@s1").unwrap();
        assert_eq!(snapshot.dataset().as_str(), "tank/data");
        assert_eq!(snapshot.tag(), "s1");
        let bookmark = BookmarkName::parse("tank/data#cursor").unwrap();
        assert_eq!(bookmark.mark(), "cursor");
    }

    #[test]
    fn rejects_names_openzfs_rejects() {
        for name in [
            "tank//data",
            "tank/.",
            "tank/..",
            "tank/naïve",
            "tank/data?",
        ] {
            assert!(DatasetName::parse(name).is_err(), "accepted {name:?}");
        }
        for name in ["1tank", "mirror", "raidz", "draid", "tank%tmp"] {
            assert!(PoolName::parse(name).is_err(), "accepted {name:?}");
        }
        for name in ["mirror-old", "raidz2", "draid1", "spare0", "log"] {
            assert!(
                PoolName::parse(name).is_ok(),
                "legacy open rejected {name:?}"
            );
            assert!(
                PoolName::parse_for_create(name).is_err(),
                "create accepted {name:?}"
            );
        }
        assert!(SnapshotName::parse("tank/data@.").is_err());
        assert!(BookmarkName::parse("tank/data#..").is_err());
    }

    #[test]
    fn accepts_openzfs_component_characters() {
        assert!(DatasetName::parse("tank/data set:v1_tmp-old%recv").is_ok());
        assert!(SnapshotName::parse("tank/data@snap 1:old%recv").is_ok());
        assert!(PoolName::parse("Tank.pool-1:old").is_ok());
    }

    #[test]
    fn enforces_entity_length_in_bytes() {
        let valid = format!("t{}", "a".repeat(254));
        assert_eq!(valid.len(), 255);
        assert!(DatasetName::parse(valid).is_ok());
        let invalid = format!("t{}", "a".repeat(255));
        assert_eq!(invalid.len(), 256);
        assert!(DatasetName::parse(invalid).is_err());
        assert!(PoolName::parse(format!("t{}", "a".repeat(238))).is_ok());
        assert!(PoolName::parse(format!("t{}", "a".repeat(239))).is_err());
    }
}
