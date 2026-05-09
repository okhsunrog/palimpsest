use std::collections::HashMap;
use std::fmt;
use std::io;
use std::process::{ExitStatus, Output, Stdio};

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

/// A planned command invocation. Owned, builder-style.
///
/// This is the value type the `CommandRunner` trait operates on. `RealRunner`
/// converts it to a `tokio::process::Command` at call time; `RecordingRunner`
/// uses it as a `HashMap` key for fixture lookup. Including stdin in the value
/// is deliberate: tests need to distinguish responses for the same `(program,
/// args)` invoked with different stdin (e.g., correct vs wrong passphrase).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Cmd {
    program: String,
    args: Vec<String>,
    stdin: Option<Vec<u8>>,
    secret_stdin: bool,
}

impl Cmd {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            ..Default::default()
        }
    }

    pub fn arg(mut self, a: impl Into<String>) -> Self {
        self.args.push(a.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Plain stdin payload. Visible in `Display`/`Debug`.
    pub fn stdin(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(bytes.into());
        self.secret_stdin = false;
        self
    }

    /// Stdin payload redacted from `Display` (length still shown). Use for
    /// passphrases and other secrets that must not leak into logs.
    pub fn stdin_secret(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(bytes.into());
        self.secret_stdin = true;
        self
    }

    pub fn program(&self) -> &str {
        &self.program
    }
    pub fn args_list(&self) -> &[String] {
        &self.args
    }
    pub fn stdin_bytes(&self) -> Option<&[u8]> {
        self.stdin.as_deref()
    }
}

impl fmt::Display for Cmd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.program)?;
        for a in &self.args {
            write!(f, " {a}")?;
        }
        match (&self.stdin, self.secret_stdin) {
            (Some(b), true) => write!(f, " <secret stdin: {} bytes>", b.len())?,
            (Some(b), false) => write!(f, " <stdin: {} bytes>", b.len())?,
            (None, _) => {}
        }
        Ok(())
    }
}

impl fmt::Debug for ChildHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChildHandle")
            .field("stdin", &self.stdin.is_some())
            .field("stdout", &self.stdout.is_some())
            .field("stderr", &self.stderr.is_some())
            .finish()
    }
}

/// An owned handle to a running child process. Returned by
/// [`CommandRunner::spawn`].
///
/// The stdio streams are boxed trait objects so that both `RealRunner` (real
/// `tokio::process::Child` handles) and `RecordingRunner` (in-memory cursors)
/// can return the same type.
///
/// **Callers must consume `stdout`/`stderr` before (or concurrently with)
/// calling `wait()`** to avoid deadlocking on the OS pipe buffer.
pub struct ChildHandle {
    /// Writable stdin. `None` if stdin was not piped (or for mock handles
    /// where writes are silently discarded via `tokio::io::sink()`).
    pub stdin: Option<Box<dyn AsyncWrite + Unpin + Send>>,
    /// Readable stdout. For `zfs send`, this carries the byte stream.
    pub stdout: Option<Box<dyn AsyncRead + Unpin + Send>>,
    /// Readable stderr. For `zfs recv`, read this to detect resume tokens.
    pub stderr: Option<Box<dyn AsyncRead + Unpin + Send>>,
    inner: ChildWaiter,
}

enum ChildWaiter {
    Process(tokio::process::Child),
    Mock(i32),
}

impl ChildHandle {
    fn from_process(mut child: tokio::process::Child) -> Self {
        let stdin = child
            .stdin
            .take()
            .map(|s| -> Box<dyn AsyncWrite + Unpin + Send> { Box::new(s) });
        let stdout = child
            .stdout
            .take()
            .map(|s| -> Box<dyn AsyncRead + Unpin + Send> { Box::new(s) });
        let stderr = child
            .stderr
            .take()
            .map(|s| -> Box<dyn AsyncRead + Unpin + Send> { Box::new(s) });
        Self {
            stdin,
            stdout,
            stderr,
            inner: ChildWaiter::Process(child),
        }
    }

    /// Creates a mock handle for use in `RecordingRunner` tests. Writes to
    /// `stdin` are silently discarded; `stdout` and `stderr` replay the given
    /// bytes.
    pub(crate) fn mock(stdout: Vec<u8>, stderr: Vec<u8>, exit_code: i32) -> Self {
        Self {
            stdin: Some(Box::new(tokio::io::sink())),
            stdout: Some(Box::new(std::io::Cursor::new(stdout))),
            stderr: Some(Box::new(std::io::Cursor::new(stderr))),
            inner: ChildWaiter::Mock(exit_code),
        }
    }

