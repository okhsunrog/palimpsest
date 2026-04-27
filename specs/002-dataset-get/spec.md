# Feature Specification: dataset::get + pool::import/export + encryption::unload_key

**Feature Branch**: `002-dataset-get`
**Created**: 2026-04-27
**Status**: Draft
**Depends on**: `001-foundation-command-runner` (merged on master). Reuses `CommandRunner`, `ZfsError`, `classify_stderr`, and the `models::common` types unchanged.

## Why this slice

Slice 001's acceptance test was a compile-canary against dead code. To get a *real* live-caller validation in archinstall_zfs, the smallest viable target is the entire `detect_pool_encryption` orchestrator at `core/src/zfs/encryption.rs:74-91`, called from `tui/src/tui/screens/pickers.rs:141` on every install that touches an existing pool. That orchestrator chains four ZFS operations:

```rust
zpool import -fN <pool>
zfs get encryption <pool>     // via detect_encryption
zfs unload-key <pool>
zpool export <pool>
```

Migrating only the inner `zfs get` would leave the orchestrator holding two runners (sync archinstall + async palimpsest) — recipe for trait-gap pain that proves nothing structural. So this slice bundles **all four** operations: `dataset::get`, `pool::import`, `pool::export`, `encryption::unload_key`. With those four, archinstall_zfs can rewrite `detect_pool_encryption` end-to-end against a single async runner, and we get a clean live-caller acceptance test.

## Consumer Scenarios & Testing *(mandatory)*

### Story 1 — End-to-end migration of `detect_pool_encryption` (Priority: P1)

archinstall_zfs's `detect_pool_encryption` is rewritten to use only `palimpsest` operations against `palimpsest::CommandRunner`. The four ZFS calls become palimpsest function calls; the function signature becomes `async fn`; existing tests pass with their `MockRunner` substituted by `palimpsest::RecordingRunner`.

**Why this priority**: This is the live-caller acceptance test. Without this, slice 002 has the same weakness as slice 001.

**Independent Test**: Replace `detect_pool_encryption`'s body with palimpsest calls. Convert the existing tests `test_detect_pool_encryption_encrypted/not_encrypted/import_fails` to use `palimpsest::RecordingRunner`. All three must pass without other code changes (sequenced→keyed runner is a localized test rewrite).

**Acceptance Scenarios**:

