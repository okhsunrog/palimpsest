//! ARC stats from `/proc/spl/kstat/zfs/arcstats`.
//!
//! File format:
//!
//! ```text
//! 23 1 0x01 147 39984 ... <kstat header — ignored>
//! name                            type data
//! hits                            4    241133333
//! misses                          4    4168191
//! ...
//! ```
//!
//! Every stat is a u64 in practice (`type 4` is `KSTAT_DATA_UINT64`); we
//! don't carry the type tag forward. `ArcStats` exposes the fields most
//! callers want as typed `u64` plus a `raw: BTreeMap<String, u64>` so
//! anything we didn't surface is still reachable without re-parsing.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ArcStatsError {
    #[error("read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("malformed arcstats header (need ≥2 lines, got {got})")]
    Header { got: usize },
}

/// Selected high-signal fields plus the full raw map. All values are
/// u64 (the kstat type 4 used throughout arcstats). Bytes for size /
/// capacity fields; counts for hits / misses.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArcStats {
    /// Current ARC size in bytes.
    pub size: u64,
    /// ARC target size in bytes (the tuning knob; size approaches c
    /// under memory pressure).
    pub c: u64,
    /// Hard minimum on c.
    pub c_min: u64,
    /// Hard maximum on c.
    pub c_max: u64,

    /// Cumulative hit count since boot.
    pub hits: u64,
    /// Cumulative miss count since boot.
    pub misses: u64,

    pub demand_data_hits: u64,
    pub demand_data_misses: u64,
    pub demand_metadata_hits: u64,
    pub demand_metadata_misses: u64,
    pub prefetch_data_hits: u64,
    pub prefetch_data_misses: u64,
    pub prefetch_metadata_hits: u64,
    pub prefetch_metadata_misses: u64,

    pub mru_hits: u64,
    pub mfu_hits: u64,
    pub mru_ghost_hits: u64,
    pub mfu_ghost_hits: u64,

    /// L2ARC size in bytes (0 when no L2 device).
    pub l2_size: u64,
    pub l2_hits: u64,
    pub l2_misses: u64,

    /// Compressed and uncompressed bytes in the ARC (when compression is
    /// in use); ratio = uncompressed_size / compressed_size.
    pub compressed_size: u64,
    pub uncompressed_size: u64,

    /// Every parsed name → value pair, including the typed fields above.
    /// Lets callers reach less-common stats without re-parsing the file.
    pub raw: BTreeMap<String, u64>,
}

impl ArcStats {
    /// Hit ratio over all accesses. NaN when no traffic has been observed
    /// yet — callers typically render NaN as `—`.
    pub fn hit_ratio(&self) -> f64 {
        if self.hits == 0 && self.misses == 0 {
            f64::NAN
        } else {
            self.hits as f64 / (self.hits as f64 + self.misses as f64)
        }
    }
}

/// Read + parse `/proc/spl/kstat/zfs/arcstats`. Returns `Err` only on
/// filesystem failure or a malformed header — individual non-numeric
/// rows are silently skipped (the kernel could add new stat types in
/// the future; we don't want a future kernel update to break us).
pub fn arc_stats() -> Result<ArcStats, ArcStatsError> {
    let path = "/proc/spl/kstat/zfs/arcstats";
    let content = std::fs::read_to_string(path).map_err(|e| ArcStatsError::Read {
        path: path.to_string(),
        source: e,
    })?;
    parse_arcstats(&content)
}

/// Same as [`arc_stats`] but reads from an arbitrary file. Used for the
/// fixture tests; archinstall ramdisk environments could also point this
/// at a snapshotted file.
pub fn arc_stats_from_path(path: impl AsRef<Path>) -> Result<ArcStats, ArcStatsError> {
    let p = path.as_ref();
    let content = std::fs::read_to_string(p).map_err(|e| ArcStatsError::Read {
        path: p.display().to_string(),
        source: e,
    })?;
    parse_arcstats(&content)
}