    /// Wait for the child process to exit and return its exit status. Callers
    /// must finish reading (or drop) `stdout`/`stderr` before calling this, or
    /// run the reads concurrently on separate tasks, to avoid pipe deadlocks.
    pub async fn wait(self) -> io::Result<ExitStatus> {
        match self.inner {
            ChildWaiter::Process(mut c) => c.wait().await,
            ChildWaiter::Mock(code) => Ok(mock_exit_status(code)),
        }
    }
}

#[cfg(unix)]
fn mock_exit_status(code: i32) -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    ExitStatus::from_raw((code & 0xff) << 8)
}

#[async_trait::async_trait]
pub trait CommandRunner: Send + Sync {
    async fn run(&self, cmd: Cmd) -> Result<Output, io::Error>;

    /// Spawn the command and return a [`ChildHandle`] with piped stdio. The
    /// `Cmd.stdin` payload (if any) is ignored — use `child_handle.stdin` to
    /// write after spawning. The default implementation returns an
    /// `io::ErrorKind::Unsupported` error; override in runners that support
    /// streaming.
    async fn spawn(&self, cmd: Cmd) -> Result<ChildHandle, io::Error> {
        let _ = cmd;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "spawn not implemented for this CommandRunner",
        ))
    }
}

pub struct RealRunner;

#[async_trait::async_trait]
impl CommandRunner for RealRunner {
    async fn run(&self, cmd: Cmd) -> Result<Output, io::Error> {
        let stdin_bytes = cmd.stdin.clone();
        let mut command = tokio::process::Command::new(&cmd.program);
        command.args(&cmd.args);

        match stdin_bytes {
            None => command.output().await,
            Some(bytes) => {
                command
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                let mut child = command.spawn()?;
                if let Some(mut stdin) = child.stdin.take() {
                    stdin.write_all(&bytes).await?;
                    stdin.shutdown().await?;
                }
                child.wait_with_output().await
            }
        }
    }

    async fn spawn(&self, cmd: Cmd) -> Result<ChildHandle, io::Error> {
        let mut command = tokio::process::Command::new(&cmd.program);
        command
            .args(&cmd.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = command.spawn()?;
        Ok(ChildHandle::from_process(child))
    }
}

/// Connection target for [`SshCommandRunner`].
#[derive(Debug, Clone)]
pub struct SshTarget {
    pub user: String,
    pub host: String,
    pub port: u16,
}

impl SshTarget {
    pub fn new(user: impl Into<String>, host: impl Into<String>, port: u16) -> Self {
        Self {
            user: user.into(),
            host: host.into(),
            port,
        }
    }

