use axum::{
    body::Body,
    extract::{Extension, Path},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use mime_guess::from_path;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "$OSTOOL_SERVER_WEB_DIST_DIR"]
struct WebUiAssets;

pub async fn serve_admin_index(
    Extension(nonce): Extension<crate::security::web::CspNonce>,
) -> Response {
    asset_response("admin/index.html".into(), Some(&nonce.0))
}

pub async fn serve_admin_history(
    Path(_path): Path<String>,
    nonce: Extension<crate::security::web::CspNonce>,
) -> Response {
    serve_admin_index(nonce).await
}

pub async fn serve_site_index(
    Extension(nonce): Extension<crate::security::web::CspNonce>,
) -> Response {
    asset_response("index.html".into(), Some(&nonce.0))
}

pub async fn serve_site_history(
    Path(_path): Path<String>,
    nonce: Extension<crate::security::web::CspNonce>,
) -> Response {
    serve_site_index(nonce).await
}

pub async fn serve_admin_asset(Path(path): Path<String>) -> Response {
    asset_response(format!("assets/{path}"), None)
}

#[cfg(test)]
pub(crate) fn first_asset_path() -> Option<String> {
    WebUiAssets::iter()
        .map(|name| name.to_string())
        .find(|name| name.starts_with("assets/"))
}

fn asset_response(path: String, nonce: Option<&str>) -> Response {
    let Some(content) = WebUiAssets::get(&path) else {
        return (StatusCode::NOT_FOUND, "asset not found").into_response();
    };

    let mime = from_path(&path).first_or_octet_stream();
    let bytes = if let Some(nonce) = nonce {
        String::from_utf8_lossy(&content.data)
            .replacen(
                "<head>",
                &format!("<head><meta name=\"csp-nonce\" content=\"{nonce}\">"),
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
