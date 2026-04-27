use std::path::PathBuf;

use crate::error::{ZfsError, classify_stderr};
use crate::runner::CommandRunner;

#[derive(Default, Clone, Debug)]
pub struct ImportOptions {
    // -f: force import even if pool appears to be in use by another system
    pub force: bool,
    // -N: do not mount any filesystems after import
    pub no_mount: bool,
    // -R <path>: alternate root directory; useful for ephemeral inspection
    pub altroot: Option<PathBuf>,
}

impl ImportOptions {
    pub fn build_args(&self, pool: &str) -> Vec<String> {
        let mut args: Vec<String> = vec!["import".into()];
        if self.force {
            args.push("-f".into());
        }
        if self.no_mount {
            args.push("-N".into());
        }
        if let Some(p) = &self.altroot {
            args.push("-R".into());
            args.push(p.display().to_string());
        }
        args.push(pool.to_string());
        args
    }
}

pub async fn import(
    runner: &dyn CommandRunner,
    pool: &str,
    opts: &ImportOptions,
) -> Result<(), ZfsError> {
    let args = opts.build_args(pool);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = runner.run("zpool", &arg_refs).await?;
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
    fn build_args_default() {
        assert_eq!(
            ImportOptions::default().build_args("tank"),
            vec!["import", "tank"]
        );
    }

    #[test]
    fn build_args_force_no_mount() {
        let opts = ImportOptions {
            force: true,
            no_mount: true,
            ..Default::default()
        };
        assert_eq!(opts.build_args("tank"), vec!["import", "-f", "-N", "tank"]);
    }

    #[test]
    fn build_args_with_altroot() {
        let opts = ImportOptions {
            force: true,
            altroot: Some(PathBuf::from("/mnt/recovery")),
            ..Default::default()
        };
        assert_eq!(
            opts.build_args("tank"),
            vec!["import", "-f", "-R", "/mnt/recovery", "tank"]
        );
    }
}