    /// Parse `[user@]host[:port]`. User defaults to `root`, port to 22.
    pub fn parse(s: &str) -> Result<Self, String> {
        let (user, rest) = match s.split_once('@') {
            Some((u, r)) => (u.to_string(), r),
            None => ("root".to_string(), s),
        };
        let (host, port) = match rest.rsplit_once(':') {
            Some((h, p)) => {
                let port: u16 = p.parse().map_err(|e| format!("port `{p}`: {e}"))?;
                (h.to_string(), port)
            }
            None => (rest.to_string(), 22),
        };
        if host.is_empty() {
            return Err("empty host".into());
        }
        Ok(Self { user, host, port })
    }
}

/// Single-quote-escape one shell token. `'` becomes `'\''`.
fn shell_quote(arg: &str) -> String {
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('\'');
    for c in arg.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

fn quote_cmdline(cmd: &Cmd) -> String {
    let mut s = shell_quote(&cmd.program);
    for a in &cmd.args {
        s.push(' ');
        s.push_str(&shell_quote(a));
    }
    s
}

/// SSH-dispatching runner. Wraps each `Cmd` in `ssh user@host -- '<quoted>'`,
/// forwards stdin transparently, returns the remote process's exit code,
/// stdout, and stderr verbatim.
///
/// Intended for integration tests against a throw-away VM (so that real
/// `zfs` operations never touch the host's pools) and for arctern's
/// `stdinserver` SSH transport.
///
/// Caveats:
/// - SSH client returns exit code 255 on connection/auth failure. The remote
///   command can also return 255 legitimately; the two cases are
///   indistinguishable.
/// - The runner shells out to the system `ssh` binary (and `sshpass` when a
///   password is configured). Both must be in `PATH`.
/// - `LogLevel=ERROR` and `UserKnownHostsFile=/dev/null` suppress most SSH
///   client diagnostics, but a small amount of SSH-side text may still mix
///   into the captured stderr.
pub struct SshCommandRunner {
    target: SshTarget,
    /// When set, dispatch via `sshpass -p <pw> ssh ...`. Used for the
    /// archzfs test ISO which boots with empty root password. Never logged.
    password: Option<String>,
}

impl SshCommandRunner {
    pub fn new(target: SshTarget) -> Self {
        Self {
            target,
            password: None,
        }
    }

    /// Construct from `PALIMPSEST_SSH_TARGET=[user@]host[:port]`. Optionally
    /// reads `PALIMPSEST_SSH_PASSWORD` for password auth via sshpass.
    pub fn from_env() -> Result<Self, String> {
        let raw = std::env::var("PALIMPSEST_SSH_TARGET")
            .map_err(|_| "PALIMPSEST_SSH_TARGET not set".to_string())?;
        let target = SshTarget::parse(&raw)?;
        let password = std::env::var("PALIMPSEST_SSH_PASSWORD").ok();
        Ok(Self { target, password })
    }

    pub fn with_password(mut self, pw: impl Into<String>) -> Self {
        self.password = Some(pw.into());
        self
    }

    /// Build `(program, args)` for the local process that wraps `cmd` in ssh.
    /// Extracted so tests can assert on the constructed argv without spawning.
    fn build_local_argv(&self, cmd: &Cmd) -> (String, Vec<String>) {
        let mut argv: Vec<String> = Vec::new();
        let program = if self.password.is_some() {
            argv.extend(["-p".into(), self.password.clone().unwrap(), "ssh".into()]);
            "sshpass".to_string()
        } else {
            "ssh".to_string()
        };
        argv.extend([
            "-o".into(),
            "StrictHostKeyChecking=no".into(),
            "-o".into(),
            "UserKnownHostsFile=/dev/null".into(),
            "-o".into(),
            "LogLevel=ERROR".into(),
        ]);
        if self.password.is_some() {
            argv.extend([
                "-o".into(),
                "PreferredAuthentications=password".into(),
                "-o".into(),
                "PubkeyAuthentication=no".into(),
            ]);
        } else {
            argv.extend(["-o".into(), "BatchMode=yes".into()]);
        }
        argv.extend(["-p".into(), self.target.port.to_string()]);
        argv.push(format!("{}@{}", self.target.user, self.target.host));
        argv.push("--".into());
        argv.push(quote_cmdline(cmd));
        (program, argv)
    }

    fn build_command(&self, cmd: &Cmd) -> tokio::process::Command {
        let (program, argv) = self.build_local_argv(cmd);
        let mut local = tokio::process::Command::new(program);
        local.args(argv);
        local
    }
}

#[async_trait::async_trait]
impl CommandRunner for SshCommandRunner {
    async fn run(&self, cmd: Cmd) -> Result<Output, io::Error> {
        let stdin_bytes = cmd.stdin_bytes().map(<[u8]>::to_vec);
        let mut local = self.build_command(&cmd);
        match stdin_bytes {
            None => local.output().await,
            Some(bytes) => {
                local
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                let mut child = local.spawn()?;
                if let Some(mut stdin) = child.stdin.take() {
                    stdin.write_all(&bytes).await?;
                    stdin.shutdown().await?;
                }
                child.wait_with_output().await
            }
        }
    }

    async fn spawn(&self, cmd: Cmd) -> Result<ChildHandle, io::Error> {
        let mut local = self.build_command(&cmd);
        local
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = local.spawn()?;
        Ok(ChildHandle::from_process(child))
    }
}

/// Fixture record for spawn responses in `RecordingRunner`.
struct SpawnFixture {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit_code: i32,
}

/// Keyed-by-`Cmd` mock runner. Tests call `record(cmd, stdout, stderr, code)`
/// to install a fixture; calls without a matching fixture return an
/// `io::ErrorKind::NotFound` naming the unmatched command.
///
/// Keyed (rather than sequenced) lookup makes refactors that reorder calls
/// non-breaking. Keying on the whole `Cmd` (including stdin) lets tests
/// distinguish, e.g., `load-key` with correct vs wrong passphrase.
pub struct RecordingRunner {
    responses: HashMap<Cmd, Output>,
    spawn_responses: HashMap<Cmd, SpawnFixture>,
}

impl RecordingRunner {
    pub fn new() -> Self {
        Self {
            responses: HashMap::new(),
            spawn_responses: HashMap::new(),
        }
    }

    pub fn record(mut self, cmd: Cmd, stdout: Vec<u8>, stderr: Vec<u8>, exit_code: i32) -> Self {
        self.responses
            .insert(cmd, make_output(stdout, stderr, exit_code));
        self
    }

    /// Install a spawn fixture: when `spawn(cmd)` is called, returns a
    /// `ChildHandle` whose stdout/stderr replay the given bytes and whose
    /// `wait()` returns the given exit code. Stdin writes are discarded.
    pub fn record_spawn(
        mut self,
        cmd: Cmd,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        exit_code: i32,
    ) -> Self {
        self.spawn_responses.insert(
            cmd,
            SpawnFixture {
                stdout,
                stderr,
                exit_code,
            },
        );
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
    async fn run(&self, cmd: Cmd) -> Result<Output, io::Error> {
        match self.responses.get(&cmd) {
            Some(out) => Ok(clone_output(out)),
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("RecordingRunner: no fixture for `{cmd}`"),
            )),
        }
    }

    async fn spawn(&self, cmd: Cmd) -> Result<ChildHandle, io::Error> {
        match self.spawn_responses.get(&cmd) {
            Some(f) => Ok(ChildHandle::mock(
                f.stdout.clone(),
                f.stderr.clone(),
                f.exit_code,
            )),
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("RecordingRunner: no spawn fixture for `{cmd}`"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_builder_basic() {
        let cmd = Cmd::new("zfs").arg("list").args(["-j", "-p"]);
        assert_eq!(cmd.program(), "zfs");
        assert_eq!(cmd.args_list(), &["list", "-j", "-p"]);
        assert!(cmd.stdin_bytes().is_none());
    }

    #[test]
    fn cmd_stdin_visible_in_display() {
        let cmd = Cmd::new("zfs").arg("recv").stdin(b"data".to_vec());
        assert_eq!(format!("{cmd}"), "zfs recv <stdin: 4 bytes>");
    }

    #[test]
    fn cmd_secret_stdin_redacted() {
        let cmd = Cmd::new("zfs")
            .args(["load-key", "tank"])
            .stdin_secret(b"hunter2".to_vec());
        assert_eq!(
            format!("{cmd}"),
            "zfs load-key tank <secret stdin: 7 bytes>"
        );
    }

    #[test]
    fn cmd_eq_hash_includes_stdin() {
        let a = Cmd::new("zfs").arg("load-key").stdin_secret(b"a".to_vec());
        let b = Cmd::new("zfs").arg("load-key").stdin_secret(b"b".to_vec());
        let a2 = Cmd::new("zfs").arg("load-key").stdin_secret(b"a".to_vec());
        assert_ne!(a, b);
        assert_eq!(a, a2);
    }

    #[tokio::test]
    async fn recording_runner_returns_fixture() {
        let runner =
            RecordingRunner::new().record(Cmd::new("echo").arg("hi"), b"hi\n".to_vec(), vec![], 0);
        let out = runner.run(Cmd::new("echo").arg("hi")).await.unwrap();
        assert_eq!(out.stdout, b"hi\n");
        assert!(out.status.success());
    }

    #[tokio::test]
    async fn recording_runner_distinguishes_stdin() {
        let runner = RecordingRunner::new()
            .record(
                Cmd::new("zfs")
                    .args(["load-key", "tank"])
                    .stdin_secret(b"correct".to_vec()),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs")
                    .args(["load-key", "tank"])
                    .stdin_secret(b"wrong".to_vec()),
                vec![],
                b"wrong key".to_vec(),
                1,
            );

        let ok = runner
            .run(
                Cmd::new("zfs")
                    .args(["load-key", "tank"])
                    .stdin_secret(b"correct".to_vec()),
            )
            .await
            .unwrap();
        assert!(ok.status.success());

        let bad = runner
            .run(
                Cmd::new("zfs")
                    .args(["load-key", "tank"])
                    .stdin_secret(b"wrong".to_vec()),
            )
            .await
            .unwrap();
        assert!(!bad.status.success());
        assert_eq!(bad.stderr, b"wrong key");
    }

    #[tokio::test]
    async fn recording_runner_unmatched_returns_not_found() {
        let runner = RecordingRunner::new();
        let err = runner
            .run(Cmd::new("zfs").arg("list"))
            .await
            .expect_err("unmatched call must error");
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(err.to_string().contains("zfs list"));
    }

    #[tokio::test]
    async fn spawn_fixture_streams_stdout_and_waits() {
        use tokio::io::AsyncReadExt;

        let runner = RecordingRunner::new().record_spawn(
            Cmd::new("zfs").args(["send", "tank/data@snap1"]),
            b"stream-data".to_vec(),
            vec![],
            0,
        );
        let mut handle = runner
            .spawn(Cmd::new("zfs").args(["send", "tank/data@snap1"]))
            .await
            .unwrap();
        let mut buf = Vec::new();
        handle
            .stdout
            .as_mut()
            .unwrap()
            .read_to_end(&mut buf)
            .await
            .unwrap();
        assert_eq!(buf, b"stream-data");
        let status = handle.wait().await.unwrap();
        assert!(status.success());
    }

    #[tokio::test]
    async fn spawn_unmatched_returns_not_found() {
        let runner = RecordingRunner::new();
        let err = runner
            .spawn(Cmd::new("zfs").args(["send", "tank/data@snap1"]))
            .await
            .expect_err("unmatched spawn must error");
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn shell_quote_simple() {
        assert_eq!(shell_quote("zfs"), "'zfs'");
        assert_eq!(shell_quote("tank/data@snap1"), "'tank/data@snap1'");
    }

    #[test]
    fn shell_quote_with_single_quote() {
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn shell_quote_with_spaces_and_metachars() {
        assert_eq!(shell_quote("a b $c `d` ; e"), "'a b $c `d` ; e'");
    }

    #[test]
    fn ssh_target_parse_full() {
        let t = SshTarget::parse("alice@example.com:2222").unwrap();
        assert_eq!(t.user, "alice");
        assert_eq!(t.host, "example.com");
        assert_eq!(t.port, 2222);
    }

    #[test]
    fn ssh_target_parse_defaults() {
        let t = SshTarget::parse("example.com").unwrap();
        assert_eq!(t.user, "root");
        assert_eq!(t.host, "example.com");
        assert_eq!(t.port, 22);
    }

    #[test]
    fn ssh_target_parse_user_only() {
        let t = SshTarget::parse("bob@host").unwrap();
        assert_eq!(t.user, "bob");
        assert_eq!(t.port, 22);
    }

    #[test]
    fn ssh_target_parse_host_port_only() {
        let t = SshTarget::parse("host:2225").unwrap();
        assert_eq!(t.user, "root");
        assert_eq!(t.host, "host");
        assert_eq!(t.port, 2225);
    }

    #[test]
    fn ssh_target_parse_rejects_empty_host() {
        assert!(SshTarget::parse("user@:22").is_err());
    }

    #[test]
    fn ssh_runner_argv_pubkey_mode() {
        let r = SshCommandRunner::new(SshTarget::new("root", "localhost", 2225));
        let (prog, argv) = r.build_local_argv(&Cmd::new("zfs").args(["list", "-j"]));
        assert_eq!(prog, "ssh");
        assert_eq!(
            argv,
            vec![
                "-o",
                "StrictHostKeyChecking=no",
                "-o",
                "UserKnownHostsFile=/dev/null",
                "-o",
                "LogLevel=ERROR",
                "-o",
                "BatchMode=yes",
                "-p",
                "2225",
                "root@localhost",
                "--",
                "'zfs' 'list' '-j'",
            ]
        );
    }

    #[test]
    fn ssh_runner_argv_password_mode_uses_sshpass() {
        let r = SshCommandRunner::new(SshTarget::new("root", "localhost", 2225)).with_password("");
        let (prog, argv) = r.build_local_argv(&Cmd::new("echo").arg("ok"));
        assert_eq!(prog, "sshpass");
        assert_eq!(&argv[0..3], &["-p", "", "ssh"]);
        assert!(argv.contains(&"PreferredAuthentications=password".to_string()));
        assert!(argv.contains(&"PubkeyAuthentication=no".to_string()));
        assert!(!argv.contains(&"BatchMode=yes".to_string()));
        assert_eq!(argv.last().unwrap(), "'echo' 'ok'");
    }

    #[test]
    fn ssh_runner_argv_quotes_arg_with_single_quote() {
        let r = SshCommandRunner::new(SshTarget::new("root", "h", 22));
        let (_, argv) = r.build_local_argv(&Cmd::new("sh").args(["-c", "echo it's me"]));
        assert_eq!(argv.last().unwrap(), "'sh' '-c' 'echo it'\\''s me'");
    }

    #[tokio::test]
    async fn spawn_mock_stdin_accepts_writes() {
        use tokio::io::AsyncWriteExt;

        let runner = RecordingRunner::new().record_spawn(
            Cmd::new("zfs").args(["recv", "tank/data"]),
            vec![],
            vec![],
            0,
        );
        let mut handle = runner
            .spawn(Cmd::new("zfs").args(["recv", "tank/data"]))
            .await
            .unwrap();
        // Writes succeed (data is discarded by tokio::io::sink()).
        handle
            .stdin
            .as_mut()
            .unwrap()
            .write_all(b"fake-stream-data")
            .await
            .unwrap();
        let status = handle.wait().await.unwrap();
        assert!(status.success());
    }
}
