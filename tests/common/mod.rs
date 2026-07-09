//! Shared helpers for integration tests. Compiled only when the `integration`
//! feature is on; relies on `ZFSKIT_SSH_TARGET=[user@]host[:port]`
//! pointing at a throwaway VM with `zpool` + `zfs` in PATH.
//!
//! Pool isolation strategy:
//! - Random pool name prefixed with `zfskit_test_` so we can never
//!   collide with a real pool and `just test-cleanup` can grep-and-destroy.
//! - 256 MiB sparse file backing in `/tmp/<pool>.img`.
//! - `-R <altroot>` import so every dataset's mountpoint is prefixed; the
//!   VM's filesystem hierarchy outside `/tmp/<pool>_root/` cannot be touched.
//! - `-m none` on the root dataset to skip auto-mount entirely.

#![cfg(feature = "integration")]
#![allow(dead_code)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use zfskit::pool::{DestroyOptions, ExportOptions, PoolCreateOptions, Vdev};
use zfskit::runner::{Cmd, CommandRunner};
use zfskit::{SshCommandRunner, Zfs, ZfsError};

/// Construct an `SshCommandRunner` from `ZFSKIT_SSH_TARGET` /
/// `ZFSKIT_SSH_PASSWORD`. Panics with a clear message if the env var is
/// unset — integration tests are unrunnable without it.
pub fn ssh_runner_from_env() -> SshCommandRunner {
    SshCommandRunner::from_env().unwrap_or_else(|e| {
        panic!(
            "integration test requires ZFSKIT_SSH_TARGET=[user@]host[:port]: {e}\n\
             tip: `just vm-up` boots the archzfs test ISO and exports the right env"
        )
    })
}

/// Stable-per-process counter combined with epoch nanoseconds to produce a
/// pool-name suffix that's unique across concurrent tests within one process
/// AND across separate process invocations.
fn unique_suffix() -> String {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}_{n:x}")
}

/// A sparse-file-backed throwaway zpool, imported under an altroot so it
/// cannot affect the host's filesystem hierarchy. Call [`Self::create`] to
/// set up, [`Self::destroy`] when done. `Drop` runs a best-effort sync
/// teardown if `destroy` was not called (e.g., on panic).
pub struct LoopbackPool {
    runner: SshCommandRunner,
    name: String,
    img_path: String,
    altroot: String,
    destroyed: bool,
}

impl LoopbackPool {
    /// Default backing-file size for a test pool. 256 MiB is enough for
    /// hundreds of small snapshots while keeping cleanup cheap.
    pub const DEFAULT_SIZE: &'static str = "256M";

    pub async fn create(runner: SshCommandRunner) -> Result<Self, ZfsError> {
        Self::create_with_size(runner, Self::DEFAULT_SIZE).await
    }

    pub async fn create_with_size(runner: SshCommandRunner, size: &str) -> Result<Self, ZfsError> {
        let suffix = unique_suffix();
        let name = format!("zfskit_test_{suffix}");
        let img_path = format!("/tmp/{name}.img");
        let altroot = format!("/tmp/{name}_root");

        run_check(&runner, Cmd::new("truncate").args(["-s", size, &img_path])).await?;
        run_check(&runner, Cmd::new("mkdir").args(["-p", &altroot])).await?;

        let opts = PoolCreateOptions::new(&name)
            .force()
            .pool_property("ashift", "12")
            .fs_property("compression", "lz4")
            .fs_property("atime", "off")
            .mountpoint("none")
            .altroot(&altroot)
            .vdev(Vdev::Stripe(vec![img_path.clone().into()]));
        zfskit::pool::create(&runner, &opts).await?;

        Ok(Self {
            runner,
            name,
            img_path,
            altroot,
            destroyed: false,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn altroot(&self) -> &str {
        &self.altroot
    }

    /// Returns a `Zfs` engine handle bound to a fresh `SshCommandRunner` for
    /// the same target. Cheap — just spawns ssh subprocesses per call.
    pub fn zfs(&self) -> Zfs {
        Zfs::with_runner(SshCommandRunner::from_env().expect("env still set"))
    }

    pub async fn destroy(mut self) -> Result<(), ZfsError> {
        self.destroyed = true;
        self.teardown_async().await
    }

    async fn teardown_async(&self) -> Result<(), ZfsError> {
        let _ = zfskit::pool::export(&self.runner, &self.name, &ExportOptions::default()).await;
        let _ =
            zfskit::pool::destroy(&self.runner, &self.name, &DestroyOptions { force: true })
                .await;
        let _ = run_check(&self.runner, Cmd::new("rm").args(["-f", &self.img_path])).await;
        let _ = run_check(&self.runner, Cmd::new("rm").args(["-rf", &self.altroot])).await;
        Ok(())
    }
}

impl Drop for LoopbackPool {
    fn drop(&mut self) {
        // Best-effort sync cleanup if destroy() wasn't called (e.g., panic).
        // We shell out via std::process directly because we can't drive the
        // async runner from Drop.
        if self.destroyed {
            return;
        }
        let target = std::env::var("ZFSKIT_SSH_TARGET").ok();
        let Some(target) = target else { return };
        let pw = std::env::var("ZFSKIT_SSH_PASSWORD").ok();
        let cmds = [
            format!("zpool destroy -f {} 2>/dev/null || true", self.name),
            format!("rm -f {} 2>/dev/null || true", self.img_path),
            format!("rm -rf {} 2>/dev/null || true", self.altroot),
        ];
        let remote = cmds.join("; ");
        let _ = sync_ssh(&target, pw.as_deref(), &remote);
    }
}

async fn run_check(runner: &SshCommandRunner, cmd: Cmd) -> Result<(), ZfsError> {
    let display = format!("{cmd}");
    let out = runner.run(cmd).await?;
    if out.status.success() {
        return Ok(());
    }
    Err(ZfsError::Other {
        exit_code: out.status.code(),
        stderr: format!(
            "`{display}` failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
    })
}

fn sync_ssh(target: &str, password: Option<&str>, remote_cmd: &str) -> std::io::Result<()> {
    use std::process::{Command, Stdio};

    let target = parse_target_for_drop(target);
    let mut cmd = match password {
        Some(pw) => {
            let mut c = Command::new("sshpass");
            c.args(["-p", pw, "ssh"]);
            c
        }
        None => Command::new("ssh"),
    };
    cmd.args([
        "-o",
        "StrictHostKeyChecking=no",
        "-o",
        "UserKnownHostsFile=/dev/null",
        "-o",
        "LogLevel=ERROR",
    ]);
    if password.is_some() {
        cmd.args([
            "-o",
            "PreferredAuthentications=password",
            "-o",
            "PubkeyAuthentication=no",
        ]);
    } else {
        cmd.args(["-o", "BatchMode=yes"]);
    }
    cmd.args(["-p", &target.2.to_string()]);
    cmd.arg(format!("{}@{}", target.0, target.1));
    cmd.arg("--");
    cmd.arg(remote_cmd);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    Ok(())
}

fn parse_target_for_drop(s: &str) -> (String, String, u16) {
    let (user, rest) = match s.split_once('@') {
        Some((u, r)) => (u.to_string(), r),
        None => ("root".to_string(), s),
    };
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().unwrap_or(22)),
        None => (rest.to_string(), 22),
    };
    (user, host, port)
}
