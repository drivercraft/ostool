//! HTTP请求处理器
//!
//! 处理各种HTTP请求的函数

use axum::{
    extract::State,
    response::{Html, Json},
};
use serde_json::json;

use crate::data::AppState;

/// 根路径处理器 - 返回Hello World页面
pub async fn root_handler() -> Html<&'static str> {
    Html(include_str!("../../web/static/index.html"))
}

/// API处理器 - 返回配置信息
pub async fn api_config_handler(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({
        "title": state.document.title(),
        "config": state.document.as_json(),
        "message": "jkconfig Web API",
        "version": "0.1.1"
    }))
}

/// 健康检查处理器
pub async fn health_check() -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

/// 静态文件处理器
pub async fn static_handler() -> Html<&'static str> {
    Html(include_str!("../../web/static/index.html"))
}
