//! 路由配置
//!
//! 定义和配置所有HTTP路由

use axum::{Router, routing::get};
use tower_http::services::ServeDir;

use super::handlers::{api_config_handler, health_check, root_handler, static_handler};
use crate::data::AppState;

/// 创建应用路由
pub fn create_routes(state: AppState) -> Router {
    Router::new()
        // 主页面路由
        .route("/", get(root_handler))
        // API路由
        .route("/api/config", get(api_config_handler))
        .route("/api/health", get(health_check))
        // 静态文件服务
        .nest_service("/static", ServeDir::new("web/static"))
        // 备用路由 - 处理SPA路由
        .fallback(get(static_handler))
        // 注入应用状态
        .with_state(state)
}
