//! Sibling to the CLI wrappers (`dataset/`, `pool/`, ...) for information
//! ZFS exposes outside of `zfs(8)` / `zpool(8)`. Mostly kstat files under
//! `/proc/spl/kstat/zfs/`.
//!
//! Module is filesystem-IO based (not a CLI wrapper, not FFI). It sits in
//! zfskit because consumers (`arctern`'s admin UI, future archinstall
//! diagnostics) all want the same parser and the same typed surface.

pub mod arc;

pub use arc::{ArcStats, arc_stats, parse_arcstats};
