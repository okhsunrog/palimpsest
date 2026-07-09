use zfskit::pool::{ExportOptions, ImportOptions, export, import};
use zfskit::{Cmd, RecordingRunner, ZfsError};

#[tokio::test]
async fn import_returns_ok_on_success() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zpool").args(["import", "-f", "-N", "tank"]),
        vec![],
        vec![],
        0,
    );
    let opts = ImportOptions {
        force: true,
        no_mount: true,
        ..Default::default()
    };
    import(&runner, "tank", &opts)
        .await
        .expect("import succeeds");
}

#[tokio::test]
async fn import_classifies_pool_not_found() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zpool").args(["import", "tank"]),
        vec![],
        b"cannot import 'tank': no such pool available\n".to_vec(),
        1,
    );
    let err = import(&runner, "tank", &ImportOptions::default())
        .await
        .expect_err("import should fail");
    let ZfsError::PoolNotFound { name } = err else {
        panic!("expected PoolNotFound, got {err:?}");
    };
    assert_eq!(name, "tank");
}

#[tokio::test]
async fn export_returns_ok_on_success() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zpool").args(["export", "tank"]),
        vec![],
        vec![],
        0,
    );
    export(&runner, "tank", &ExportOptions::default())
        .await
        .expect("export succeeds");
}

#[tokio::test]
async fn export_force_passes_dash_f() {
    let runner = RecordingRunner::new().record(
        Cmd::new("zpool").args(["export", "-f", "tank"]),
        vec![],
        vec![],
        0,
    );
    let opts = ExportOptions { force: true };
    export(&runner, "tank", &opts)
        .await
        .expect("export -f succeeds");
}
