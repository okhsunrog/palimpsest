# zfskit

Async ZFS toolkit for Rust. Wraps `zfs(8)` and `zpool(8)` via `tokio::process::Command`. Used by two consumers: `archinstall_zfs` (Arch Linux installer) and `arctern` (ZFS replication daemon).

See `docs/constitution.md` for the durable design decisions and `docs/specs/` for per-slice specifications.

## Conventions

- Rust edition 2024.
- Async-only. Never block the runtime; never expose sync APIs from this crate.
- Add deps via `cargo add`. Do not hand-edit version strings in Cargo.toml.
- Errors via `thiserror`. No `anyhow`/`eyre` in this library — those belong at the application boundary.
- Tests use `RecordingRunner` against captured JSON fixtures in `tests/fixtures/`. Unit tests must not invoke real `zfs(8)`.
- Prefer `-j` (native ZFS JSON output) over tab-splitting. Require OpenZFS ≥ 2.2.
- No `libzfs`/`libzfs_core` FFI. CLI-only. This is deliberate; see `docs/constitution.md`.
- Comment only the WHY, never the WHAT. Default to no comment.
- No emojis in code, comments, or commit messages.

## Layout

```
src/
  runner.rs         CommandRunner trait, RealRunner, RecordingRunner
  error.rs          ZfsError enum, classify_stderr regex classifier
  dataset/          zfs list/get/set/create/destroy/snapshot/rollback/...
  pool/             zpool list/status/create/import/export/...
  hold.rs           zfs hold/release/holds (idempotent)
  bookmark.rs       zfs bookmark create/list/destroy (idempotent via guid)
  send/             zfs send + flags + dry-run sizing + Child-owning stream
  recv/             zfs recv + flags + Child-owning stream
  resume_token.rs   parse `zfs send -nvt <token>` output, validate against SendArgs
  encryption.rs     load-key/unload-key/change-key/status detection
  feature.rs        version/capability detection (cached)
  models/           serde types for `-j` JSON output
tests/
  fixtures/         captured *.json outputs from real ZFS, used in unit tests
docs/
  constitution.md   durable design decisions
  specs/            per-slice specifications (001-foundation.md, 002-..., ...)
```

