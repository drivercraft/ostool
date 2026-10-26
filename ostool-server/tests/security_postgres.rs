//! Public security boundaries with real PostgreSQL; no production bypass switches.
#[path = "support/database.rs"]
mod test_database;
#[path = "support/security.rs"]
mod test_security;
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{HeaderMap, Request, StatusCode},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::Utc;
use ostool_server::{
    AppState, AuthConfig, BuiltinTftpConfig, ServerConfig, TftpConfig, build_app_state,
    build_router, config::Edition, security::crypto, tftp::service::build_tftp_manager,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
struct Reply {
    status: StatusCode,
    body: Value,
    headers: HeaderMap,
}
struct Fixture {
    state: AppState,
    app: Router,
    _root: tempfile::TempDir,
    origin: String,
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
            auth: AuthConfig {
                public_base_url: Some("http://127.0.0.1:4190".into()),
                bootstrap_admin_email: Some("admin@security.test".into()),
                bootstrap_admin_password: Some("security-admin-password-123".into()),
                ..Default::default()
            },
            tftp: TftpConfig::Builtin(BuiltinTftpConfig {
                enabled: false,
                ..BuiltinTftpConfig::default_with_root(root.path().join("tftp"))
            }),
            ..Default::default()
        };
        config.security.mfa_encryption_key = Some(STANDARD.encode([7u8; 32]));
        test_database::prepare(&root.path().join("config.toml"), &mut config)
            .await
            .unwrap();
        let state = build_app_state(
            root.path().join("config.toml"),
            config.clone(),
            build_tftp_manager(&config.tftp),
        )
        .await
        .unwrap();
        Self {
            app: build_router(state.clone()),
            state,
            _root: root,
            origin: "http://127.0.0.1:4190".into(),
        }
    }
    async fn request(&self, method: &str, path: &str, cookies: Option<&str>, body: Value) -> Reply {
        self.custom(method, path, cookies, body, Some(&self.origin), None)
            .await
    }
    async fn custom(
        &self,
        method: &str,
        path: &str,
        cookies: Option<&str>,
        mut body: Value,
        origin: Option<&str>,
        bearer: Option<&str>,
    ) -> Reply {
        if (path == "/api/v1/auth/register" || path.starts_with("/api/v1/admin/users"))
            && body.is_object()
            && body.get("username").is_none()
        {
            body["username"] = json!(
                body["email"]
                    .as_str()
                    .unwrap_or("fixture")
                    .split('@')
                    .next()
                    .unwrap()
                    .replace('.', "_")
            );
        }
        if path == "/api/v1/auth/register" && body.get("legal").is_none() {
            body["legal"] = json!({"accepted":true,"terms_version":1,"privacy_version":1});
        }
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .header("user-agent", "ostool-security-test")
            .header(
                "x-ostool-surface",
                if cookies.is_some_and(|c| c.contains("ostool-admin-")) {
                    "admin"
                } else {
                    "website"
                },
            )
            .header("x-forwarded-for", "127.0.0.1");
        if let Some(cookies) = cookies {
            request = request.header("cookie", cookies);
        }
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        if let Some(token) = bearer {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let mut request = request.body(Body::from(body.to_string())).unwrap();
        request.extensions_mut().insert(ConnectInfo(
            "203.0.113.5:4190".parse::<std::net::SocketAddr>().unwrap(),
        ));
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap();
        Reply {
            status,
            headers,
            body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        }
    }
    async fn proof(&self, action: &str) -> Value {
        let result = self
            .request(
                "GET",
                &format!("/api/v1/auth/captcha?action={action}"),
                None,
                Value::Null,
            )
            .await;
        assert_eq!(result.status, StatusCode::OK, "{}", result.body);
        let id: Uuid = result.body["id"].as_str().unwrap().parse().unwrap();
        sqlx::query("UPDATE security_challenges SET secret_hash=$2 WHERE id=$1")
            .bind(id)
            .bind(crypto::proof_hash(
                &format!("captcha:{action}"),
                id,
                "AB29Q",
            ))
            .execute(self.state.database.pool())
            .await
            .unwrap();
        json!({"captcha_id":id,"captcha_code":"AB29Q"})
    }
    async fn login(&self, admin: bool) -> Reply {
        self.request("POST",if admin{"/api/v1/admin/auth/login"}else{"/api/v1/auth/login"},None,json!({"email":"admin@security.test","password":"security-admin-password-123","verification":self.proof(if admin{"admin_login"}else{"login"}).await})).await
    }
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn captcha_is_png_single_use_action_bound_expiring_and_server_required() {
    let f = Fixture::new(true).await;
    let image = f
        .request(
            "GET",
            "/api/v1/auth/captcha?action=login",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(image.status, StatusCode::OK);
    let png = STANDARD
        .decode(
            image.body["image_data_url"]
                .as_str()
                .unwrap()
                .strip_prefix("data:image/png;base64,")
                .unwrap(),
        )
        .unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(image.headers["cache-control"], "no-store");
    let missing = f
        .request(
            "POST",
            "/api/v1/auth/login",
            None,
            json!({"email":"admin@security.test","password":"security-admin-password-123"}),
        )
        .await;
    assert_eq!(missing.status, StatusCode::BAD_REQUEST);
    let wrong_action = f.proof("register").await;
    let failed=f.request("POST","/api/v1/auth/login",None,json!({"email":"admin@security.test","password":"security-admin-password-123","verification":wrong_action})).await;
    assert_eq!(failed.body["code"], "captcha_invalid");
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM security_challenges WHERE id=$1)")
            .bind(
                wrong_action["captcha_id"]
                    .as_str()
                    .unwrap()
                    .parse::<Uuid>()
                    .unwrap(),
            )
            .fetch_one(f.state.database.pool())
            .await
            .unwrap();
    assert!(!exists);
    let proof = f.proof("login").await;
    let first=f.request("POST","/api/v1/auth/login",None,json!({"email":"admin@security.test","password":"security-admin-password-123","verification":proof})).await;
    assert_eq!(first.status, StatusCode::OK);
    assert!(first.body.get("access_token").is_none());
    assert!(first.body.get("refresh_token").is_none());
    let second=f.request("POST","/api/v1/auth/login",None,json!({"email":"admin@security.test","password":"security-admin-password-123","verification":proof})).await;
    assert_eq!(second.status, StatusCode::BAD_REQUEST);
    let expired = f.proof("admin_login").await;
    sqlx::query("UPDATE security_challenges SET expires_at=now()-interval '1 second' WHERE id=$1")
        .bind(
            expired["captcha_id"]
                .as_str()
                .unwrap()
                .parse::<Uuid>()
                .unwrap(),
        )
        .execute(f.state.database.pool())
        .await
        .unwrap();
    let result=f.request("POST","/api/v1/admin/auth/login",None,json!({"email":"admin@security.test","password":"security-admin-password-123","verification":expired})).await;
    assert_eq!(result.status, StatusCode::BAD_REQUEST);
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn admin_actions_skip_step_up_but_keep_session_isolation_and_csrf() {
    let f = Fixture::new(true).await;
    let web = f.login(false).await;
    let website = test_security::cookie_header(&web.headers);
    assert_eq!(
        f.request("GET", "/api/v1/admin/users", Some(&website), Value::Null)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.custom(
            "POST",
            "/api/v1/admin/roles",
            None,
            json!({"name":"forbidden role","administrator":false}),
            Some(&f.origin),
            Some(test_security::access_cookie(&web.headers)),
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.request("POST", "/api/v1/admin/roles", None, json!({}))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let backend = f.login(true).await;
    assert_eq!(backend.status, StatusCode::OK);
    for cookie in backend.headers.get_all("set-cookie") {
        let value = cookie.to_str().unwrap();
        assert!(
            value.contains("HttpOnly")
                && value.contains("SameSite=Strict")
                && value.contains("Path=/")
        );
    }
    let cookies = test_security::cookie_header(&backend.headers);
    assert_eq!(
        f.request("GET", "/api/v1/admin/users", Some(&cookies), Value::Null)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        f.custom(
            "POST",
            "/api/v1/admin/roles",
            Some(&cookies),
            json!({"password":"security-admin-password-123"}),
            Some("https://evil.test"),
            None
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.custom(
            "POST",
            "/api/v1/admin/roles",
            Some(&cookies),
            json!({"password":"security-admin-password-123"}),
            None,
            None
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    sqlx::query("UPDATE oauth_sessions SET step_up_until=now()-interval '1 second'")
        .execute(f.state.database.pool())
        .await
        .unwrap();
    let allowed = f
        .request(
            "POST",
            "/api/v1/admin/roles",
            Some(&cookies),
            json!({"name":"test role","description":"","administrator":false}),
        )
        .await;
    assert_eq!(allowed.status, StatusCode::CREATED, "{}", allowed.body);
    let sessions = f
        .request(
            "POST",
            "/api/v1/auth/security/sessions/others",
            Some(&cookies),
            Value::Null,
        )
        .await;
    assert_eq!(sessions.status, StatusCode::OK, "{}", sessions.body);
    let enrollment = f
        .request(
            "POST",
            "/api/v1/auth/security/enrollment",
            Some(&cookies),
            Value::Null,
        )
        .await;
    assert_eq!(enrollment.status, StatusCode::OK, "{}", enrollment.body);
    // An admin's website session still follows personal-account verification rules.
    let blocked = f
        .request(
            "POST",
            "/api/v1/auth/personal-access-tokens",
            Some(&website),
            json!({"name":"website token"}),
        )
        .await;
    // Revoking other sessions above also revokes the first website session.
    assert_eq!(blocked.status, StatusCode::UNAUTHORIZED);
    let web = f.login(false).await;
    let website = test_security::cookie_header(&web.headers);
    sqlx::query("UPDATE oauth_sessions SET step_up_until=NULL WHERE client_id='web'")
        .execute(f.state.database.pool())
        .await
        .unwrap();
    let blocked = f
        .request(
            "POST",
            "/api/v1/auth/personal-access-tokens",
            Some(&website),
            json!({"name":"website token"}),
        )
        .await;
    assert_eq!(blocked.body["code"], "reauth_required");
    let verified = f
        .request(
            "POST",
            "/api/v1/auth/step-up",
            Some(&website),
            json!({"password":"security-admin-password-123"}),
        )
        .await;
    assert_eq!(verified.status, StatusCode::OK, "{}", verified.body);
    let allowed = f
        .request(
            "POST",
            "/api/v1/auth/personal-access-tokens",
            Some(&website),
            json!({"name":"website token"}),
        )
        .await;
    assert_eq!(allowed.status, StatusCode::CREATED, "{}", allowed.body);
    let headers = f
        .request("GET", "/admin/login", None, Value::Null)
        .await
        .headers;
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert_eq!(headers["x-frame-options"], "DENY");
    assert!(
        headers["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("nonce-")
    );
    assert_eq!(
        f.request("POST", "/api/v1/loaders/poll", None, json!({}))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn failed_logins_are_bounded_and_forwarded_ip_cannot_bypass_limits() {
    let f = Fixture::new(true).await;
    for _ in 0..6 {
        let result=f.request("POST","/api/v1/auth/login",None,json!({"email":"admin@security.test","password":"incorrect-password","verification":f.proof("login").await})).await;
        assert_eq!(result.status, StatusCode::UNAUTHORIZED, "{}", result.body);
    }
    let locked = f.login(false).await;
    assert_eq!(locked.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(locked.headers.contains_key("retry-after"));
    assert!(
        locked.body["details"]["retry_after_seconds"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(!locked.body.to_string().contains("password_hash"));
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn refresh_rotation_preserves_absolute_expiry_and_detects_replay() {
    let f = Fixture::new(true).await;
    let login = f.login(false).await;
    let cookies = test_security::cookie_header(&login.headers);
    let original: chrono::DateTime<Utc> =
        sqlx::query_scalar("SELECT absolute_expires_at FROM oauth_sessions")
            .fetch_one(f.state.database.pool())
            .await
            .unwrap();
    let refreshed = f
        .request("POST", "/api/v1/auth/refresh", Some(&cookies), Value::Null)
        .await;
    assert_eq!(refreshed.status, StatusCode::OK, "{}", refreshed.body);
    let next = test_security::cookie_header(&refreshed.headers);
    let latest: chrono::DateTime<Utc> = sqlx::query_scalar(
        "SELECT absolute_expires_at FROM oauth_sessions WHERE revoked_at IS NULL",
    )
    .fetch_one(f.state.database.pool())
    .await
    .unwrap();
    assert_eq!(original, latest);
    assert_eq!(
        f.request("GET", "/api/v1/auth/me", Some(&next), Value::Null)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        f.request("POST", "/api/v1/auth/refresh", Some(&cookies), Value::Null)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.request("GET", "/api/v1/auth/me", Some(&next), Value::Null)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn totp_enrollment_login_recovery_codes_and_administrator_requirement() {
    let f = Fixture::new(true).await;
    let login = f.login(true).await;
    let cookies = test_security::cookie_header(&login.headers);
    let enrollment = f
        .request(
            "POST",
            "/api/v1/auth/security/enrollment",
            Some(&cookies),
            Value::Null,
        )
        .await;
    assert_eq!(enrollment.status, StatusCode::OK, "{}", enrollment.body);
    let secret = data_encoding::BASE32_NOPAD
        .decode(enrollment.body["secret"].as_str().unwrap().as_bytes())
        .unwrap();
    let confirmation=f.request("POST","/api/v1/auth/security/confirm",Some(&cookies),json!({"challenge_id":enrollment.body["challenge_id"],"code":crypto::totp(&secret,Utc::now().timestamp()/30)})).await;
    assert_eq!(confirmation.status, StatusCode::OK, "{}", confirmation.body);
    let recovery = confirmation.body["recovery_codes"].as_array().unwrap();
    assert_eq!(recovery.len(), 8);
    let stored: String = sqlx::query_scalar("SELECT secret_ciphertext FROM mfa_credentials")
        .fetch_one(f.state.database.pool())
        .await
        .unwrap();
    assert!(!stored.contains(enrollment.body["secret"].as_str().unwrap()));
    assert_eq!(
        f.request("GET", "/api/v1/auth/me", Some(&cookies), Value::Null)
            .await
            .status,
        StatusCode::OK
    );
    f.request("POST", "/api/v1/auth/logout", Some(&cookies), Value::Null)
        .await;
    let pending = f.login(true).await;
    assert_eq!(pending.status, StatusCode::ACCEPTED, "{}", pending.body);
    assert!(!pending.headers.contains_key("set-cookie"));
    let invalid = f
        .request(
            "POST",
            "/api/v1/auth/mfa/verify",
            None,
            json!({"challenge_id":pending.body["challenge_id"],"code":""}),
        )
        .await;
    assert_eq!(invalid.status, StatusCode::UNAUTHORIZED);
    let authenticated=f.request("POST","/api/v1/auth/mfa/verify",None,json!({"challenge_id":pending.body["challenge_id"],"code":crypto::totp(&secret,Utc::now().timestamp()/30+1)})).await;
    assert_eq!(
        authenticated.status,
        StatusCode::OK,
        "{}",
        authenticated.body
    );
    let second = f.login(true).await;
    let recovered = f
        .request(
            "POST",
            "/api/v1/auth/mfa/verify",
            None,
            json!({"challenge_id":second.body["challenge_id"],"code":recovery[0]}),
        )
        .await;
    assert_eq!(recovered.status, StatusCode::OK, "{}", recovered.body);
    let third = f.login(true).await;
    let replay = f
        .request(
            "POST",
            "/api/v1/auth/mfa/verify",
            None,
            json!({"challenge_id":third.body["challenge_id"],"code":recovery[0]}),
        )
        .await;
    assert_eq!(replay.status, StatusCode::UNAUTHORIZED);
    let restored = test_security::cookie_header(&recovered.headers);
    f.state.config.write().await.security.require_admin_mfa = true;
    assert_eq!(
        f.request(
            "DELETE",
            "/api/v1/auth/security/mfa",
            Some(&restored),
            Value::Null
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    let details: Vec<String> = sqlx::query_scalar("SELECT details::text FROM audit_events")
        .fetch_all(f.state.database.pool())
        .await
        .unwrap();
    let text = details.join(" ");
    assert!(!text.contains("security-admin-password-123"));
    assert!(!text.contains(recovery[0].as_str().unwrap()));
    assert!(!text.contains(enrollment.body["secret"].as_str().unwrap()));
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn local_edition_keeps_anonymous_board_access_and_hides_identity_features() {
    let f = Fixture::new(false).await;
    assert_eq!(
        f.request("GET", "/api/v1/board-types", None, Value::Null)
            .await
            .status,
        StatusCode::OK
    );
    for path in [
        "/api/v1/auth/captcha?action=login",
        "/api/v1/auth/security",
        "/api/v1/account",
        "/api/v1/admin/users",
        "/api/v1/admin/rentals",
        "/api/v1/admin/mail-deliveries",
    ] {
        assert_eq!(
            f.request("GET", path, None, Value::Null).await.status,
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
    let config = f
        .request("GET", "/api/v1/auth/config", None, Value::Null)
        .await;
    assert_eq!(config.body["captcha_required"], false);
    assert_eq!(config.body["enabled"], false);
}

/// A loopback SMTP sink exercises the real mail transport, without external delivery.
async fn smtp_sink() -> (
    u16,
    tokio::sync::mpsc::Receiver<String>,
    tokio::task::JoinHandle<()>,
) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (sender, receiver) = tokio::sync::mpsc::channel(8);
    let task = tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let sender = sender.clone();
            tokio::spawn(async move {
                let (read, mut write) = socket.into_split();
                write
                    .write_all(b"220 localhost test SMTP\r\n")
                    .await
                    .unwrap();
                let mut lines = BufReader::new(read).lines();
                let mut data = false;
                let mut message = String::new();
                while let Some(line) = lines.next_line().await.unwrap() {
                    if data {
                        if line == "." {
                            sender.send(message.clone()).await.unwrap();
                            write.write_all(b"250 accepted\r\n").await.unwrap();
                            data = false;
                        } else {
                            message.push_str(&line);
                            message.push_str("\r\n");
                        }
                    } else if line.starts_with("EHLO") || line.starts_with("HELO") {
                        write.write_all(b"250 localhost\r\n").await.unwrap();
                    } else if line == "DATA" {
                        write.write_all(b"354 send message\r\n").await.unwrap();
                        data = true;
                    } else if line == "QUIT" {
                        write.write_all(b"221 goodbye\r\n").await.unwrap();
                        break;
                    } else {
                        write.write_all(b"250 ok\r\n").await.unwrap();
                    }
                }
            });
        }
    });
    (port, receiver, task)
}
async fn mail_code(receiver: &mut tokio::sync::mpsc::Receiver<String>) -> String {
    let message = tokio::time::timeout(std::time::Duration::from_secs(5), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    let (headers, body) = message.split_once("\r\n\r\n").unwrap();
    let body = if headers
        .to_lowercase()
        .contains("content-transfer-encoding: base64")
    {
        String::from_utf8(
            STANDARD
                .decode(body.split_whitespace().collect::<String>())
                .unwrap(),
        )
        .unwrap()
    } else {
        body.replace("=\r\n", "")
    };
    body.split(|c: char| !c.is_ascii_digit())
        .find(|part| part.len() == 6)
        .expect("mail contains a six digit code")
        .to_owned()
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn optional_smtp_verification_and_password_recovery_revoke_old_credentials() {
    let f = Fixture::new(true).await;
    let (port, mut messages, mail_task) = smtp_sink().await;
    {
        let mut config = f.state.config.write().await;
        config.security.smtp_host = Some("127.0.0.1".into());
        config.security.smtp_from = Some("security@ostool.test".into());
        config.security.smtp_port = port;
        config.security.smtp_tls = ostool_server::security::config::SmtpTls::LoopbackPlaintext;
        config.security.registration_email_verification = true;
        config.security.registration_email_policy.ttl_secs = 1200;
        config.security.registration_email_policy.resend_secs = 90;
        config.security.validate(true).unwrap();
    }
    let email = "mail-member@security.test";
    let missing = f.request("POST", "/api/v1/auth/register", None,
        json!({"email":email,"password":"initial-mail-password-123","verification":f.proof("register").await})).await;
    assert_eq!(missing.status, StatusCode::BAD_REQUEST);
    let sent = f.request("POST", "/api/v1/auth/email-code", None,
        json!({"email":email,"action":"register","verification":f.proof("registration_email").await})).await;
    assert_eq!(sent.status, StatusCode::OK, "{}", sent.body);
    assert_eq!(sent.body["expires_in"], 1200);
    assert_eq!(sent.body["resend_after"], 90);
    let code = mail_code(&mut messages).await;
    assert!(!sent.body.to_string().contains(&code));
    let register = f.request("POST", "/api/v1/auth/register", None,
        json!({"email":email,"password":"initial-mail-password-123","verification":f.proof("register").await,
        "email_challenge_id":sent.body["challenge_id"],"email_code":code})).await;
    assert_eq!(register.status, StatusCode::CREATED, "{}", register.body);
    let cookies = test_security::cookie_header(&register.headers);
    let verified: bool =
        sqlx::query_scalar("SELECT email_verified_at IS NOT NULL FROM users WHERE email=$1")
            .bind(email)
            .fetch_one(f.state.database.pool())
            .await
            .unwrap();
    assert!(verified);
    let reused = f.request("POST", "/api/v1/auth/register", None,
        json!({"email":email,"password":"initial-mail-password-123","verification":f.proof("register").await,
        "email_challenge_id":sent.body["challenge_id"],"email_code":code})).await;
    assert_eq!(reused.status, StatusCode::BAD_REQUEST);
    f.state
        .security_cache
        .as_ref()
        .unwrap()
        .clear("email-resend", email)
        .await
        .unwrap();
    let reset = f.request("POST", "/api/v1/auth/email-code", None,
        json!({"email":email,"action":"password_reset","verification":f.proof("password_reset").await})).await;
    assert_eq!(reset.status, StatusCode::OK, "{}", reset.body);
    let reset_code = mail_code(&mut messages).await;
    let changed = f.request("POST", "/api/v1/auth/password-reset", None,
        json!({"challenge_id":reset.body["challenge_id"],"code":reset_code,"new_password":"replacement-mail-password-123"})).await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
    let history: i64 = sqlx::query_scalar("SELECT count(*) FROM password_change_events")
        .fetch_one(f.state.database.pool())
        .await
        .unwrap();
    assert_eq!(history, 1);
    let delivery_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM mail_deliveries WHERE status='sent' AND completed_at IS NOT NULL",
    )
    .fetch_one(f.state.database.pool())
    .await
    .unwrap();
    assert_eq!(delivery_count, 2);
    let consumed:bool=sqlx::query_scalar("SELECT consumed_at IS NOT NULL AND length(code_hash)=64 FROM password_reset_tokens WHERE id=$1")
        .bind(reset.body["challenge_id"].as_str().unwrap().parse::<Uuid>().unwrap()).fetch_one(f.state.database.pool()).await.unwrap();
    assert!(consumed);

    assert_eq!(
        f.request("GET", "/api/v1/auth/me", Some(&cookies), Value::Null)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let login = f.request("POST", "/api/v1/auth/login", None,
        json!({"email":email,"password":"replacement-mail-password-123","verification":f.proof("login").await})).await;
    assert_eq!(login.status, StatusCode::OK, "{}", login.body);
    let unknown = f.request("POST", "/api/v1/auth/email-code", None,
        json!({"email":"unknown@security.test","action":"password_reset","verification":f.proof("password_reset").await})).await;
    assert_eq!(unknown.status, StatusCode::OK);
    assert_eq!(unknown.body["message"], reset.body["message"]);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), messages.recv())
            .await
            .is_err()
    );
    mail_task.abort();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn administrator_enrollment_is_restricted_and_session_revocation_is_owner_scoped() {
    let f = Fixture::new(true).await;
    f.state.config.write().await.security.require_admin_mfa = true;
    let enrollment = f.login(true).await;
    assert_eq!(enrollment.body["enrollment_required"], true);
    let restricted = test_security::cookie_header(&enrollment.headers);
    assert_eq!(
        f.request("GET", "/api/v1/admin/users", Some(&restricted), Value::Null)
            .await
            .body["code"],
        "mfa_enrollment_required"
    );
    assert_eq!(
        f.request(
            "GET",
            "/api/v1/auth/security",
            Some(&restricted),
            Value::Null
        )
        .await
        .status,
        StatusCode::OK
    );
    f.state.config.write().await.security.require_admin_mfa = false;
    let first = f.login(true).await;
    let old = test_security::cookie_header(&first.headers);
    f.state.config.write().await.security.require_admin_mfa = true;
    assert_eq!(
        f.request("GET", "/api/v1/admin/users", Some(&old), Value::Null)
            .await
            .body["code"],
        "mfa_enrollment_required"
    );
    assert_eq!(
        f.request("GET", "/api/v1/auth/me", Some(&old), Value::Null)
            .await
            .body["mfa_enrollment_required"],
        true
    );
    f.state.config.write().await.security.require_admin_mfa = false;
    let current = test_security::cookie_header(&first.headers);
    let other = f.login(false).await;
    let other_cookie = test_security::cookie_header(&other.headers);
    let outsider = f
        .state
        .database
        .create_user(
            "outsider",
            "outsider@security.test",
            "outsider-password-123",
            None,
            false,
        )
        .await
        .unwrap();
    let auth = f.state.config.read().await.auth.clone();
    let outsider_grant = f
        .state
        .database
        .issue_grant(
            outsider,
            "ostool-cli",
            "board:operate offline_access",
            &auth,
        )
        .await
        .unwrap();
    assert_eq!(
        f.request(
            "DELETE",
            &format!(
                "/api/v1/auth/security/sessions/{}",
                outsider_grant.session_id
            ),
            Some(&current),
            Value::Null
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        f.request(
            "POST",
            "/api/v1/auth/security/sessions/others",
            Some(&current),
            Value::Null
        )
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        f.request("GET", "/api/v1/auth/me", Some(&current), Value::Null)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        f.request("GET", "/api/v1/auth/me", Some(&other_cookie), Value::Null)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert!(
        f.state
            .database
            .authenticate_token(&outsider_grant.access_token)
            .await
            .unwrap()
            .is_some()
    );
    f.state.config.write().await.auth.public_base_url = Some("https://boards.example.test".into());
    let secure = f.login(true).await;
    for cookie in secure.headers.get_all("set-cookie") {
        let cookie = cookie.to_str().unwrap();
        assert!(cookie.starts_with("__Host-ostool-admin-"));
        assert!(cookie.contains("; Secure"));
        assert!(!cookie.contains("Domain="));
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn legal_publication_consent_and_configuration_are_atomic_and_versioned() {
    let f = Fixture::new(true).await;
    let logged = f.login(true).await;
    let cookies = test_security::cookie_header(&logged.headers);
    let initial = f.request("GET", "/api/v1/site", None, Value::Null).await;
    assert_eq!(initial.status, StatusCode::OK);
    let mut settings = initial.body["settings"].clone();
    settings["name"] = json!("安全测试站点");
    settings["terms_body"] =
        json!("新用户协议\n禁止未授权访问设备。\n<script>not executable</script>");
    let updated = f
        .request(
            "PUT",
            "/api/v1/admin/site-settings",
            Some(&cookies),
            json!({"revision":initial.body["revision"],"settings":settings}),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.body);
    let stale = f
        .request(
            "PUT",
            "/api/v1/admin/site-settings",
            Some(&cookies),
            json!({"revision":initial.body["revision"],"settings":settings}),
        )
        .await;
    assert_eq!(stale.status, StatusCode::CONFLICT);
    let current = f.request("GET", "/api/v1/site", None, Value::Null).await;
    assert_eq!(current.body["terms_version"], 2);
    assert_eq!(current.body["privacy_version"], 1);
    let old = f
        .request("GET", "/api/v1/site/legal/terms/1", None, Value::Null)
        .await;
    assert_eq!(old.body["body"], initial.body["settings"]["terms_body"]);
    assert!(
        sqlx::query(
            "UPDATE legal_document_versions SET body='tampered' WHERE kind='terms' AND version=1"
        )
        .execute(f.state.database.pool())
        .await
        .is_err()
    );

    let mut register = json!({"email":"consent@security.test","password":"consent-password-123",
        "verification":f.proof("register").await,"legal":{"accepted":false,"terms_version":2,"privacy_version":1}});
    assert_eq!(
        f.request("POST", "/api/v1/auth/register", None, register.clone())
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    register["legal"]["accepted"] = json!(true);
    register["legal"]["terms_version"] = json!(1);
    register["verification"] = f.proof("register").await;
    let expired = f
        .request("POST", "/api/v1/auth/register", None, register.clone())
        .await;
    assert_eq!(expired.body["code"], "legal_changed");
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM users WHERE email='consent@security.test'")
            .fetch_one(f.state.database.pool())
            .await
            .unwrap();
    assert_eq!(count, 0);
    register["legal"]["terms_version"] = json!(2);
    register["verification"] = f.proof("register").await;
    let created = f
        .request("POST", "/api/v1/auth/register", None, register)
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let proofs:i64=sqlx::query_scalar("SELECT count(*) FROM user_legal_acceptances a JOIN users u ON u.id=a.user_id WHERE u.email='consent@security.test' AND length(ip_hash)=64 AND length(user_agent_hash)=64").fetch_one(f.state.database.pool()).await.unwrap();
    assert_eq!(proofs, 2);
    let metadata:bool=sqlx::query_scalar("SELECT last_login_at IS NOT NULL AND updated_at>=created_at FROM users WHERE email='consent@security.test'").fetch_one(f.state.database.pool()).await.unwrap();
    assert!(metadata);
    settings["support_url"] = json!("javascript:alert(1)");
    let invalid = f
        .request(
            "PUT",
            "/api/v1/admin/site-settings",
            Some(&cookies),
            json!({"revision":current.body["revision"],"settings":settings}),
        )
        .await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn public_site_stream_cannot_expose_management_or_identity_data() {
    use futures_util::StreamExt;
    let f = Fixture::new(true).await;
    for disabled in [false, true] {
        let board = ostool_server::BoardConfig {
            reset_command: None,
            id: if disabled {
                "disabled-device"
            } else {
                "enabled-device"
            }
            .into(),
            board_type: "public-test".into(),
            tags: vec!["arm64".into()],
            serial: None,
            power_management: ostool_server::PowerManagementConfig::Custom(
                ostool_server::CustomPowerManagement {
                    power_on_cmd: "private-power-credential".into(),
                    power_off_cmd: "private-power-command".into(),
                },
            ),
            boot: ostool_server::BootConfig::Pxe(Default::default()),
            network_identity: None,
            notes: Some("private-operator-notes".into()),
            disabled,
        };
        ostool_server::persistence::BoardStore::new(f.state.database.clone())
            .write_board(&board)
            .await
            .unwrap();
        f.state.boards.write().await.insert(board.id.clone(), board);
    }
    f.state.admin_events.invalidate(&["boards"]);
    let catalog = f
        .request("GET", "/api/v1/site/devices", None, Value::Null)
        .await;
    assert_eq!(catalog.status, StatusCode::OK);
    let rows = catalog.body.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    for row in rows {
        assert_eq!(row.as_object().unwrap().len(), 4);
        assert_eq!(row["board_type"], "public-test");
        assert_eq!(row["tags"], json!(["arm64"]));
    }
    assert!(rows.iter().any(|row| row["disabled"] == true));
    // Wait on one SSE subscription until the device projection is published.
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/site/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let mut changes = response.into_body().into_data_stream();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(chunk) = changes.next().await {
            let chunk = chunk.unwrap();
            let text = String::from_utf8_lossy(&chunk);
            if text.contains("enabled-device") && text.contains("disabled-device") {
                return;
            }
        }
        panic!("public device stream ended");
    })
    .await
    .unwrap();
    drop(changes);

    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/site/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    let mut data = serde_json::Map::new();
    let mut text = String::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(chunk) = stream.next().await {
            let chunk = String::from_utf8(chunk.unwrap().to_vec()).unwrap();
            text.push_str(&chunk);
            for line in chunk.lines().filter_map(|line| line.strip_prefix("data: ")) {
                let body: Value = serde_json::from_str(line).unwrap();
                if let Some(values) = body["data"].as_object() {
                    data.extend(values.clone());
                }
            }
            if data.contains_key("site") && data.contains_key("device_catalog") {
                return;
            }
        }
        panic!("public stream ended before publishing both resources");
    })
    .await
    .unwrap();
    assert_eq!(data.len(), 2);
    assert_eq!(data["device_catalog"], catalog.body);
    assert!(!text.contains("private-power"));
    assert!(!text.contains("private-operator"));
    assert!(!text.contains("password_hash"));
    assert!(!text.contains("mail_deliveries"));
    assert!(!text.contains("admin@security.test"));
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn redis_windows_are_shared_atomic_expiring_and_fail_closed() {
    let f = Fixture::new(true).await;
    let first = f.state.security_cache.as_ref().unwrap().clone();
    let second =
        ostool_server::security::cache::SecurityCache::connect(&*f.state.config.read().await)
            .await
            .unwrap();
    let key = Uuid::new_v4().to_string();
    let mut operations = Vec::new();
    for n in 0..30 {
        let cache = if n % 2 == 0 {
            first.clone()
        } else {
            second.clone()
        };
        let key = key.clone();
        operations.push(tokio::spawn(async move {
            cache.limit("parallel-test", &key, 5, 1).await.is_ok()
        }));
    }
    let mut allowed = 0;
    for operation in operations {
        allowed += usize::from(operation.await.unwrap());
    }
    assert_eq!(allowed, 5);
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert!(second.limit("parallel-test", &key, 5, 1).await.is_ok());
    assert_eq!(
        ostool_server::security::store::limit(&None, "failure", "test", 1, 60)
            .await
            .unwrap_err()
            .status,
        StatusCode::SERVICE_UNAVAILABLE
    );
    let mut config = f.state.config.read().await.clone();
    config.security.redis_url = None;
    assert!(
        ostool_server::security::cache::SecurityCache::connect(&config)
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn hardware_access_rejects_external_and_invalid_proxy_sources() {
    let f = Fixture::new(true).await;
    // Direct public sources cannot claim loopback through an untrusted XFF.
    for path in [
        "/api/v1/loaders/poll",
        "/api/v1/loader-updates/missing/image",
    ] {
        let method = if path.ends_with("poll") {
            "POST"
        } else {
            "GET"
        };
        assert_eq!(
            f.request(method, path, None, Value::Null).await.status,
            StatusCode::FORBIDDEN
        );
    }
    let proxy = "127.0.0.1".parse::<std::net::IpAddr>().unwrap();
    f.state.config.write().await.audit.trusted_proxies = vec![proxy];
    for forwarded in [
        None,
        Some("invalid, 203.0.113.10"),
        Some("203.0.113.10"),
        Some(""),
    ] {
        for (method, path, expected) in [
            ("POST", "/api/v1/loaders/poll", StatusCode::FORBIDDEN),
            (
                "GET",
                "/api/v1/loader-updates/missing/image",
                StatusCode::FORBIDDEN,
            ),
            (
                "GET",
                "/share/sessions/missing/probe",
                StatusCode::UNAUTHORIZED,
            ),
            (
                "GET",
                "/boot/sessions/missing/probe",
                StatusCode::UNAUTHORIZED,
            ),
        ] {
            let mut request = Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json");
            if let Some(value) = forwarded {
                request = request.header("x-forwarded-for", value);
            }
            let mut request = request.body(Body::from("{}")).unwrap();
            request
                .extensions_mut()
                .insert(ConnectInfo(std::net::SocketAddr::new(proxy, 40000)));
            assert_eq!(
                f.app.clone().oneshot(request).await.unwrap().status(),
                expected,
                "{path} {forwarded:?}"
            );
        }
    }
    // The public user listener never grants device access based on a private source alone.
    let mut request = Request::builder()
        .uri("/api/v1/loader-updates/missing/image")
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "192.168.20.10:40000"
            .parse::<std::net::SocketAddr>()
            .unwrap(),
    ));
    assert_eq!(
        f.app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn account_settings_are_validated_private_versioned_persistent_and_enforced() {
    let f = Fixture::new(true).await;
    let endpoint = "/api/v1/admin/account-settings";
    assert_eq!(
        f.request("GET", endpoint, None, Value::Null).await.status,
        StatusCode::UNAUTHORIZED
    );
    let login = f.login(true).await;
    let cookies = test_security::cookie_header(&login.headers);
    let initial = f
        .request("GET", endpoint, Some(&cookies), Value::Null)
        .await;
    assert_eq!(initial.status, StatusCode::OK);
    let mut settings = initial.body["settings"].clone();
    assert!(settings.get("smtp_password").is_none());
    assert!(settings.get("mfa_encryption_key").is_none());
    assert!(settings.get("redis_url").is_none());
    assert!(settings.get("turnstile_secret").is_none());
    settings["login_max_failures"] = json!(2);
    let invalid = f
        .request(
            "PUT",
            endpoint,
            Some(&cookies),
            json!({"revision":1,"settings":settings}),
        )
        .await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
    assert_eq!(f.state.config.read().await.security.login_max_failures, 6);
    f.state.config.write().await.security.smtp_password = Some("deploy-smtp-secret".into());
    settings["smtp_username"] = json!("smtp-user");
    settings["smtp_host"] = json!("smtp.example.test");
    settings["smtp_from"] = json!("noreply@example.test");
    settings["login_max_failures"] = json!(3);
    settings["allow_registration"] = json!(false);
    settings["password_reset_enabled"] = json!(false);
    settings["max_active_sessions"] = json!(1);
    settings["email_policy"]["max_attempts"] = json!(2);
    let saved = f
        .request(
            "PUT",
            endpoint,
            Some(&cookies),
            json!({"revision":1,"settings":settings}),
        )
        .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.body);
    assert_eq!(saved.body["revision"], 2);
    assert_eq!(
        f.request(
            "PUT",
            endpoint,
            Some(&cookies),
            json!({"revision":1,"settings":settings})
        )
        .await
        .status,
        StatusCode::CONFLICT
    );
    let site = f.request("GET", "/api/v1/site", None, Value::Null).await;
    assert_eq!(site.body["auth"]["allow_registration"], false);
    assert_eq!(site.body["auth"]["password_reset_available"], false);
    assert!(site.body.get("account_settings").is_none());
    let rejected=f.request("POST","/api/v1/auth/register",None,json!({"email":"closed@security.test","password":"closed-registration-password-123","verification":f.proof("register").await})).await;
    assert_eq!(rejected.status, StatusCode::FORBIDDEN);
    assert_eq!(f.request("POST", "/api/v1/auth/password-reset", None, json!({"challenge_id":Uuid::new_v4(),"code":"123456","new_password":"replacement-password-123"})).await.status, StatusCode::FORBIDDEN);
    let mut reloaded = f.state.config.read().await.clone();
    reloaded.auth.allow_registration = true;
    reloaded.auth.max_active_sessions = 10;
    reloaded.security.login_max_failures = 6;
    // 部署环境只提供密码，用户名等非密钥字段由数据库恢复。
    reloaded.security.smtp_username = None;
    let env_file = f._root.path().join("runtime.env");
    std::fs::write(
        &env_file,
        "OSTOOL_EDITION=public\nOSTOOL_SMTP_PASSWORD=deploy-smtp-secret\n",
    )
    .unwrap();
    ostool_server::environment::apply(&mut reloaded, Some(&env_file)).unwrap();
    ostool_server::api::site::load_account_settings(&f.state.database, &mut reloaded)
        .await
        .unwrap();
    assert!(!reloaded.auth.allow_registration);
    assert_eq!(reloaded.auth.max_active_sessions, 1);
    assert_eq!(reloaded.security.login_max_failures, 3);
    assert_eq!(
        reloaded.security.smtp_username.as_deref(),
        Some("smtp-user")
    );
    assert_eq!(
        reloaded.security.smtp_password.as_deref(),
        Some("deploy-smtp-secret")
    );
    let persisted: Value =
        sqlx::query_scalar("SELECT payload FROM server_records WHERE name='account-settings'")
            .fetch_one(f.state.database.pool())
            .await
            .unwrap();
    assert!(persisted["settings"].get("smtp_password").is_none());
    assert!(!persisted.to_string().contains("deploy-smtp-secret"));
    let local = Fixture::new(false).await;
    let mut invalid = reloaded.clone();
    invalid.security.smtp_username = None;
    assert!(
        ostool_server::api::site::load_account_settings(&local.state.database, &mut invalid)
            .await
            .is_err()
    );
    assert!(
        local
            .state
            .database
            .record::<Value>("account-settings")
            .await
            .unwrap()
            .is_none()
    );
    // 新登录会撤销超额旧登录链；网站和后台按客户端隔离计数。
    let next = f.login(true).await;
    assert_eq!(next.status, StatusCode::OK);
    assert_eq!(
        f.request("GET", endpoint, Some(&cookies), Value::Null)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let current = test_security::cookie_header(&next.headers);
    let user = f
        .state
        .database
        .find_user("admin@security.test")
        .await
        .unwrap()
        .unwrap();
    let proof = Uuid::new_v4();
    sqlx::query("INSERT INTO email_verification_tokens(id,purpose,user_id,email_hash,code_hash,security_version,expires_at) VALUES($1,'email:verify',$2,$3,$4,$5,now()+interval '10 minutes')")
        .bind(proof).bind(user.id).bind(crypto::hash(&user.email)).bind(crypto::proof_hash("email:verify",proof,"123456")).bind(user.security_version).execute(f.state.database.pool()).await.unwrap();
    for _ in 0..2 {
        assert_eq!(
            f.request(
                "POST",
                "/api/v1/auth/security/email/confirm",
                Some(&current),
                json!({"challenge_id":proof,"code":"000000"})
            )
            .await
            .status,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        f.request(
            "POST",
            "/api/v1/auth/security/email/confirm",
            Some(&current),
            json!({"challenge_id":proof,"code":"123456"})
        )
        .await
        .status,
        StatusCode::BAD_REQUEST
    );
    let invalidated: bool = sqlx::query_scalar(
        "SELECT invalidated_at IS NOT NULL FROM email_verification_tokens WHERE id=$1",
    )
    .bind(proof)
    .fetch_one(f.state.database.pool())
    .await
    .unwrap();
    assert!(invalidated);
    // 强制管理员 MFA 时，已建立但未绑定验证器的后台推送也必须停止。
    use futures_util::StreamExt;
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/admin/events")
                .header("cookie", &current)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    let snapshot = stream.next().await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&snapshot).contains("event: snapshot"));
    settings["require_admin_mfa"] = json!(true);
    assert_eq!(
        f.request(
            "PUT",
            endpoint,
            Some(&current),
            json!({"revision":2,"settings":settings})
        )
        .await
        .status,
        StatusCode::OK
    );
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        while let Some(chunk) = stream.next().await {
            if String::from_utf8_lossy(&chunk.unwrap()).contains("event: auth-expired") {
                return;
            }
        }
        panic!("administrator event stream did not enforce mandatory MFA");
    })
    .await
    .unwrap();
    assert_eq!(
        Fixture::new(false)
            .await
            .request("GET", endpoint, None, Value::Null)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn explicit_origins_preserve_auth_and_reject_unlisted_websites() {
    let mut f = Fixture::new(true).await;
    f.origin = "https://192.168.20.10:3443".into();
    // The same configuration supports localhost, LAN addresses and extra domains.
    {
        let mut config = f.state.config.write().await;
        config.auth.public_base_url = Some(f.origin.clone());
        config.auth.allowed_origins = vec![
            "https://127.0.0.1:3443".into(),
            "https://boards.example.com".into(),
        ];
        config.validate().unwrap();
    }
    for (origin, expected) in [
        ("https://192.168.20.10:3443", StatusCode::OK),
        ("https://127.0.0.1:3443", StatusCode::OK),
        ("https://boards.example.com", StatusCode::OK),
        ("http://192.168.20.10:4190", StatusCode::FORBIDDEN),
        ("http://localhost:4190/path", StatusCode::FORBIDDEN),
        ("http://localhost:4191", StatusCode::FORBIDDEN),
        ("http://attacker.example", StatusCode::FORBIDDEN),
    ] {
        let proof = f.proof("admin_login").await;
        let response = f.custom(
            "POST", "/api/v1/admin/auth/login", None,
            json!({"email":"admin@security.test","password":"security-admin-password-123","verification":proof}),
            Some(origin), None,
        ).await;
        assert_eq!(response.status, expected, "{origin}");
        if expected == StatusCode::OK {
            assert!(response.headers.get_all("set-cookie").iter().any(|cookie| {
                let cookie = cookie.to_str().unwrap();
                cookie.starts_with("__Host-")
                    && cookie.contains("Secure")
                    && cookie.contains("HttpOnly")
            }));
        }
    }
    assert_eq!(
        f.request("GET", "/api/v1/admin/users", None, Value::Null)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn durable_browser_sessions_survive_server_state_recreation_and_refresh() {
    let f = Fixture::new(true).await;
    let website_login = f.login(false).await;
    let admin_login = f.login(true).await;
    assert_eq!(website_login.status, StatusCode::OK);
    assert_eq!(admin_login.status, StatusCode::OK);
    let website_cookie = test_security::cookie_header(&website_login.headers);
    let admin_cookie = test_security::cookie_header(&admin_login.headers);
    let config = f.state.config.read().await.clone();
    let path = f._root.path().join("config.toml");
    drop(f.app);
    drop(f.state);
    let state = build_app_state(path, config.clone(), build_tftp_manager(&config.tftp))
        .await
        .unwrap();
    let restarted = Fixture {
        app: build_router(state.clone()),
        state,
        origin: f.origin,
        _root: f._root,
    };
    for cookie in [&website_cookie, &admin_cookie] {
        assert_eq!(
            restarted
                .request("GET", "/api/v1/auth/me", Some(cookie), Value::Null)
                .await
                .status,
            StatusCode::OK
        );
    }
    assert_eq!(
        restarted
            .request(
                "GET",
                "/api/v1/admin/users",
                Some(&admin_cookie),
                Value::Null
            )
            .await
            .status,
        StatusCode::OK
    );
    sqlx::query("UPDATE oauth_sessions SET access_expires_at=now()-interval '1 minute'")
        .execute(restarted.state.database.pool())
        .await
        .unwrap();
    for cookie in [&website_cookie, &admin_cookie] {
        let refresh = restarted
            .request("POST", "/api/v1/auth/refresh", Some(cookie), Value::Null)
            .await;
        assert_eq!(refresh.status, StatusCode::OK, "{}", refresh.body);
        let renewed = test_security::cookie_header(&refresh.headers);
        assert_eq!(
            restarted
                .request("GET", "/api/v1/auth/me", Some(&renewed), Value::Null)
                .await
                .status,
            StatusCode::OK
        );
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn unavailable_session_storage_closes_stream_without_expiring_credentials() {
    use futures_util::StreamExt;
    let f = Fixture::new(true).await;
    let login = f.login(true).await;
    assert_eq!(login.status, StatusCode::OK);
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/admin/events")
                .header("cookie", test_security::cookie_header(&login.headers))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    f.state.database.pool().close().await;
    let mut stream = response.into_body().into_data_stream();
    let first = tokio::time::timeout(std::time::Duration::from_secs(3), stream.next())
        .await
        .unwrap();
    assert!(
        first.is_none(),
        "a storage outage must close the stream, not publish auth-expired"
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn global_usernames_are_required_unique_and_support_both_login_identifiers() {
    let f = Fixture::new(true).await;
    let password = "username-test-password-123";
    let registration = |username: &str, email: &str, verification: Value| json!({"username":username,"email":email,"password":password,"verification":verification});
    let first = f
        .request(
            "POST",
            "/api/v1/auth/register",
            None,
            registration("Alice_01", "alice@identity.test", f.proof("register").await),
        )
        .await;
    assert_eq!(first.status, StatusCode::CREATED, "{}", first.body);
    let cookie = test_security::cookie_header(&first.headers);
    let me = f
        .request("GET", "/api/v1/auth/me", Some(&cookie), Value::Null)
        .await;
    assert_eq!(me.body["username"], "alice_01");
    let duplicate = f
        .request(
            "POST",
            "/api/v1/auth/register",
            None,
            registration("ALICE_01", "other@identity.test", f.proof("register").await),
        )
        .await;
    assert_eq!(duplicate.status, StatusCode::CONFLICT);
    for username in ["", "ab", "bad name", "bad@email"] {
        assert_eq!(
            f.request(
                "POST",
                "/api/v1/auth/register",
                None,
                registration(username, "invalid@identity.test", f.proof("register").await)
            )
            .await
            .status,
            StatusCode::BAD_REQUEST
        );
    }
    for identifier in ["ALICE_01", "alice@identity.test"] {
        let result = f.request("POST", "/api/v1/auth/login", None, json!({"email":identifier,"password":password,"verification":f.proof("login").await})).await;
        assert_eq!(result.status, StatusCode::OK, "{}", result.body);
    }
    let db = &f.state.database;
    assert_eq!(
        db.login_subject("ALICE_01").await.unwrap(),
        db.login_subject("alice@identity.test").await.unwrap()
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn username_and_email_share_login_failure_limits() {
    let f = Fixture::new(true).await;
    for index in 0..7 {
        let identifier = if index % 2 == 0 {
            "admin"
        } else {
            "admin@security.test"
        };
        let response = f.request("POST", "/api/v1/auth/login", None, json!({"email":identifier,"password":"incorrect-password-123","verification":f.proof("login").await})).await;
        assert_eq!(
            response.status,
            if index < 6 {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::TOO_MANY_REQUESTS
            }
        );
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn page_bootstrap_is_safe_and_available_without_waiting_for_database_queries() {
    let f = Fixture::new(true).await;
    f.state.database.pool().close().await;
    let response = f
        .app
        .clone()
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = String::from_utf8(
        to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(html.contains("id=\"ostool-bootstrap\""));
    assert!(html.contains("\"edition\":\"public\""));
    assert!(!html.contains("security-admin-password-123"));
    assert!(!html.contains("mfa_encryption_key"));
    assert!(!html.contains("redis_url"));
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn username_edits_revoke_credentials_but_display_name_edits_do_not() {
    let f = Fixture::new(true).await;
    let login = f.login(true).await;
    let cookie = test_security::cookie_header(&login.headers);
    let me = f
        .request("GET", "/api/v1/auth/me", Some(&cookie), Value::Null)
        .await;
    let endpoint = format!("/api/v1/admin/users/{}", me.body["id"].as_str().unwrap());
    let mut input = json!({"username":"admin","email":"admin@security.test","display_name":"Renamed display","role_id":"admin","disabled":false});
    assert_eq!(
        f.request("PUT", &endpoint, Some(&cookie), input.clone())
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        f.request("GET", "/api/v1/auth/me", Some(&cookie), Value::Null)
            .await
            .status,
        StatusCode::OK
    );
    input["username"] = json!("renamed-engineer");
    assert_eq!(
        f.request("PUT", &endpoint, Some(&cookie), input)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        f.request("GET", "/api/v1/auth/me", Some(&cookie), Value::Null)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let login = f.request("POST", "/api/v1/admin/auth/login", None, json!({"email":"RENAMED-ENGINEER","password":"security-admin-password-123","verification":f.proof("admin_login").await})).await;
    assert_eq!(login.status, StatusCode::OK);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn settings_secrets_are_encrypted_write_only_versioned_and_recoverable() {
    let f = Fixture::new(true).await;
    let login = f.login(true).await;
    let cookies = test_security::cookie_header(&login.headers);
    let endpoint = "/api/v1/admin/account-settings";
    let mut view = f
        .request("GET", endpoint, Some(&cookies), Value::Null)
        .await
        .body;
    view["settings"]["smtp_username"] = json!("smtp-account");
    view["settings"]["smtp_host"] = json!("smtp.example.test");
    view["settings"]["smtp_from"] = json!("noreply@example.test");
    let saved=f.request("PUT",endpoint,Some(&cookies),json!({"revision":view["revision"],"settings":view["settings"],"secret_updates":{"smtp_password":"private-smtp-password","turnstile_secret":"private-turnstile-key"}})).await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.body);
    let payload = f
        .state
        .database
        .record::<Value>("account-settings")
        .await
        .unwrap()
        .unwrap();
    let text = payload.to_string();
    assert!(!text.contains("private-smtp-password"));
    assert!(!text.contains("private-turnstile-key"));
    assert!(
        payload["secrets"]["smtp_password"]
            .as_str()
            .unwrap()
            .starts_with("v1:")
    );
    let mut restarted = f.state.config.read().await.clone();
    restarted.security.smtp_password = None;
    restarted.security.turnstile_secret = None;
    ostool_server::api::site::load_account_settings(&f.state.database, &mut restarted)
        .await
        .unwrap();
    assert_eq!(
        restarted.security.smtp_password.as_deref(),
        Some("private-smtp-password")
    );
    assert_eq!(
        restarted.security.turnstile_secret.as_deref(),
        Some("private-turnstile-key")
    );
    let mut wrong_key = restarted.clone();
    wrong_key.security.mfa_encryption_key = Some(STANDARD.encode([8u8; 32]));
    assert!(
        ostool_server::api::site::load_account_settings(&f.state.database, &mut wrong_key)
            .await
            .is_err()
    );
    view = f
        .request("GET", endpoint, Some(&cookies), Value::Null)
        .await
        .body;
    assert_eq!(view["secrets"]["smtp_password"], true);
    assert_eq!(view["secrets"]["turnstile_secret"], true);
    assert!(!view.to_string().contains("private-smtp-password"));
    let unchanged=f.request("PUT",endpoint,Some(&cookies),json!({"revision":view["revision"],"settings":view["settings"],"secret_updates":{"smtp_password":""}})).await;
    assert_eq!(unchanged.status, StatusCode::OK);
    assert_eq!(
        f.state
            .config
            .read()
            .await
            .security
            .smtp_password
            .as_deref(),
        Some("private-smtp-password")
    );
    assert_eq!(f.request("PUT",endpoint,Some(&cookies),json!({"revision":view["revision"],"settings":view["settings"],"secret_updates":{"smtp_password":"stale-edit"}})).await.status,StatusCode::CONFLICT);
    view = f
        .request("GET", endpoint, Some(&cookies), Value::Null)
        .await
        .body;
    view["settings"]["smtp_username"] = Value::Null;
    let cleared=f.request("PUT",endpoint,Some(&cookies),json!({"revision":view["revision"],"settings":view["settings"],"secret_updates":{"clear_smtp_password":true,"clear_turnstile_secret":true}})).await;
    assert_eq!(cleared.status, StatusCode::OK, "{}", cleared.body);
    view = f
        .request("GET", endpoint, Some(&cookies), Value::Null)
        .await
        .body;
    assert_eq!(view["secrets"]["smtp_password"], false);
    assert_eq!(view["secrets"]["turnstile_secret"], false);
    let audit = sqlx::query_scalar::<_, String>("SELECT details::text FROM audit_events")
        .fetch_all(f.state.database.pool())
        .await
        .unwrap()
        .join(" ");
    assert!(!audit.contains("private-smtp-password"));
    assert!(!audit.contains("private-turnstile-key"));
    f.state.config.write().await.security.mfa_encryption_key = None;
    view["settings"]["smtp_username"] = json!("smtp-account");
    assert_eq!(f.request("PUT",endpoint,Some(&cookies),json!({"revision":view["revision"],"settings":view["settings"],"secret_updates":{"smtp_password":"must-not-store-without-key"}})).await.status,StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn management_pages_are_bounded_filtered_and_member_scoped() {
    use futures_util::StreamExt;
    let f = Fixture::new(true).await;
    let admin = f.login(true).await;
    let cookies = test_security::cookie_header(&admin.headers);
    let pool = f.state.database.pool();
    sqlx::query("INSERT INTO users(username,email,password_hash) SELECT 'page_user_'||lpad(n::text,3,'0'),'page_user_'||n||'@security.test',password_hash FROM users CROSS JOIN generate_series(1,150) n WHERE is_admin")
        .execute(pool).await.unwrap();
    f.state.admin_events.invalidate(&["users"]);
    let first = f
        .request(
            "GET",
            "/api/v1/admin/users?page_size=10",
            Some(&cookies),
            Value::Null,
        )
        .await;
    let second = f
        .request(
            "GET",
            "/api/v1/admin/users?page=2&page_size=10",
            Some(&cookies),
            Value::Null,
        )
        .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.body);
    assert_eq!(first.body["total"], 151);
    assert_eq!(first.body["items"].as_array().unwrap().len(), 10);
    assert_eq!(second.body["page"], 2);
    for row in first.body["items"].as_array().unwrap() {
        assert!(
            !second.body["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|other| other["id"] == row["id"])
        );
        assert!(row.get("password_hash").is_none());
        assert!(row.get("security_version").is_none());
    }
    let literal = f
        .request(
            "GET",
            "/api/v1/admin/users?q=%25",
            Some(&cookies),
            Value::Null,
        )
        .await;
    assert_eq!(literal.body["total"], 0);
    let search = f
        .request(
            "GET",
            "/api/v1/admin/users?q=page_user_00",
            Some(&cookies),
            Value::Null,
        )
        .await;
    assert_eq!(search.body["total"], 9);
    for query in [
        "page_size=101",
        "page=0",
        "status=unknown",
        "page=9223372036854775807",
    ] {
        assert_eq!(
            f.request(
                "GET",
                &format!("/api/v1/admin/users?{query}"),
                Some(&cookies),
                Value::Null
            )
            .await
            .status,
            StatusCode::BAD_REQUEST
        );
    }
    let owner: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE username='page_user_001'")
        .fetch_one(pool)
        .await
        .unwrap();
    let base = Utc::now() + chrono::Duration::days(2);
    sqlx::query("INSERT INTO board_rentals(board_id,user_id,starts_at,ends_at) SELECT 'page-board',$1,$2+n*interval '1 hour',$2+(n+1)*interval '1 hour' FROM generate_series(0,119) n")
        .bind(owner).bind(base).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO board_rentals(board_id,user_id,starts_at,ends_at) SELECT 'other-board',id,$1,$1+interval '1 hour' FROM users WHERE is_admin")
        .bind(base).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO rental_requests(board_id,user_id,starts_at,ends_at) SELECT board_id,user_id,starts_at,ends_at FROM board_rentals")
        .execute(pool).await.unwrap();
    f.state
        .admin_events
        .invalidate(&["rentals", "rental_requests"]);
    let rentals = f
        .request(
            "GET",
            "/api/v1/admin/rentals?page_size=10&status=scheduled",
            Some(&cookies),
            Value::Null,
        )
        .await;
    assert_eq!(rentals.body["total"], 121, "{}", rentals.body);
    assert_eq!(rentals.body["items"].as_array().unwrap().len(), 10);
    let requests = f
        .request(
            "GET",
            "/api/v1/admin/rental-requests?page_size=10&status=pending",
            Some(&cookies),
            Value::Null,
        )
        .await;
    assert_eq!(requests.body["total"], 121);
    let member=f.request("POST","/api/v1/auth/login",None,json!({"email":"page_user_001","password":"security-admin-password-123","verification":f.proof("login").await})).await;
    assert_eq!(member.status, StatusCode::OK, "{}", member.body);
    let member_cookie = test_security::cookie_header(&member.headers);
    for endpoint in ["rentals", "rental-requests"] {
        let page = f
            .request(
                "GET",
                &format!("/api/v1/account/{endpoint}?page=2&page_size=10"),
                Some(&member_cookie),
                Value::Null,
            )
            .await;
        assert_eq!(page.body["total"], 120, "{}", page.body);
        assert_eq!(page.body["items"].as_array().unwrap().len(), 10);
        assert!(
            page.body["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["user_id"] == owner.to_string())
        );
        let admin_id: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE is_admin")
            .fetch_one(pool)
            .await
            .unwrap();
        let scoped = f
            .request(
                "GET",
                &format!("/api/v1/account/{endpoint}?user={admin_id}"),
                Some(&member_cookie),
                Value::Null,
            )
            .await;
        assert_eq!(scoped.body["total"], 0);
    }
    let summary = f
        .request("GET", "/api/v1/account", Some(&member_cookie), Value::Null)
        .await;
    assert_eq!(summary.body["account_summary"]["scheduled"], 120);
    assert!(summary.body.get("my_rentals").is_none());
    assert!(summary.body.get("my_requests").is_none());
    let context = ostool_server::api::listing::editor(
        &f.state,
        &ostool_server::api::listing::EditorFilters {
            board: "page-board".into(),
            user: Some(owner),
            from: Some(base - chrono::Duration::minutes(1)),
            to: Some(base + chrono::Duration::hours(1)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(context["bookings"].as_array().unwrap().len(), 1);
    assert_eq!(context["users"].as_array().unwrap().len(), 50);
    assert_eq!(context["more_users"], true);
    assert_eq!(context["users"][0]["id"], owner.to_string());
    assert!(context["users"][0].get("password_hash").is_none());
    let response=f.app.clone().oneshot(Request::builder()
        .uri("/api/v1/admin/events?surface=admin&topics=listing,users&list_view=users&list_page_size=10")
        .header("cookie",&cookies).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let frame = String::from_utf8(frame.to_vec()).unwrap();
    let event: Value = serde_json::from_str(
        frame
            .lines()
            .find_map(|line| line.strip_prefix("data: "))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(event["data"]["listing"]["total"], 151);
    assert!(
        event["data"]
            .as_object()
            .unwrap()
            .keys()
            .all(|key| key == "listing" || key == "users")
    );
    if let Some(marker) = event["data"].get("users") {
        assert!(marker.get("revision").is_some());
    }
    assert!(frame.len() < 30000);
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn reserved_names_are_enforced_at_registration_without_creating_partial_accounts() {
    let f = Fixture::new(true).await;
    let mut safe = None;
    for (index, (username, name, code)) in [
        ("my_ADMIN", "研发成员", Some("reserved_username")),
        ("administrator01", "研发成员", Some("reserved_username")),
        ("a_d_m_i_n", "研发成员", Some("reserved_username")),
        (
            "engineer_one",
            "管\u{200b}理 员",
            Some("reserved_display_name"),
        ),
        ("engineer_two", "ＡＤＭＩＮ", Some("reserved_display_name")),
        ("kernel_engineer", "操作系统研发工程师", None),
    ]
    .into_iter()
    .enumerate()
    {
        let reply=f.request("POST","/api/v1/auth/register",None,json!({"username":username,"email":format!("person{index}@names.test"),"display_name":name,"password":"reserved-names-password-123","verification":f.proof("register").await})).await;
        if let Some(code) = code {
            assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.body);
            assert_eq!(reply.body["code"], code);
        } else {
            assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
            safe = Some(reply);
        }
    }
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM users WHERE email LIKE '%@names.test'")
            .fetch_one(f.state.database.pool())
            .await
            .unwrap();
    assert_eq!(count, 1);
    let accepted: i64 = sqlx::query_scalar("SELECT count(*) FROM user_legal_acceptances")
        .fetch_one(f.state.database.pool())
        .await
        .unwrap();
    assert_eq!(accepted, 2);
    let cookie = test_security::cookie_header(&safe.unwrap().headers);
    let changed = f
        .request(
            "PUT",
            "/api/v1/account/profile",
            Some(&cookie),
            json!({"email":"person5@names.test","display_name":"官方管理员"}),
        )
        .await;
    assert_eq!(changed.status, StatusCode::BAD_REQUEST);
    assert_eq!(changed.body["code"], "reserved_display_name");
    let name: Option<String> =
        sqlx::query_scalar("SELECT display_name FROM users WHERE username='kernel_engineer'")
            .fetch_one(f.state.database.pool())
            .await
            .unwrap();
    assert_eq!(name.as_deref(), Some("操作系统研发工程师"));
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn reserved_name_settings_apply_to_admin_writes_and_survive_restart_without_locking_existing_accounts()
 {
    let f = Fixture::new(true).await;
    let login = f.login(true).await;
    assert_eq!(login.status, StatusCode::OK);
    let cookie = test_security::cookie_header(&login.headers);
    let body = |username: &str, name: &str| json!({"username":username,"email":"engineer@names.test","display_name":name,"role_id":"member","disabled":false,"password":"reserved-names-password-123"});
    for (username, name, expected) in [
        ("new_admin", "工程师", "reserved_username"),
        ("new_engineer", "平台管理员", "reserved_display_name"),
    ] {
        let r = f
            .request(
                "POST",
                "/api/v1/admin/users",
                Some(&cookie),
                body(username, name),
            )
            .await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.body);
        assert_eq!(r.body["code"], expected);
    }
    let user = f
        .state
        .database
        .create_user(
            "existing_admin_name",
            "existing@names.test",
            "reserved-names-password-123",
            Some("平台管理员"),
            false,
        )
        .await
        .unwrap();
    let path = format!("/api/v1/admin/users/{}", user.id);
    let mut input = body("existing_admin_name", "平台管理员");
    input["email"] = json!("existing@names.test");
    input["password"] = Value::Null;
    assert_eq!(
        f.request("PUT", &path, Some(&cookie), input.clone())
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    input["username"] = json!("another_admin");
    assert_eq!(
        f.request("PUT", &path, Some(&cookie), input.clone())
            .await
            .body["code"],
        "reserved_username"
    );
    input["username"] = json!("existing_admin_name");
    input["display_name"] = json!("新的管理员");
    assert_eq!(
        f.request("PUT", &path, Some(&cookie), input.clone())
            .await
            .body["code"],
        "reserved_display_name"
    );
    let auth = f.state.config.read().await.auth.clone();
    let grant = f
        .state
        .database
        .issue_grant(user.clone(), "web", "profile", &auth)
        .await
        .unwrap();
    let unchanged = f
        .custom(
            "PUT",
            "/api/v1/account/profile",
            None,
            json!({"email":"existing@names.test","display_name":"平台管理员"}),
            Some(&f.origin),
            Some(&grant.access_token),
        )
        .await;
    assert_eq!(
        unchanged.status,
        StatusCode::NO_CONTENT,
        "{}",
        unchanged.body
    );
    let changed = f
        .custom(
            "PUT",
            "/api/v1/account/profile",
            None,
            json!({"email":"existing@names.test","display_name":"超级管理员"}),
            Some(&f.origin),
            Some(&grant.access_token),
        )
        .await;
    assert_eq!(changed.body["code"], "reserved_display_name");
    let endpoint = "/api/v1/admin/account-settings";
    let initial = f.request("GET", endpoint, Some(&cookie), Value::Null).await;
    let mut settings = initial.body["settings"].clone();
    settings["name_policy"] =
        json!({"username_keywords":["operator"],"display_name_keywords":["平台代表"]});
    let save = f
        .request(
            "PUT",
            endpoint,
            Some(&cookie),
            json!({"revision":initial.body["revision"],"settings":settings}),
        )
        .await;
    assert_eq!(save.status, StatusCode::OK, "{}", save.body);
    let rejected = f
        .request(
            "POST",
            "/api/v1/admin/users",
            Some(&cookie),
            body("operator_01", "普通工程师"),
        )
        .await;
    assert_eq!(rejected.body["code"], "reserved_username");
    let rejected = f
        .request(
            "POST",
            "/api/v1/admin/users",
            Some(&cookie),
            body("new_engineer", "平 台 代 表"),
        )
        .await;
    assert_eq!(rejected.body["code"], "reserved_display_name");
    // Replacing the lists is deliberate; former default words are no longer prohibited.
    let accepted = f
        .request(
            "POST",
            "/api/v1/admin/users",
            Some(&cookie),
            body("new_admin", "平台管理员"),
        )
        .await;
    assert_eq!(accepted.status, StatusCode::CREATED, "{}", accepted.body);
    let mut config = f.state.config.read().await.clone();
    config.security.name_policy = Default::default();
    ostool_server::api::site::load_account_settings(&f.state.database, &mut config)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&config.security.name_policy).unwrap(),
        settings["name_policy"]
    );
    let current = f.request("GET", endpoint, Some(&cookie), Value::Null).await;
    for bad in [
        json!({"username_keywords":[""],"display_name_keywords":[]}),
        json!({"username_keywords":["admin","ＡＤＭＩＮ"],"display_name_keywords":[]}),
    ] {
        let mut invalid = current.body["settings"].clone();
        invalid["name_policy"] = bad;
        assert_eq!(
            f.request(
                "PUT",
                endpoint,
                Some(&cookie),
                json!({"revision":current.body["revision"],"settings":invalid})
            )
            .await
            .status,
            StatusCode::BAD_REQUEST
        );
    }
    let unchanged = f.request("GET", endpoint, Some(&cookie), Value::Null).await;
    assert_eq!(unchanged.body["revision"], current.body["revision"]);
    assert_eq!(
        unchanged.body["settings"]["name_policy"],
        settings["name_policy"]
    );
    assert_eq!(f.login(true).await.status, StatusCode::OK);
    let site = f.request("GET", "/api/v1/site", None, Value::Null).await;
    assert!(!site.body.to_string().contains("username_keywords"));
}
