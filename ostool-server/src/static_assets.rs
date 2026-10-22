use axum::{
    body::Body,
    extract::{Extension, Path, State},
    http::{HeaderValue, StatusCode, header},
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
    Path(_path): Path<String>,
    state: State<crate::AppState>,
    nonce: Extension<crate::security::web::CspNonce>,
) -> Response {
    serve_admin_index(state, nonce).await
}

pub async fn serve_site_index(
    State(state): State<crate::AppState>,
    Extension(nonce): Extension<crate::security::web::CspNonce>,
) -> Response {
    index_response("index.html", &state, &nonce.0).await
}

pub async fn serve_site_history(
    Path(_path): Path<String>,
    state: State<crate::AppState>,
    nonce: Extension<crate::security::web::CspNonce>,
) -> Response {
    serve_site_index(state, nonce).await
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
