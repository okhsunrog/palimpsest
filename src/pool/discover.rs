//! Parser for `zpool import` discovery output (no `-j` flag in OpenZFS 2.4.x).
//!
//! Wire shape, paraphrased from `zpool-import(8)`:
//!
//! ```text
//!    pool: <name>
//!      id: <numeric guid>
//!   state: <ONLINE | DEGRADED | FAULTED | UNAVAIL>
//!  action: <free text, may span lines>
//!  status: <free text, only present for non-ONLINE pools>
//!  config:
//!
//!     <vdev tree>
//!
//!    pool: <next pool>
//!     ...
//! ```
//!
//! When no pools are importable, ZFS prints `no pools available to import`
//! (often to stderr, with a non-zero exit code). Our `discover()` combines
//! stdout+stderr because OpenZFS isn't always consistent about which stream
//! the pool list lands on, mirroring archinstall_zfs's prior behavior.
use crate::error::ZfsError;
use crate::runner::{Cmd, CommandRunner};

/// One importable pool surfaced by `zpool import`. Carries enough to render
/// in a picker UI; richer detail (config tree, full action text) is dropped
/// because it's free-form and not load-bearing for any current consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredPool {
    pub name: String,
    pub id: String,
    pub state: String,
    /// First line of the `status:` field, when present. Absent for ONLINE pools.
    pub status: Option<String>,
}

/// Pure parser: takes the combined stdout+stderr of `zpool import` (no args)
/// and returns the importable pools. Invalid or empty input yields an empty
/// Vec — never errors, since the "no pools available to import" case is
/// indistinguishable from the parser standpoint.
pub fn parse_discovery(text: &str) -> Vec<DiscoveredPool> {
    let mut pools = Vec::new();
    let mut current: Option<DiscoveredPool> = None;

    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("pool:") {
            if let Some(p) = current.take() {
                pools.push(p);
            }
            current = Some(DiscoveredPool {
                name: rest.trim().to_string(),
                id: String::new(),
                state: String::new(),
                status: None,
            });
            continue;
        }
        let Some(p) = current.as_mut() else {
            continue;
        };
        if let Some(rest) = trimmed.strip_prefix("id:") {
            p.id = rest.trim().to_string();
        } else if let Some(rest) = trimmed.strip_prefix("state:") {
            p.state = rest.trim().to_string();
        } else if let Some(rest) = trimmed.strip_prefix("status:") {
            // Capture only the first line; status text often wraps onto
            // continuation lines that aren't keyed and are therefore noise.
            p.status = Some(rest.trim().to_string());
        }
    }
    if let Some(p) = current.take() {
        pools.push(p);
    }
    pools.retain(|p| !p.name.is_empty());
    pools
}

/// `zpool import` with no arguments — list pools available for import on the
/// system. Returns an empty Vec when no pools are visible (including ZFS's
/// non-zero "no pools available to import" exit). True execution failures
/// (failing to spawn `zpool`) propagate as `ZfsError::Spawn`.
pub async fn discover(runner: &dyn CommandRunner) -> Result<Vec<DiscoveredPool>, ZfsError> {
    let output = runner.run(Cmd::new("zpool").arg("import")).await?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    Ok(parse_discovery(&combined))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_input_yields_empty_vec() {
        assert!(parse_discovery("").is_empty());
        assert!(parse_discovery("no pools available to import\n").is_empty());
    }

    #[test]
    fn parse_single_online_pool() {
        let text = "   pool: tank\n     id: 9316430153991325696\n  state: ONLINE\n";
        let pools = parse_discovery(text);
        assert_eq!(pools.len(), 1);
        assert_eq!(pools[0].name, "tank");
        assert_eq!(pools[0].id, "9316430153991325696");
        assert_eq!(pools[0].state, "ONLINE");
        assert!(pools[0].status.is_none());
    }

    #[test]
    fn parse_multiple_pools_separated_by_blank_lines() {
        let text = "\
   pool: a
     id: 1
  state: ONLINE

   pool: b
     id: 2
  state: ONLINE
";
        let pools = parse_discovery(text);
        assert_eq!(pools.len(), 2);
        assert_eq!(pools[0].name, "a");
        assert_eq!(pools[1].name, "b");
    }

    #[test]
    fn parse_captures_status_for_degraded() {
        let text = "\
   pool: dpool
     id: 42
  state: DEGRADED
 status: One or more devices could not be opened.
 action: Attach the missing device.
";
        let pools = parse_discovery(text);
        assert_eq!(pools.len(), 1);
        assert_eq!(pools[0].state, "DEGRADED");
        assert_eq!(
            pools[0].status.as_deref(),
            Some("One or more devices could not be opened.")
        );
    }

    #[test]
    fn parse_ignores_lines_before_first_pool() {
        let text = "some preamble text\n   pool: tank\n  state: ONLINE\n";
        let pools = parse_discovery(text);
        assert_eq!(pools.len(), 1);
        assert_eq!(pools[0].name, "tank");
    }

    #[test]
    fn parse_ignores_unknown_keys() {
        let text = "\
   pool: tank
     id: 1
  state: ONLINE
 cachefile: /etc/zfs/zpool.cache
 comment: my pool
 config:
\ttank ONLINE
";
        let pools = parse_discovery(text);
        assert_eq!(pools.len(), 1);
        assert_eq!(pools[0].state, "ONLINE");
    }
}
