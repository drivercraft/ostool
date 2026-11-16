use axum::{
    body::Body,
    extract::{Extension, Path, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use mime_guess::from_path;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "$OSTOOL_SERVER_WEB_DIST_DIR"]
struct WebUiAssets;

pub async fn serve_admin_index(
    State(state): State<crate::AppState>,
    Extension(nonce): Extension<crate::security::web::CspNonce>,
) -> Response {
    index_response("admin/index.html", &state, &nonce.0).await
}

pub async fn serve_admin_history(
    uri: Uri,
    state: State<crate::AppState>,
    nonce: Extension<crate::security::web::CspNonce>,
) -> Response {
    document_status(serve_admin_index(state, nonce).await, uri.path())
}

pub async fn serve_site_index(
    State(state): State<crate::AppState>,
    Extension(nonce): Extension<crate::security::web::CspNonce>,
) -> Response {
    index_response("index.html", &state, &nonce.0).await
}

pub async fn serve_site_history(
    uri: Uri,
    state: State<crate::AppState>,
    nonce: Extension<crate::security::web::CspNonce>,
) -> Response {
    document_status(serve_site_index(state, nonce).await, uri.path())
}

// Keep these document paths aligned with the two React routers. Resource IDs
// are resolved by the application; only an unknown page shape is a document 404.
fn is_document_path(path: &str) -> bool {
    let path = path.trim_end_matches('/');
    if path.is_empty() {
        return true;
    }
    let parts: Vec<_> = path.split('/').collect();
    match parts.as_slice() {
        [
            "",
            "" | "register" | "rental-request" | "password-reset" | "login" | "devices" | "docs"
            | "about" | "account" | "qcl",
        ] => true,
        ["", "legal", "terms" | "privacy"] => true,
        ["", "devices", id] if !id.is_empty() => true,
        [
            "",
            "account",
            "profile" | "security" | "password" | "rentals" | "requests" | "tokens",
        ] => true,
        ["", "account", "allocations" | "requests", "new"] => true,
        [
            "",
            "qcl",
            "login" | "overview" | "jobs" | "operations" | "audits" | "dashboard" | "boards"
            | "device-documents" | "dtbs" | "sessions" | "tftp" | "server" | "settings" | "users"
            | "account" | "rental-requests" | "rentals",
        ] => true,
        ["", "qcl", "boards" | "device-documents" | "rentals", id] if !id.is_empty() => true,
        ["", "qcl", "users", "roles"] => true,
        ["", "qcl", "users", "roles", id] if !id.is_empty() => true,
        _ => false,
    }
}

fn document_status(mut response: Response, path: &str) -> Response {
    if !is_document_path(path) {
        *response.status_mut() = StatusCode::NOT_FOUND;
    }
    response
}

pub(crate) fn is_browser_page_request(method: &Method, uri: &Uri, headers: &HeaderMap) -> bool {
    let path = uri.path();
    matches!(*method, Method::GET | Method::HEAD)
        && !matches!(
            path.split('/').nth(1),
            Some("api" | "assets" | "device-files" | "boot" | "share" | "oauth")
        )
        && headers
            .get(header::ACCEPT)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|accept| {
                accept
                    .split(',')
                    .any(|part| part.split(';').next().unwrap_or_default().trim() == "text/html")
            })
}

pub async fn serve_not_found(
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    state: State<crate::AppState>,
    nonce: Extension<crate::security::web::CspNonce>,
) -> Response {
    if is_browser_page_request(&method, &uri, &headers) {
        let mut response = serve_site_index(state, nonce).await;
        *response.status_mut() = StatusCode::NOT_FOUND;
        response
    } else {
        crate::api::error::ApiError::not_found("route not found").into_response()
    }
}

pub async fn serve_admin_asset(Path(path): Path<String>) -> Response {
    asset_response(format!("assets/{path}"), None, None)
}

#[cfg(test)]
pub(crate) fn first_asset_path() -> Option<String> {
    WebUiAssets::iter()
        .map(|name| name.to_string())
        .find(|name| name.starts_with("assets/"))
}

