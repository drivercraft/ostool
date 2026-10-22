//! Real PostgreSQL hardware-lease lifecycle; every fixture uses a disposable schema.
#[path = "support/database.rs"]
mod test_database;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{Duration, Utc};
use ostool_server::{
    AppState, BoardConfig, BootConfig, BuiltinTftpConfig, CustomPowerManagement,
    PowerManagementConfig, PxeProfile, ServerConfig, TftpConfig, build_app_state, build_router,
    config::Edition, persistence::leases::LeaseFilters, session::Session,
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
        let board = board();
        state.board_store.write_board(&board).await.unwrap();
        state.boards.write().await.insert(board.id.clone(), board);
        state.sync_board_runtime_states().await;
        Self {
            app: build_router(state.clone()),
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
    async fn wait_ended(&self, id: &str) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while self.state.get_session(id).await.is_some() {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }
}
fn board() -> BoardConfig {
    BoardConfig {
        id: "history-board".into(),
        board_type: "history-fixture".into(),
        tags: vec![],
        serial: None,
        notes: None,
        disabled: false,
        network_identity: None,
        boot: BootConfig::Pxe(PxeProfile::default()),
        power_management: PowerManagementConfig::Custom(CustomPowerManagement {
            power_on_cmd: "true".into(),
            power_off_cmd: "true".into(),
        }),
    }
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn local_lease_lifecycle_persists_filters_and_survives_restart() {
    let f = Fixture::new(false).await;
    let id = f.create(None).await;
    let first = f
        .state
        .database
        .lease_page(&LeaseFilters::default())
        .await
        .unwrap();
    assert_eq!(first.total, 1);
    assert_eq!(first.items[0].session_id, id);
    assert!(first.items[0].ended_at.is_none());
    let heartbeat = f.state.heartbeat_session(&id).await.unwrap();
    let active = f
        .state
        .database
        .lease_page(&LeaseFilters::default())
        .await
        .unwrap();
    assert_eq!(
        active.items[0].last_heartbeat_at.timestamp_millis(),
        heartbeat.last_heartbeat_at.timestamp_millis()
    );
    // Serial transport refreshes in-memory leases frequently; the watchdog
    // checkpoints those updates without putting database I/O on each frame.
    let serial_heartbeat = f.state.session_state(&id).await.unwrap().heartbeat().await;
    f.state.cleanup_expired_sessions().await.unwrap();
    let checkpoint = f
        .state
        .database
        .lease_page(&LeaseFilters::default())
        .await
        .unwrap();
    assert_eq!(
        checkpoint.items[0].last_heartbeat_at.timestamp_millis(),
        serial_heartbeat.last_heartbeat_at.timestamp_millis()
    );
    let (status, _) = f
        .call(
            "DELETE",
            &format!("/api/v1/admin/sessions/{id}"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    f.wait_ended(&id).await;
    // Duplicate finalization must not rewrite the original reason/timestamp.
    f.state
        .database
        .finish_lease(&id, "client_released")
        .await
        .unwrap();
    let (status, page) = f
        .call(
            "GET",
            "/api/v1/admin/session-history?state=ended&q=%25_&page_size=1",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["total"], 1);
    assert_eq!(page["items"][0]["end_reason"], "admin_released");
    assert!(page["items"][0]["ended_at"].is_string());
    assert_eq!(page["items"][0]["release_failures"], 0);
    assert_eq!(
        f.call(
            "GET",
            "/api/v1/admin/session-history?page_size=101",
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        f.call(
            "GET",
            "/api/v1/admin/session-history?state=unknown",
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let config = f.state.config.read().await.clone();
    let restarted = build_app_state(
        (*f.state.config_path).clone(),
        config.clone(),
        build_tftp_manager(&config.tftp),
    )
    .await
    .unwrap();
    let saved = restarted
        .database
        .lease_page(&LeaseFilters::default())
        .await
        .unwrap();
    assert_eq!(saved.items[0].end_reason.as_deref(), Some("admin_released"));
    assert_eq!(saved.total, 1);
    restarted
        .board_store
        .delete_board("history-board")
        .await
        .unwrap();
    assert_eq!(
        restarted
            .database
            .lease_page(&LeaseFilters::default())
            .await
            .unwrap()
            .total,
        1
    );
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn rental_identity_snapshots_survive_user_deletion_and_member_cannot_read_history() {
    let f = Fixture::new(true).await;
    let db = &f.state.database;
    let user = db
        .create_user(
            "lease_member",
            "lease@example.test",
            "lease-fixture-password-123",
            None,
            false,
        )
        .await
        .unwrap();
    let auth = f.state.config.read().await.auth.clone();
    let grant = db
        .issue_grant(user.clone(), "ostool", "board:operate", &auth)
        .await
        .unwrap();
    let rental:uuid::Uuid=sqlx::query_scalar("INSERT INTO board_rentals(board_id,user_id,starts_at,ends_at) VALUES($1,$2,now()-interval '1 hour',now()+interval '1 hour') RETURNING id").bind("history-board").bind(user.id).fetch_one(db.pool()).await.unwrap();
    assert_eq!(
        f.call("GET", "/api/v1/admin/session-history", None, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.call(
            "GET",
            "/api/v1/admin/session-history",
            Some(&grant.access_token),
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let id = f.create(Some(&grant.access_token)).await;
    let record = db
        .lease_page(&LeaseFilters::default())
        .await
        .unwrap()
        .items
        .remove(0);
    assert_eq!(record.username.as_deref(), Some("lease_member"));
    assert_eq!(record.user_id, Some(user.id));
    assert_eq!(record.rental_id, Some(rental));
    sqlx::query("UPDATE users SET username='renamed_member' WHERE id=$1")
        .bind(user.id)
        .execute(db.pool())
        .await
        .unwrap();
    // Delete user/rental authorization, then run the same deadline enforcement as production.
    sqlx::query("DELETE FROM users WHERE id=$1")
        .bind(user.id)
        .execute(db.pool())
        .await
        .unwrap();
    ostool_server::api::rentals::enforce(&f.state)
        .await
        .unwrap();
    f.wait_ended(&id).await;
    let record = db
        .lease_page(&LeaseFilters::default())
        .await
        .unwrap()
        .items
        .remove(0);
    assert_eq!(record.username.as_deref(), Some("lease_member"));
    assert_eq!(record.user_id, Some(user.id));
    assert_eq!(record.rental_id, Some(rental));
    assert_eq!(record.end_reason.as_deref(), Some("rental_revoked"));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM session_owners")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        0
    );
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn interrupted_leases_are_not_restored_and_shutdown_and_expiry_have_distinct_reasons() {
    let f = Fixture::new(false).await;
    let db = &f.state.database;
    let orphan = Session::new("history-board".into(), Some("crashed-client".into()));
    db.start_lease(&orphan, &board()).await.unwrap();
    db.stopping_lease(&orphan.id, "client_released")
        .await
        .unwrap();
    db.lease_release_failed(&orphan.id, "fixture cleanup error")
        .await
        .unwrap();
    let config = f.state.config.read().await.clone();
    let restarted = build_app_state(
        (*f.state.config_path).clone(),
        config.clone(),
        build_tftp_manager(&config.tftp),
    )
    .await
    .unwrap();
    assert!(restarted.sessions.read().await.is_empty());
    let record = db
        .lease_page(&LeaseFilters::default())
        .await
        .unwrap()
        .items
        .remove(0);
    assert_eq!(record.end_reason.as_deref(), Some("server_interrupted"));
    assert_eq!(record.stop_reason.as_deref(), Some("client_released"));
    assert_eq!(record.release_failures, 1);
    assert_eq!(
        record.last_release_error.as_deref(),
        Some("fixture cleanup error")
    );
    assert_eq!(
        record.last_heartbeat_at.timestamp_millis(),
        orphan.last_heartbeat_at.timestamp_millis()
    );
    let session = restarted
        .create_session("history-fixture", &[], None)
        .await
        .unwrap();
    restarted
        .session_state(&session.id)
        .await
        .unwrap()
        .bind_rental_deadline(Utc::now() - Duration::seconds(1))
        .await;
    restarted.cleanup_expired_sessions().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while restarted.get_session(&session.id).await.is_some() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let next = restarted
        .create_session("history-fixture", &[], None)
        .await
        .unwrap();
    restarted.shutdown_sessions().await.unwrap();
    let records = db
        .lease_page(&LeaseFilters {
            page_size: Some(2),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(records.total, 3);
    assert_eq!(records.items.len(), 2);
    assert_eq!(records.items[0].session_id, next.id);
    assert_eq!(
        records.items[0].end_reason.as_deref(),
        Some("server_shutdown")
    );
    assert_eq!(
        records.items[1].end_reason.as_deref(),
        Some("lease_expired")
    );
    let page = db
        .lease_page(&LeaseFilters {
            page: Some(i64::MAX),
            page_size: Some(2),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(page.page, 2);
    assert_eq!(page.items.len(), 1);
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn failed_history_write_cannot_allocate_hardware() {
    let f = Fixture::new(false).await;
    sqlx::query("DROP TABLE session_history")
        .execute(f.state.database.pool())
        .await
        .unwrap();
    let (status, _) = f
        .call(
            "POST",
            "/api/v1/sessions",
            None,
            json!({"board_type":"history-fixture"}),
        )
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(f.state.sessions.read().await.is_empty());
    assert!(
        f.state
            .board_runtime_status("history-board")
            .await
            .unwrap()
            .active_session_id
            .is_none()
    );
}
