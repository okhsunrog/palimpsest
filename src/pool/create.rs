use std::path::PathBuf;

use crate::error::{ZfsError, classify_stderr};
use crate::names::PoolName;
use crate::runner::{Cmd, CommandRunner};

/// RAID-Z parity level. Encodes the wire syntax `raidz1` / `raidz2` / `raidz3`.
#[derive(Debug, Clone, Copy)]
pub enum RaidZLevel {
    One,
    Two,
    Three,
}

impl RaidZLevel {
    fn cli_token(self) -> &'static str {
        match self {
            Self::One => "raidz1",
            Self::Two => "raidz2",
            Self::Three => "raidz3",
        }
    }
}

/// One vdev (a top-level device-grouping in a pool). A pool is built from one
/// or more vdevs; with multiple vdevs the data is striped across them, while
/// each vdev's internal redundancy is determined by its variant.
#[derive(Debug, Clone)]
pub enum Vdev {
    /// Plain stripe (no redundancy). One device = single disk; multiple
    /// devices = striped across them.
    Stripe(Vec<PathBuf>),
    /// `mirror device device...` — N-way mirror.
    Mirror(Vec<PathBuf>),
    /// `raidz{1,2,3} device device...`.
    RaidZ(RaidZLevel, Vec<PathBuf>),
}

impl Vdev {
    fn append_args(&self, args: &mut Vec<String>) {
        match self {
            Vdev::Stripe(devs) => {
                for d in devs {
                    args.push(d.display().to_string());
                }
            }
            Vdev::Mirror(devs) => {
                args.push("mirror".into());
                for d in devs {
                    args.push(d.display().to_string());
                }
            }
            Vdev::RaidZ(level, devs) => {
                args.push(level.cli_token().into());
                for d in devs {
                    args.push(d.display().to_string());
                }
            }
        }
    }

