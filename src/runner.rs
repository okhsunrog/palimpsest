use std::process::Output;

#[async_trait::async_trait]
pub trait CommandRunner: Send + Sync {
    async fn run(&self, program: &str, args: &[&str]) -> Result<Output, std::io::Error>;
}

pub struct RealRunner;

#[async_trait::async_trait]
impl CommandRunner for RealRunner {
    async fn run(&self, program: &str, args: &[&str]) -> Result<Output, std::io::Error> {
        tokio::process::Command::new(program).args(args).output().await
    }
}

pub struct RecordingRunner {
    // Tracks fixture lookup. Final shape (keyed-by-args vs sequenced) is open;
    // see docs/specs/001-foundation.md "Open questions".
}

#[async_trait::async_trait]
impl CommandRunner for RecordingRunner {
    async fn run(&self, program: &str, args: &[&str]) -> Result<Output, std::io::Error> {
        let _ = (program, args);
        todo!("spec 001-foundation: load fixture from tests/fixtures/ and return its Output")
    }
}
