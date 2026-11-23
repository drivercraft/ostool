//! DTB enable/disable contracts against an isolated PostgreSQL schema.
#[path = "support/database.rs"]
mod test_database;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use ostool_server::{
    AppState, BoardConfig, BootConfig, BuiltinTftpConfig, CustomPowerManagement,
    PowerManagementConfig, ServerConfig, TftpConfig, build_app_state, build_router,
    config::{Edition, UbootProfile},
    tftp::service::build_tftp_manager,
};
use serde_json::{Value, json};
use tower::ServiceExt;
struct Fixture {
    state: AppState,
    app: Router,
    _root: tempfile::TempDir,
}
impl Fixture {
    async fn new(public: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut config = ServerConfig {
            edition: if public {
                Edition::Public
            } else {
                Edition::Local
            },
            data_dir: root.path().join("data"),
            tftp: TftpConfig::Builtin(BuiltinTftpConfig {
                enabled: false,
                ..BuiltinTftpConfig::default_with_root(root.path().join("tftp"))
            }),
            ..Default::default()
        };
        let path = root.path().join("config.toml");
        test_database::prepare(&path, &mut config).await.unwrap();
        let state = build_app_state(path, config.clone(), build_tftp_manager(&config.tftp))
            .await
            .unwrap();
        state
            .dtb_store
            .write("board.dtb", b"dtb bytes")
            .await
            .unwrap();
        let board = board();
        state.board_store.write_board(&board).await.unwrap();
        state.boards.write().await.insert(board.id.clone(), board);
        state.sync_board_runtime_states().await;
        Self {
            app: if public {
                build_router(state.clone())
            } else {
                test_database::authenticated_router(&state).await.unwrap()
            },
            state,
            _root: root,
        }
    }
    async fn call(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Value,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let response = self
            .app
            .clone()
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let data = to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap();
        (status, serde_json::from_slice(&data).unwrap_or(Value::Null))
    }
    async fn create(&self, token: Option<&str>) -> String {
        let (status, data) = self
            .call(
                "POST",
                "/api/v1/sessions",
                token,
                json!({"board_type":"history-fixture","client_name":"fixture_%_client"}),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{data}");
        data["session_id"].as_str().unwrap().to_owned()
    }
}
fn board() -> BoardConfig {
    BoardConfig {
        allow_instant_use: false,
        reset_command: None,
        id: "history-board".into(),
        board_type: "history-fixture".into(),
        tags: vec![],
        serial: None,
        notes: None,
        disabled: false,
        network_identity: None,
        boot: BootConfig::Uboot(UbootProfile {
            dtb_name: Some("board.dtb".into()),
            ..Default::default()
        }),
        power_management: PowerManagementConfig::Custom(CustomPowerManagement {
            power_on_cmd: "true".into(),
            power_off_cmd: "true".into(),
        }),
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn disabled_dtbs_cannot_be_selected_allocated_or_read_and_status_survives_replacement() {
    let f = Fixture::new(false).await;
    let path = "/api/v1/admin/dtbs/board.dtb";
    let (status, file) = f.call("PATCH", path, None, json!({"disabled":true})).await;
    assert_eq!(status, StatusCode::OK, "{file}");
    assert_eq!(file["disabled"], true);
    assert!(f.state.dtb_store.read("board.dtb").await.is_err());
    assert!(
        f.state
            .create_session("history-fixture", &[], None)
            .await
            .is_err()
    );
    let (status, _) = f.call("POST", "/api/v1/admin/boards", None, json!({"id":"new-board","board_type":"test","disabled":true,"boot":{"kind":"uboot","dtb_name":"board.dtb"},"power_management":{"kind":"custom","power_on_cmd":"true","power_off_cmd":"true"}})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    f.state
        .dtb_store
        .write("board.dtb", b"replacement")
        .await
        .unwrap();
    assert!(
        f.state
            .dtb_store
            .get("board.dtb")
            .await
            .unwrap()
            .unwrap()
            .disabled
    );
    let renamed = f
        .state
        .dtb_store
        .rename("board.dtb", "renamed.dtb")
        .await
        .unwrap();
    assert!(renamed.disabled);
    assert_eq!(
        f.call(
            "PATCH",
            "/api/v1/admin/dtbs/renamed.dtb",
            None,
            json!({"disabled":false})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        f.state.dtb_store.read("renamed.dtb").await.unwrap(),
        b"replacement"
    );
    assert_eq!(
        f.call(
            "PATCH",
            "/api/v1/admin/dtbs/missing.dtb",
            None,
            json!({"disabled":true})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        f.call(
            "PATCH",
            "/api/v1/admin/dtbs/renamed.dtb",
            None,
            json!({"disabled":"yes"})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn active_sessions_protect_dtbs_and_cached_downloads_still_check_status() {
    let f = Fixture::new(false).await;
    let id = f.create(None).await;
    let path = format!("/api/v1/sessions/{id}/dtb");
    assert_eq!(
        f.call("GET", &path, None, Value::Null).await.0,
        StatusCode::OK
    );
    assert_eq!(
        f.call(
            "PATCH",
            "/api/v1/admin/dtbs/board.dtb",
            None,
            json!({"disabled":true})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert!(
        !f.state
            .dtb_store
            .get("board.dtb")
            .await
            .unwrap()
            .unwrap()
            .disabled
    );
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/admin/dtbs/board.dtb")
                .header("X-Dtb-Name", "active-renamed.dtb")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    // Simulate an out-of-band status change after the session cached a DTB.
    f.state
        .dtb_store
        .set_disabled("board.dtb", true)
        .await
        .unwrap();
    assert_eq!(
        f.call("GET", &path, None, Value::Null).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        f.call("GET", &format!("{path}/download"), None, Value::Null)
            .await
            .0,
        StatusCode::CONFLICT
    );
    f.state
        .dtb_store
        .set_disabled("board.dtb", false)
        .await
        .unwrap();
    assert_eq!(
        f.call("GET", &path, None, Value::Null).await.0,
        StatusCode::OK
    );
    f.call(
        "DELETE",
        &format!("/api/v1/admin/sessions/{id}?force=true"),
        None,
        Value::Null,
    )
    .await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn dtb_status_changes_require_management_permission() {
    let f = Fixture::new(true).await;
    let path = "/api/v1/admin/dtbs/board.dtb";
    assert_eq!(
        f.call("PATCH", path, None, json!({"disabled":true}))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let manager = f
        .state
        .database
        .create_user(
            "dtb_admin",
            "dtb-admin@test.local",
            "dtb-admin-password",
            None,
            true,
        )
        .await
        .unwrap();
    let manager = f
        .state
        .database
        .issue_grant(
            manager,
            "admin-web",
            "board:operate",
            &f.state.config.read().await.auth,
        )
        .await
        .unwrap();
    assert_eq!(f.call("POST", "/api/v1/admin/roles", Some(&manager.access_token), json!({"code":"dtb_reader","name":"DTB reader","enabled":true,"description":"","permissions":["dtbs.read"]})).await.0, StatusCode::CREATED);
    let role: String = sqlx::query_scalar("SELECT id FROM roles WHERE code='dtb_reader'")
        .fetch_one(f.state.database.pool())
        .await
        .unwrap();
    let reader = f
        .state
        .database
        .create_user(
            "dtb_reader",
            "dtb-reader@test.local",
            "dtb-reader-password",
            None,
            false,
        )
        .await
        .unwrap();
    sqlx::query("UPDATE users SET role_id=$1 WHERE id=$2")
        .bind(role)
        .bind(reader.id)
        .execute(f.state.database.pool())
        .await
        .unwrap();
    let reader = f
        .state
        .database
        .find_user("dtb-reader@test.local")
        .await
        .unwrap()
        .unwrap();
    let reader = f
        .state
        .database
        .issue_grant(
            reader,
            "admin-web",
            "board:operate",
            &f.state.config.read().await.auth,
        )
        .await
        .unwrap();
    assert_eq!(
        f.call("GET", path, Some(&reader.access_token), Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call(
            "PATCH",
            path,
            Some(&reader.access_token),
            json!({"disabled":true})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert!(
        !f.state
            .dtb_store
            .get("board.dtb")
            .await
            .unwrap()
            .unwrap()
            .disabled
    );
}