/// Pure parser for arcstats text. Splits whitespace, takes name +
/// last column, ignores unparseable rows.
pub fn parse_arcstats(content: &str) -> Result<ArcStats, ArcStatsError> {
    let mut lines = content.lines();
    let _header = lines.next();
    let _columns = lines.next();
    if _header.is_none() || _columns.is_none() {
        let got = if _header.is_some() { 1 } else { 0 };
        return Err(ArcStatsError::Header { got });
    }

    let mut raw: BTreeMap<String, u64> = BTreeMap::new();
    for line in lines {
        let mut parts = line.split_whitespace();
        let Some(name) = parts.next() else { continue };
        let Some(_ty) = parts.next() else { continue };
        let Some(value) = parts.next() else { continue };
        let Ok(v) = value.parse::<u64>() else {
            continue;
        };
        raw.insert(name.to_string(), v);
    }

    let g = |k: &str| raw.get(k).copied().unwrap_or(0);
    Ok(ArcStats {
        size: g("size"),
        c: g("c"),
        c_min: g("c_min"),
        c_max: g("c_max"),
        hits: g("hits"),
        misses: g("misses"),
        demand_data_hits: g("demand_data_hits"),
        demand_data_misses: g("demand_data_misses"),
        demand_metadata_hits: g("demand_metadata_hits"),
        demand_metadata_misses: g("demand_metadata_misses"),
        prefetch_data_hits: g("prefetch_data_hits"),
        prefetch_data_misses: g("prefetch_data_misses"),
        prefetch_metadata_hits: g("prefetch_metadata_hits"),
        prefetch_metadata_misses: g("prefetch_metadata_misses"),
        mru_hits: g("mru_hits"),
        mfu_hits: g("mfu_hits"),
        mru_ghost_hits: g("mru_ghost_hits"),
        mfu_ghost_hits: g("mfu_ghost_hits"),
        l2_size: g("l2_size"),
        l2_hits: g("l2_hits"),
        l2_misses: g("l2_misses"),
        compressed_size: g("compressed_size"),
        uncompressed_size: g("uncompressed_size"),
        raw,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> String {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/arcstats.txt");
        std::fs::read_to_string(path).expect("fixture present")
    }

    #[test]
    fn parses_typed_fields_from_real_kernel_output() {
        let s = parse_arcstats(&fixture()).unwrap();
        assert_eq!(s.hits, 241_139_387);
        assert_eq!(s.misses, 4_168_210);
        assert_eq!(s.demand_data_hits, 67_407_277);
        assert_eq!(s.mru_hits, 36_691_253);
        // Sanity: c_max >= c, both > 0.
        assert!(s.c_max >= s.c);
        assert!(s.size > 0);
    }

    #[test]
    fn raw_map_captures_fields_not_in_struct() {
        let s = parse_arcstats(&fixture()).unwrap();
        // `deleted` and `evict_skip` aren't in the typed surface but
        // should be reachable through raw.
        assert!(s.raw.contains_key("deleted"));
        assert!(s.raw.contains_key("evict_skip"));
    }

    #[test]
    fn hit_ratio_is_sensible() {
        let s = parse_arcstats(&fixture()).unwrap();
        let r = s.hit_ratio();
        assert!(r > 0.95, "ratio = {r} (expected high for a warm cache)");
        assert!(r <= 1.0);
    }

    #[test]
    fn empty_input_errors() {
        let err = parse_arcstats("").unwrap_err();
        assert!(matches!(err, ArcStatsError::Header { got: 0 }));
    }

    #[test]
    fn one_line_input_errors() {
        let err = parse_arcstats("23 1 0x01 ...\n").unwrap_err();
        assert!(matches!(err, ArcStatsError::Header { got: 1 }));
    }

    #[test]
    fn unparseable_rows_are_skipped_not_fatal() {
        // A future kernel could add a row whose last column isn't a u64
        // (a string, say). We don't want to crash on that — just skip.
        let input =
            "header\nname type data\nhits 4 12345\nweird_string_stat 7 abcdef\nmisses 4 678\n";
        let s = parse_arcstats(input).unwrap();
        assert_eq!(s.hits, 12345);
        assert_eq!(s.misses, 678);
        assert!(!s.raw.contains_key("weird_string_stat"));
    }
}
