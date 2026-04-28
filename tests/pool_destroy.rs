use palimpsest::pool::{DestroyOptions, destroy};
use palimpsest::{Cmd, RecordingRunner};

#[tokio::test]
async fn destroy_default_succeeds() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zpool").args(["destroy", "tank"]),
        vec![],
        vec![],
        0,
    );
    destroy(&runner, "tank", &DestroyOptions::default())
        .await
        .expect("destroy succeeds");
}

#[tokio::test]
async fn destroy_force_passes_dash_f() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zpool").args(["destroy", "-f", "tank"]),
        vec![],
        vec![],
        0,
    );
    let opts = DestroyOptions { force: true };
    destroy(&runner, "tank", &opts)
        .await
        .expect("destroy -f succeeds");
}
