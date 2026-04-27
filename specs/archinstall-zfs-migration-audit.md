# archinstall_zfs → palimpsest migration audit

Reference document, not a slice. Captures the migration intel surfaced during slice 001 and 002 work so future sessions have it without re-deriving from the archinstall_zfs source tree.

## Plain-text stdout parsing sites (the interesting list)

These are the only callsites in archinstall_zfs that actually parse stdout text (not exit codes). They're the migration *targets* that need real palimpsest API support.

| Site | Command | What it parses | Slice |
|---|---|---|---|
| `core/src/zfs/encryption.rs:50` (`detect_encryption`) | `zfs get -H -o value encryption <pool>` | trims stdout, compares to `"off"` | **slice 002 (ready)** — migrates to `palimpsest::dataset::get_property` |
| `core/src/zfs/pool.rs:118` (`discover_importable_pools`) | `zpool import` (no args) | scans stdout+stderr for `pool: <name>` lines | future slice — needs `palimpsest::pool::discover` |

### Critical finding: `zpool import` has no `-j` flag

Verified locally in OpenZFS 2.4.1: `zpool import --help` does not list `-j`, and `zpool import -j` errors with `invalid option 'j'`. The discovery form returns plain text:

```
   pool: tank
     id: 9316430153991325696
  state: ONLINE
 action: ...
 config: ...
```

When palimpsest grows a `pool::discover` operation, it will need a **custom line-based parser**, not serde. Capture a real fixture from the VM at that time. The current archinstall regex is `^pool:\s+(\S+)` against combined stdout+stderr.

## Already on `-j` (handled by palimpsest's foundation slice)

These callsites already use `-j`, so migrating them to palimpsest's typed API is a mechanical type-swap once the corresponding palimpsest operation lands:

| Site | Command | Status |
|---|---|---|
| `dataset.rs::list_datasets` | `zfs list -j` | migrated as canary in slice 001 |
| `dataset.rs::list_all_datasets` | `zfs list -j` | dead code; canary covers |
| `dataset.rs::get_property` (line 97) | `zfs get -j` | migrate via `palimpsest::dataset::get_property` (slice 002 ready) |
| `dataset.rs::list_mounts` (line 101) | `zfs mount -j` | needs `palimpsest::dataset::list_mounts` (future slice) |
| `pool.rs::list_pools` (line 104) | `zpool list -j` | needs `palimpsest::pool::list` (future slice) |
| `pool.rs::pool_status` (line 108) | `zpool status -j` | needs `palimpsest::pool::status` (future slice) |

## Exit-code-only side-effect commands

These callsites *don't parse stdout at all* — they just check exit code. Each becomes a thin `async fn ...(runner, ...) -> Result<(), ZfsError>` in palimpsest. No JSON, no models, just args+classify.

**`zfs` side-effects:**
- `zfs create` (multiple call sites)
- `zfs set`
- `zfs mount`
- `zfs umount`
- `zfs load-key` (line 43, 114) — **slice 002 ships `palimpsest::encryption::load_key`**
- `zfs unload-key` (line 62, 65, 86, 121) — **slice 002 ships `palimpsest::encryption::unload_key`**

**`zpool` side-effects:**
- `zpool create` (pool.rs:59) — needs `palimpsest::pool::create`
- `zpool import` (pool.rs:66/77; encryption.rs:76/100) — **slice 002 ships `palimpsest::pool::import`**
- `zpool export` (pool.rs:86; encryption.rs:87/123) — **slice 002 ships `palimpsest::pool::export`**
- `zpool set` (pool.rs:98; bootmenu.rs:271) — needs `palimpsest::pool::set`

## Existence probes (reformulate, don't reimplement)

These "exists?" probes use exit-code from a list query as a boolean signal:

- `dataset_exists` (`dataset.rs:106`) — `zfs list -H <name>`, exit 0 ⇒ exists
- `pool_exists` (`pool.rs:111`) — `zpool list <name>`, exit 0 ⇒ exists

