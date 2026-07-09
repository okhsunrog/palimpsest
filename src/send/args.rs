use thiserror::Error;

use crate::names::{BookmarkName, DatasetName, NameError, SnapshotName};

#[derive(Error, Debug)]
pub enum SendArgsError {
    #[error(transparent)]
    InvalidName(#[from] NameError),

    #[error(
        "dry-run size estimate is not applicable with a resume token; \
         use resume_token::decode() for the token decode + estimated-size operation"
    )]
    DryRunWithResumeToken,

    #[error("zfs send -t cannot be combined with {option}")]
    IncompatibleResumeOption { option: &'static str },

    #[error("{operation} requires a snapshot target")]
    SnapshotTargetRequired { operation: &'static str },

    #[error("{operation} cannot use a bookmark as its incremental source")]
    MultiSnapshotFromBookmark { operation: &'static str },

    #[error("resume token must not be empty")]
    EmptyResumeToken,
}

impl From<SendArgsError> for crate::error::ZfsError {
    fn from(error: SendArgsError) -> Self {
        match error {
            SendArgsError::InvalidName(error) => Self::InvalidName(error),
            error => Self::InvalidInput {
                message: error.to_string(),
            },
        }
    }
}

/// Source for an incremental or resume send.
#[derive(Debug, Clone)]
pub enum SendFrom {
    /// `-i <from_snap>` — include only the diff since `from_snap`.
    Incremental(String),
    /// `-I <from_snap>` — include all intermediate snapshots since `from_snap`.
    IncrementalAll(String),
    /// `-t <token>` — resume an interrupted send from a resume token.
    ResumeToken(String),
}

/// Arguments for a `zfs send` invocation. Covers both full and incremental
/// sends, resume tokens, and the replication flags zrepl uses.
#[derive(Debug, Clone)]
pub struct SendArgs {
    /// Dataset, volume, or snapshot to send. Empty for a resume-token send,
    /// where the token encodes the target.
    pub target: String,
    pub from: Option<SendFrom>,
    /// `-R` — replicate the entire dataset tree.
    pub replicate: bool,
    /// `-w` — send raw (encrypted) data without decrypting.
    pub raw: bool,
    /// `-p` — include dataset properties in the stream.
    pub properties: bool,
    /// `-c` — use compressed WRITE records where available.
    pub compressed: bool,
    /// `-L` — allow large (> 128 KiB) blocks in the stream.
    pub large_blocks: bool,
    /// `-e` — use embedded-data WRITE records to reduce stream size.
    pub embedded: bool,
}

impl SendArgs {
    pub fn new(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            from: None,
            replicate: false,
            raw: false,
            properties: false,
            compressed: false,
            large_blocks: false,
            embedded: false,
        }
    }

    /// Construct a resumed send. OpenZFS encodes the original stream shape in
    /// the token, so callers normally need not copy feature flags onto it.
    pub fn resume(token: impl Into<String>) -> Self {
        Self::new("").resume_token(token)
    }

    pub fn from(mut self, from: SendFrom) -> Self {
        self.from = Some(from);
        self
    }

    pub fn incremental(mut self, from_snap: impl Into<String>) -> Self {
        self.from = Some(SendFrom::Incremental(from_snap.into()));
        self
    }

    pub fn incremental_all(mut self, from_snap: impl Into<String>) -> Self {
        self.from = Some(SendFrom::IncrementalAll(from_snap.into()));
        self
    }

    pub fn resume_token(mut self, token: impl Into<String>) -> Self {
        self.target.clear();
        self.from = Some(SendFrom::ResumeToken(token.into()));
        self
    }

    pub fn replicate(mut self) -> Self {
        self.replicate = true;
        self
    }
    pub fn raw(mut self) -> Self {
        self.raw = true;
        self
    }
    pub fn properties(mut self) -> Self {
        self.properties = true;
        self
    }
    pub fn compressed(mut self) -> Self {
        self.compressed = true;
        self
    }
    pub fn large_blocks(mut self) -> Self {
        self.large_blocks = true;
        self
    }
    pub fn embedded(mut self) -> Self {
        self.embedded = true;
        self
    }

    /// Build the `zfs send` arg list for a real (streaming) send. When
    /// `dry_run` is true, prepends `-nvP` for a size-estimate dry run.
    /// Returns `DryRunWithResumeToken` if `dry_run` is true and the from
    /// source is a resume token (use `resume_token::decode()` instead).
    pub fn build_args(&self, dry_run: bool) -> Result<Vec<String>, SendArgsError> {
        let mut args = vec!["send".to_string()];

        if let Some(SendFrom::ResumeToken(token)) = &self.from {
            if token.is_empty() {
                return Err(SendArgsError::EmptyResumeToken);
            }
            for (enabled, option) in [
                (self.replicate, "-R/replicate"),
                (self.properties, "-p/properties"),
            ] {
                if enabled {
                    return Err(SendArgsError::IncompatibleResumeOption { option });
                }
            }
        } else {
            let target_is_snapshot = if self.target.contains('@') {
                SnapshotName::parse(&self.target)?;
                true
            } else {
                DatasetName::parse(&self.target)?;
                false
            };
            if self.replicate && !target_is_snapshot {
                return Err(SendArgsError::SnapshotTargetRequired {
                    operation: "replicated send (-R)",
                });
            }
            if matches!(self.from, Some(SendFrom::IncrementalAll(_))) && !target_is_snapshot {
                return Err(SendArgsError::SnapshotTargetRequired {
                    operation: "incremental-all send (-I)",
                });
            }
            if let Some(from) = &self.from {
                let source = match from {
                    SendFrom::Incremental(source) | SendFrom::IncrementalAll(source) => source,
                    SendFrom::ResumeToken(_) => unreachable!("handled above"),
                };
                validate_incremental_source(source)?;
                if (self.replicate || matches!(from, SendFrom::IncrementalAll(_)))
                    && (source.starts_with('#') || source.contains('#'))
                {
                    return Err(SendArgsError::MultiSnapshotFromBookmark {
                        operation: if self.replicate {
                            "zfs send -R"
                        } else {
                            "zfs send -I"
                        },
                    });
                }
            }
        }

        if dry_run {
            if matches!(self.from, Some(SendFrom::ResumeToken(_))) {
                return Err(SendArgsError::DryRunWithResumeToken);
            }
            args.extend(["-n", "-v", "-P"].map(str::to_string));
        }

        if self.replicate {
            args.push("-R".to_string());
        }
        if self.raw {
            args.push("-w".to_string());
        }
        if self.properties {
            args.push("-p".to_string());
        }
        if self.compressed {
            args.push("-c".to_string());
        }
        if self.large_blocks {
            args.push("-L".to_string());
        }
        if self.embedded {
            args.push("-e".to_string());
        }

        match &self.from {
            None => {
                args.push(self.target.clone());
            }
            Some(SendFrom::Incremental(from)) => {
                args.push("-i".to_string());
                args.push(from.clone());
                args.push(self.target.clone());
            }
            Some(SendFrom::IncrementalAll(from)) => {
                args.push("-I".to_string());
                args.push(from.clone());
                args.push(self.target.clone());
            }
            Some(SendFrom::ResumeToken(token)) => {
                args.push("-t".to_string());
                args.push(token.clone());
                // No snapshot arg for resume token send.
            }
        }

        Ok(args)
    }
}

fn validate_incremental_source(source: &str) -> Result<(), NameError> {
    if let Some(tag) = source.strip_prefix('@') {
        SnapshotName::parse(format!("pool@{tag}"))?;
    } else if let Some(mark) = source.strip_prefix('#') {
        BookmarkName::parse(format!("pool#{mark}"))?;
    } else if source.contains('@') {
        SnapshotName::parse(source)?;
    } else if source.contains('#') {
        BookmarkName::parse(source)?;
    } else {
        // OpenZFS accepts a bare short name and interprets it as a snapshot,
        // while warning that an explicit '@' would be less ambiguous.
        SnapshotName::parse(format!("pool@{source}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_send_args() {
        let args = SendArgs::new("tank/data@snap1").build_args(false).unwrap();
        assert_eq!(args, vec!["send", "tank/data@snap1"]);
    }

    #[test]
    fn dataset_head_send_is_supported() {
        assert_eq!(
            SendArgs::new("tank/data").build_args(false).unwrap(),
            vec!["send", "tank/data"]
        );
    }

    #[test]
    fn validates_send_target_and_incremental_sources() {
        assert!(matches!(
            SendArgs::new("tank//data@snap").build_args(false),
            Err(SendArgsError::InvalidName(_))
        ));
        for source in [
            "@old",
            "#cursor",
            "old",
            "tank/data@old",
            "tank/data#cursor",
        ] {
            SendArgs::new("tank/data@new")
                .incremental(source)
                .build_args(false)
                .unwrap();
        }
        assert!(matches!(
            SendArgs::new("tank/data@new")
                .incremental("bad/source")
                .build_args(false),
            Err(SendArgsError::InvalidName(_))
        ));
    }

    #[test]
    fn snapshot_only_modes_reject_dataset_heads_and_bookmarks() {
        assert!(matches!(
            SendArgs::new("tank/data").replicate().build_args(false),
            Err(SendArgsError::SnapshotTargetRequired { .. })
        ));
        assert!(matches!(
            SendArgs::new("tank/data@new")
                .incremental_all("tank/data#cursor")
                .build_args(false),
            Err(SendArgsError::MultiSnapshotFromBookmark { .. })
        ));
        assert!(matches!(
            SendArgs::new("tank/data@new")
                .incremental("tank/data#cursor")
                .replicate()
                .build_args(false),
            Err(SendArgsError::MultiSnapshotFromBookmark { .. })
        ));
    }

    #[test]
    fn full_dry_run_args() {
        let args = SendArgs::new("tank/data@snap1").build_args(true).unwrap();
        assert_eq!(args, vec!["send", "-n", "-v", "-P", "tank/data@snap1"]);
    }

    #[test]
    fn incremental_send_args() {
        let args = SendArgs::new("tank/data@snap2")
            .incremental("tank/data@snap1")
            .build_args(false)
            .unwrap();
        assert_eq!(
            args,
            vec!["send", "-i", "tank/data@snap1", "tank/data@snap2"]
        );
    }

    #[test]
    fn incremental_dry_run_args() {
        let args = SendArgs::new("tank/data/home@snap2")
            .incremental("tank/data/home@snap1")
            .build_args(true)
            .unwrap();
        assert_eq!(
            args,
            vec![
                "send",
                "-n",
                "-v",
                "-P",
                "-i",
                "tank/data/home@snap1",
                "tank/data/home@snap2",
            ]
        );
    }

    #[test]
    fn resume_token_send_args() {
        let token = "1-abc123";
        let args = SendArgs::resume(token).build_args(false).unwrap();
        assert_eq!(args, vec!["send", "-t", "1-abc123"]);
    }

    #[test]
    fn resume_token_must_not_be_empty() {
        assert!(matches!(
            SendArgs::resume("").build_args(false),
            Err(SendArgsError::EmptyResumeToken)
        ));
    }

    #[test]
    fn dry_run_with_resume_token_is_error() {
        let err = SendArgs::resume("1-abc").build_args(true).unwrap_err();
        assert!(matches!(err, SendArgsError::DryRunWithResumeToken));
    }

    #[test]
    fn resume_rejects_normal_send_flags() {
        for args in [
            SendArgs::resume("token").replicate(),
            SendArgs::resume("token").properties(),
        ] {
            assert!(matches!(
                args.build_args(false),
                Err(SendArgsError::IncompatibleResumeOption { .. })
            ));
        }
        assert_eq!(
            SendArgs::resume("token")
                .raw()
                .compressed()
                .large_blocks()
                .embedded()
                .build_args(false)
                .unwrap(),
            vec!["send", "-w", "-c", "-L", "-e", "-t", "token"]
        );
    }

    #[test]
    fn all_replication_flags() {
        let args = SendArgs::new("tank/data@snap1")
            .replicate()
            .raw()
            .properties()
            .compressed()
            .large_blocks()
            .embedded()
            .build_args(false)
            .unwrap();
        assert_eq!(
            args,
            vec![
                "send",
                "-R",
                "-w",
                "-p",
                "-c",
                "-L",
                "-e",
                "tank/data@snap1"
            ]
        );
    }
}
