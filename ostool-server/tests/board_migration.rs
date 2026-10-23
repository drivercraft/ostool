//! The deployed migration executable is tested against real PostgreSQL.
#[path = "support/database.rs"]
mod test_database;
use ostool_server::{
    BoardConfig, BootConfig, CustomPowerManagement, PowerManagementConfig, PxeProfile,
    ServerConfig, UbootProfile, persistence::Database,
};
use serde_json::Value;
use std::process::{Command, Output};
struct Fixture {
    root: tempfile::TempDir,
    database: Database,
    config: ServerConfig,
}
impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut config = ServerConfig {
            data_dir: root.path().join("data"),
            ..Default::default()
        };
        let database = test_database::prepare(&root.path().join("server.toml"), &mut config)
            .await
            .unwrap();
        std::fs::create_dir(root.path().join("boards")).unwrap();
        std::fs::create_dir(root.path().join("dtbs")).unwrap();
        std::fs::write(root.path().join("empty.env"), "").unwrap();
        Self {
            root,
            database,
            config,
        }
    }
    fn source(&self, board: &BoardConfig) {
        std::fs::write(
            self.root
                .path()
                .join("boards")
                .join(format!("{}.toml", board.id)),
            toml::to_string_pretty(board).unwrap(),
        )
        .unwrap();
    }
    async fn run(&self, dry: bool) -> Output {
        let path = self.root.path().to_owned();
        let url = self.config.database.url.clone().unwrap();
        let schema = self.config.database.schema.clone().unwrap();
        tokio::task::spawn_blocking(move || {
            let mut command = Command::new(env!("CARGO_BIN_EXE_ostool-server-migrate-boards"));
            command
                .args([
                    "--board-dir",
                    path.join("boards").to_str().unwrap(),
                    "--dtb-dir",
                    path.join("dtbs").to_str().unwrap(),
                    "--env-file",
                    path.join("empty.env").to_str().unwrap(),
                ])
                .env("OSTOOL_DATABASE_URL", url)
                .env("OSTOOL_DATABASE_SCHEMA", schema)
                .env("OSTOOL_EDITION", "local");
            if dry {
                command.arg("--dry-run");
            }
            command.output().unwrap()
        })
        .await
        .unwrap()
    }
    async fn count(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM board_records")
            .fetch_one(self.database.pool())
            .await
            .unwrap()
    }
}
fn board(id: &str) -> BoardConfig {
    BoardConfig {
        reset_command: None,
        id: id.into(),
        board_type: "migration-test".into(),
        tags: vec!["source".into()],
        notes: Some("original inventory".into()),
        serial: None,
        disabled: false,
        network_identity: None,
        boot: BootConfig::Pxe(PxeProfile::default()),
        power_management: PowerManagementConfig::Custom(CustomPowerManagement {
            power_on_cmd: "true".into(),
            power_off_cmd: "true".into(),
        }),
    }
}
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[tokio::test]
#[ignore = "requires OSTOOL_TEST_DATABASE_URL and disposable PostgreSQL"]
async fn imports_boards_and_referenced_dtbs_once_without_changing_sources() {
    let f = Fixture::new().await;
    let mut first = board("board-a");
    first.network_identity = Some(ostool_server::BoardNetworkIdentity {
        mac_address: "02:00:00:00:00:01".parse().unwrap(),
    });
    first.boot = BootConfig::Uboot(UbootProfile {
        use_tftp: false,
        dtb_name: Some("board.dtb".into()),
        kernel_load_addr: None,
        fit_load_addr: None,
        bootm_addr: None,
        network_mode: Default::default(),
        board_ip: None,
        server_ip: None,
        netmask: None,
        gatewayip: None,
    });
    f.source(&first);
    let mut second = board("board-b");
    second.network_identity = Some(ostool_server::BoardNetworkIdentity {
        mac_address: "02:00:00:00:00:02".parse().unwrap(),
    });
    second.boot = BootConfig::UefiHttp(ostool_server::UefiHttpProfile {
        boot_arch: Some(ostool_server::config::UefiBootArch::X86_64),
        serial_parameters: None,
    });
    f.source(&second);
    std::fs::write(
        f.root.path().join("dtbs/board.dtb"),
        b"original-dtb-content",
    )
    .unwrap();
    std::fs::write(f.root.path().join("dtbs/unreferenced.dtb"), b"not-imported").unwrap();
    let source = std::fs::read(f.root.path().join("boards/board-a.toml")).unwrap();
    let dry = f.run(true).await;
    success(&dry);
    assert_eq!(f.count().await, 0);
    let audit_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_events WHERE action='board.import'")
            .fetch_one(f.database.pool())
            .await
            .unwrap();
    assert_eq!(audit_count, 0);
    assert!(String::from_utf8_lossy(&dry.stdout).contains("Would import: 2 boards, 1 DTBs"));
    success(&f.run(false).await);
    assert_eq!(f.count().await, 2);
    let record: Value = sqlx::query_scalar("SELECT payload FROM board_records WHERE id='board-a'")
        .fetch_one(f.database.pool())
        .await
        .unwrap();
    assert_eq!(record, serde_json::to_value(&first).unwrap());
    let content: Vec<u8> =
        sqlx::query_scalar("SELECT content FROM dtb_records WHERE name='board.dtb'")
            .fetch_one(f.database.pool())
            .await
            .unwrap();
    assert_eq!(content, b"original-dtb-content");
    let dtb_count: i64 = sqlx::query_scalar("SELECT count(*) FROM dtb_records")
        .fetch_one(f.database.pool())
        .await
        .unwrap();
    assert_eq!(dtb_count, 1);
    let repeat = f.run(false).await;
    success(&repeat);
    assert_eq!(f.count().await, 2);
    let audit_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_events WHERE action='board.import'")
            .fetch_one(f.database.pool())
            .await
            .unwrap();
    assert_eq!(audit_count, 1);
    assert!(
        String::from_utf8_lossy(&repeat.stdout)
            .contains("Identical records skipped: 2 boards, 1 DTBs")
    );
    assert_eq!(
        std::fs::read(f.root.path().join("boards/board-a.toml")).unwrap(),
        source
    );
    assert_eq!(
        std::fs::read(f.root.path().join("dtbs/board.dtb")).unwrap(),
        content
    );
    // Deleting the import sources cannot affect the running repository.
    std::fs::remove_dir_all(f.root.path().join("boards")).unwrap();
    assert_eq!(
        ostool_server::persistence::BoardStore::new(f.database.clone())
            .load_all()
            .await
            .unwrap()
            .len(),
        2
    );
}
#[tokio::test]
#[ignore = "requires OSTOOL_TEST_DATABASE_URL and disposable PostgreSQL"]
async fn conflicts_and_invalid_input_never_partially_import() {
    let f = Fixture::new().await;
    let initial = board("board-z");
    ostool_server::persistence::BoardStore::new(f.database.clone())
        .write_board(&initial)
        .await
        .unwrap();
    f.source(&board("board-a"));
    let mut changed = initial.clone();
    changed.notes = Some("do not overwrite".into());
    f.source(&changed);
    let result = f.run(false).await;
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("different contents"));
    assert_eq!(f.count().await, 1);
    std::fs::remove_file(f.root.path().join("boards/board-z.toml")).unwrap();
    std::fs::write(
        f.root.path().join("boards/bad.toml"),
        "id='broken'\nboard_type='invalid'",
    )
    .unwrap();
    assert!(!f.run(false).await.status.success());
    assert_eq!(f.count().await, 1);
}
#[tokio::test]
#[ignore = "requires OSTOOL_TEST_DATABASE_URL and disposable PostgreSQL"]
async fn duplicate_macs_and_missing_referenced_dtb_are_rejected() {
    let f = Fixture::new().await;
    let mut a = board("board-a");
    a.network_identity = Some(ostool_server::BoardNetworkIdentity {
        mac_address: "02:00:00:00:00:42".parse().unwrap(),
    });
    let mut b = a.clone();
    b.id = "board-b".into();
    f.source(&a);
    f.source(&b);
    let result = f.run(false).await;
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("duplicate source MAC"));
    assert_eq!(f.count().await, 0);
    std::fs::remove_file(f.root.path().join("boards/board-b.toml")).unwrap();
    ostool_server::persistence::BoardStore::new(f.database.clone())
        .write_board(&BoardConfig {
            reset_command: None,
            id: "existing".into(),
            ..a.clone()
        })
        .await
        .unwrap();
    let result = f.run(false).await;
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("already assigned"));
    assert_eq!(f.count().await, 1);
    a.network_identity = None;
    a.boot = BootConfig::Uboot(UbootProfile {
        use_tftp: false,
        dtb_name: Some("missing.dtb".into()),
        kernel_load_addr: None,
        fit_load_addr: None,
        bootm_addr: None,
        network_mode: Default::default(),
        board_ip: None,
        server_ip: None,
        netmask: None,
        gatewayip: None,
    });
    f.source(&a);
    assert!(!f.run(false).await.status.success());
    assert_eq!(f.count().await, 1);
}
#[tokio::test]
#[ignore = "requires OSTOOL_TEST_DATABASE_URL and disposable PostgreSQL"]
async fn file_id_mismatch_and_dtb_conflict_preserve_database() {
    let f = Fixture::new().await;
    std::fs::write(
        f.root.path().join("boards/wrong.toml"),
        toml::to_string(&board("different")).unwrap(),
    )
    .unwrap();
    assert!(!f.run(false).await.status.success());
    assert_eq!(f.count().await, 0);
    std::fs::remove_file(f.root.path().join("boards/wrong.toml")).unwrap();
    let mut a = board("board-a");
    a.boot = BootConfig::Uboot(UbootProfile {
        use_tftp: false,
        dtb_name: Some("board.dtb".into()),
        kernel_load_addr: None,
        fit_load_addr: None,
        bootm_addr: None,
        network_mode: Default::default(),
        board_ip: None,
        server_ip: None,
        netmask: None,
        gatewayip: None,
    });
    f.source(&a);
    std::fs::write(f.root.path().join("dtbs/board.dtb"), b"source").unwrap();
    ostool_server::DtbStore::new(f.database.clone())
        .write("board.dtb", b"database")
        .await
        .unwrap();
    assert!(!f.run(false).await.status.success());
    assert_eq!(f.count().await, 0);
    assert_eq!(
        ostool_server::DtbStore::new(f.database.clone())
            .read("board.dtb")
            .await
            .unwrap(),
        b"database"
    );
}