**Migration approach**: don't add a `dataset::exists` or `pool::exists` to palimpsest. Use `palimpsest::dataset::list` (or `::pool::list` when it lands) with the name as the only root and check the result `Vec` for emptiness. Same wire effect, more general API.

## Real palimpsest gap: `run_with_stdin`

`encryption.rs:65` (`verify_passphrase`) calls `runner.run_with_stdin("zfs", &["load-key", pool], password.as_bytes())`. archinstall_zfs's `CommandRunner` trait has a `run_with_stdin` method; **palimpsest's does not**. The `CommandRunner` trait in `src/runner.rs` exposes only `async fn run(&self, program, args)`.

When `verify_passphrase` migrates, palimpsest needs one of:

1. **Extend the existing trait** with `async fn run_with_stdin(&self, program, args, stdin: &[u8])`. Implementers (`RealRunner`, `RecordingRunner`) gain a method. Consumers that don't need stdin ignore it. Backwards-compatible if the new method has a default that calls `run` and discards stdin (or returns "not implemented" for `RecordingRunner` keyed by args-only).
2. **Sibling trait** `CommandRunnerStdin: CommandRunner` for stdin-aware ops only. Cleaner separation but more types to thread.

Decide when that consumer is migrated. Probably (1) — the trait is small, the extension is simple, and most consumers will eventually want it.

## Captured stderr patterns

Real OpenZFS 2.4.1 stderr stored under `tests/fixtures/`. Reference for future classifier work or test rewrites:

| Pattern | Fixture | Used by |
|---|---|---|
| `cannot open 'X': dataset does not exist` | `err_dataset_not_found.txt` | classifier |
| `cannot import 'X': no such pool available` | (not captured; spec text only) | classifier (slice 002 added) |
| `cannot destroy snapshot X: it's being held. Run...` | `err_busy_held.txt` | future "destroy with holds" slice |
| `cannot hold snapshot 'X': tag already exists on this dataset` | `err_hold_already_exists.txt` | future hold idempotency |
| `cannot create bookmark 'X': bookmark exists` | `err_bookmark_exists.txt` | future bookmark idempotency |
| `bad property list: invalid property 'X'` (+ giant help dump) | `err_invalid_property.txt` | could become `ZfsError::UnknownProperty` if needed |
| `Key unload error: Key already unloaded for 'X'.` | `err_unload_key_not_loaded.stderr` | **slice 002 idempotency** |
| `Key load error: Key already loaded for 'X'.` | `err_load_key_already.stderr` | **slice 002 idempotency** |
| `Key unload error: 'X' is not encrypted.` | `err_unload_key_unencrypted.stderr` | propagates as `Other` (informational) |
| `cannot receive new filesystem stream: ... A resuming stream can be generated by running: zfs send -t TOKEN` | `send_recv_interrupted.stderr` | future recv slice — extract token, return `RecvError::NeedsResumeToken` |

## Other format quirks (no `-j` even in 2.4.1)

For these, parsers will be hand-rolled when their slice lands:

- `zfs holds` → tab-separated `dataset@snap\ttag\ttimestamp_unix`. Fixture: `holds.txt`.
- `zfs send -nP` → tab-separated `full\t<snap>\t<bytes>` or `incremental\t<from>\t<to>\t<bytes>` plus a trailing `size\t<bytes>` line. Fixtures: `send_dry_run_full.txt`, `send_dry_run_incremental.txt`.
- `zfs send -nvt <token>` → nvlist text format (`nvlist version: 0` followed by `\tkey = value` lines). Fixture: `send_resume_token_decoded.txt`.
- `zpool import` (discovery) — see above.

## What this audit does **not** cover

- archinstall_zfs's installer pipeline (the 12-phase install flow). Those are application logic, not ZFS calls.
- `bootmenu.rs` ZFSBootMenu integration. Application-specific.
- AUR / package-management calls. Not ZFS.
- libalpm bindings. Not ZFS.

The audit is scoped to the boundary palimpsest is meant to subsume: the `zfs(8)` and `zpool(8)` CLI surface used by archinstall_zfs.
