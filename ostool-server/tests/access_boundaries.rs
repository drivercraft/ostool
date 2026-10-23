//! Public device/file boundaries, using disposable storage and harmless power commands.
#[path = "support/database.rs"]
mod test_database;
use axum::{
    body::{Body, Bytes, to_bytes},
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use ostool_server::{
    config::Edition,
    persistence::{BoardStore, postgres::AccessGrant},
    tftp::service::{BuiltinTftpManager, TftpManager, build_tftp_manager},
    *,
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;

struct Fixture {
    state: AppState,
    app: axum::Router,
    grant: AccessGrant,
    _root: tempfile::TempDir,
}
impl Fixture {
    async fn new() -> anyhow::Result<Self> {
        let root = tempfile::tempdir()?;
        let mut config = ServerConfig {
            edition: Edition::Public,
            listen_addr: "127.0.0.1:0".parse()?,
            data_dir: root.path().join("data"),
            tftp: TftpConfig::Builtin(BuiltinTftpConfig {
                enabled: false,
                ..BuiltinTftpConfig::default_with_root(root.path().join("tftp"))
            }),
            ..Default::default()
        };
        let db = test_database::prepare(&root.path().join("test.env"), &mut config).await?;
        let user = db
            .create_user(
                "upload_user",
                "uploads@test.local",
                "upload-test-password-123",
                None,
                false,
            )
            .await?;
        let grant = db
            .issue_grant(
                user.clone(),
                "ostool-cli",
                "board:operate offline_access",
                &config.auth,
            )
            .await?;
        for id in ["board-one", "board-two"] {
            let board = BoardConfig {
                reset_command: None,
                id: id.into(),
                board_type: "upload-test".into(),
                tags: vec![],
                serial: None,
                boot: BootConfig::Uboot(UbootProfile::default()),
                power_management: PowerManagementConfig::Custom(CustomPowerManagement {
                    power_on_cmd: "true".into(),
                    power_off_cmd: "true".into(),
                }),
                notes: None,
                network_identity: None,
                disabled: false,
            };
            BoardStore::new(db.clone()).write_board(&board).await?;
            sqlx::query("INSERT INTO board_rentals(board_id,user_id,starts_at,ends_at) VALUES($1,$2,now()-interval '1 minute',now()+interval '1 hour')").bind(id).bind(user.id).execute(db.pool()).await?;
        }
        let state = build_app_state(
            root.path().join("test.env"),
            config.clone(),
            build_tftp_manager(&config.tftp),
        )
        .await?;
        state.ensure_data_dirs().await?;
        Ok(Self {
            app: build_router(state.clone()),
            state,
            grant,
            _root: root,
        })
    }
    async fn acquire(&self, id: &str) -> anyhow::Result<String> {
        let response = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/sessions")
                    .header(
                        "authorization",
                        format!("Bearer {}", self.grant.access_token),
                    )
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"board_type":"upload-test","board_id":id}).to_string(),
                    ))?,
            )
            .await?;
        let status = response.status();
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        Ok(body["session_id"].as_str().unwrap().into())
    }
    fn upload(&self, sid: &str, path: &str, body: Body) -> Request<Body> {
        Request::builder()
            .method("PUT")
            .uri(format!("/api/v1/sessions/{sid}/files"))
            .header(
                "authorization",
                format!("Bearer {}", self.grant.access_token),
            )
            .header("x-file-path", path)
            .body(body)
            .unwrap()
    }
    async fn put(&self, sid: &str, path: &str, bytes: Vec<u8>) -> anyhow::Result<StatusCode> {
        Ok(self
            .app
            .clone()
            .oneshot(self.upload(sid, path, Body::from(bytes)))
            .await?
            .status())
    }
    async fn paused(
        &self,
        sid: &str,
    ) -> anyhow::Result<(
        tokio::sync::mpsc::Sender<Result<Bytes, std::io::Error>>,
        tokio::task::JoinHandle<Result<axum::response::Response, std::convert::Infallible>>,
    )> {
        let (sender, receiver) = tokio::sync::mpsc::channel(2);
        sender.send(Ok(Bytes::from_static(b"first"))).await?;
        let started = Arc::new(tokio::sync::Notify::new());
        let stream_start = started.clone();
        let stream = futures_util::stream::unfold((receiver, true), move |(mut rx, first)| {
            let started = stream_start.clone();
            async move {
                if first {
                    started.notify_one();
                }
                rx.recv().await.map(|v| (v, (rx, false)))
            }
        });
        let task = tokio::spawn(self.app.clone().oneshot(self.upload(
            sid,
            "paused.bin",
            Body::from_stream(stream),
        )));
        tokio::time::timeout(Duration::from_secs(5), started.notified()).await?;
        Ok((sender, task))
    }
    async fn incoming_empty(&self) -> anyhow::Result<bool> {
        let root = self
            .state
            .tftp_manager
            .read()
            .await
            .root_dir()
            .join("ostool/.incoming");
        Ok(tokio::fs::read_dir(root)
            .await?
            .next_entry()
            .await?
            .is_none())
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn release_cancels_inflight_upload_without_recreating_files() -> anyhow::Result<()> {
    let f = Fixture::new().await?;
    let sid = f.acquire("board-one").await?;
    let (sender, upload) = f.paused(&sid).await?;
    f.state
        .request_session_stop(&sid, session::SessionStopReason::ApiDelete)
        .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while f.state.get_session(&sid).await.is_some() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    drop(sender);
    assert_eq!(upload.await??.status(), StatusCode::CONFLICT);
    assert!(
        f.state
            .tftp_manager
            .read()
            .await
            .get_session_file(&sid, "paused.bin")
            .await?
            .is_none()
    );
    assert!(f.incoming_empty().await?);
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn upload_commit_rechecks_revoked_credentials_and_rental_deadlines() -> anyhow::Result<()> {
    let f = Fixture::new().await?;
    let sid = f.acquire("board-one").await?;
    let (sender, upload) = f.paused(&sid).await?;
    sqlx::query("UPDATE oauth_sessions SET revoked_at=now() WHERE id=$1")
        .bind(f.grant.session_id)
        .execute(f.state.database.pool())
        .await?;
    drop(sender);
    assert_eq!(upload.await??.status(), StatusCode::UNAUTHORIZED);
    assert!(f.incoming_empty().await?);
    let f = Fixture::new().await?;
    let sid = f.acquire("board-one").await?;
    let (sender, upload) = f.paused(&sid).await?;
    sqlx::query(
        "UPDATE board_rentals SET ends_at=now()-interval '1 second' WHERE board_id='board-one'",
    )
    .execute(f.state.database.pool())
    .await?;
    drop(sender);
    assert_eq!(upload.await??.status(), StatusCode::FORBIDDEN);
    assert!(f.incoming_empty().await?);
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn quotas_bound_counts_capacity_and_concurrency_but_allow_overwrite() -> anyhow::Result<()> {
    let f = Fixture::new().await?;
    let one = f.acquire("board-one").await?;
    let two = f.acquire("board-two").await?;
    {
        let mut c = f.state.config.write().await;
        c.upload_limits.session_file_max_mib = 1;
        c.upload_limits.session_total_max_mib = 1;
        c.upload_limits.session_max_files = 1;
        c.upload_limits.storage_max_mib = 1;
        c.upload_limits.max_concurrent_uploads = 1;
    }
    assert_eq!(
        f.put(&one, "one.bin", vec![1; 600 * 1024]).await?,
        StatusCode::CREATED
    );
    assert_eq!(
        f.put(&one, "two.bin", vec![2; 1]).await?,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        f.put(&one, "one.bin", vec![2; 600 * 1024]).await?,
        StatusCode::CREATED
    );
    assert_eq!(
        f.put(&two, "one.bin", vec![3; 600 * 1024]).await?,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        f.put(&one, "one.bin", vec![4; 2 * 1024 * 1024]).await?,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let file = f
        .state
        .tftp_manager
        .read()
        .await
        .get_session_file(&one, "one.bin")
        .await?
        .unwrap();
    assert!(
        tokio::fs::read(file.disk_path)
            .await?
            .iter()
            .all(|v| *v == 2)
    );
    let (sender, upload) = f.paused(&two).await?;
    assert_eq!(
        f.put(&one, "one.bin", vec![1]).await?,
        StatusCode::TOO_MANY_REQUESTS
    );
    drop(sender);
    assert_eq!(upload.await??.status(), StatusCode::CREATED);
    assert!(f.incoming_empty().await?);
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn stalled_upload_times_out_and_releases_its_slot_and_tempfile() -> anyhow::Result<()> {
    let f = Fixture::new().await?;
    let sid = f.acquire("board-one").await?;
    f.state
        .config
        .write()
        .await
        .upload_limits
        .upload_timeout_secs = 1;
    let (sender, upload) = f.paused(&sid).await?;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), upload)
            .await???
            .status(),
        StatusCode::REQUEST_TIMEOUT
    );
    drop(sender);
    assert!(f.incoming_empty().await?);
    assert_eq!(f.put(&sid, "ok.bin", vec![1]).await?, StatusCode::CREATED);
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn device_listener_serves_files_but_never_user_or_admin_apis() -> anyhow::Result<()> {
    let f = Fixture::new().await?;
    let sid = f.acquire("board-one").await?;
    assert_eq!(
        f.put(&sid, "payload.bin", b"only device data".to_vec())
            .await?,
        StatusCode::CREATED
    );
    f.state
        .config
        .write()
        .await
        .security
        .hardware_allowed_networks = vec!["192.168.7.0/24".into()];
    let router = hardware::build_router(f.state.clone());
    let path = format!("/share/sessions/{sid}/payload.bin");
    for (uri, peer, forwarded, expected) in [
        (path.as_str(), "192.168.7.10:40000", None, StatusCode::OK),
        (
            path.as_str(),
            "203.0.113.1:40000",
            Some("192.168.7.10"),
            StatusCode::FORBIDDEN,
        ),
        (
            "/api/v1/admin/users",
            "192.168.7.10:40000",
            None,
            StatusCode::NOT_FOUND,
        ),
        (
            "/api/v1/board-types",
            "192.168.7.10:40000",
            None,
            StatusCode::NOT_FOUND,
        ),
    ] {
        let mut request = Request::builder().uri(uri);
        if let Some(value) = forwarded {
            request = request.header("x-forwarded-for", value);
        }
        let mut request = request.body(Body::empty())?;
        request
            .extensions_mut()
            .insert(ConnectInfo(peer.parse::<std::net::SocketAddr>()?));
        let response = router.clone().oneshot(request).await?;
        assert_eq!(response.status(), expected, "{uri}");
        if expected == StatusCode::OK {
            assert_eq!(
                to_bytes(response.into_body(), 1024).await?.as_ref(),
                b"only device data"
            );
        }
    }
    let mut request = Request::builder().uri(path).body(Body::empty())?;
    request.extensions_mut().insert(ConnectInfo(
        "192.168.7.10:40000".parse::<std::net::SocketAddr>()?,
    ));
    assert_eq!(
        f.app.oneshot(request).await?.status(),
        StatusCode::UNAUTHORIZED
    );
    Ok(())
}

#[tokio::test]
async fn builtin_tftp_rejects_write_requests_and_preserves_downloads() -> anyhow::Result<()> {
    use std::{
        net::UdpSocket,
        process::{Command, Stdio},
    };
    if let Ok(port) = std::env::var("OSTOOL_TEST_READONLY_TFTP_PORT") {
        let manager = BuiltinTftpManager::new(BuiltinTftpConfig {
            enabled: true,
            root_dir: std::env::current_dir()?.join("downloads"),
            bind_addr: format!("127.0.0.1:{port}").parse()?,
        });
        manager.start_if_needed().await?;
        std::future::pending::<()>().await;
        return Ok(());
    }
    let root = tempfile::tempdir()?;
    std::fs::create_dir(root.path().join("downloads"))?;
    std::fs::write(root.path().join("downloads/known.bin"), b"readonly")?;
    let probe = UdpSocket::bind("127.0.0.1:0")?;
    let port = probe.local_addr()?.port();
    drop(probe);
    let mut child = Command::new(std::env::current_exe()?)
        .args([
            "--exact",
            "builtin_tftp_rejects_write_requests_and_preserves_downloads",
            "--nocapture",
        ])
        .current_dir(root.path())
        .env("OSTOOL_TEST_READONLY_TFTP_PORT", port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let result = (|| -> anyhow::Result<()> {
        let socket = UdpSocket::bind("127.0.0.1:0")?;
        socket.set_read_timeout(Some(Duration::from_millis(150)))?;
        let mut buf = [0u8; 512];
        let mut received = false;
        for _ in 0..20 {
            socket.send_to(b"\x00\x02blocked.bin\x00octet\x00", ("127.0.0.1", port))?;
            if let Ok((n, _)) = socket.recv_from(&mut buf) {
                assert!(n >= 4);
                assert_eq!(&buf[..2], &[0, 5]);
                received = true;
                break;
            }
        }
        anyhow::ensure!(received, "TFTP did not reject WRQ");
        socket.send_to(b"\x00\x01known.bin\x00octet\x00", ("127.0.0.1", port))?;
        let (n, peer) = socket.recv_from(&mut buf)?;
        assert_eq!(&buf[..4], &[0, 3, 0, 1]);
        assert_eq!(&buf[4..n], b"readonly");
        socket.send_to(&[0, 4, 0, 1], peer)?;
        assert!(!root.path().join("blocked.bin").exists());
        assert!(!root.path().join("downloads/blocked.bin").exists());
        Ok(())
    })();
    let _ = child.kill();
    let _ = child.wait();
    result
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn restoration_failure_keeps_board_reserved_until_baseline_is_restored() -> anyhow::Result<()>
{
    let f = Fixture::new().await?;
    let allow = f._root.path().join("allow-restore");
    let done = f._root.path().join("restored");
    let quote =
        |p: &std::path::Path| format!("'{}'", p.display().to_string().replace('\'', "'\\''"));
    let command = format!(
        "test -f {} && printf restored > {}",
        quote(&allow),
        quote(&done)
    );
    f.state
        .boards
        .write()
        .await
        .get_mut("board-one")
        .unwrap()
        .reset_command = Some(command);
    let sid = f.acquire("board-one").await?;
    f.state
        .request_session_stop(&sid, session::SessionStopReason::ApiDelete)
        .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while f
            .state
            .board_runtime_status("board-one")
            .await
            .unwrap()
            .last_release_error
            .is_none()
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    assert!(f.state.get_session(&sid).await.is_some());
    assert!(!done.exists());
    assert_eq!(
        f.state
            .create_session_with_board_id("upload-test", "board-one", &[], None)
            .await
            .unwrap_err(),
        ostool_server::board_pool::BoardAllocationWithIdStatus::NoAvailableBoard
    );
    tokio::fs::write(allow, b"allow").await?;
    tokio::time::timeout(Duration::from_secs(10), async {
        while f.state.get_session(&sid).await.is_some() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    assert_eq!(tokio::fs::read(done).await?, b"restored");
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn device_callbacks_reject_redirects_and_oversized_status_responses() -> anyhow::Result<()> {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    let f = Fixture::new().await?;
    let forbidden = TcpListener::bind("127.0.0.1:0").await?;
    for oversized in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let target = forbidden.local_addr()?;
        let fake = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            let _ = socket.read(&mut request).await;
            let response = if oversized {
                "HTTP/1.1 200 OK\r\nContent-Length: 65537\r\nConnection: close\r\n\r\n".to_string()
            } else {
                format!(
                    "HTTP/1.1 302 Found\r\nLocation: http://{target}/private\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
            };
            let _ = socket.write_all(response.as_bytes()).await;
        });
        let mac = "02:00:00:00:00:01".parse()?;
        let announcement = httpboot_protocol::LoaderAnnouncement {
            protocol_version: httpboot_protocol::DEVICE_PROTOCOL_VERSION,
            mac_address: mac,
            current_mac_address: mac,
            arch: httpboot_protocol::BootArch::X86_64,
            loader_version: "test".into(),
            boot_epoch: "test-epoch".into(),
            http_port: address.port(),
            serial_id: None,
            serial_ready: false,
        };
        let error = device::reconcile(f.state.clone(), announcement, address)
            .await
            .unwrap_err();
        if oversized {
            assert!(error.to_string().contains("too large"));
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(100), forbidden.accept())
                .await
                .is_err()
        );
        fake.await?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn account_quotas_apply_across_multiple_rented_boards() -> anyhow::Result<()> {
    let f = Fixture::new().await?;
    let one = f.acquire("board-one").await?;
    let two = f.acquire("board-two").await?;
    {
        let mut c = f.state.config.write().await;
        c.upload_limits.session_file_max_mib = 1;
        c.upload_limits.user_total_max_mib = 1;
        c.upload_limits.user_max_concurrent_uploads = 1;
    }
    assert_eq!(
        f.put(&one, "one.bin", vec![1; 600 * 1024]).await?,
        StatusCode::CREATED
    );
    assert_eq!(
        f.put(&two, "two.bin", vec![1; 600 * 1024]).await?,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let (sender, task) = f.paused(&one).await?;
    assert_eq!(
        f.put(&two, "tiny.bin", vec![1]).await?,
        StatusCode::TOO_MANY_REQUESTS
    );
    drop(sender);
    assert_eq!(task.await??.status(), StatusCode::CREATED);
    assert!(f.incoming_empty().await?);
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn stale_cleanup_removes_only_owned_ended_session_directories() -> anyhow::Result<()> {
    let f = Fixture::new().await?;
    let sid = f.acquire("board-one").await?;
    assert_eq!(
        f.put(&sid, "active.bin", vec![1]).await?,
        StatusCode::CREATED
    );
    let root = f.state.tftp_manager.read().await.root_dir().to_path_buf();
    let stage = root.join("ostool/.incoming");
    tokio::fs::write(stage.join("upload-interrupted"), b"partial").await?;
    tokio::fs::write(stage.join("operator-file"), b"keep").await?;
    let ended = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO session_history(session_id,board_id,board_type,created_at,last_heartbeat_at,expires_at,ended_at,end_reason) VALUES($1,'old','test',now(),now(),now(),now(),'server_interrupted')").bind(&ended).execute(f.state.database.pool()).await?;
    let orphan = root.join("ostool/sessions").join(&ended);
    tokio::fs::create_dir_all(&orphan).await?;
    tokio::fs::write(orphan.join("late.bin"), b"old").await?;
    let unmanaged = root.join("ostool/sessions/operator-dir");
    tokio::fs::create_dir_all(&unmanaged).await?;
    uploads::cleanup_interrupted(&f.state).await?;
    assert!(!stage.join("upload-interrupted").exists());
    assert!(stage.join("operator-file").exists());
    assert!(!orphan.exists());
    assert!(unmanaged.exists());
    assert!(
        f.state
            .tftp_manager
            .read()
            .await
            .get_session_file(&sid, "active.bin")
            .await?
            .is_some()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn stalled_downloads_are_bounded_and_cancelled_when_the_session_ends() -> anyhow::Result<()> {
    let f = Fixture::new().await?;
    let sid = f.acquire("board-one").await?;
    assert_eq!(
        f.put(&sid, "large.bin", vec![1; 256 * 1024]).await?,
        StatusCode::CREATED
    );
    let router = hardware::build_router(f.state.clone());
    let request = || {
        let mut request = Request::builder()
            .uri(format!("/share/sessions/{sid}/large.bin"))
            .body(Body::empty())
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(
            "127.0.0.1:50000".parse::<std::net::SocketAddr>().unwrap(),
        ));
        request
    };
    let first = router.clone().oneshot(request()).await?;
    let second = router.clone().oneshot(request()).await?;
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(
        router.clone().oneshot(request()).await?.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    f.state
        .request_session_stop(&sid, session::SessionStopReason::ApiDelete)
        .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while f.state.get_session(&sid).await.is_some() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    let first_result = to_bytes(first.into_body(), 1024 * 1024).await;
    let second_result = to_bytes(second.into_body(), 1024 * 1024).await;
    assert!(first_result.is_err() || first_result.unwrap().len() < 256 * 1024);
    assert!(second_result.is_err() || second_result.unwrap().len() < 256 * 1024);
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn returned_download_url_reaches_the_device_listener_without_exposing_management()
-> anyhow::Result<()> {
    let f = Fixture::new().await?;
    let user_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let device_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let user_address = user_listener.local_addr()?;
    let device_address = device_listener.local_addr()?;
    {
        let mut c = f.state.config.write().await;
        c.listen_addr = user_address;
        c.hardware_listen_addr = Some(device_address);
    }
    let user_app = f.app.clone();
    let device_app = hardware::build_router(f.state.clone());
    let users = tokio::spawn(async move {
        axum::serve(
            user_listener,
            user_app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let devices = tokio::spawn(async move {
        axum::serve(
            device_listener,
            device_app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let created: Value = client
        .post(format!("http://{user_address}/api/v1/sessions"))
        .bearer_auth(&f.grant.access_token)
        .json(&json!({"board_type":"upload-test","board_id":"board-one"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let sid = created["session_id"].as_str().unwrap();
    let uploaded: Value = client
        .put(format!("http://{user_address}/api/v1/sessions/{sid}/files"))
        .bearer_auth(&f.grant.access_token)
        .header("x-file-path", "routed.bin")
        .body("device route")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let url = reqwest::Url::parse(uploaded["http_url"].as_str().unwrap())?;
    assert_eq!(url.port(), Some(device_address.port()));
    assert_eq!(
        client
            .get(url.clone())
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?
            .as_ref(),
        b"device route"
    );
    assert_eq!(
        client
            .get(format!("http://{user_address}{}", url.path()))
            .send()
            .await?
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        client
            .get(format!("http://{device_address}/api/v1/admin/users"))
            .send()
            .await?
            .status(),
        StatusCode::NOT_FOUND
    );
    f.state
        .request_session_stop(sid, session::SessionStopReason::ApiDelete)
        .await;
    users.abort();
    devices.abort();
    Ok(())
}