The send/recv/hold/bookmark/resume_token modules are arctern-specific. The pool/* modules are archinstall_zfs-driven. Both consumers share dataset/, encryption.rs, runner, error, and models.

## Commands

- `cargo check`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `cargo add <crate>` to add dependencies
- `cargo fmt`
- `just vm-up` / `just vm-down` / `just vm-ssh` — manage the integration-test VM
- `just test-integration` — run feature-gated integration tests against the running VM
- `just test-vm` — one-shot boot + integration tests + shutdown (for CI)
- `just test-cleanup` — sweep stale `zfskit_test_*` pools inside the VM after a panic

## How to add an operation

1. Add a fixture under `tests/fixtures/<op>_<case>.json` — capture from a real ZFS command if possible (see "Capturing fixtures" below).
2. Define the model under `src/models/` — serde struct matching the `-j` output.
3. Implement the operation under `src/<area>/<op>.rs` — build args, run via `CommandRunner`, parse with serde, classify errors via `classify_stderr`.
4. Write the parser test against the fixture using `RecordingRunner`.
5. If the operation has structured failure modes (resume tokens, validation), add a dedicated error type that wraps `ZfsError`.

## Integration testing against real ZFS

Unit tests use `RecordingRunner` + JSON fixtures and don't require ZFS. The `integration` cargo feature unlocks `tests/integration_*.rs`, which exercise actual `zfs` and `zpool` commands via [`SshCommandRunner`] dispatched into a throwaway VM.

**Why a VM, not host ZFS?** ZFS is a kernel module; loopback-file pools created on the host run inside the host's kernel. With altroot import they cannot affect host filesystem hierarchy, but they do appear in `zpool list` and consume kernel ARC memory. Routing through a VM keeps the host pools and namespaces 100% untouched and gives crash-safe cleanup (just power off the VM).

**Reuse from sibling archinstall_zfs repo**: the same archzfs test ISO that's used for fixture capture (`~/code/archinstall_zfs/gen_iso/out/archzfs-*-testing-*.iso`) is the boot medium. The justfile here boots it on port 2226 (archinstall_zfs uses 2222) so both VMs can run side by side.

**Inner-loop dev**:

```bash
just vm-up            # ~10 s; leave running
just test-integration # repeat as you iterate
just vm-down          # when done
```

**Pool isolation inside the VM** (see `tests/common/mod.rs`):

- Random pool name `zfskit_test_<nanos>_<seq>` — collision with anything real is impossible.
- 256 MiB sparse file in `/tmp/<pool>.img`.
- `-R /tmp/<pool>_root` altroot import — every dataset's mountpoint is prefixed under the altroot, so the VM filesystem outside that path is untouchable.
- `-m none` on the root dataset for belt-and-suspenders.
- `LoopbackPool::destroy()` on success path; `Drop` runs sync best-effort cleanup if a test panics; `just test-cleanup` mops up after that fails too.

**Adding integration tests**: drop a new file under `tests/integration_<area>.rs` with `#![cfg(feature = "integration")]` at top, `mod common;`, then `let pool = LoopbackPool::create(ssh_runner_from_env()).await?;` and operate via `pool.zfs()`.

## Capturing fixtures from a clean ZFS environment

Real fixtures come from a QEMU VM running the archzfs test ISO. The VM gives you root + ZFS without touching the host's pools, and a clean predictable state (`tank/...` instead of whatever the host has).

**Prerequisites** (paths from the sibling `archinstall_zfs` repo):

- Test ISO: `~/code/archinstall_zfs/gen_iso/out/archzfs-linux-lts-pre-testing-*.iso`. Built by `just iso-test` from that repo. **Must be the test ISO, not the full one** — only the test ISO has `PermitEmptyPasswords yes` (set when the build is rendered with `--fast`).
- QEMU disk + UEFI vars: `~/code/archinstall_zfs/gen_iso/{arch.qcow2,my_vars.fd}`. Created by `just qemu-setup` from that repo if missing.
- Host packages: `qemu-system-x86_64`, `edk2-ovmf` (or equivalent), `sshpass`.

**Boot, capture, shutdown** — minimum-viable workflow:

```bash
# 1. Boot the test ISO in headless QEMU on a free SSH port
qemu-system-x86_64 \
  -enable-kvm -cpu host -m 4096 -smp 2 \
  -boot order=d -display none \
  -net nic -net user,hostfwd=tcp::2225-:22 \
  -machine type=q35,smm=on,accel=kvm,usb=on \
  -global ICH9-LPC.disable_s3=1 -no-reboot \
  -drive if=pflash,format=raw,unit=0,file=/usr/share/edk2/x64/OVMF_CODE.4m.fd,read-only=on \
  -drive if=pflash,format=raw,unit=1,file=$HOME/code/archinstall_zfs/gen_iso/my_vars.fd \
  -cdrom $HOME/code/archinstall_zfs/gen_iso/out/archzfs-linux-lts-pre-testing-*.iso \
  -drive file=$HOME/code/archinstall_zfs/gen_iso/arch.qcow2,format=qcow2,if=none,id=disk0 \
  -device virtio-blk-pci,drive=disk0,serial=archzfs-test-disk \
  > /tmp/qemu.log 2>&1 &

# 2. SSH (root, empty password — must use sshpass, plain ssh hangs on the prompt)
SSH='sshpass -p "" ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
     -o PreferredAuthentications=password -o PubkeyAuthentication=no -p 2225 root@localhost'
SCP='sshpass -p "" scp -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -P 2225'

# Wait for SSH (typically ~5-10 s on a warm host)
until $SSH "echo ready" >/dev/null 2>&1; do sleep 2; done

# 3. Set up a clean test pool inside the VM (sparse-file backed; ephemeral)
$SSH 'bash -s' <<'VM'
set +e
zpool destroy tank 2>/dev/null
rm -f /tmp/tank.img /tmp/keyfile
truncate -s 2G /tmp/tank.img
zpool create -f -o ashift=12 -O compression=lz4 -O atime=off tank /tmp/tank.img
zfs create tank/data
zfs create tank/data/home
zfs set mountpoint=/mnt/test tank/data
zfs snapshot tank/data/home@snap1
zfs snapshot tank/data/home@snap2
zfs bookmark tank/data/home@snap1 tank/data/home#bm1
echo -n test12345 > /tmp/keyfile && chmod 0400 /tmp/keyfile
zfs create -o encryption=aes-256-gcm -o keyformat=passphrase -o keylocation=file:///tmp/keyfile tank/encrypted
zfs hold mytag tank/data/home@snap1
zfs set quota=100M tank/data
mkdir -p /tmp/fix
VM

# 4. Capture (always set +e in the script — many captures expect non-zero exits;
#    redirect: stdout → file, stderr separately if you only want the error text)
$SSH 'zfs get -j -p encryption tank > /tmp/fix/dataset_get_encryption_off.json'
$SSH 'zfs unload-key tank/encrypted; zfs unload-key tank/encrypted 2> /tmp/fix/err_unload_key_not_loaded.stderr'
# ... etc

# 5. Pull fixtures back
$SCP -r 'root@localhost:/tmp/fix/*' tests/fixtures/

# 6. Power off (clean)
$SSH "poweroff" || true
```

**Stderr-redirect gotchas:**

- `cmd > file 2>&1` — stdout AND stderr to file (use for full transcripts).
- `cmd > /dev/null 2> file` — stderr only to file (use for error-pattern fixtures).
- `cmd 2>&1 1>/dev/null > file` is **wrong** — produces an empty file. Order matters: redirections are evaluated left-to-right with current targets at the time of evaluation.

**Common operations that don't have `-j`** (need text parsers, not serde):

- `zfs holds` — tab-separated `dataset@snap\ttag\ttimestamp_unix`
- `zfs send -nP` — tab-separated `full|incremental\t…\t<bytes>` plus a `size\t<bytes>` row
- `zfs send -nvt <token>` — nvlist text format (`\tkey = value` lines)
- `zpool import` (discovery form, no args) — block-formatted text with `pool: <name>` headers

Capture these as `.txt` and write hand-rolled parsers in their owning slice.

**When in doubt about command output format**: SSH in, run the command with `--help`, and try `-j` directly. `invalid option 'j'` ⇒ JSON not supported on that command in the local OpenZFS version.

<!-- SPECKIT START -->
For additional context about technologies to be used, project structure,
shell commands, and other important information, read the current plan
<!-- SPECKIT END -->