1. **Given** archinstall_zfs depends on palimpsest as a path dependency, **When** `detect_pool_encryption` is rewritten to call `palimpsest::pool::import → palimpsest::dataset::get_property("encryption") → palimpsest::encryption::unload_key → palimpsest::pool::export`, **Then** the function compiles, the existing async cascade in the installer pipeline accepts the `async fn` signature, and the rewritten tests pass.
2. **Given** a `RecordingRunner` programmed for the encrypted-pool case, **When** `detect_pool_encryption` is called, **Then** it returns `true` and all four ZFS commands are issued in order (verified by RecordingRunner's call log, future enhancement).

### Story 2 — Single-property convenience read (Priority: P1)

`get_property(runner, dataset, property) -> PropertyValue` is the dominant consumer pattern. Used by `detect_encryption` (via Story 1) and many future consumers.

**Why this priority**: dominant API shape; if this is awkward, `dataset::get` has failed.

**Independent Test**: `dataset_get_encryption_off.json` and `dataset_get_encryption_on.json` fixtures, assert returned `PropertyValue` round-trips correctly including the `PropertySourceKind`.

**Acceptance Scenarios**:

1. **Given** the captured `dataset_get_encryption_off.json`, **When** `get_property(&runner, "tank", "encryption")` is called, **Then** it returns `Ok(PropertyValue { value: "off", source.kind: PropertySourceKind::Default, .. })`.
2. **Given** the captured encrypted fixture, **When** `get_property(&runner, "tank/encrypted", "encryption")` is called, **Then** it returns `value == "aes-256-gcm"`.

### Story 3 — Multi-property batch read (Priority: P2)

Encryption-aware code commonly reads `encryption`, `keystatus`, `keyformat`, `keylocation` together; a batch API avoids four shells.

**Independent Test**: `dataset_get_encryption_batch.json` fixture, assert all 4 keys present.

**Acceptance Scenarios**:

1. **Given** the batch fixture, **When** `get(&runner, &GetOptions { properties: vec!["encryption","keystatus","keyformat","keylocation"], datasets: vec!["tank/encrypted"], ..default() })` is called, **Then** the returned `Vec<ZfsGetEntry>` has length 1 with all four keys in the property map.

### Story 4 — Pool import / export (Priority: P1)

`pool::import(&runner, "tank", &ImportOptions { force: true, no_mount: true, .. })` issues `zpool import -fN tank` and returns `Result<(), ZfsError>`. `pool::export(&runner, "tank", &ExportOptions::default())` issues `zpool export tank`. Errors classified via `classify_stderr` plus a small handful of pool-specific stderr patterns ("pool already exists", "no such pool", "pool is busy").

**Independent Test**: with a `RecordingRunner` returning success or canned stderr, assert the command line constructed and the error variants returned.

**Acceptance Scenarios**:

1. **Given** a runner returning exit 0 with empty stderr for `zpool import -fN tank`, **When** `pool::import("tank", &ImportOptions { force: true, no_mount: true, .. })` is called, **Then** it returns `Ok(())`.
2. **Given** stderr `cannot import 'tank': no such pool available`, **When** `pool::import` is called, **Then** it returns `Err(ZfsError::PoolNotFound { name: "tank" })` (new variant added to `ZfsError`).
3. **Given** a runner programmed for export success, **When** `pool::export("tank", &ExportOptions::default())` is called, **Then** it returns `Ok(())` and the constructed command is `zpool export tank`.

### Story 5 — encryption::unload_key (Priority: P2)

`encryption::unload_key(&runner, "tank") -> Result<(), ZfsError>` issues `zfs unload-key tank`. Idempotent: if the key is already unloaded, returns `Ok(())` (per ZFS behavior + matching zrepl's pattern).

**Independent Test**: runner returning success → `Ok(())`; runner returning the "Key load error: Keys must be loaded for encryption to be set" stderr → `Ok(())` (idempotent).

**Acceptance Scenarios**:

1. **Given** a runner returning exit 0, **When** `encryption::unload_key("tank")` is called, **Then** it returns `Ok(())`.
2. **Given** a runner returning the already-unloaded stderr, **When** the same call is made, **Then** it returns `Ok(())` (idempotent).

### Edge Cases

- `pool::import` against a pool that's already imported. ZFS prints "cannot import 'tank': a pool with that name already exists". Currently classified as `ZfsError::Other`; if archinstall_zfs needs to distinguish, add a `PoolAlreadyImported` variant. Defer until needed.
- `dataset::get` with an unknown property. Stderr is `bad property list: invalid property '<name>'` (captured in `tests/fixtures/err_invalid_property.txt`). Surfaces as `ZfsError::Other` for now.
- `pool::export` against a busy pool. Stderr is `cannot export 'tank': pool is busy`. Add `PoolBusy { name }` variant if needed by callers; for v1, route through `Other`.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: `dataset::get(runner, &GetOptions) -> Result<Vec<ZfsGetEntry>, ZfsError>`. Builds `zfs get -j -p [-r] [-d N] [-t types] [-s sources] -o name,property,value,source <props> <datasets...>`. Sorts result by name.
- **FR-002**: `dataset::get_property(runner, dataset, property) -> Result<PropertyValue, ZfsError>` convenience.
- **FR-003**: `pub type ZfsGetEntry = ZfsListEntry`. Captured fixtures confirm structural identity with `zfs list -j` entries.
- **FR-004**: `PropertySourceKind::cli_name(self) -> &'static str` returns `"none"|"local"|"default"|"inherited"|"received"|"temporary"`.
- **FR-005**: `pool::import(runner, pool: &str, opts: &ImportOptions) -> Result<(), ZfsError>` where `ImportOptions { force, no_mount, altroot: Option<PathBuf>, .. }`. Default = no flags. `force` adds `-f`, `no_mount` adds `-N`, `altroot` adds `-R <path>`.
- **FR-006**: `pool::export(runner, pool: &str, opts: &ExportOptions) -> Result<(), ZfsError>` where `ExportOptions { force }`. `force` adds `-f`.
- **FR-007**: `encryption::unload_key(runner, dataset: &str) -> Result<(), ZfsError>`. Idempotent: returns `Ok(())` if key was already unloaded.
- **FR-008**: `ZfsError` gains a `PoolNotFound { name: String }` variant. `classify_stderr` recognizes the patterns `cannot import 'X': no such pool available` and `cannot open 'X': no such pool` (note: distinct from existing `cannot open 'X': dataset does not exist` — they share regex shape but have different stderr endings; both already covered by existing classifier patterns at the dataset level — pool-specific recognition is a refinement).
- **FR-009**: At least 8 fixture-driven tests cover: `get_property` off, `get_property` on, `get` batch, `get` recursive, `get` source-filter, `pool::import` success, `pool::export` success, `unload_key` idempotence.

### Key Entities

- **GetOptions** — recursive, depth, types, sources, datasets, properties.
- **ImportOptions** — force, no_mount, altroot.
- **ExportOptions** — force.
- **ZfsGetEntry** — type alias for `ZfsListEntry`.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: `archinstall_zfs/core/src/zfs/encryption.rs::detect_pool_encryption` is rewritten to use only palimpsest functions, becomes `async fn`, and the existing tests `test_detect_pool_encryption_encrypted/not_encrypted/import_fails` pass after rewriting them to use `palimpsest::RecordingRunner` (sequenced → keyed). No other code changes.
- **SC-002**: `cargo test -p palimpsest` runs to green on a machine with no ZFS installed.
- **SC-003**: `cargo clippy --all-targets -- -D warnings` is clean.
- **SC-004**: At least 8 fixture-driven tests covering the surfaces in FR-009.
- **SC-005**: Total slice size: ~700-900 LoC of palimpsest source plus tests, plus ~30-60 LoC changed in archinstall_zfs (rewriting `detect_pool_encryption` and the three test functions).

## Assumptions

- `zfs get -j` JSON schema is identical to `zfs list -j`. **Confirmed** via captured fixtures.
- `zpool import` and `zpool export` produce no JSON; success/failure is exit code + stderr. archinstall_zfs's existing logic confirms this.
- The 3-hop sync→async cascade from `detect_encryption` to `wizard::activate_item` (per recon) is tractable: rewrite the three sync intermediates as `async fn` and propagate `.await`.

## Open Questions

1. **`get_property` on missing-property semantics**. If JSON parses but the property is absent from the entry, return `ZfsError::Other` with a clear message. Typed `PropertyNotFound` deferred.
2. **`"all"` as a property name vs. `GetOptions::all_properties: bool`**. The string form mirrors `zfs(8)`; the bool is more Rusty. Decide during implementation; lean toward the string.
3. **Pool error variants**. Whether `PoolNotFound` is enough for now, or whether `PoolAlreadyImported` and `PoolBusy` should land in this slice. Decide during the migration: if archinstall_zfs's tests need to match on those specifically, add them; otherwise route through `Other` and add later.

## Migration plan (informative)

```rust
// before (encryption.rs)
pub fn detect_pool_encryption(runner: &dyn CommandRunner, pool: &str) -> bool {
    let import = runner.run("zpool", &["import", "-fN", pool]);
    if import.is_err() || !import.as_ref().unwrap().success() { return false; }
    let encrypted = detect_encryption(runner, pool).unwrap_or(false);
    let _ = runner.run("zfs", &["unload-key", pool]);
    let _ = runner.run("zpool", &["export", pool]);
    encrypted
}

// after
pub async fn detect_pool_encryption(
    runner: &dyn palimpsest::CommandRunner,
    pool: &str,
) -> bool {
    use palimpsest::pool::ImportOptions;
    let import_opts = ImportOptions { force: true, no_mount: true, ..Default::default() };
    if palimpsest::pool::import(runner, pool, &import_opts).await.is_err() {
        tracing::debug!(pool, "ephemeral import failed for encryption detection");
        return false;
    }
    let encrypted = match palimpsest::dataset::get_property(runner, pool, "encryption").await {
        Ok(p) => p.value != "off" && !p.value.is_empty(),
        Err(_) => false,
    };
    let _ = palimpsest::encryption::unload_key(runner, pool).await;
    let _ = palimpsest::pool::export(runner, pool, &Default::default()).await;
    tracing::info!(pool, encrypted, "detected pool encryption state");
    encrypted
}
```

The signature changes from `&dyn archinstall::CommandRunner` to `&dyn palimpsest::CommandRunner` and gains `async`. The 3-hop cascade up through `pick_existing_pool` → `wizard::activate_item` is rewritten to propagate `.await`. The 3 affected tests (`test_detect_pool_encryption_*`) switch their fixture-runner from archinstall's sequenced FIFO to palimpsest's keyed-by-`(program, args)`.
