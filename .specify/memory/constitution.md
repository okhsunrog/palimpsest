# palimpsest Constitution

A unified async ZFS toolkit for Rust, designed to serve two consumers:

- **archinstall_zfs** — Arch Linux installer; heavy on pool/dataset creation, mount, encryption.
- **arctern** — ZFS replication daemon; heavy on send/recv, holds, bookmarks, resume tokens.

The two overlap at runner, error model, dataset list/get/create/destroy/mount, and encryption status — about 40% of the surface — and that overlap justifies the shared crate.

## Core Principles

### I. CLI-Only, No FFI

Every ZFS operation is implemented by spawning `zfs(8)` or `zpool(8)` via `tokio::process::Command`. We do not link `libzfs` or `libzfs_core`. OpenZFS guarantees a stable ABI only for `libzfs_core`, which does not cover `list`/`get`, so any FFI implementation would still need a CLI fallback. Both consumers also need to work in environments where the `zfs(8)` CLI is the most reliable interface (Arch live ISO; long-lived daemons across distro upgrades). If profiling someday shows fork+exec is a real bottleneck, we revisit; we do not pay for FFI complexity preemptively.

### II. Async-Only

`tokio::process::Command` throughout. The crate exposes no synchronous API. Streaming `zfs send`/`zfs recv` over QUIC requires async stream handles. archinstall_zfs already runs in tokio; converting its blocking ZFS calls is mechanical.

### III. Native JSON Output

OpenZFS 2.2 (2023-10) ships JSON output flags (`-j`) on `zfs list`, `zfs get`, `zfs mount`, `zpool list`, `zpool status`, `zfs holds`. We require it. Tab-separated `-H -p` parsing is legacy and unsupported. For the few commands without `-j` — `zfs send -nP` (size estimate) and `zfs send -nvt <token>` (resume token decode) — we use focused regex parsers.

### IV. Typed Errors at the Operation Boundary

Errors use `thiserror`. `anyhow`/`eyre` is forbidden in this library; consumers convert at the application boundary. One shared `ZfsError` enum covers the common cases (DatasetNotFound, PermissionDenied, Busy, NoSpace, Spawn, Other). A central `classify_stderr(stderr, exit_code) -> ZfsError` regex classifier feeds all command results, so the regex set lives in exactly one place. Dedicated error types exist only where the operation has structured payload that callers must match on (`RecvError::NeedsResumeToken`, `SendArgsError`, `ResumeTokenParseError`, `DestroySnapshotsError`).

### V. Idempotency at the Operation Layer

Holds, bookmarks, snapshot creation, and similar operations are idempotent inside palimpsest — not in callers. `hold(snap, tag)` returns success if the hold already exists with the same tag. `bookmark(snap, name)` checks GUID equivalence before erroring on "bookmark exists." This avoids every caller re-implementing the same retry-or-ignore logic and matches what zrepl learned over years.

### VI. Test Without ZFS

Unit tests must not invoke real `zfs(8)`. The `CommandRunner` trait has a `RecordingRunner` impl that returns canned outputs from JSON fixtures captured under `tests/fixtures/`. Smoke tests against a real zpool live behind a `--features integration` gate and are not run in CI by default. The acceptance test for any slice that touches a shared module is migrating one real callsite in archinstall_zfs.

## Scope and Non-Goals

**In scope.** Linux OpenZFS ≥ 2.2. Datasets (list, get, set, create, destroy, snapshot, rollback, rename, mount, umount). Pools (list, status, create, import, export, set). Holds and bookmarks (idempotent). Send/recv with all the flag combinations zrepl uses (raw, properties, large blocks, compressed, embedded, replicate, resume). Resume token parsing and validation. Encryption status, key load/unload/change. Version and capability detection.

**Out of scope.** No `libzfs`/`libzfs_core` FFI. No sync API. No support for OpenZFS < 2.2, Solaris ZFS, ZFS-on-FUSE, or FreeBSD-only flag variants. No backwards compatibility with any prior ZFS toolkit. No channel programs (`zfs program`), no delegation (`zfs allow`). No live progress streaming for scrub/trim/resilver — status snapshots via `zpool status -j` are in scope.

## Development Workflow

Each operation lands as a self-contained slice under `specs/NNN-<name>/spec.md`. The slice includes API sketch, parser test plan, error classification cases, and an acceptance test that migrates one real callsite in archinstall_zfs (or, for replication-only operations, an integration test against a loopback zpool). Slices are merged in dependency order: foundation (runner, error, dataset/list) before everything else; pool/* and dataset extensions in parallel; hold/bookmark before send/recv; send/recv before replication driver consumers in arctern.

## Governance

Pre-1.0. Breaking changes are allowed in any minor version. Once both consumers are stable on palimpsest, we cut 1.0 and switch to semver discipline. Amendments to this constitution are decided in PR review with explicit reference to which principle is being modified and why.

**Version**: 0.1.0 | **Ratified**: 2026-04-27 | **Last Amended**: 2026-04-27
