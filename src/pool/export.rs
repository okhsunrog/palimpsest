use crate::error::{ZfsError, classify_stderr};
use crate::runner::CommandRunner;

#[derive(Default, Clone, Debug)]
pub struct ExportOptions {
    // -f: forcefully unmount filesystems before exporting
    pub force: bool,
}

impl ExportOptions {
    pub fn build_args(&self, pool: &str) -> Vec<String> {
        let mut args: Vec<String> = vec!["export".into()];
        if self.force {
            args.push("-f".into());
        }
        args.push(pool.to_string());
        args
    }
}

pub async fn export(
    runner: &dyn CommandRunner,
    pool: &str,
    opts: &ExportOptions,
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
            ExportOptions::default().build_args("tank"),
            vec!["export", "tank"]
        );
    }

    #[test]
    fn build_args_force() {
        let opts = ExportOptions { force: true };
        assert_eq!(opts.build_args("tank"), vec!["export", "-f", "tank"]);
    }
}