#[tokio::test]
#[ignore = "requires OSTOOL_TEST_DATABASE_URL and disposable PostgreSQL"]
async fn normal_import_initializes_empty_schema_without_starting_server() {
    let f = Fixture::new().await;
    f.source(&board("board-a"));
    let schema = f.config.database.schema.as_deref().unwrap();
    // A closed/empty target is initialized by the CLI itself, never by the runtime.
    sqlx::query(&format!("DROP SCHEMA \"{schema}\" CASCADE"))
        .execute(f.database.pool())
        .await
        .unwrap();
    let check = f.run(true).await;
    assert!(!check.status.success());
    assert!(String::from_utf8_lossy(&check.stderr).contains("requires the target schema"));
    success(&f.run(false).await);
    assert_eq!(f.count().await, 1);
    assert_eq!(
        ostool_server::persistence::BoardStore::new(f.database.clone())
            .load_all()
            .await
            .unwrap()["board-a"]
            .board_type,
        "migration-test"
    );
}

#[tokio::test]
#[ignore = "requires OSTOOL_TEST_DATABASE_URL and disposable PostgreSQL"]
async fn database_write_failure_rolls_back_the_entire_import() {
    let f = Fixture::new().await;
    f.source(&board("board-a"));
    f.source(&board("board-z"));
    sqlx::query(
        "ALTER TABLE board_records ADD CONSTRAINT reject_test_board CHECK(id <> 'board-z')",
    )
    .execute(f.database.pool())
    .await
    .unwrap();
    let result = f.run(false).await;
    assert!(!result.status.success());
    assert_eq!(f.count().await, 0);
    let audit_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_events WHERE action='board.import'")
            .fetch_one(f.database.pool())
            .await
            .unwrap();
    assert_eq!(audit_count, 0);
}