    fn validate(&self) -> Result<(), ZfsError> {
        let (kind, minimum, actual) = match self {
            Vdev::Stripe(devices) => ("stripe", 1, devices.len()),
            Vdev::Mirror(devices) => ("mirror", 2, devices.len()),
            Vdev::RaidZ(RaidZLevel::One, devices) => ("raidz1", 2, devices.len()),
            Vdev::RaidZ(RaidZLevel::Two, devices) => ("raidz2", 3, devices.len()),
            Vdev::RaidZ(RaidZLevel::Three, devices) => ("raidz3", 4, devices.len()),
        };
        if actual < minimum {
            return Err(ZfsError::InvalidInput {
                message: format!("{kind} vdev requires at least {minimum} device(s), got {actual}"),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct PoolCreateOptions {
    pub name: String,
    /// `-f`: force creation even if devices appear in use.
    pub force: bool,
    /// `-o key=value` pool properties.
    pub pool_properties: Vec<(String, String)>,
    /// `-O key=value` filesystem properties applied to the pool's root dataset.
    pub fs_properties: Vec<(String, String)>,
    /// `-m <mountpoint>` for the root dataset. `Some("none")` is the typical
    /// "don't auto-mount" idiom; `Some("legacy")` for /etc/fstab management;
    /// `None` to omit the flag entirely (zfs picks the default).
    pub mountpoint: Option<String>,
    /// `-R <altroot>` — alternate root for the pool. Persisted only across
    /// the lifetime of the import, so this is the right knob for installers
    /// that want everything mounted under /mnt.
    pub altroot: Option<PathBuf>,
    /// Vdev list. At least one vdev is required (with at least one device).
    pub vdevs: Vec<Vdev>,
}

impl PoolCreateOptions {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            force: false,
            pool_properties: Vec::new(),
            fs_properties: Vec::new(),
            mountpoint: None,
            altroot: None,
            vdevs: Vec::new(),
        }
    }

    pub fn force(mut self) -> Self {
        self.force = true;
        self
    }

    pub fn pool_property(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.pool_properties.push((k.into(), v.into()));
        self
    }

    pub fn fs_property(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.fs_properties.push((k.into(), v.into()));
        self
    }

    pub fn mountpoint(mut self, mp: impl Into<String>) -> Self {
        self.mountpoint = Some(mp.into());
        self
    }

    pub fn altroot(mut self, p: impl Into<PathBuf>) -> Self {
        self.altroot = Some(p.into());
        self
    }

    pub fn vdev(mut self, v: Vdev) -> Self {
        self.vdevs.push(v);
        self
    }

    pub fn build_args(&self) -> Result<Vec<String>, ZfsError> {
        PoolName::parse_for_create(&self.name)?;
        if self.vdevs.is_empty() {
            return Err(ZfsError::InvalidInput {
                message: "PoolCreateOptions::vdevs must contain at least one vdev".to_string(),
            });
        }
        for vdev in &self.vdevs {
            vdev.validate()?;
        }
        let mut args: Vec<String> = vec!["create".into()];
        if self.force {
            args.push("-f".into());
        }
        for (k, v) in &self.pool_properties {
            args.push("-o".into());
            args.push(format!("{k}={v}"));
        }
        for (k, v) in &self.fs_properties {
            args.push("-O".into());
            args.push(format!("{k}={v}"));
        }
        if let Some(m) = &self.mountpoint {
            args.push("-m".into());
            args.push(m.clone());
        }
        if let Some(p) = &self.altroot {
            args.push("-R".into());
            args.push(p.display().to_string());
        }
        args.push(self.name.clone());
        for vdev in &self.vdevs {
            vdev.append_args(&mut args);
        }
        Ok(args)
    }
}

pub async fn create(runner: &dyn CommandRunner, opts: &PoolCreateOptions) -> Result<(), ZfsError> {
    let args = opts.build_args()?;
    let output = runner.run(Cmd::new("zpool").args(args)).await?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(classify_stderr(&stderr, output.status.code()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_args_minimal_single_disk() {
        let opts =
            PoolCreateOptions::new("tank").vdev(Vdev::Stripe(vec![PathBuf::from("/dev/sda1")]));
        assert_eq!(
            opts.build_args().unwrap(),
            vec!["create", "tank", "/dev/sda1"]
        );
    }

    #[test]
    fn build_args_rejects_invalid_pool_name() {
        let error = PoolCreateOptions::new("raidz2-test")
            .vdev(Vdev::Stripe(vec![PathBuf::from("/dev/sda")]))
            .build_args()
            .unwrap_err();
        assert!(matches!(error, ZfsError::InvalidName(_)));
    }

    #[test]
    fn build_args_archinstall_shape() {
        // Mirrors archinstall_zfs's DEFAULT_POOL_OPTIONS + altroot + a single disk.
        let opts = PoolCreateOptions::new("tank")
            .force()
            .pool_property("ashift", "12")
            .fs_property("acltype", "posixacl")
            .fs_property("compression", "lz4")
            .mountpoint("none")
            .altroot("/mnt")
            .vdev(Vdev::Stripe(vec![PathBuf::from(
                "/dev/disk/by-id/test-part2",
            )]));
        assert_eq!(
            opts.build_args().unwrap(),
            vec![
                "create",
                "-f",
                "-o",
                "ashift=12",
                "-O",
                "acltype=posixacl",
                "-O",
                "compression=lz4",
                "-m",
                "none",
                "-R",
                "/mnt",
                "tank",
                "/dev/disk/by-id/test-part2",
            ]
        );
    }

    #[test]
    fn build_args_mirror_pair() {
        let opts = PoolCreateOptions::new("tank").vdev(Vdev::Mirror(vec![
            PathBuf::from("/dev/sda"),
            PathBuf::from("/dev/sdb"),
        ]));
        assert_eq!(
            opts.build_args().unwrap(),
            vec!["create", "tank", "mirror", "/dev/sda", "/dev/sdb"]
        );
    }

    #[test]
    fn build_args_raidz2_quad() {
        let opts = PoolCreateOptions::new("tank").vdev(Vdev::RaidZ(
            RaidZLevel::Two,
            vec![
                PathBuf::from("/dev/sda"),
                PathBuf::from("/dev/sdb"),
                PathBuf::from("/dev/sdc"),
                PathBuf::from("/dev/sdd"),
            ],
        ));
        assert_eq!(
            opts.build_args().unwrap(),
            vec![
                "create", "tank", "raidz2", "/dev/sda", "/dev/sdb", "/dev/sdc", "/dev/sdd"
            ]
        );
    }

    #[test]
    fn build_args_stripe_of_two_mirrors() {
        let opts = PoolCreateOptions::new("tank")
            .vdev(Vdev::Mirror(vec![
                PathBuf::from("/dev/sda"),
                PathBuf::from("/dev/sdb"),
            ]))
            .vdev(Vdev::Mirror(vec![
                PathBuf::from("/dev/sdc"),
                PathBuf::from("/dev/sdd"),
            ]));
        assert_eq!(
            opts.build_args().unwrap(),
            vec![
                "create", "tank", "mirror", "/dev/sda", "/dev/sdb", "mirror", "/dev/sdc",
                "/dev/sdd",
            ]
        );
    }

    #[test]
    fn build_args_rejects_no_vdevs() {
        let opts = PoolCreateOptions::new("tank");
        let err = opts.build_args().expect_err("empty vdevs must error");
        let ZfsError::InvalidInput { message } = err else {
            panic!("expected InvalidInput");
        };
        assert!(message.contains("at least one vdev"));
    }

    #[test]
    fn build_args_rejects_empty_vdev() {
        let opts = PoolCreateOptions::new("tank").vdev(Vdev::Mirror(vec![]));
        assert!(opts.build_args().is_err());
    }
}
