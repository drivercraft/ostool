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
    let router = f.app.clone();
    let request = || {
        let mut request = Request::builder()
            .uri(format!("/share/sessions/{sid}/large.bin"))
            .header("authorization", format!("Bearer {}", f.grant.access_token))
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
async fn public_download_url_uses_the_authenticated_user_api_without_internal_addresses()
-> anyhow::Result<()> {
    let f = Fixture::new().await?;
    let user_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let user_address = user_listener.local_addr()?;
    {
        let mut c = f.state.config.write().await;
        c.listen_addr = user_address;
        c.auth.public_base_url = Some("https://boards.example.com".into());
        c.http_boot.public_base_url = Some("http://192.168.7.10:4999".into());
    }
    let user_app = f.app.clone();
    let users = tokio::spawn(async move {
        axum::serve(
            user_listener,
            user_app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
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
    assert_eq!(
        url.origin().ascii_serialization(),
        "https://boards.example.com"
    );
    assert!(uploaded["tftp_url"].is_null());
    let backend_url = format!("http://{user_address}{}", url.path());
    assert_eq!(
        client
            .get(&backend_url)
            .bearer_auth(&f.grant.access_token)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?
            .as_ref(),
        b"device route"
    );
    assert_eq!(
        client.get(&backend_url).send().await?.status(),
        StatusCode::UNAUTHORIZED
    );
    f.state
        .request_session_stop(sid, session::SessionStopReason::ApiDelete)
        .await;
    users.abort();
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn public_httpboot_responses_keep_device_network_and_uart_configuration_on_server()
-> anyhow::Result<()> {
    let f = Fixture::new().await?;
    {
        let mut boards = f.state.boards.write().await;
        let board = boards.get_mut("board-one").unwrap();
        board.boot = serde_json::from_value(json!({"kind":"httpboot","boot_arch":"x86_64",
            "serial_parameters":{"baud_rate":115200,"data_bits":8,"parity":"none","stop_bits":"one","flow_control":"none"}}))?;
        f.state.board_store.write_board(board).await?;
    }
    f.state.config.write().await.auth.public_base_url = Some("https://boards.example.com".into());
    let sid = f.acquire("board-one").await?;
    for suffix in ["boot-profile", "tftp", "serial", ""] {
        let path = if suffix.is_empty() {
            format!("/api/v1/sessions/{sid}")
        } else {
            format!("/api/v1/sessions/{sid}/{suffix}")
        };
        let response = f
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("authorization", format!("Bearer {}", f.grant.access_token))
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::OK, "{suffix}");
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
        assert!(body["server_ip"].is_null());
        assert!(body["netmask"].is_null());
        assert!(body["interface"].is_null());
        assert!(body["port"].is_null());
        assert!(body["boot"]["serial_parameters"].is_null());
        if suffix == "boot-profile" {
            assert_eq!(body["http_base_url"], "https://boards.example.com/");
            // Deserialize with the existing ostool model, without adding client protocol changes.
            let profile: ostool::board::client::BootProfileResponse = serde_json::from_value(body)?;
            assert!(matches!(
                profile.boot,
                ostool::board::client::BootConfig::UefiHttp(_)
            ));
        }
    }
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/v1/sessions/{sid}/http-boot/files"))
                .header("authorization", format!("Bearer {}", f.grant.access_token))
                .header("x-file-path", "kernel.bin")
                .body(Body::from("kernel"))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
    assert_eq!(
        body["http_url"],
        format!("https://boards.example.com/boot/sessions/{sid}/kernel.bin")
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn both_editions_share_user_routes_and_only_local_bypasses_authentication()
-> anyhow::Result<()> {
    let f = Fixture::new().await?;
    let sid = f.acquire("board-one").await?;
    f.put(&sid, "data.bin", b"private data".to_vec()).await?;
    for peer in ["127.0.0.1:4567", "192.168.7.10:4567", "203.0.113.10:4567"] {
        for (method, tail) in [
            ("GET", ""),
            ("GET", "/boot-profile"),
            ("GET", "/serial"),
            ("GET", "/serial/ws"),
            ("POST", "/uboot/prepare"),
            ("GET", "/uboot/prepare"),
            ("POST", "/uboot/boot"),
            ("POST", "/commands"),
            ("POST", "/heartbeat"),
            ("POST", "/board/power-on"),
            ("POST", "/board/power-off"),
            ("PUT", "/files"),
            ("DELETE", "/files/data.bin"),
            ("PUT", "/http-boot/files"),
            ("PUT", "/http-boot/kernel"),
            ("DELETE", ""),
        ] {
            let mut request = Request::builder()
                .method(method)
                .uri(format!("/api/v1/sessions/{sid}{tail}"))
                .header("x-forwarded-for", "127.0.0.1")
                .header("x-file-path", "data.bin")
                .body(Body::empty())?;
            request
                .extensions_mut()
                .insert(ConnectInfo(peer.parse::<std::net::SocketAddr>()?));
            assert_eq!(
                f.app.clone().oneshot(request).await?.status(),
                StatusCode::UNAUTHORIZED,
                "{method} {tail} from {peer}"
            );
        }
        for prefix in ["share", "boot"] {
            let mut request = Request::builder()
                .uri(format!("/{prefix}/sessions/{sid}/data.bin"))
                .header("x-forwarded-for", "127.0.0.1")
                .body(Body::empty())?;
            request
                .extensions_mut()
                .insert(ConnectInfo(peer.parse::<std::net::SocketAddr>()?));
            assert_eq!(
                f.app.clone().oneshot(request).await?.status(),
                StatusCode::UNAUTHORIZED
            );
        }
    }
    f.state.config.write().await.edition = Edition::Local;
    for path in [
        format!("/api/v1/sessions/{sid}"),
        format!("/api/v1/sessions/{sid}/boot-profile"),
        format!("/share/sessions/{sid}/data.bin"),
    ] {
        assert_eq!(
            f.app
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty())?)
                .await?
                .status(),
            StatusCode::OK
        );
    }
    assert_eq!(
        f.app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/v1/sessions/{sid}/files"))
                    .header("x-file-path", "local.bin")
                    .body(Body::from("local data"))?
            )
            .await?
            .status(),
        StatusCode::CREATED
    );
    Ok(())
}

// The fixture emulates a running TFTP service while exercising the real file store.
// UDP RRQ/WRQ behavior is tested separately above, in an isolated server subprocess.
struct ReadyTftp(Arc<dyn TftpManager>);
#[async_trait::async_trait]
impl TftpManager for ReadyTftp {
    async fn start_if_needed(&self) -> anyhow::Result<()> {
        Ok(())
    }
    async fn reconcile(&self) -> anyhow::Result<()> {
        Ok(())
    }
    async fn status(&self) -> anyhow::Result<ostool_server::tftp::status::TftpStatus> {
        let mut status = self.0.status().await?;
        status.enabled = true;
        status.healthy = true;
        Ok(status)
    }
    async fn put_session_file(
        &self,
        id: &str,
        path: &str,
        data: &[u8],
    ) -> anyhow::Result<ostool_server::tftp::files::TftpFileRef> {
        self.0.put_session_file(id, path, data).await
    }
    async fn get_session_file(
        &self,
        id: &str,
        path: &str,
    ) -> anyhow::Result<Option<ostool_server::tftp::files::TftpFileRef>> {
        self.0.get_session_file(id, path).await
    }
    async fn list_session_files(
        &self,
        id: &str,
    ) -> anyhow::Result<Vec<ostool_server::tftp::files::TftpFileRef>> {
        self.0.list_session_files(id).await
    }
    async fn remove_session_file(&self, id: &str, path: &str) -> anyhow::Result<()> {
        self.0.remove_session_file(id, path).await
    }
    async fn remove_session_dir(&self, id: &str) -> anyhow::Result<()> {
        self.0.remove_session_dir(id).await
    }
    fn root_dir(&self) -> &std::path::Path {
        self.0.root_dir()
    }
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn managed_uboot_keeps_tftp_and_shared_command_addresses_on_the_server() -> anyhow::Result<()>
{
    exercise_managed_boot(true, false).await
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn managed_uboot_uses_server_serial_transfer_without_tftp() -> anyhow::Result<()> {
    exercise_managed_boot(false, false).await
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn managed_uboot_disconnect_cancels_preparation_and_releases_the_device() -> anyhow::Result<()>
{
    exercise_managed_boot(true, true).await
}

#[cfg(unix)]
async fn exercise_managed_boot(tftp: bool, disconnect: bool) -> anyhow::Result<()> {
    let _ = env_logger::builder().is_test(true).try_init();
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_serial::SerialPort;
    let f = Fixture::new().await?;
    let (mut master, mut slave) = tokio_serial::SerialStream::pair()?;
    slave.set_exclusive(false)?;
    let path = slave.name().unwrap();
    drop(slave);
    let user_listener = tokio::net::TcpListener::bind("127.0.0.2:0").await?;
    let user_address = user_listener.local_addr()?;
    {
        let mut config = f.state.config.write().await;
        config.listen_addr = user_address;
        config.auth.public_base_url = Some("https://boards.example.com".into());
    }
    let device_address = hardware::download_address(&*f.state.config.read().await)?.unwrap();
    let device_listener = tokio::net::TcpListener::bind(device_address).await?;
    {
        let mut boards = f.state.boards.write().await;
        let board = boards.get_mut("board-one").unwrap();
        board.serial = Some(SerialConfig {
            key: SerialPortKey {
                kind: SerialPortKeyKind::UsbPath,
                value: path,
            },
            baud_rate: 115200,
            resolved_device_path: None,
            resolved_usb_path: None,
        });
        board.boot = BootConfig::Uboot(UbootProfile {
            use_tftp: true,
            server_ip: Some("192.168.44.1".into()),
            board_ip: Some("192.168.44.2".into()),
            network_mode: UbootNetworkMode::StaticIp,
            netmask: Some("255.255.255.0".into()),
            ..Default::default()
        });
        f.state.board_store.write_board(board).await?;
    }
    let manager = f.state.tftp_manager.read().await.clone();
    if tftp {
        *f.state.tftp_manager.write().await = Arc::new(ReadyTftp(manager.clone()));
    }
    let transcript = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let recorded = transcript.clone();
    let emulator = tokio::spawn(async move {
        let mut line = Vec::new();
        let mut booted = false;
        loop {
            let byte = match master.read_u8().await {
                Ok(byte) => byte,
                Err(error) if error.raw_os_error() == Some(5) => {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            if byte == 3 {
                line.clear();
                master.write_all(b"=> <INTERRUPT>\n").await?;
                continue;
            }
            if byte != b'\n' {
                line.push(byte);
                master.write_all(&[byte]).await?;
                continue;
            }
            let command = String::from_utf8(std::mem::take(&mut line))?;
            recorded.lock().unwrap().push(command.clone());
            if command.starts_with("bootm ") {
                booted = true;
                master.write_all(b"\nfixture guest ready\n# ").await?;
            } else if command.starts_with("loady ") {
                master.write_all(b"\nC").await?;
                for block in 0..2 {
                    let marker = master.read_u8().await?;
                    anyhow::ensure!(marker == 1 || marker == 2, "wrong YMODEM marker");
                    let mut packet = vec![0; 2 + if marker == 1 { 128 } else { 1024 } + 2];
                    master.read_exact(&mut packet).await?;
                    anyhow::ensure!(
                        packet[0] == block && packet[1] == !block,
                        "wrong YMODEM sequence"
                    );
                    let data = &packet[2..packet.len() - 2];
                    let crc = u16::from_be_bytes(packet[packet.len() - 2..].try_into()?);
                    anyhow::ensure!(
                        uboot_shell::crc::crc16_ccitt(0, data) == crc,
                        "wrong YMODEM CRC"
                    );
                    if block == 1 {
                        anyhow::ensure!(
                            data.starts_with(b"fixture FIT"),
                            "wrong serial image bytes"
                        );
                    }
                    master.write_all(&[6]).await?;
                }
                anyhow::ensure!(master.read_u8().await? == 4, "missing EOT");
                master.write_all(&[6]).await?;
                let mut last = [0; 133];
                master.read_exact(&mut last).await?;
                anyhow::ensure!(
                    last[0] == 1 && last[1] == 0 && last[3] == 0,
                    "wrong end block"
                );
                master.write_all(b"\x06C\n=> ").await?;
            } else if booted {
                if let Some(url) = command.strip_prefix("curl ") {
                    let response = reqwest::Client::builder()
                        .no_proxy()
                        .build()?
                        .get(url)
                        .send()
                        .await?
                        .error_for_status()?
                        .bytes()
                        .await?;
                    anyhow::ensure!(
                        response == b"shared fixture bytes".as_slice(),
                        "wrong shared file"
                    );
                    master.write_all(b"\nshared-command-ok\n# ").await?;
                }
            } else {
                let value = if command.starts_with("echo $kernel_addr_r") {
                    "80200000"
                } else if command.starts_with("echo $kernel_comp_addr_r") {
                    "84000000"
                } else if command.starts_with("echo $fdt_addr_r") {
                    "88000000"
                } else {
                    ""
                };
                master
                    .write_all(format!("\n{value}\ncmd-ok\n=> ").as_bytes())
                    .await?;
            }
        }
        #[allow(unreachable_code)]
        Ok::<_, anyhow::Error>(())
    });
    let user_app = f.app.clone();
    let users = tokio::spawn(async move {
        axum::serve(
            user_listener,
            user_app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap()
    });
    let device_app = hardware::download_router(f.state.clone());
    let devices = tokio::spawn(async move {
        axum::serve(
            device_listener,
            device_app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap()
    });
    let sid = f.acquire("board-one").await?;
    f.put(&sid, "shared.txt", b"shared fixture bytes".to_vec())
        .await?;
    let base = format!("http://{user_address}/");
    let mut probe = tokio::process::Command::new(std::env::current_exe()?)
        .args([
            "--ignored",
            "--exact",
            "managed_uboot_client_probe",
            "--nocapture",
        ])
        .env("OSTOOL_MANAGED_TEST_BASE", &base)
        .env("OSTOOL_MANAGED_TEST_SESSION", &sid)
        .env(
            "OSTOOL_MANAGED_TEST_DISCONNECT",
            if disconnect { "true" } else { "false" },
        )
        .env("OSTOOL_BOARD_ACCESS_TOKEN", &f.grant.access_token)
        .env("HOME", f._root.path())
        .env("XDG_CONFIG_HOME", f._root.path().join("config"))
        .env("XDG_CACHE_HOME", f._root.path().join("cache"))
        .env("NO_PROXY", "127.0.0.1,127.0.0.2,localhost")
        .env("no_proxy", "127.0.0.1,127.0.0.2,localhost")
        .kill_on_drop(true)
        .spawn()?;
    let result = tokio::time::timeout(Duration::from_secs(30), probe.wait()).await??;
    users.abort();
    devices.abort();
    emulator.abort();
    anyhow::ensure!(result.success(), "actual CLI managed boot probe failed");
    let commands = transcript.lock().unwrap().clone();
    if disconnect {
        tokio::time::timeout(Duration::from_secs(5), async {
            while f.state.get_session(&sid).await.is_some() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        assert!(!commands.iter().any(|c| c.starts_with("bootm ")));
    } else {
        if tftp {
            assert!(
                commands
                    .iter()
                    .any(|c| c.starts_with("setenv serverip 192.168.44.1"))
            );
            assert!(
                commands
                    .iter()
                    .any(|c| c.starts_with(&format!("tftp ostool/sessions/{sid}/boot/kernel.fit")))
            );
        } else {
            assert!(commands.iter().any(|c| c.starts_with("loady 0x84000000")));
            assert!(!commands.iter().any(|c| c.starts_with("tftp ")));
        }
        assert!(commands.iter().any(|c| c.starts_with("bootm 0x84000000")));
        assert!(commands.iter().any(|c| c.starts_with(&format!(
            "curl http://{device_address}/share/sessions/{sid}/shared.txt"
        ))));
    }
    Ok(())
}

#[tokio::test]
#[ignore = "child process of the managed U-Boot fixture"]
async fn managed_uboot_client_probe() -> anyhow::Result<()> {
    use ostool::board::{
        client::{BoardServerClient, UbootPrepareRequest},
        global_config::{AuthMode, BoardEndpoint},
        serial_stream::connect_serial_stream_with_commands,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_util::compat::{FuturesAsyncReadCompatExt, FuturesAsyncWriteCompatExt};
    let Ok(base) = std::env::var("OSTOOL_MANAGED_TEST_BASE") else {
        return Ok(());
    };
    let sid = std::env::var("OSTOOL_MANAGED_TEST_SESSION")?;
    // Only the disposable loopback fixture bypasses the production HTTPS endpoint constructor.
    let url = url::Url::parse(&base)?;
    anyhow::ensure!(
        url.host_str() == Some("127.0.0.2"),
        "test endpoint must stay on loopback"
    );
    let client = BoardServerClient::new_with_endpoint(BoardEndpoint {
        base_url: url,
        auth_mode: AuthMode::Required,
    })?;
    let profile = client.get_boot_profile(&sid).await?;
    assert!(profile.server_side_operations);
    assert!(profile.server_ip.is_none());
    assert!(profile.interface.is_none());
    let ostool::board::client::BootConfig::Uboot(profile) = profile.boot else {
        panic!("wrong profile")
    };
    assert!(profile.board_ip.is_none());
    assert!(profile.server_ip.is_none());
    let ids = client
        .register_shell_commands(&sid, &["curl ${sessionFile:shared.txt}".into()])
        .await?;
    assert_eq!(ids.len(), 1);
    client
        .prepare_uboot(
            &sid,
            &UbootPrepareRequest {
                timeout_secs: Some(10),
                ..Default::default()
            },
        )
        .await?;
    let ws = client.resolve_ws_url(&format!("/api/v1/sessions/{sid}/serial/ws"))?;
    let (tx, rx, tasks) = connect_serial_stream_with_commands(
        ws,
        client.websocket_authorization().await?,
        ids.clone(),
    )
    .await?;
    let mut tx = tx.compat_write();
    let mut rx = rx.compat();
    let addresses = client.wait_uboot_prepared(&sid).await?;
    assert_eq!(addresses.kernel_load_addr, 0x80200000);
    assert_eq!(addresses.fit_load_addr, 0x84000000);
    if std::env::var("OSTOOL_MANAGED_TEST_DISCONNECT").as_deref() == Ok("true") {
        drop(tx);
        drop(rx);
        tasks.shutdown_with_timeout(Duration::from_secs(2)).await?;
        return Ok(());
    }
    client
        .upload_session_file(&sid, "boot/kernel.fit", b"fixture FIT".to_vec())
        .await?;
    client
        .boot_uboot(&sid, "boot/kernel.fit", Some("console=ttyS0"))
        .await?;
    let mut data = Vec::new();
    while !data.ends_with(b"# ") {
        data.push(rx.read_u8().await?);
    }
    assert!(String::from_utf8_lossy(&data).contains("fixture guest ready"));
    tx.write_all(format!("\u{1e}{}\u{1f}\n", ids[0]).as_bytes())
        .await?;
    tx.flush().await?;
    data.clear();
    while !data.ends_with(b"# ") {
        data.push(rx.read_u8().await?);
    }
    assert!(String::from_utf8_lossy(&data).contains("shared-command-ok"));
    drop(tx);
    drop(rx);
    tasks.shutdown_with_timeout(Duration::from_secs(2)).await?;
    client.delete_session(&sid).await?;
    Ok(())
}
