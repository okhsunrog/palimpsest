# Feature Specification: Foundation — Command Runner, Error Model, Dataset List

**Feature Branch**: `001-foundation-command-runner`
**Created**: 2026-04-27
**Status**: Draft
**Input**: Smallest end-to-end vertical that exercises the whole stack: command runner + error model + one ZFS read operation. Validates the core abstractions before any operation-heavy slice (pool, snapshot, send/recv) clones the pattern.

## Consumer Scenarios & Testing *(mandatory)*

The "users" of this feature are the two downstream Rust crates that depend on palimpsest. Stories are consumption scenarios.

### Story 1 — Migration of an existing list_datasets callsite (Priority: P1)

`archinstall_zfs` already has a function that lists ZFS datasets to populate the installer's dataset picker. Migrating one such callsite to use `palimpsest::dataset::list` proves the foundation is sound: the runner abstraction, the error model, and the JSON parsing all get exercised in production code, against a real consumer's tests.

**Why this priority**: Without a working real-consumer migration, every later slice is built on unproven abstractions. This is the foundational acceptance test.

**Independent Test**: Replace one call to the existing `archinstall_zfs` `list_datasets` (or equivalent) with `palimpsest::dataset::list`, then run archinstall_zfs's existing test suite. Pass = foundation is correct.

**Acceptance Scenarios**:

1. **Given** archinstall_zfs depends on palimpsest via a path dependency, **When** an existing dataset-listing callsite is rewritten to call `palimpsest::dataset::list` with equivalent options, **Then** the callsite compiles and archinstall_zfs's existing tests pass without modification.
2. **Given** the dataset listing returns an entry, **When** the consumer reads its name, type, and properties, **Then** the values match what `zfs list -j` produced for that dataset.

### Story 2 — Test without ZFS (Priority: P1)

Both downstream crates need to write unit tests that exercise palimpsest-using code without requiring a real zpool. The `RecordingRunner` impl of `CommandRunner` returns canned outputs from JSON fixtures.

**Why this priority**: Tests that require real ZFS run only on machines with zpools; tests that don't run everywhere. This unblocks CI on hosted runners and unblocks the contributor experience.

**Independent Test**: Write a test that constructs a `RecordingRunner` keyed to a fixture file, calls `palimpsest::dataset::list`, and asserts on the parsed result. The test runs to green on a machine with no ZFS installed.

**Acceptance Scenarios**:

1. **Given** a `tests/fixtures/dataset_list_simple.json` capturing real `zfs list -j` output, **When** a `RecordingRunner` configured with that fixture is passed to `dataset::list`, **Then** the call returns the expected `Vec<ZfsListEntry>` without invoking any subprocess.

### Story 3 — Stable error classification (Priority: P2)

Both consumers need to distinguish "dataset doesn't exist" from "permission denied" from "out of disk space" without parsing stderr themselves. A central regex classifier produces a typed `ZfsError`.

**Why this priority**: Once any consumer code starts matching on stderr substrings, the abstraction has failed. Getting this right at the start prevents leakage.

**Independent Test**: Feed known-stderr strings into `classify_stderr` and assert the returned variant is correct. Done as a pure unit test.

**Acceptance Scenarios**:

1. **Given** stderr contains `cannot open 'tank/foo': dataset does not exist`, **When** `classify_stderr` is called with that stderr and exit code 1, **Then** it returns `ZfsError::DatasetNotFound { name: "tank/foo" }`.
2. **Given** stderr contains `permission denied`, **When** `classify_stderr` is called, **Then** it returns `ZfsError::PermissionDenied`.
3. **Given** stderr contains `out of space`, **When** `classify_stderr` is called, **Then** it returns `ZfsError::NoSpace`.
4. **Given** stderr matches none of the known patterns, **When** `classify_stderr` is called, **Then** it returns `ZfsError::Other { stderr, exit_code }` carrying the raw stderr for the consumer's logs.

### Edge Cases

