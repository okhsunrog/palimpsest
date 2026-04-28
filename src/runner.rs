use std::collections::HashMap;
use std::process::Output;

#[async_trait::async_trait]
pub trait CommandRunner: Send + Sync {
    async fn run(&self, program: &str, args: &[&str]) -> Result<Output, std::io::Error>;
}

pub struct RealRunner;

#[async_trait::async_trait]
impl CommandRunner for RealRunner {
    async fn run(&self, program: &str, args: &[&str]) -> Result<Output, std::io::Error> {
        tokio::process::Command::new(program)
            .args(args)
            .output()
            .await
    }
}

// RecordingRunner is keyed on (program, args). Tests call `record(...)` to add fixtures
// and the runner returns them on matching calls. Calls without a matching fixture
// return an io::Error with a precise NotFound message naming the unmatched call.
//
// The keyed (vs sequenced) shape was chosen because tests in this crate generally
// don't have deterministic call order (e.g. many helpers parallelize independent
// queries) and keying makes refactors that change call order non-breaking.
pub struct RecordingRunner {
    responses: HashMap<RunnerKey, Output>,
}

#[derive(PartialEq, Eq, Hash)]
struct RunnerKey {
    program: String,
    args: Vec<String>,
}

impl RecordingRunner {
    pub fn new() -> Self {
        Self {
            responses: HashMap::new(),
        }
    }

    pub fn record(
        mut self,
        program: &str,
        args: &[&str],
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        exit_code: i32,
    ) -> Self {
        let key = RunnerKey {
            program: program.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
        };
        self.responses
            .insert(key, make_output(stdout, stderr, exit_code));
        self
    }
}

impl Default for RecordingRunner {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl CommandRunner for RecordingRunner {
    async fn run(&self, program: &str, args: &[&str]) -> Result<Output, std::io::Error> {
        let key = RunnerKey {
            program: program.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
        };
        match self.responses.get(&key) {
            Some(out) => Ok(clone_output(out)),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "RecordingRunner: no fixture for `{program} {}`",
                    args.join(" ")
                ),
            )),
        }
    }
}

#[cfg(unix)]
fn make_output(stdout: Vec<u8>, stderr: Vec<u8>, code: i32) -> Output {
    use std::os::unix::process::ExitStatusExt;
    Output {
        status: std::process::ExitStatus::from_raw((code & 0xff) << 8),
        stdout,
        stderr,
    }
}

fn clone_output(out: &Output) -> Output {
    Output {
        status: out.status,
        stdout: out.stdout.clone(),
        stderr: out.stderr.clone(),
    }
}
