use axum::{
    body::Body,
    extract::Path,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use mime_guess::from_path;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "web/dist"]
struct WebUiAssets;

pub async fn serve_admin_index() -> Response {
    index_response()
}

pub async fn serve_admin_history(Path(_path): Path<String>) -> Response {
    index_response()
}

pub async fn serve_admin_asset(Path(path): Path<String>) -> Response {
    asset_response(format!("assets/{path}"))
}

#[cfg(test)]
pub(crate) fn first_asset_path() -> Option<String> {
    WebUiAssets::iter()
        .map(|name| name.to_string())
        .find(|name| name.starts_with("assets/"))
}

fn index_response() -> Response {
    asset_response("index.html".to_string())
}

fn asset_response(path: String) -> Response {
    let Some(content) = WebUiAssets::get(&path) else {
        return (StatusCode::NOT_FOUND, "asset not found").into_response();
    };

    let mime = from_path(&path).first_or_octet_stream();
    let mut response = Response::new(Body::from(content.data));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(mime.as_ref())
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response
}
