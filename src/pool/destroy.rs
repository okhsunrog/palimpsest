use crate::error::{ZfsError, classify_stderr};
use crate::runner::{Cmd, CommandRunner};

#[derive(Default, Clone, Debug)]
pub struct DestroyOptions {
    /// `-f`: force destroy even if pool is in use / mounted.
    pub force: bool,
}

impl DestroyOptions {
    pub fn build_args(&self, pool: &str) -> Vec<String> {
        let mut args: Vec<String> = vec!["destroy".into()];
        if self.force {
            args.push("-f".into());
        }
        args.push(pool.into());
        args
    }
}

pub async fn destroy(
    runner: &dyn CommandRunner,
    pool: &str,
    opts: &DestroyOptions,
) -> Result<(), ZfsError> {
    let output = runner
        .run(Cmd::new("zpool").args(opts.build_args(pool)))
        .await?;
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
            DestroyOptions::default().build_args("tank"),
            vec!["destroy", "tank"]
        );
    }

    #[test]
    fn build_args_force() {
        let opts = DestroyOptions { force: true };
        assert_eq!(opts.build_args("tank"), vec!["destroy", "-f", "tank"]);
    }
}
