//! Durable, append-only operation audit. Request/response bodies and credentials are never stored.
use crate::{
    AppState,
    api::{auth::CurrentUser, error::ApiError},
    persistence::postgres::User,
};
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{ConnectInfo, MatchedPath, Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, QueryBuilder, Row};
use std::{
    collections::BTreeMap,
    net::{IpAddr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct Actor {
    pub id: Option<Uuid>,
    pub name: String,
    pub kind: String,
}
#[derive(Clone)]
pub struct Target(pub String);
#[derive(Clone)]
pub struct CompletedDetails(pub Value);
impl Actor {
    pub fn user(user: &User) -> Self {
        Self {
            id: Some(user.id),
            name: user
                .display_name
                .as_ref()
                .filter(|name| !name.is_empty())
                .map(|name| format!("{name} · {}", user.email))
                .unwrap_or_else(|| user.email.clone()),
            kind: if user.is_admin { "admin" } else { "member" }.into(),
        }
    }
    pub fn system() -> Self {
        Self {
            id: None,
            name: "系统".into(),
            kind: "system".into(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct Context {
    pub request_id: Uuid,
    pub actor: Actor,
    pub source: String,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
}
#[derive(Clone)]
struct AuditFacets {
    at: Instant,
    actions: Value,
    resources: Vec<String>,
}
#[derive(Clone, Default)]
pub struct AuditHealth {
    revision: Arc<AtomicU64>,
    failures: Arc<AtomicU64>,
    healthy: Arc<AtomicBool>,
    facets: Arc<tokio::sync::Mutex<Option<AuditFacets>>>,
}
impl AuditHealth {
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }
    pub fn snapshot(&self) -> Value {
        json!({"healthy":self.healthy.load(Ordering::Relaxed),"write_failures":self.failures.load(Ordering::Relaxed)})
    }
    fn changed(&self, state: &AppState) {
        self.healthy.store(true, Ordering::Relaxed);
        self.revision.fetch_add(1, Ordering::Relaxed);
        state.admin_events.invalidate(&["audit", "dashboard"]);
    }
    fn failed(&self) {
        self.healthy.store(false, Ordering::Relaxed);
        self.failures.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Event {
    pub id: Uuid,
    pub request_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub actor_id: Option<Uuid>,
    pub actor_name: String,
    pub actor_kind: String,
    pub source: String,
    pub action: String,
    pub summary: String,
    pub resource: String,
    pub target_id: Option<String>,
    pub method: Option<String>,
    pub route: Option<String>,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub result: String,
    pub status_code: Option<i32>,
    pub duration_ms: Option<i64>,
    pub details: Value,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Filters {
    pub q: Option<String>,
    pub action: Option<String>,
    pub resource: Option<String>,
    pub result: Option<String>,
    pub source: Option<String>,
    pub actor: Option<String>,
    pub ip: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}
impl Filters {
    pub fn from_stream(query: &BTreeMap<String, String>) -> Result<Self, ApiError> {
        let values = query
            .iter()
            .filter_map(|(k, v)| {
                k.strip_prefix("audit_")
                    .filter(|k| *k != "view")
                    .map(|k| (k.to_string(), v.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        let mut result = Self::default();
        for (key, value) in values {
            if value.is_empty() {
                continue;
            }
            match key.as_str() {
                "q" => result.q = Some(value),
                "action" => result.action = Some(value),
                "resource" => result.resource = Some(value),
                "result" => result.result = Some(value),
                "source" => result.source = Some(value),
                "actor" => result.actor = Some(value),
                "ip" => result.ip = Some(value),
                "from" => {
                    result.from = Some(
                        value
                            .parse()
                            .map_err(|_| ApiError::bad_request("起始时间无效"))?,
                    )
                }
                "to" => {
                    result.to = Some(
                        value
                            .parse()
                            .map_err(|_| ApiError::bad_request("截止时间无效"))?,
                    )
                }
                "page" => {
                    result.page = Some(
                        value
                            .parse()
                            .map_err(|_| ApiError::bad_request("页码无效"))?,
                    )
                }
                "page_size" => {
                    result.page_size = Some(
                        value
                            .parse()
                            .map_err(|_| ApiError::bad_request("分页大小无效"))?,
                    )
                }
                _ => {}
            }
        }
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<(), ApiError> {
        if [
            self.q.as_ref(),
            self.action.as_ref(),
            self.resource.as_ref(),
            self.result.as_ref(),
            self.source.as_ref(),
            self.actor.as_ref(),
            self.ip.as_ref(),
        ]
        .into_iter()
        .flatten()
        .any(|value| value.chars().count() > 256)
        {
            return Err(ApiError::bad_request("筛选条件不能超过 256 字符"));
        }
        if self.from.zip(self.to).is_some_and(|(from, to)| from >= to) {
            return Err(ApiError::bad_request("起始时间必须早于截止时间"));
        }
        if self.page.is_some_and(|p| p < 1)
            || self.page_size.is_some_and(|p| !(1..=100).contains(&p))
        {
            return Err(ApiError::bad_request("页码至少为 1，每页 1 至 100 条"));
        }
        if self
            .ip
            .as_ref()
            .is_some_and(|ip| !ip.is_empty() && ip.parse::<IpAddr>().is_err())
        {
            return Err(ApiError::bad_request("IP 地址无效"));
        }
        Ok(())
    }
    fn sql(&self, prefix: &str) -> QueryBuilder<'_, Postgres> {
        let mut sql = QueryBuilder::new(format!("{prefix} FROM audit_events WHERE true"));
        for (column, value) in [
            ("action", &self.action),
            ("resource", &self.resource),
            ("result", &self.result),
            ("source", &self.source),
        ] {
            if let Some(value) = value.as_deref().filter(|v| !v.is_empty()) {
                sql.push(format!(" AND {column}=")).push_bind(value);
            }
        }
        if let Some(value) = self.ip.as_deref().filter(|v| !v.is_empty()) {
            sql.push(" AND ip=").push_bind(
                value
                    .parse::<IpAddr>()
                    .map(|ip| ip.to_string())
                    .unwrap_or_default(),
            );
        }
        if let Some(value) = self.actor.as_deref().filter(|v| !v.is_empty()) {
            sql.push(" AND actor_name ILIKE ")
                .push_bind(format!("%{value}%"));
        }
        if let Some(value) = self.q.as_deref().filter(|v| !v.is_empty()) {
            sql.push(" AND concat_ws(' ',summary,actor_name,actor_id::text,action,resource,target_id,request_id::text,details->>'claimed_identity') ILIKE ").push_bind(format!("%{value}%"));
        }
        if let Some(value) = self.from {
            sql.push(" AND created_at>=").push_bind(value);
        }
        if let Some(value) = self.to {
            sql.push(" AND created_at<").push_bind(value);
        }
        sql
    }
}
fn db_error(error: sqlx::Error) -> ApiError {
    log::error!("audit database operation failed: {error}");
    ApiError::service_unavailable("审计数据库暂时不可用")
}
pub async fn page(state: &AppState, filters: &Filters) -> Result<Value, ApiError> {
    filters.validate()?;
    let pool = state.database.pool();
    let mut tx = pool.begin().await.map_err(db_error)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    sqlx::query("SET LOCAL statement_timeout='5s'")
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    let total: i64 = filters
        .sql("SELECT count(*)")
        .build_query_scalar()
        .fetch_one(&mut *tx)
        .await
        .map_err(db_error)?;
    let page_size = filters.page_size.unwrap_or(20);
    let page = filters
        .page
        .unwrap_or(1)
        .min(((total + page_size - 1) / page_size).max(1));
    let items = filters
        .sql("SELECT *")
        .push(" ORDER BY created_at DESC,id DESC LIMIT ")
        .push_bind(page_size)
        .push(" OFFSET ")
        .push_bind((page - 1) * page_size)
        .build_query_as::<Event>()
        .fetch_all(&mut *tx)
        .await
        .map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    // Shared facets change rarely. Do not aggregate the entire log for every page/subscriber.
    let mut facets = state.audit.facets.lock().await;
    if facets
        .as_ref()
        .is_none_or(|facets| facets.at.elapsed().as_secs() >= 60)
    {
        let actions=sqlx::query("SELECT action,max(summary) AS label FROM audit_events GROUP BY action ORDER BY action").fetch_all(pool).await.map_err(db_error)?.into_iter().map(|row|Ok(json!({"value":row.try_get::<String,_>("action")?,"label":row.try_get::<String,_>("label")?}))).collect::<Result<Vec<_>,sqlx::Error>>().map_err(db_error)?;
        let resources: Vec<String> =
            sqlx::query_scalar("SELECT DISTINCT resource FROM audit_events ORDER BY resource")
                .fetch_all(pool)
                .await
                .map_err(db_error)?;
        *facets = Some(AuditFacets {
            at: Instant::now(),
            actions: Value::Array(actions),
            resources,
        });
    }
    let cached = facets.as_ref().expect("initialized audit facets").clone();
    drop(facets);
    Ok(
        json!({"active":true,"query":filters,"items":items,"total":total,"page":page,"page_size":page_size,"actions":cached.actions,"resources":cached.resources,"retention_days":state.config.read().await.audit.retention_days}),
    )
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/audits", get(list))
        .route("/api/v1/admin/audits/export", get(export))
        .route("/api/v1/admin/audits/view", post(viewed))
        .route("/api/v1/admin/audits/cleanup", post(manual_cleanup))
        .route("/api/v1/admin/audits/{id}", get(detail))
}
async fn list(
    State(state): State<AppState>,
    Query(filters): Query<Filters>,
) -> Result<Json<Value>, ApiError> {
    page(&state, &filters).await.map(Json)
}
async fn viewed() -> StatusCode {
    StatusCode::NO_CONTENT
}
async fn detail(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Event>, ApiError> {
    sqlx::query_as("SELECT * FROM audit_events WHERE id=$1")
        .bind(id)
        .fetch_optional(state.database.pool())
        .await
        .map_err(db_error)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("日志不存在"))
}
pub fn csv_cell(value: &str) -> String {
    let first = value
        .trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
        .chars()
        .next();
    let prefix = if first.is_some_and(|c| matches!(c, '=' | '+' | '-' | '@')) {
        "'"
    } else {
        ""
    };
    format!("\"{prefix}{}\"", value.replace('"', "\"\""))
}
async fn export(
    State(state): State<AppState>,
    Query(filters): Query<Filters>,
) -> Result<Response, ApiError> {
    filters.validate()?;
    let pool = state.database.pool();
    let count: i64 = filters
        .sql("SELECT count(*)")
        .build_query_scalar()
        .fetch_one(pool)
        .await
        .map_err(db_error)?;
    if count > 10000 {
        return Err(ApiError::bad_request(
            "单次最多导出 10000 条，请缩小时间范围或筛选条件",
        ));
    }
    let rows = filters
        .sql("SELECT *")
        .push(" ORDER BY created_at DESC,id DESC LIMIT 10001")
        .build_query_as::<Event>()
        .fetch_all(pool)
        .await
        .map_err(db_error)?;
    if rows.len() > 10000 {
        return Err(ApiError::conflict("日志数量已变化，请缩小筛选范围后重试"));
    }
    let mut csv = String::from(
        "\u{feff}时间,操作者,身份,来源,操作,资源,对象,结果,HTTP状态,耗时毫秒,IP,请求ID,方法,路由,详情\r\n",
    );
    for row in rows {
        let fields = [
            row.created_at.to_rfc3339(),
            row.actor_name,
            row.actor_kind,
            row.source,
            row.summary,
            row.resource,
            row.target_id.unwrap_or_default(),
            row.result,
            row.status_code.map(|v| v.to_string()).unwrap_or_default(),
            row.duration_ms.map(|v| v.to_string()).unwrap_or_default(),
            row.ip.unwrap_or_default(),
            row.request_id.to_string(),
            row.method.unwrap_or_default(),
            row.route.unwrap_or_default(),
            row.details.to_string(),
        ];
        csv.push_str(
            &fields
                .iter()
                .map(|s| csv_cell(s))
                .collect::<Vec<_>>()
                .join(","),
        );
        csv.push_str("\r\n");
    }
    Ok((
        [
            ("content-type", "text/csv; charset=utf-8"),
            (
                "content-disposition",
                "attachment; filename=ostool-audit.csv",
            ),
            ("cache-control", "no-store"),
        ],
        csv,
    )
        .into_response())
}
#[derive(Deserialize)]
struct Cleanup {
    keep_days: u32,
}
async fn manual_cleanup(
    State(state): State<AppState>,
    Json(input): Json<Cleanup>,
) -> Result<(axum::extract::Extension<CompletedDetails>, Json<Value>), ApiError> {
    let count = cleanup(&state, input.keep_days).await?;
    Ok((
        axum::extract::Extension(CompletedDetails(json!({"deleted":count}))),
        Json(json!({"deleted":count})),
    ))
}
pub async fn cleanup(state: &AppState, days: u32) -> Result<u64, ApiError> {
    if !(1..=3650).contains(&days) {
        return Err(ApiError::bad_request("至少保留 1 天，最多 3650 天"));
    }
    let count = sqlx::query("DELETE FROM audit_events WHERE created_at<$1 AND result<>'pending'")
        .bind(Utc::now() - chrono::Duration::days(i64::from(days)))
        .execute(state.database.pool())
        .await
        .map_err(db_error)?
        .rows_affected();
    state.audit.changed(state);
    Ok(count)
}

/// Persist the scheduled deletion intent before changing historical records.
pub async fn run_retention(state: &AppState, days: u32) -> Result<u64, ApiError> {
    let context = Context {
        request_id: Uuid::new_v4(),
        actor: Actor::system(),
        source: "system".into(),
        ip: None,
        user_agent: None,
    };
    let id = begin(
        state,
        &context,
        "audit.retention",
        "执行审计保留策略",
        "audits",
        None,
        None,
        None,
        &json!({"keep_days":days}),
    )
    .await?;
    let start = Instant::now();
    let result = cleanup(state, days).await;
    finish(
        state,
        id,
        &context.actor,
        if result.is_ok() { "success" } else { "failed" },
        None,
        start.elapsed().as_millis() as i64,
        Some(json!({"keep_days":days,"deleted":result.as_ref().ok()})),
    )
    .await?;
    result
}

fn text(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect()
}
pub fn client_ip(peer: Option<IpAddr>, headers: &HeaderMap, trusted: &[IpAddr]) -> Option<IpAddr> {
    let mut address = peer?.to_canonical();
    let trusted: Vec<IpAddr> = trusted.iter().map(|ip| ip.to_canonical()).collect();
    if !trusted.contains(&address) {
        return Some(address);
    }
    // Unknown/malformed forwarding from a trusted proxy must not inherit the
    // proxy's loopback/private IP and accidentally acquire hardware-network access.
    let header = headers.get("x-forwarded-for")?.to_str().ok()?;
    let chain = header
        .split(',')
        .map(|ip| ip.trim().parse::<IpAddr>())
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    for next in chain.into_iter().rev() {
        if !trusted.contains(&address) {
            break;
        }
        address = next.to_canonical();
    }
    Some(address)
}
struct Intent {
    action: String,
    summary: &'static str,
    resource: String,
    target: Option<String>,
}
fn intent(method: &Method, path: &str, payload: &Value) -> Option<Intent> {
    if path.ends_with("/heartbeat")
        || path.starts_with("/api/v1/loaders/")
        || path == "/oauth/device/code"
    {
        return None;
    }
    let read = method == Method::GET;
    if read
        && ![
            "/api/v1/admin/audits",
            "/api/v1/admin/events",
            "/api/v1/account/events",
            "/api/v1/admin/users",
            "/api/v1/admin/roles",
            "/api/v1/admin/server-config",
            "/api/v1/admin/tftp",
        ]
        .iter()
        .any(|prefix| path.starts_with(prefix))
    {
        return None;
    }
    if !read
        && !matches!(
            *method,
            Method::POST | Method::PUT | Method::PATCH | Method::DELETE
        )
    {
        return None;
    }
    let tail = path
        .strip_prefix("/api/v1/admin/")
        .or_else(|| path.strip_prefix("/api/v1/account/"))
        .or_else(|| path.strip_prefix("/api/v1/"))
        .or_else(|| path.strip_prefix("/oauth/"))?;
    let parts: Vec<_> = tail.split('/').collect();
    let resource = parts[0];
    let (action, summary) = match (resource, parts.get(1).copied(), method.as_str()) {
        ("events", _, "GET") if path.contains("/admin/") => ("admin.subscribe", "订阅管理工作区"),
        ("events", _, "GET") => ("account.subscribe", "订阅个人工作台"),
        ("users", _, "GET") => ("user.view", "查看用户信息"),
        ("roles", _, "GET") => ("role.view", "查看角色信息"),
        ("server-config", _, "GET") => ("settings.view", "查看系统设置"),
        ("tftp", _, "GET") => ("tftp.view", "查看 TFTP 配置"),
        ("account-settings", _, _) => ("system.account.update", "更新账号与邮件安全策略"),
        ("site-settings", _, _) => ("system.site.update", "更新站点与协议配置"),
        ("auth", Some("login"), _) if path.starts_with("/api/v1/admin/") => {
            ("auth.admin_login", "管理员登录")
        }
        ("auth", Some("login"), _) => ("auth.login", "用户登录"),
        ("auth", Some("register"), _) => ("auth.register", "用户注册"),
        ("auth", Some("mfa"), _) => ("auth.mfa.verify", "双重验证登录"),
        ("auth", Some("step-up"), _) => ("auth.step_up", "敏感操作身份复核"),
        ("auth", Some("email-code"), _) => ("auth.email.send", "发送邮箱验证码"),
        ("auth", Some("password-reset"), _) => ("auth.password.reset", "通过邮箱重置密码"),
        ("auth", Some("logout"), _) => ("auth.logout", "退出登录"),
        ("auth", Some("refresh"), _) => ("auth.refresh", "轮换浏览器登录会话"),
        ("auth", Some("security"), _) => ("auth.security", "维护账号安全设置"),
        ("auth", Some("device"), _) => ("auth.device.authorize", "授权 CLI 设备登录"),
        ("auth", Some("personal-access-tokens"), "DELETE") => {
            ("auth.pat.revoke", "撤销个人访问凭据")
        }
        ("auth", Some("personal-access-tokens"), _) => ("auth.pat.create", "创建个人访问凭据"),
        ("token", _, _) => ("auth.token", "换取登录凭据"),
        ("revoke", _, _) => ("auth.logout", "退出登录"),
        ("profile", _, _) => ("account.profile", "修改个人资料"),
        ("password", _, _) => ("account.password", "修改密码"),
        ("audits", Some("export"), _) => ("audit.export", "导出审计日志"),
        ("audits", Some("cleanup"), _) => ("audit.cleanup", "清理审计日志"),
        ("audits", _, _) => ("audit.view", "查看审计日志"),
        ("power-actions", _, _) => ("power.submit", "提交电源操作"),
        ("server-config", _, _) => ("settings.update", "修改系统设置"),
        ("tftp", Some("reconcile"), _) => ("tftp.reconcile", "应用 TFTP 配置"),
        ("tftp", _, _) => ("tftp.update", "修改 TFTP 配置"),
        ("rental-requests", _, _) if parts.last() == Some(&"review") => {
            if payload["decision"] == "approve" {
                ("rental.approve", "批准租赁申请")
            } else {
                ("rental.reject", "拒绝租赁申请")
            }
        }
        ("boards", _, "POST") if parts.get(2) == Some(&"loader-updates") => {
            ("ota.queue", "创建 OTA 任务")
        }
        ("boards", _, "DELETE") if parts.get(2) == Some(&"loader-updates") => {
            ("ota.cancel", "取消 OTA 任务")
        }
        ("boards", _, _) if parts.len() > 2 => ("board.operation", "执行开发板操作"),
        ("boards", _, "POST") => ("board.create", "新增开发板"),
        ("boards", _, "DELETE") => ("board.delete", "删除开发板"),
        ("boards", _, _) => ("board.update", "修改开发板"),
        ("users", _, "POST") => ("user.create", "新增用户"),
        ("users", _, "DELETE") => ("user.delete", "删除用户"),
        ("users", _, _) => ("user.update", "修改用户"),
        ("roles", _, "POST") => ("role.create", "新增角色"),
        ("roles", _, "DELETE") => ("role.delete", "删除角色"),
        ("roles", _, _) => ("role.update", "修改角色"),
        ("rentals", _, "POST") => ("rental.create", "新增租赁"),
        ("rentals", _, "DELETE") => ("rental.cancel", "取消租赁"),
        ("rentals", _, _) => ("rental.update", "修改租赁"),
        ("rental-requests", _, "POST") => ("rental.apply", "提交租赁申请"),
        ("rental-requests", _, _) => ("rental.withdraw", "撤回租赁申请"),
        ("dtbs", _, "POST") => ("dtb.create", "上传 DTB"),
        ("dtbs", _, "DELETE") => ("dtb.delete", "删除 DTB"),
        ("dtbs", _, _) => ("dtb.update", "修改 DTB"),
        ("sessions", None, _) => ("session.create", "创建板卡会话"),
        ("sessions", _, "DELETE") if parts.len() == 2 => ("session.release", "释放板卡会话"),
        ("sessions", _, _) => ("session.operation", "执行会话操作"),
        ("loader-images", _, "DELETE") => ("ota.image.delete", "删除 OTA 镜像"),
        ("loader-images", _, _) => ("ota.image.upload", "上传 OTA 镜像"),
        ("virtual-devices", _, "DELETE") => ("virtual.delete", "删除虚拟设备"),
        ("virtual-devices", _, _) => ("virtual.create", "创建虚拟设备"),
        _ => ("operation.request", "API 操作"),
    };
    let target = if resource == "auth" {
        None
    } else {
        parts
            .get(1)
            .filter(|value| !matches!(**value, "view" | "export" | "cleanup" | "reconcile"))
            .map(|s| text(s, 256))
    }
    .or_else(|| {
        ["id", "board_id", "name", "request_id", "email"]
            .iter()
            .find_map(|key| payload[*key].as_str().map(|s| text(s, 128)))
    });
    Some(Intent {
        action: action.into(),
        summary,
        resource: resource.into(),
        target,
    })
}
fn safe_details(payload: &Value) -> Value {
    let mut details = serde_json::Map::new();
    for key in [
        "id",
        "board_id",
        "board_type",
        "role_id",
        "user_id",
        "request_id",
        "disabled",
        "administrator",
        "action",
        "decision",
        "name",
        "starts_at",
        "ends_at",
        "keep_days",
    ] {
        let value = &payload[key];
        if let Some(s) = value.as_str() {
            details.insert(key.into(), json!(text(s, 128)));
        } else if value.is_boolean() || value.is_number() {
            details.insert(key.into(), value.clone());
        }
    }
    if let Some(values) = payload["permissions"].as_array() {
        details.insert(
            "permissions".into(),
            json!(
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|code| crate::security::permissions::CATALOG
                        .iter()
                        .any(|p| p.code == *code))
                    .take(64)
                    .collect::<Vec<_>>()
            ),
        );
    }
    if let Some(fields) = payload.as_object() {
        let known = [
            "username",
            "email",
            "display_name",
            "password",
            "current_password",
            "new_password",
            "power_management",
            "reset_command",
            "serial",
            "boot",
            "network",
            "tftp",
            "upload_limits",
            "notes",
            "user_id",
            "role_id",
            "administrator",
            "permissions",
            "allocation_policy",
            "allow_instant_use",
            "disabled",
            "tags",
            "network_identity",
            "id",
            "board_type",
            "starts_at",
            "ends_at",
        ];
        details.insert(
            "changed_fields".into(),
            json!(
                known
                    .into_iter()
                    .filter(|key| fields.contains_key(*key))
                    .collect::<Vec<_>>()
            ),
        );
    }
    Value::Object(details)
}
pub async fn middleware(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let request_metric = state.monitor.request();
    let response = audited_request(&state, request, next).await;
    request_metric.finish(response.status().as_u16());
    response
}
async fn audited_request(state: &AppState, request: Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    let method = request.method().clone();
    if intent(&method, &path, &Value::Null).is_none() {
        return next.run(request).await;
    }
    let (mut parts, body) = request.into_parts();
    let mut payload = Value::Null;
    let mut oversized = false;
    let body = if parts
        .headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| s.starts_with("application/json"))
    {
        match to_bytes(body, 2 * 1024 * 1024).await {
            Ok(bytes) => {
                payload = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
                Body::from(bytes)
            }
            Err(_) => {
                oversized = true;
                Body::empty()
            }
        }
    } else {
        body
    };
    let Some(mut intent) = intent(&method, &path, &payload) else {
        return next.run(Request::from_parts(parts, body)).await;
    };
    if intent.resource == "dtbs" && intent.target.is_none() {
        intent.target = parts
            .headers
            .get("x-dtb-name")
            .and_then(|name| name.to_str().ok())
            .map(|name| text(name, 256));
    }
    let config = state.config.read().await;
    let mut actor = Actor {
        id: None,
        name: if config.edition.is_public() {
            "未认证请求"
        } else {
            "本地操作者"
        }
        .into(),
        kind: if config.edition.is_public() {
            "anonymous"
        } else {
            "local"
        }
        .into(),
    };
    let ip = client_ip(
        parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|v| v.0.ip()),
        &parts.headers,
        &config.audit.trusted_proxies,
    )
    .map(|ip| ip.to_string());
    let public = config.edition.is_public();
    drop(config);
    let token = parts
        .headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|value| {
            value
                .strip_prefix("Bearer ")
                .or_else(|| value.strip_prefix("bearer "))
        })
        .map(str::to_owned)
        .or_else(|| {
            if matches!(
                path.as_str(),
                "/api/v1/admin/events" | "/api/v1/account/events"
            ) {
                url::form_urlencoded::parse(parts.uri.query().unwrap_or_default().as_bytes())
                    .find_map(|(key, value)| (key == "access_token").then(|| value.into_owned()))
            } else {
                None
            }
        });
    if public
        && let Some(token) = token
        && let Ok(Some(user)) = state.database.authenticate_token(&token).await
    {
        actor = Actor::user(&user);
    }
    let source = if path.starts_with("/api/v1/admin/") {
        "admin"
    } else if path.starts_with("/api/v1/account/") || path.starts_with("/api/v1/auth/") {
        "website"
    } else {
        "cli"
    };
    let context = Context {
        request_id: parts
            .extensions
            .get::<crate::security::web::RequestId>()
            .map(|id| id.0)
            .unwrap_or_else(Uuid::new_v4),
        actor,
        source: source.into(),
        ip,
        user_agent: parts
            .headers
            .get("user-agent")
            .and_then(|v| v.to_str().ok())
            .map(|s| text(s, 512)),
    };
    let mut details = safe_details(&payload);
    if intent.resource == "dtbs"
        && let Some(name) = parts
            .headers
            .get("x-dtb-name")
            .and_then(|name| name.to_str().ok())
    {
        details["requested_name"] = json!(text(name, 256));
    }
    if matches!(
        intent.action.as_str(),
        "auth.login" | "auth.admin_login" | "auth.register"
    ) && let Some(email) = payload["email"].as_str()
    {
        details["claimed_identity"] = json!(text(email, 254));
    }
    let route = parts
        .extensions
        .get::<MatchedPath>()
        .map(|path| path.as_str().to_string())
        .unwrap_or_else(|| "unmatched-api-route".into());
    let id = match begin(
        state,
        &context,
        &intent.action,
        intent.summary,
        &intent.resource,
        intent.target.as_deref(),
        Some(method.as_str()),
        Some(&route),
        &details,
    )
    .await
    {
        Ok(id) => Some(id),
        Err(_) if method == Method::GET => None,
        Err(error) => return error.into_response(),
    };
    parts.extensions.insert(context.clone());
    if oversized {
        let _ = finish(
            state,
            id.expect("mutation intent"),
            &context.actor,
            "failed",
            Some(413),
            0,
            None,
        )
        .await;
        let mut response = ApiError::payload_too_large("JSON 请求超过限制").into_response();
        response.headers_mut().insert(
            "x-request-id",
            HeaderValue::from_str(&context.request_id.to_string()).expect("UUID"),
        );
        return response;
    }
    let timer = Instant::now();
    let mut response = next.run(Request::from_parts(parts, body)).await;
    if path.starts_with("/api/v1/admin/audits") {
        response
            .headers_mut()
            .insert("cache-control", HeaderValue::from_static("no-store"));
    }
    let actor = response
        .extensions()
        .get::<CurrentUser>()
        .map(|user| Actor::user(&user.0))
        .unwrap_or_else(|| context.actor.clone());
    let final_details = if response.extensions().get::<Target>().is_some()
        || response.extensions().get::<CompletedDetails>().is_some()
    {
        let mut value = details.clone();
        if let Some(target) = response.extensions().get::<Target>() {
            value["created_id"] = json!(text(&target.0, 256));
        }
        if let Some(extra) = response
            .extensions()
            .get::<CompletedDetails>()
            .and_then(|extra| extra.0.as_object())
        {
            for (key, item) in extra {
                value[key] = item.clone();
            }
        }
        Some(value)
    } else {
        None
    };
    let status = response.status().as_u16();
    let result = if status == 202 {
        "accepted"
    } else if status < 400 {
        "success"
    } else if matches!(status, 401 | 403) {
        "denied"
    } else {
        "failed"
    };
    if let Some(id) = id {
        if finish(
            state,
            id,
            &actor,
            result,
            Some(status),
            timer.elapsed().as_millis() as i64,
            final_details,
        )
        .await
        .is_err()
        {
            response
                .headers_mut()
                .insert("x-audit-status", HeaderValue::from_static("pending"));
        }
    } else {
        response
            .headers_mut()
            .insert("x-audit-status", HeaderValue::from_static("unavailable"));
    }
    response.headers_mut().insert(
        "x-request-id",
        HeaderValue::from_str(&context.request_id.to_string()).expect("UUID header"),
    );
    response
}
#[allow(clippy::too_many_arguments)]
async fn begin(
    state: &AppState,
    ctx: &Context,
    action: &str,
    summary: &str,
    resource: &str,
    target: Option<&str>,
    method: Option<&str>,
    route: Option<&str>,
    details: &Value,
) -> Result<Uuid, ApiError> {
    let id = Uuid::new_v4();
    let result=sqlx::query("INSERT INTO audit_events(id,request_id,actor_id,actor_name,actor_kind,source,action,summary,resource,target_id,method,route,ip,user_agent,result,details) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,'pending',$15)").bind(id).bind(ctx.request_id).bind(ctx.actor.id).bind(&ctx.actor.name).bind(&ctx.actor.kind).bind(&ctx.source).bind(action).bind(summary).bind(resource).bind(target).bind(method).bind(route).bind(&ctx.ip).bind(&ctx.user_agent).bind(details).execute(state.database.pool()).await;
    if let Err(error) = result {
        state.audit.failed();
        return Err(db_error(error));
    }
    state.audit.changed(state);
    Ok(id)
}
async fn finish(
    state: &AppState,
    id: Uuid,
    actor: &Actor,
    result: &str,
    status: Option<u16>,
    duration: i64,
    details: Option<Value>,
) -> Result<(), ApiError> {
    let update=sqlx::query("UPDATE audit_events SET finished_at=now(),actor_id=$2,actor_name=$3,actor_kind=$4,result=$5,status_code=$6,duration_ms=$7,details=COALESCE($8::jsonb,details),target_id=COALESCE($8::jsonb->>'created_id',target_id) WHERE id=$1").bind(id).bind(actor.id).bind(&actor.name).bind(&actor.kind).bind(result).bind(status.map(i32::from)).bind(duration).bind(details).execute(state.database.pool()).await;
    if let Err(error) = update {
        state.audit.failed();
        return Err(db_error(error));
    }
    state.audit.changed(state);
    Ok(())
}
pub async fn system_event(
    state: &AppState,
    action: &str,
    summary: &str,
    resource: &str,
    target: Option<&str>,
    success: bool,
    details: Value,
) {
    let ctx = Context {
        request_id: Uuid::new_v4(),
        actor: Actor::system(),
        source: "system".into(),
        ip: None,
        user_agent: None,
    };
    event(
        state, &ctx, action, summary, resource, target, success, details,
    )
    .await;
}
#[allow(clippy::too_many_arguments)]
pub async fn event(
    state: &AppState,
    ctx: &Context,
    action: &str,
    summary: &str,
    resource: &str,
    target: Option<&str>,
    success: bool,
    details: Value,
) {
    if let Ok(id) = begin(
        state, ctx, action, summary, resource, target, None, None, &details,
    )
    .await
    {
        let _ = finish(
            state,
            id,
            &ctx.actor,
            if success { "success" } else { "failed" },
            None,
            details
                .get("duration_ms")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            None,
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secrets_are_never_copied_to_details() {
        let input = json!({"password":"TOP_SECRET","new_password":"TOP_SECRET","token":"TOP_SECRET","power_management":{"power_on_cmd":"TOP_SECRET"},"notes":"TOP_SECRET","id":"board-one","disabled":true});
        let result = safe_details(&input).to_string();
        assert!(!result.contains("TOP_SECRET"));
        assert!(result.contains("board-one"));
    }
    #[test]
    fn forwarded_addresses_require_a_trusted_immediate_peer() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("1.2.3.4, 10.0.0.2"),
        );
        let peer = "127.0.0.1".parse().unwrap();
        assert_eq!(client_ip(Some(peer), &headers, &[]), Some(peer));
        assert_eq!(
            client_ip(Some(peer), &headers, &[peer]),
            Some("10.0.0.2".parse().unwrap())
        );
        assert_eq!(
            client_ip(Some(peer), &headers, &[peer, "10.0.0.2".parse().unwrap()]),
            Some("1.2.3.4".parse().unwrap())
        );
    }
    #[test]
    fn invalid_forwarding_never_inherits_a_trusted_proxy_address() {
        let peer = "127.0.0.1".parse().unwrap();
        let mut headers = HeaderMap::new();
        assert_eq!(client_ip(Some(peer), &headers, &[peer]), None);
        for value in ["", "invalid", "invalid, 203.0.113.10", "203.0.113.10,"] {
            headers.insert("x-forwarded-for", HeaderValue::from_str(value).unwrap());
            assert_eq!(client_ip(Some(peer), &headers, &[peer]), None);
            assert_eq!(client_ip(Some(peer), &headers, &[]), Some(peer));
        }
    }
    #[test]
    fn csv_neutralizes_formulas_and_escapes_quotes() {
        assert_eq!(csv_cell(" =SUM(1,2)"), "\"' =SUM(1,2)\"");
        assert_eq!(csv_cell("a\"b"), "\"a\"\"b\"");
    }
}
