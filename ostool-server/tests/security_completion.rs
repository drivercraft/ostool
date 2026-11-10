//! Regressions for uniform authentication, database artifacts and device credentials.
#[path = "support/database.rs"]
mod test_database;
use anyhow::Result;
use axum::{
    body::{Body, Bytes, to_bytes},
    http::{Request, StatusCode},
};
use ostool_server::{
    config::Edition, persistence::postgres::AccessGrant, tftp::service::build_tftp_manager, *,
};
use serde_json::{Value, json};
use std::time::Duration;
use tower::ServiceExt;
struct Fixture {
    state: AppState,
    app: axum::Router,
    member: AccessGrant,
    admin: AccessGrant,
    _root: tempfile::TempDir,
}
impl Fixture {
    async fn new() -> Result<Self> {
        let root = tempfile::tempdir()?;
        let mut config = ServerConfig {
            edition: Edition::Public,
            data_dir: root.path().join("data"),
            listen_addr: "127.0.0.1:0".parse()?,
            ..Default::default()
        };
        config.security.mfa_encryption_key =
            Some("AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=".into());
        config.tftp = TftpConfig::Builtin(BuiltinTftpConfig::default_with_root(
            root.path().join("cache"),
        ));
        let db = test_database::prepare(&root.path().join("server.env"), &mut config).await?;
        let admin = db
            .create_user(
                "admin",
                "admin@completion.test",
                "completion-password-123",
                None,
                true,
            )
            .await?;
        let member = db
            .create_user(
                "member",
                "member@completion.test",
                "completion-password-123",
                None,
                false,
            )
            .await?;
        let admin = db
            .issue_grant(admin, "admin-web", "board:operate", &config.auth)
            .await?;
        let member = db
            .issue_grant(member, "ostool-cli", "board:operate", &config.auth)
            .await?;
        let state = build_app_state(
            root.path().join("server.env"),
            config.clone(),
            build_tftp_manager(&config.tftp),
        )
        .await?;
        state.ensure_data_dirs().await?;
        Ok(Self {
            app: build_router(state.clone()),
            state,
            member,
            admin,
            _root: root,
        })
    }
    async fn call(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Value,
    ) -> Result<(StatusCode, Value)> {
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
            .oneshot(request.body(Body::from(body.to_string()))?)
            .await?;
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
        Ok((
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        ))
    }
    async fn board(&self, id: &str, http: bool, instant: bool) -> Result<()> {
        let value = json!({"id":id,"allow_instant_use":instant,"board_type":"completion","tags":[],"serial":null,"notes":null,"disabled":false,
   "boot":if http{json!({"kind":"httpboot"})}else{json!({"kind":"uboot","use_tftp":false})},
   "network_identity":if http{json!({"mac_address":"02:00:00:00:11:09"})}else{Value::Null},
   "power_management":{"kind":"custom","power_on_cmd":"true","power_off_cmd":"true"}});
        let (status, body) = self
            .call(
                "POST",
                "/api/v1/admin/boards",
                Some(&self.admin.access_token),
                value,
            )
            .await?;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        Ok(())
    }
    async fn session(&self, id: &str) -> Result<String> {
        sqlx::query("INSERT INTO board_rentals(board_id,user_id,starts_at,ends_at) VALUES($1,$2,now()-interval '1 minute',now()+interval '1 hour')").bind(id).bind(self.member.user.id).execute(self.state.database.pool()).await?;
        let (status, body) = self
            .call(
                "POST",
                "/api/v1/sessions",
                Some(&self.member.access_token),
                json!({"board_id":id,"board_type":"completion"}),
            )
            .await?;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        Ok(body["session_id"].as_str().unwrap().into())
    }
    async fn upload(&self, sid: &str, path: &str, bytes: Vec<u8>) -> Result<StatusCode> {
        Ok(self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/v1/sessions/{sid}/files"))
                    .header(
                        "authorization",
                        format!("Bearer {}", self.member.access_token),
                    )
                    .header("x-file-path", path)
                    .body(Body::from(bytes))?,
            )
            .await?
            .status())
    }
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn public_requires_authentication_while_local_keeps_anonymous_board_access() -> Result<()> {
    let f = Fixture::new().await?;
    for edition in [Edition::Public] {
        f.state.config.write().await.edition = edition;
        for path in [
            "/api/v1/admin/boards",
            "/api/v1/account",
            "/api/v1/board-types",
        ] {
            assert_eq!(
                f.call("GET", path, None, Value::Null).await?.0,
                StatusCode::UNAUTHORIZED
            );
        }
        assert_eq!(
            f.call(
                "GET",
                "/api/v1/admin/boards",
                Some(&f.member.access_token),
                Value::Null
            )
            .await?
            .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            f.call(
                "GET",
                "/api/v1/admin/boards",
                Some(&f.admin.access_token),
                Value::Null
            )
            .await?
            .0,
            StatusCode::OK
        );
    }
    f.state.config.write().await.edition = Edition::Local;
    assert_eq!(
        f.call("GET", "/api/v1/admin/boards", None, Value::Null)
            .await?
            .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call("GET", "/api/v1/board-types", None, Value::Null)
            .await?
            .0,
        StatusCode::OK
    );
    for path in [
        "/api/v1/account",
        "/api/v1/admin/users",
        "/api/v1/admin/rentals",
    ] {
        assert_eq!(
            f.call("GET", path, None, Value::Null).await?.0,
            StatusCode::NOT_FOUND
        );
    }
    f.state.config.write().await.edition = Edition::Public;
    let mut disabled = f.state.config.read().await.clone();
    disabled.auth.enabled = false;
    assert!(disabled.validate().is_err());
    if let TftpConfig::Builtin(config) = &mut disabled.tftp {
        config.enabled = true;
    }
    disabled.auth.enabled = true;
    assert!(disabled.validate().is_err());
    assert_eq!(
        f.call(
            "PUT",
            "/api/v1/admin/tftp",
            Some(&f.admin.access_token),
            serde_json::to_value(&disabled.tftp)?
        )
        .await?
        .0,
        StatusCode::BAD_REQUEST
    );
    assert!(!f.state.config.read().await.tftp.enabled());
    Ok(())
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn database_artifacts_survive_cache_loss_and_download_grants_are_file_scoped() -> Result<()> {
    let f = Fixture::new().await?;
    f.board("files", false, false).await?;
    let sid = f.session("files").await?;
    let data = vec![73; 3 * persistence::files::CHUNK_SIZE + 17];
    assert_eq!(
        f.upload(&sid, "dir/blob.bin", data.clone()).await?,
        StatusCode::CREATED
    );
    assert_eq!(
        f.upload(&sid, "second.bin", b"different".to_vec()).await?,
        StatusCode::CREATED
    );
    let cache = f.state.tftp_manager.read().await.root_dir().to_owned();
    tokio::fs::remove_dir_all(&cache).await?;
    let file = f
        .state
        .session_files
        .get(&sid, "dir/blob.bin")
        .await?
        .unwrap();
    assert_eq!(f.state.session_files.read(&file).await?, data);
    let path = format!("/share/sessions/{sid}/dir/blob.bin");
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&path)
                .header("authorization", format!("Bearer {}", f.member.access_token))
                .header("range", "bytes=262140-262155")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        to_bytes(response.into_body(), 1024).await?.as_ref(),
        &data[262140..262156]
    );
    let url = device_security::file_url(&f.state, &sid, "dir/blob.bin").await?;
    let route = format!("{}?{}", url.path(), url.query().unwrap());
    let response = f
        .app
        .clone()
        .oneshot(Request::builder().uri(&route).body(Body::empty())?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), data.len()).await?.as_ref(),
        data
    );
    assert_eq!(
        f.call("GET", url.path(), None, Value::Null).await?.0,
        StatusCode::UNAUTHORIZED
    );
    let second = f
        .state
        .session_files
        .get(&sid, "second.bin")
        .await?
        .unwrap();
    let wrong = format!("/device-files/{}?{}", second.id, url.query().unwrap());
    assert_eq!(
        f.call("GET", &wrong, None, Value::Null).await?.0,
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("UPDATE device_file_grants SET expires_at=now()-interval '1 second'")
        .execute(f.state.database.pool())
        .await?;
    assert_eq!(
        f.call("GET", &route, None, Value::Null).await?.0,
        StatusCode::UNAUTHORIZED
    );
    let old_url = device_security::file_url(&f.state, &sid, "second.bin").await?;
    assert_eq!(
        f.upload(&sid, "second.bin", b"replacement".to_vec())
            .await?,
        StatusCode::CREATED
    );
    assert!(f.state.session_files.get_by_id(second.id).await?.is_none());
    let old_route = format!("{}?{}", old_url.path(), old_url.query().unwrap());
    assert_eq!(
        f.call("GET", &old_route, None, Value::Null).await?.0,
        StatusCode::UNAUTHORIZED
    );
    let manager = f.state.tftp_manager.read().await.clone();
    let cached = manager
        .get_session_file(&sid, "dir/blob.bin")
        .await?
        .unwrap();
    assert_eq!(tokio::fs::read(cached.disk_path).await?, data);
    let fresh = device_security::file_url(&f.state, &sid, "dir/blob.bin").await?;
    f.state
        .request_session_stop(&sid, session::SessionStopReason::ApiDelete)
        .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while f.state.get_session(&sid).await.is_some() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    assert!(
        f.state
            .session_files
            .materialize(&file, &cache)
            .await
            .is_err()
    );
    assert!(!cache.join("ostool/sessions").join(&sid).exists());
    let expired = format!("{}?{}", fresh.path(), fresh.query().unwrap());
    assert_eq!(
        f.call("GET", &expired, None, Value::Null).await?.0,
        StatusCode::UNAUTHORIZED
    );
    Ok(())
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn kernel_upload_admission_and_timeout_cover_receiving_the_request_body() -> Result<()> {
    let f = Fixture::new().await?;
    f.board("kernel", true, false).await?;
    let sid = f.session("kernel").await?;
    {
        let mut config = f.state.config.write().await;
        config.upload_limits.max_concurrent_uploads = 1;
        config.upload_limits.user_max_concurrent_uploads = 1;
        config.upload_limits.upload_timeout_secs = 1;
    }
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let (mut send, _keep) = tokio::sync::oneshot::channel::<()>();
    let stream = futures_util::stream::once(async move {
        let _ = entered.send(());
        let _ = send.closed().await;
        Ok::<_, std::io::Error>(Bytes::from_static(b"pending"))
    });
    let request = |body: Body| {
        Request::builder()
            .method("PUT")
            .uri(format!("/api/v1/sessions/{sid}/http-boot/kernel"))
            .header("authorization", format!("Bearer {}", f.member.access_token))
            .header(
                httpboot_protocol::DEVICE_PROTOCOL_VERSION_HEADER,
                httpboot_protocol::DEVICE_PROTOCOL_VERSION.to_string(),
            )
            .body(body)
            .unwrap()
    };
    let first = tokio::spawn(f.app.clone().oneshot(request(Body::from_stream(stream))));
    waiting.await?;
    let second = f
        .app
        .clone()
        .oneshot(request(Body::from("another")))
        .await?;
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    let first = tokio::time::timeout(Duration::from_secs(3), first).await???;
    assert_eq!(first.status(), StatusCode::REQUEST_TIMEOUT);
    assert!(f.state.session_files.list(&sid).await?.is_empty());
    assert_eq!(
        f.upload(&sid, "after-timeout.bin", b"ok".to_vec()).await?,
        StatusCode::CREATED
    );
    Ok(())
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn front_end_claims_are_owned_limited_and_cannot_bypass_bookings() -> Result<()> {
    let f = Fixture::new().await?;
    f.board("shared-a", false, true).await?;
    f.board("shared-b", false, true).await?;
    let web = f
        .state
        .database
        .issue_grant(
            f.member.user.clone(),
            "web",
            "board:operate",
            &f.state.config.read().await.auth,
        )
        .await?;
    assert_eq!(
        f.call(
            "POST",
            "/api/v1/account/allocations",
            Some(&web.access_token),
            json!({"board_id":"shared-a"})
        )
        .await?
        .0,
        StatusCode::FORBIDDEN
    );
    sqlx::query("INSERT INTO roles(id,code,name,permissions,allocation_policy) VALUES('instant','instant','Instant',ARRAY['board.use','board.instant'],'{\"max_duration_minutes\":30,\"max_concurrent_sessions\":1,\"min_available_minutes\":5,\"board_types\":[]}'::jsonb)").execute(f.state.database.pool()).await?;
    sqlx::query("UPDATE users SET role_id='instant' WHERE id=$1")
        .bind(f.member.user.id)
        .execute(f.state.database.pool())
        .await?;
    let web = f
        .state
        .database
        .issue_grant(
            f.state
                .database
                .user_by_id(f.member.user.id)
                .await?
                .unwrap(),
            "web",
            "board:operate",
            &f.state.config.read().await.auth,
        )
        .await?;
    let (status, claim) = f
        .call(
            "POST",
            "/api/v1/account/allocations",
            Some(&web.access_token),
            json!({"board_id":"shared-a"}),
        )
        .await?;
    assert_eq!(status, StatusCode::CREATED, "{claim}");
    assert_eq!(
        f.call(
            "POST",
            "/api/v1/account/allocations",
            Some(&web.access_token),
            json!({"board_id":"shared-b"})
        )
        .await?
        .0,
        StatusCode::CONFLICT
    );
    let route = format!(
        "/api/v1/account/allocations/{}",
        claim["id"].as_str().unwrap()
    );
    let other = f
        .state
        .database
        .issue_grant(
            f.admin.user.clone(),
            "web",
            "board:operate",
            &f.state.config.read().await.auth,
        )
        .await?;
    assert_eq!(
        f.call("DELETE", &route, Some(&other.access_token), Value::Null)
            .await?
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        f.call("DELETE", &route, Some(&web.access_token), Value::Null)
            .await?
            .0,
        StatusCode::NO_CONTENT
    );
    sqlx::query("INSERT INTO board_rentals(board_id,user_id,starts_at,ends_at) VALUES('shared-b',$1,now()-interval '1 minute',now()+interval '1 hour')").bind(f.admin.user.id).execute(f.state.database.pool()).await?;
    assert_eq!(
        f.call(
            "POST",
            "/api/v1/account/allocations",
            Some(&web.access_token),
            json!({"board_id":"shared-b"})
        )
        .await?
        .0,
        StatusCode::CONFLICT
    );
    Ok(())
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis, Node.js"]
async fn device_callbacks_require_provisioned_tls_and_never_follow_udp_callback_addresses()
-> Result<()> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let f = Fixture::new().await?;
    f.board("secure-device", true, false).await?;
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/device-tls");
    let token = "a-separate-device-token-with-at-least-32-characters";
    let mut child = tokio::process::Command::new("node")
        .arg(fixture.join("server.mjs"))
        .arg(fixture.join("cert.pem"))
        .arg(fixture.join("key.pem"))
        .arg(token)
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut line = String::new();
    tokio::time::timeout(
        Duration::from_secs(5),
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line),
    )
    .await??;
    let port: u16 = line.trim().parse()?;
    let announcement = httpboot_protocol::LoaderAnnouncement {
        protocol_version: httpboot_protocol::DEVICE_PROTOCOL_VERSION,
        mac_address: "02:00:00:00:11:09".parse()?,
        current_mac_address: "02:00:00:00:11:09".parse()?,
        arch: "x86_64".into(),
        loader_version: "secure-test".into(),
        boot_epoch: "secure-epoch".into(),
        http_port: 1,
        serial_id: None,
        serial_ready: false,
    };
    assert!(
        device::reconcile(
            f.state.clone(),
            announcement.clone(),
            "127.0.0.1:12345".parse()?
        )
        .await
        .is_err()
    );
    let path = "/api/v1/admin/boards/secure-device/device-connection";
    assert_eq!(
        f.call(
            "PUT",
            path,
            Some(&f.admin.access_token),
            json!({"endpoint":format!("http://127.0.0.1:{port}"),"token":token})
        )
        .await?
        .0,
        StatusCode::BAD_REQUEST
    );
    let ca = tokio::fs::read_to_string(fixture.join("ca.pem")).await?;
    let (status, view) = f
        .call(
            "PUT",
            path,
            Some(&f.admin.access_token),
            json!({"endpoint":format!("https://127.0.0.1:{port}"),"token":token,"ca_pem":ca}),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert!(!view.to_string().contains(token));
    let ciphertext: String = sqlx::query_scalar(
        "SELECT token_ciphertext FROM device_connections WHERE board_id='secure-device'",
    )
    .fetch_one(f.state.database.pool())
    .await?;
    assert!(ciphertext.starts_with("v1:"));
    assert!(!ciphertext.contains(token));
    assert_eq!(
        f.call("GET", path, Some(&f.member.access_token), Value::Null)
            .await?
            .0,
        StatusCode::FORBIDDEN
    );
    device::reconcile(
        f.state.clone(),
        announcement.clone(),
        "127.0.0.1:12345".parse()?,
    )
    .await?;
    assert_eq!(f.call("PUT",path,Some(&f.admin.access_token),json!({"endpoint":format!("https://127.0.0.1:{port}"),"token":"wrong-credential-with-more-than-32-characters","ca_pem":ca})).await?.0,StatusCode::OK);
    assert!(
        device::reconcile(f.state.clone(), announcement, "127.0.0.1:12345".parse()?)
            .await
            .is_err()
    );
    child.kill().await?;
    for oversized in [false, true] {
        let forbidden = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let mode = if oversized {
            "oversized".to_string()
        } else {
            format!("redirect:{}", forbidden.local_addr()?.port())
        };
        let mut server = tokio::process::Command::new("node")
            .arg(fixture.join("server.mjs"))
            .arg(fixture.join("cert.pem"))
            .arg(fixture.join("key.pem"))
            .arg(token)
            .arg(mode)
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let mut port = String::new();
        tokio::time::timeout(
            Duration::from_secs(5),
            BufReader::new(server.stdout.take().unwrap()).read_line(&mut port),
        )
        .await??;
        let endpoint = format!("https://127.0.0.1:{}", port.trim());
        assert_eq!(
            f.call(
                "PUT",
                path,
                Some(&f.admin.access_token),
                json!({"endpoint":endpoint,"token":token,"ca_pem":ca})
            )
            .await?
            .0,
            StatusCode::OK
        );
        let error = device::reconcile_provisioned(f.state.clone(), "secure-device")
            .await
            .unwrap_err();
        if oversized {
            assert!(format!("{error:#}").contains("too large"));
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(100), forbidden.accept())
                .await
                .is_err()
        );
        server.kill().await?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn database_publication_rechecks_revocation_and_cancellation_after_staging_waits()
-> Result<()> {
    for revoke in [true, false] {
        let f = Fixture::new().await?;
        f.board("commit-race", false, false).await?;
        let sid = f.session("commit-race").await?;
        assert_eq!(
            f.upload(&sid, "protected.bin", b"original contents".to_vec())
                .await?,
            StatusCode::CREATED
        );
        let mut lock = f.state.database.pool().begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended(current_schema()||':ostool-session-artifacts',1109))").execute(&mut *lock).await?;
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *lock)
            .await?;
        let request = Request::builder()
            .method("PUT")
            .uri(format!("/api/v1/sessions/{sid}/files"))
            .header("authorization", format!("Bearer {}", f.member.access_token))
            .header("x-file-path", "protected.bin")
            .body(Body::from("replacement that must not be published"))?;
        let upload = tokio::spawn(f.app.clone().oneshot(request));
        tokio::time::timeout(Duration::from_secs(5),async {
            loop {
                let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_locks h JOIN pg_locks w ON h.locktype=w.locktype AND h.database=w.database AND h.classid=w.classid AND h.objid=w.objid AND h.objsubid=w.objsubid WHERE h.pid=$1 AND h.locktype='advisory' AND h.granted AND NOT w.granted)").bind(pid).fetch_one(f.state.database.pool()).await?;
                if waiting {break;}
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Ok::<_,anyhow::Error>(())
        }).await??;
        if revoke {
            sqlx::query("UPDATE oauth_sessions SET revoked_at=now() WHERE user_id=$1")
                .bind(f.member.user.id)
                .execute(f.state.database.pool())
                .await?;
        } else {
            f.state
                .request_session_stop(&sid, session::SessionStopReason::ApiDelete)
                .await;
        }
        lock.commit().await?;
        let response = tokio::time::timeout(Duration::from_secs(5), upload).await???;
        assert_eq!(
            response.status(),
            if revoke {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::CONFLICT
            }
        );
        if revoke {
            let file = f
                .state
                .session_files
                .get(&sid, "protected.bin")
                .await?
                .unwrap();
            assert_eq!(
                f.state.session_files.read(&file).await?,
                b"original contents"
            );
        } else {
            tokio::time::timeout(Duration::from_secs(5), async {
                while f.state.get_session(&sid).await.is_some() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await?;
            assert!(f.state.session_files.list(&sid).await?.is_empty());
        }
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL, Redis and Node.js"]
async fn removing_device_credentials_cancels_a_pending_authenticated_callback() -> Result<()> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let f = Fixture::new().await?;
    f.board("revoked-device", true, false).await?;
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/device-tls");
    let token = "independent-device-credential-for-revocation";
    let mut child = tokio::process::Command::new("node")
        .arg(fixture.join("server.mjs"))
        .arg(fixture.join("cert.pem"))
        .arg(fixture.join("key.pem"))
        .arg(token)
        .arg("pause")
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut port = String::new();
    tokio::time::timeout(Duration::from_secs(5), output.read_line(&mut port)).await??;
    let ca = tokio::fs::read_to_string(fixture.join("ca.pem")).await?;
    let path = "/api/v1/admin/boards/revoked-device/device-connection";
    assert_eq!(f.call("PUT",path,Some(&f.admin.access_token),json!({"endpoint":format!("https://127.0.0.1:{}",port.trim()),"token":token,"ca_pem":ca})).await?.0,StatusCode::OK);
    let controller = device::start_authenticated_devices(f.state.clone());
    let mut authenticated = String::new();
    tokio::time::timeout(Duration::from_secs(5), output.read_line(&mut authenticated)).await??;
    assert_eq!(authenticated.trim(), "authenticated");
    assert_eq!(
        f.call("DELETE", path, Some(&f.admin.access_token), Value::Null)
            .await?
            .0,
        StatusCode::NO_CONTENT
    );
    let mut closed = String::new();
    tokio::time::timeout(Duration::from_secs(3), output.read_line(&mut closed)).await??;
    assert_eq!(closed.trim(), "closed");
    controller.abort();
    child.kill().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn upload_deadline_includes_database_publication_and_releases_admission_slots() -> Result<()>
{
    let f = Fixture::new().await?;
    f.board("deadline", false, false).await?;
    let sid = f.session("deadline").await?;
    assert_eq!(
        f.upload(&sid, "atomic.bin", b"original".to_vec()).await?,
        StatusCode::CREATED
    );
    {
        let mut config = f.state.config.write().await;
        config.upload_limits.upload_timeout_secs = 2;
        config.upload_limits.max_concurrent_uploads = 1;
    }
    let mut lock = f.state.database.pool().begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended(current_schema()||':ostool-session-artifacts',1109))").execute(&mut *lock).await?;
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *lock)
        .await?;
    let request = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/sessions/{sid}/files"))
        .header("authorization", format!("Bearer {}", f.member.access_token))
        .header("x-file-path", "atomic.bin")
        .body(Body::from("already received body"))?;
    let upload = tokio::spawn(f.app.clone().oneshot(request));
    tokio::time::timeout(Duration::from_secs(3),async {loop {
        let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_locks h JOIN pg_locks w ON h.locktype=w.locktype AND h.database=w.database AND h.classid=w.classid AND h.objid=w.objid AND h.objsubid=w.objsubid WHERE h.pid=$1 AND h.locktype='advisory' AND h.granted AND NOT w.granted)").bind(pid).fetch_one(f.state.database.pool()).await?;
        if waiting{break;}tokio::time::sleep(Duration::from_millis(10)).await;
    }Ok::<_,anyhow::Error>(())}).await??;
    let response = tokio::time::timeout(Duration::from_secs(4), upload).await???;
    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    lock.commit().await?;
    let file = f
        .state
        .session_files
        .get(&sid, "atomic.bin")
        .await?
        .unwrap();
    assert_eq!(f.state.session_files.read(&file).await?, b"original");
    assert_eq!(
        f.upload(&sid, "after-deadline.bin", b"ok".to_vec()).await?,
        StatusCode::CREATED
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL, Redis and Node.js"]
async fn cancelled_begin_does_not_poison_pool_or_block_subsequent_login() -> Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let f = Fixture::new().await?;
    let mut config = f.state.config.read().await.database.clone();
    let mut child = tokio::process::Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/postgres-begin-proxy.mjs"
        ))
        .arg(config.url.as_ref().unwrap())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
    let port = tokio::time::timeout(Duration::from_secs(5), output.next_line())
        .await??
        .unwrap()
        .parse()?;
    let mut url = url::Url::parse(config.url.as_ref().unwrap())?;
    url.set_host(Some("127.0.0.1"))?;
    url.set_port(Some(port)).unwrap();
    url.query_pairs_mut().append_pair("sslmode", "disable");
    config.url = Some(url.to_string());
    config.max_connections = 1;
    config.migrate = false;
    let db = persistence::Database::connect(&config).await?;
    let mut input = child.stdin.take().unwrap();
    // Repeat on the same single pooled connection to cover connection reuse.
    for iteration in 0..3 {
        input.write_all(b"arm\n").await?;
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), output.next_line())
                .await??
                .as_deref(),
            Some("armed")
        );
        let caller_db = db.clone();
        let caller = tokio::spawn(async move { caller_db.begin().await });
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), output.next_line())
                .await??
                .as_deref(),
            Some("ready-held")
        );
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        input.write_all(b"resume\n").await?;
        let display_name = format!("after-cancel-{iteration}");
        // This autocommit write would remain uncommitted in a leaked transaction,
        // holding the same users row lock that issue_grant needs during login.
        tokio::time::timeout(
            Duration::from_secs(3),
            sqlx::query("UPDATE users SET display_name=$1 WHERE id=$2")
                .bind(&display_name)
                .bind(f.member.user.id)
                .execute(db.pool()),
        )
        .await??;
        let user = f
            .state
            .database
            .find_user(&f.member.user.email)
            .await?
            .unwrap();
        assert_eq!(user.display_name.as_deref(), Some(display_name.as_str()));
        tokio::time::timeout(
            Duration::from_secs(3),
            f.state.database.issue_grant(
                user,
                "ostool-cli",
                "board:operate",
                &f.state.config.read().await.auth,
            ),
        )
        .await??;
    }
    db.pool().close().await;
    child.kill().await?;
    Ok(())
}