- A `zfs list -j` output containing a dataset whose name contains a non-UTF-8 byte. (ZFS forbids this in practice but we must not panic — wrap in serde error → `ZfsError::Other`.)
- A `RecordingRunner` lookup miss (test author asks for a fixture that wasn't loaded). Should return a clear test-time panic or an error with a precise message identifying which `(program, args)` was unmatched.
- An empty `zfs list -j` result (no datasets match the filter). Returns `Ok(vec![])`.
- A property request that includes a property the local OpenZFS version doesn't know about. ZFS prints a per-line stderr warning and exits non-zero; we surface this as `ZfsError::Other` rather than silently dropping.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The `CommandRunner` trait MUST expose `async fn run(&self, program: &str, args: &[&str]) -> Result<std::process::Output, std::io::Error>`. The streaming variant for `zfs send`/`zfs recv` is deferred to a later slice.
- **FR-002**: `RealRunner` MUST implement `CommandRunner` by delegating to `tokio::process::Command::new(program).args(args).output().await`.
- **FR-003**: `RecordingRunner` MUST implement `CommandRunner` by returning a pre-loaded fixture matching the call's `(program, args)`. On miss, it MUST return a deterministic error or panic with a message naming the unmatched call.
- **FR-004**: The `ZfsError` enum MUST include the variants `DatasetNotFound { name }`, `PermissionDenied`, `Busy { name }`, `NoSpace`, `Spawn(io::Error)`, `Other { exit_code, stderr }`. Additional variants MAY be added if foundation slice fixtures reveal patterns that warrant them.
- **FR-005**: `classify_stderr(stderr: &str, exit_code: Option<i32>) -> ZfsError` MUST return the most specific matching variant for known patterns and `Other` otherwise. The function MUST live in exactly one module (`error`); other modules MUST NOT inline regex matching against stderr.
- **FR-006**: `dataset::list` MUST accept a `ListOptions` struct supporting at minimum: recursive listing, max depth, dataset type filter (filesystem, volume, snapshot, bookmark, all), root dataset selection, and a property selection list.
- **FR-007**: `dataset::list` MUST construct the `zfs list -j -H -p` command line and parse its JSON output via serde into `Vec<ZfsListEntry>`.
- **FR-008**: `models::dataset::ZfsListEntry` MUST expose `name: String`, dataset `type`, and a property map keyed by property name.
- **FR-009**: At least three captured `zfs list -j` JSON outputs MUST exist in `tests/fixtures/` covering: a single-dataset case, a recursive listing with children, and a mixed listing including snapshots and bookmarks alongside filesystems.

### Key Entities

- **CommandRunner** — Async trait abstracting subprocess execution. Two impls: `RealRunner` (tokio::process) and `RecordingRunner` (fixture-driven).
- **ZfsError** — Typed error enum for the common failure modes any operation can produce. Built by `classify_stderr` from raw stderr + exit code.
- **ZfsListEntry** — One row of `zfs list -j` output. Contains the dataset name, type, and a property map.
- **ListOptions** — Builder-style request describing what `zfs list` should ask ZFS for.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An `archinstall_zfs` callsite that previously called the project's internal `list_datasets` is rewritten to call `palimpsest::dataset::list`, and `cargo test` in archinstall_zfs passes without other code changes.
- **SC-002**: `cargo test -p palimpsest` runs to green on a machine with no ZFS installed (fixture-driven tests only).
- **SC-003**: The `error` module contains exactly one definition of `classify_stderr`; no other module under `src/` matches stderr against any regex.
- **SC-004**: At least 3 fixture files exist under `tests/fixtures/` and are exercised by at least 3 distinct parser tests.
- **SC-005**: Total slice size is approximately 400 LoC of palimpsest source plus its tests, plus 10–30 LoC changed in archinstall_zfs.

## Assumptions

- archinstall_zfs is reachable from the local filesystem at `~/code/archinstall_zfs/` and is willing to take a `path = "../palimpsest"` dependency during development.
- The local OpenZFS version is ≥ 2.2 (`zfs --version` output checked at the start of work; if not, capture fixtures from a machine that does have 2.2+).
- The exact JSON schema of `zfs list -j` is stable enough across OpenZFS 2.2 and 2.3 that one set of serde structs covers both. If the schema diverges, we accept that as a finding and add per-version handling.
- `tokio::process::Command::output().await` returns `std::process::Output`, not a tokio-specific variant. (Confirmed during scaffolding.)

## Open Questions

- Should `RecordingRunner` match calls by `(program, args)` (keyed) or by call order (sequenced)? Keyed is more robust to refactoring; sequenced is simpler for tests with deterministic call order. Probably keyed; revisit once a few real tests are written.
- Whether to expose property values as a typed enum (`ZfsPropertyValue::Bytes(u64) | Bool(bool) | …`) or as an untyped string + a typed accessor. JSON output gives us hints (numeric vs string), but real usage favors string-with-getter. Defer until a second slice (`dataset::get`) shows the consumer pattern.
