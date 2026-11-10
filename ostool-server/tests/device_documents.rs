//! Public documentation is durable, revisioned and isolated from operational secrets.
#[path = "support/database.rs"]
mod test_database;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use futures_util::StreamExt;
use ostool_server::{config::Edition, tftp::service::build_tftp_manager, *};
use serde_json::{Value, json};
use tower::ServiceExt;

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and Redis"]
async fn authored_documents_are_public_revisioned_and_follow_board_lifecycle() -> anyhow::Result<()>
{
    let root = tempfile::tempdir()?;
    let mut config = ServerConfig {
        edition: Edition::Public,
        data_dir: root.path().join("data"),
        ..Default::default()
    };
    config.tftp = TftpConfig::Builtin(BuiltinTftpConfig::default_with_root(
        root.path().join("cache"),
    ));
    let db = test_database::prepare(&root.path().join("server.env"), &mut config).await?;
    let admin = db
        .create_user(
            "doc_admin",
            "doc-admin@test.local",
            "document-password-123",
            None,
            true,
        )
        .await?;
    let member = db
        .create_user(
            "doc_member",
            "doc-member@test.local",
            "document-password-123",
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
    let app = build_router(state.clone());
    let call = |method: &str, path: &str, token: Option<&str>, body: Value| {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        app.clone()
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
    };
    let mut board = json!({"id":"doc-board","board_type":"arm64","tags":["lab"],"notes":"PRIVATE OPERATIONS","boot":{"kind":"uboot","use_tftp":false},"power_management":{"kind":"custom","power_on_cmd":"true # PRIVATE POWER","power_off_cmd":"true"}});
    assert_eq!(
        call(
            "POST",
            "/api/v1/admin/boards",
            Some(&admin.access_token),
            board.clone()
        )
        .await?
        .status(),
        StatusCode::CREATED
    );
    let path = "/api/v1/admin/device-documents/doc-board";
    assert_eq!(
        call("PUT", path, None, json!({})).await?.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call("PUT", path, Some(&member.access_token), json!({}))
            .await?
            .status(),
        StatusCode::FORBIDDEN
    );
    let document = json!({"title":"Public detail","summary":"A development board","description":"Public description\n<script>not executable</script>","image_url":"/assets/board.png","highlights":["UART development"],"specifications":[{"name":"Memory","value":"8 GiB"}]});
    let body = json!({"revision":0,"document":document});
    assert_eq!(
        call("PUT", path, Some(&admin.access_token), body.clone())
            .await?
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        call("PUT", path, Some(&admin.access_token), body)
            .await?
            .status(),
        StatusCode::CONFLICT
    );
    let response = call(
        "GET",
        "/api/v1/site/device-documents/doc-board",
        None,
        Value::Null,
    )
    .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await?;
    let view: Value = serde_json::from_slice(&bytes)?;
    assert_eq!(view["document"], document);
    assert_eq!(view["revision"], 1);
    assert!(!String::from_utf8_lossy(&bytes).contains("PRIVATE"));
    let mut invalid = document.clone();
    invalid["image_url"] = json!("javascript:alert(1)");
    assert_eq!(
        call(
            "PUT",
            path,
            Some(&admin.access_token),
            json!({"revision":1,"document":invalid})
        )
        .await?
        .status(),
        StatusCode::BAD_REQUEST
    );
    let mut invalid = document.clone();
    invalid["description"] = json!("x".repeat(16001));
    assert_eq!(
        call(
            "PUT",
            path,
            Some(&admin.access_token),
            json!({"revision":1,"document":invalid})
        )
        .await?
        .status(),
        StatusCode::BAD_REQUEST
    );
    let response = call(
        "GET",
        "/api/v1/site/events?device_document_id=doc-board&board_editor_id=doc-board",
        None,
        Value::Null,
    )
    .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            let text = String::from_utf8_lossy(&chunk);
            assert!(!text.contains("PRIVATE"));
            assert!(!text.contains("power_management"));
            if text.contains("device_document\"") && text.contains("Public detail") {
                return Ok::<_, anyhow::Error>(());
            }
        }
        anyhow::bail!("missing selected public document")
    })
    .await??;
    drop(stream);
    let reconnected = persistence::Database::connect(&config.database).await?;
    assert_eq!(
        reconnected
            .device_document("doc-board")
            .await?
            .document
            .title,
        "Public detail"
    );
    board["id"] = json!("renamed-doc-board");
    assert_eq!(
        call(
            "PUT",
            "/api/v1/admin/boards/doc-board",
            Some(&admin.access_token),
            board
        )
        .await?
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        db.device_document("renamed-doc-board")
            .await?
            .document
            .title,
        "Public detail"
    );
    assert_eq!(
        call(
            "GET",
            "/api/v1/site/device-documents/doc-board",
            None,
            Value::Null
        )
        .await?
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            "DELETE",
            "/api/v1/admin/boards/renamed-doc-board",
            Some(&admin.access_token),
            Value::Null
        )
        .await?
        .status(),
        StatusCode::NO_CONTENT
    );
    assert!(db.device_document_summaries().await?.is_empty());
    Ok(())
}
