use thiserror::Error;

#[derive(Error, Debug)]
pub enum ZfsError {
    #[error("dataset not found: {name}")]
    DatasetNotFound { name: String },

    #[error("permission denied")]
    PermissionDenied,

    #[error("dataset is busy: {name}")]
    Busy { name: String },

    #[error("out of space")]
    NoSpace,

    #[error("failed to spawn zfs: {0}")]
    Spawn(#[from] std::io::Error),

    #[error("zfs exited with {exit_code:?}: {stderr}")]
    Other {
        exit_code: Option<i32>,
        stderr: String,
    },
}

pub fn classify_stderr(stderr: &str, exit_code: Option<i32>) -> ZfsError {
    let _ = (stderr, exit_code);
    todo!("spec 001-foundation: implement regex classifier with at minimum DatasetNotFound, PermissionDenied, NoSpace patterns")
}
