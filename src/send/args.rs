use thiserror::Error;

#[derive(Error, Debug)]
pub enum SendArgsError {
    #[error(
        "dry-run size estimate is not applicable with a resume token; \
         use resume_token::decode() for the token decode + estimated-size operation"
    )]
    DryRunWithResumeToken,
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
    /// Snapshot to send. Ignored when `from` is `ResumeToken` (the token
    /// encodes the destination).
    pub snapshot: String,
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
    pub fn new(snapshot: impl Into<String>) -> Self {
        Self {
            snapshot: snapshot.into(),
            from: None,
            replicate: false,
            raw: false,
            properties: false,
            compressed: false,
            large_blocks: false,
            embedded: false,
        }
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
                args.push(self.snapshot.clone());
            }
            Some(SendFrom::Incremental(from)) => {
                args.push("-i".to_string());
                args.push(from.clone());
                args.push(self.snapshot.clone());
            }
            Some(SendFrom::IncrementalAll(from)) => {
                args.push("-I".to_string());
                args.push(from.clone());
                args.push(self.snapshot.clone());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_send_args() {
        let args = SendArgs::new("tank/data@snap1")
            .build_args(false)
            .unwrap();
        assert_eq!(args, vec!["send", "tank/data@snap1"]);
    }

    #[test]
    fn full_dry_run_args() {
        let args = SendArgs::new("tank/data@snap1")
            .build_args(true)
            .unwrap();
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
        let args = SendArgs::new("ignored")
            .resume_token(token)
            .build_args(false)
            .unwrap();
        assert_eq!(args, vec!["send", "-t", "1-abc123"]);
    }

    #[test]
    fn dry_run_with_resume_token_is_error() {
        let err = SendArgs::new("tank/data@snap1")
            .resume_token("1-abc")
            .build_args(true)
            .unwrap_err();
        assert!(matches!(err, SendArgsError::DryRunWithResumeToken));
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
            vec!["send", "-R", "-w", "-p", "-c", "-L", "-e", "tank/data@snap1"]
        );
    }
}