async fn index_response(path: &str, state: &crate::AppState, nonce: &str) -> Response {
    let config = crate::api::auth::public_config(&*state.config.read().await);
    asset_response(path.into(), Some(nonce), Some(config))
}

fn asset_response(
    path: String,
    nonce: Option<&str>,
    config: Option<serde_json::Value>,
) -> Response {
    let Some(content) = WebUiAssets::get(&path) else {
        return (StatusCode::NOT_FOUND, "asset not found").into_response();
    };

    let mime = from_path(&path).first_or_octet_stream();
    let bytes = if let Some(nonce) = nonce {
        let bootstrap = config.map(|value| {
            let json = value.to_string().replace('<', "\\u003c");
            format!("<script type=\"application/json\" id=\"ostool-bootstrap\" nonce=\"{nonce}\">{json}</script>")
        }).unwrap_or_default();
        String::from_utf8_lossy(&content.data)
            .replacen(
                "<head>",
                &format!("<head><meta name=\"csp-nonce\" content=\"{nonce}\">{bootstrap}"),
                1,
            )
            .into_bytes()
    } else {
        content.data.into_owned()
    };
    let mut response = Response::new(Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(mime.as_ref())
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if path == "index.html" || path == "admin/index.html" {
            "no-store"
        } else {
            "public, max-age=31536000, immutable"
        }),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_routes_distinguish_pages_from_unknown_paths() {
        for path in [
            "/",
            "/devices/demo",
            "/account/profile",
            "/account/allocations/new",
            "/account/requests/new",
            "/qcl/",
            "/qcl/login/",
            "/qcl/overview",
            "/qcl/boards/new",
            "/qcl/boards/demo",
            "/qcl/device-documents/demo",
            "/qcl/users/roles",
            "/qcl/users/roles/new",
            "/qcl/users/roles/demo",
            "/qcl/rentals/new",
            "/qcl/rentals/demo",
            "/qcl/settings/",
        ] {
            assert!(is_document_path(path), "{path}");
        }
        for path in [
            "/missing",
            "/admin",
            "/docs/missing",
            "/login/missing",
            "/about/missing",
            "/devices/demo/missing",
            "/account/missing",
            "/qcl/missing",
            "/qcl/boards/demo/missing",
            "/qcl/users/roles/demo/missing",
        ] {
            assert!(!is_document_path(path), "{path}");
        }
    }

    #[test]
    fn only_browser_page_requests_can_receive_the_website_fallback() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ACCEPT,
            HeaderValue::from_static("text/html,application/xhtml+xml"),
        );
        for method in [Method::GET, Method::HEAD] {
            assert!(is_browser_page_request(
                &method,
                &"/missing".parse().unwrap(),
                &headers
            ));
        }
        assert!(!is_browser_page_request(
            &Method::POST,
            &"/missing".parse().unwrap(),
            &headers
        ));
        for path in [
            "/api",
            "/api/v1/missing",
            "/assets/missing.js",
            "/device-files/missing",
            "/boot/missing",
            "/share/missing",
            "/oauth/missing",
        ] {
            assert!(
                !is_browser_page_request(&Method::GET, &path.parse().unwrap(), &headers),
                "{path}"
            );
        }
        headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        assert!(!is_browser_page_request(
            &Method::GET,
            &"/missing".parse().unwrap(),
            &headers
        ));
    }

    #[tokio::test]
    async fn missing_documents_preserve_entry_theme_bootstrap_and_security_headers() {
        for entry in ["index.html", "admin/index.html"] {
            let response = document_status(
                asset_response(
                    entry.into(),
                    Some("test-nonce"),
                    Some(serde_json::json!({"edition":"local"})),
                ),
                "/missing",
            );
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            assert_eq!(response.headers()[header::CONTENT_TYPE], "text/html");
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let body = String::from_utf8(body.to_vec()).unwrap();
            assert!(body.contains("test-nonce"));
            assert!(body.contains("ostool-bootstrap"));
            assert!(body.contains("/assets/"));
        }
        let response = asset_response("assets/does-not-exist.js".into(), None, None);
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_ne!(response.headers()[header::CONTENT_TYPE], "text/html");
    }
}
