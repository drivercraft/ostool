//! One capability catalog for role editing, HTTP authorization and scoped events.
use crate::{
    AppState,
    api::error::ApiError,
    persistence::{Database, postgres::User},
};
use axum::http::Method;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize)]
pub struct Capability {
    pub code: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub group: &'static str,
    pub requires: &'static [&'static str],
}
macro_rules! cap {($code:literal,$name:literal,$desc:literal,$group:literal,[$($requires:literal),*])=>{Capability{code:$code,name:$name,description:$desc,group:$group,requires:&[$($requires),*]}}}
pub const CATALOG: &[Capability] = &[
    cap!(
        "system.admin",
        "全部管理权限",
        "管理全部后台功能与角色授权",
        "系统与授权",
        []
    ),
    cap!(
        "console.access",
        "进入管理后台",
        "使用独立后台登录，不授予其他管理能力",
        "系统与授权",
        []
    ),
    cap!(
        "dashboard.read",
        "查看仪表盘",
        "查看服务器、后台任务和运营统计",
        "运行监控",
        ["console.access"]
    ),
    cap!(
        "devices.read",
        "查看设备",
        "查看设备清单与发现状态",
        "设备管理",
        ["console.access"]
    ),
    cap!(
        "devices.manage",
        "维护设备",
        "新增、编辑、删除设备及虚拟设备（仅授予可信维护人员）",
        "设备管理",
        ["devices.read", "dtbs.read"]
    ),
    cap!(
        "devices.power",
        "操作设备电源",
        "执行设备上电、下电命令",
        "设备管理",
        ["devices.manage"]
    ),
    cap!(
        "dtbs.read",
        "查看 DTB",
        "查看设备树文件和元数据",
        "设备管理",
        ["console.access"]
    ),
    cap!(
        "dtbs.manage",
        "维护 DTB",
        "上传、修改和删除设备树",
        "设备管理",
        ["dtbs.read"]
    ),
    cap!(
        "ota.read",
        "查看设备升级",
        "查看装载器镜像与升级任务",
        "设备管理",
        ["devices.read"]
    ),
    cap!(
        "ota.manage",
        "管理设备升级",
        "上传镜像、安排和取消升级",
        "设备管理",
        ["ota.read"]
    ),
    cap!(
        "sessions.read",
        "查看会话",
        "查看活动会话和持久化使用历史",
        "使用与租赁",
        ["console.access"]
    ),
    cap!(
        "sessions.manage",
        "释放会话",
        "结束设备会话并执行清理",
        "使用与租赁",
        ["sessions.read"]
    ),
    cap!(
        "rentals.read",
        "查看租赁",
        "查看租赁清单和可用日历",
        "使用与租赁",
        ["console.access"]
    ),
    cap!(
        "rentals.manage",
        "管理租赁",
        "为用户分配、调整和取消租赁",
        "使用与租赁",
        ["rentals.read", "devices.read", "users.read"]
    ),
    cap!(
        "requests.read",
        "查看租赁申请",
        "查看用户提交的申请",
        "使用与租赁",
        ["console.access"]
    ),
    cap!(
        "requests.manage",
        "审核租赁申请",
        "批准或拒绝申请",
        "使用与租赁",
        ["requests.read", "rentals.manage"]
    ),
    cap!(
        "users.read",
        "查看用户",
        "查看用户清单、状态与角色",
        "用户与角色",
        ["roles.read"]
    ),
    cap!(
        "users.manage",
        "管理用户",
        "维护普通用户资料、角色及密码",
        "用户与角色",
        ["users.read", "roles.read"]
    ),
    cap!(
        "roles.read",
        "查看角色",
        "查看角色、能力和使用限制",
        "用户与角色",
        ["console.access"]
    ),
    cap!(
        "roles.manage",
        "编辑角色权限",
        "创建角色及分配自己已拥有的能力",
        "用户与角色",
        ["roles.read"]
    ),
    cap!(
        "site.read",
        "查看站点设置",
        "查看站点资料、协议和发布历史",
        "系统设置",
        ["console.access"]
    ),
    cap!(
        "site.manage",
        "维护站点设置",
        "发布站点信息和用户协议",
        "系统设置",
        ["site.read"]
    ),
    cap!(
        "server.read",
        "查看运行配置",
        "查看网络、TFTP 和上传限制",
        "系统设置",
        ["console.access"]
    ),
    cap!(
        "server.manage",
        "修改运行配置",
        "维护网络、TFTP 和上传限制",
        "系统设置",
        ["server.read"]
    ),
    cap!(
        "security.read",
        "查看账号策略",
        "查看登录、注册、邮件和命名策略",
        "系统设置",
        ["console.access"]
    ),
    cap!(
        "security.manage",
        "修改账号策略",
        "维护认证策略及业务密钥",
        "系统设置",
        ["security.read"]
    ),
    cap!(
        "audits.read",
        "查看审计日志",
        "查看操作历史和审计详情",
        "审计日志",
        ["console.access"]
    ),
    cap!(
        "audits.export",
        "导出审计日志",
        "导出筛选后的审计记录",
        "审计日志",
        ["audits.read"]
    ),
    cap!(
        "audits.manage",
        "清理审计日志",
        "按保留策略清理历史记录",
        "审计日志",
        ["audits.read"]
    ),
    cap!(
        "board.use",
        "使用已分配设备",
        "在有效租赁时段通过 CLI 或网站使用设备",
        "前台设备使用",
        []
    ),
    cap!(
        "rental.request",
        "申请租赁",
        "从前台提交待审核的设备使用申请",
        "前台设备使用",
        ["board.use"]
    ),
    cap!(
        "board.instant",
        "即时分配设备",
        "自动领取开放的空闲设备，不需要逐次审核",
        "前台设备使用",
        ["board.use"]
    ),
];
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct AllocationPolicy {
    pub max_duration_minutes: u32,
    pub max_concurrent_sessions: u32,
    pub min_available_minutes: u32,
    pub board_types: Vec<String>,
}
impl Default for AllocationPolicy {
    fn default() -> Self {
        Self {
            max_duration_minutes: 120,
            max_concurrent_sessions: 1,
            min_available_minutes: 10,
            board_types: vec![],
        }
    }
}
impl AllocationPolicy {
    pub fn normalized(mut self) -> Result<Self, ApiError> {
        self.validate()?;
        self.board_types = self
            .board_types
            .into_iter()
            .map(|v| v.trim().to_owned())
            .collect();
        self.board_types.sort();
        self.board_types.dedup();
        Ok(self)
    }
    pub fn validate(&self) -> Result<(), ApiError> {
        if !(1..=1440).contains(&self.max_duration_minutes)
            || !(1..=16).contains(&self.max_concurrent_sessions)
            || self.min_available_minutes == 0
            || self.min_available_minutes > self.max_duration_minutes
            || self.board_types.len() > 64
            || self
                .board_types
                .iter()
                .any(|v| v.trim().is_empty() || v.len() > 100)
        {
            return Err(ApiError::bad_request(
                "即时使用限制无效：时长为 1–1440 分钟，并发为 1–16，最短可用时长不得超过单次时长",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Access {
    pub role_id: String,
    pub permissions: BTreeSet<String>,
    pub allocation_policy: AllocationPolicy,
}
impl Access {
    pub fn allows(&self, code: &str) -> bool {
        self.permissions.contains("system.admin") || self.permissions.contains(code)
    }
}
pub fn ensure_delegable(
    access: &Access,
    permissions: &[String],
    policy: &AllocationPolicy,
) -> Result<(), ApiError> {
    if permissions.iter().any(|code| !access.allows(code)) {
        return Err(ApiError::forbidden("不能授予自己没有的权限"));
    }
    if access.allows("system.admin") || !permissions.iter().any(|p| p == "board.instant") {
        return Ok(());
    }
    let own = &access.allocation_policy;
    if policy.max_duration_minutes > own.max_duration_minutes
        || policy.max_concurrent_sessions > own.max_concurrent_sessions
        || policy.min_available_minutes < own.min_available_minutes
        || (!own.board_types.is_empty()
            && (policy.board_types.is_empty()
                || policy
                    .board_types
                    .iter()
                    .any(|t| !own.board_types.contains(t))))
    {
        return Err(ApiError::forbidden(
            "不能授予超过自身时长、并发或设备范围的即时使用权限",
        ));
    }
    Ok(())
}

pub fn expand(values: &[String]) -> Result<Vec<String>, ApiError> {
    if values.len() > CATALOG.len() {
        return Err(ApiError::bad_request("权限数量超限"));
    }
    let mut result = BTreeSet::new();
    let mut pending = values.to_vec();
    while let Some(code) = pending.pop() {
        let item = CATALOG
            .iter()
            .find(|p| p.code == code)
            .ok_or_else(|| ApiError::bad_request("包含未知权限"))?;
        if result.insert(code) {
            pending.extend(item.requires.iter().map(|v| v.to_string()));
        }
    }
    Ok(result.into_iter().collect())
}
pub async fn access(db: &Database, user: &User) -> Result<Access, ApiError> {
    let row=sqlx::query("SELECT r.id AS role_id,u.is_admin,r.permissions,r.allocation_policy FROM users u JOIN roles r ON r.id=u.role_id WHERE u.id=$1 AND NOT u.disabled AND r.enabled").bind(user.id).fetch_optional(db.pool()).await?.ok_or_else(||ApiError::unauthorized("账号或角色不可用"))?;
    let mut permissions: BTreeSet<String> = row
        .try_get::<Vec<String>, _>("permissions")?
        .into_iter()
        .collect();
    if row.try_get::<bool, _>("is_admin")? {
        permissions.insert("system.admin".into());
    }
    let allocation_policy =
        serde_json::from_value(row.try_get("allocation_policy")?).map_err(anyhow::Error::from)?;
    Ok(Access {
        role_id: row.try_get("role_id")?,
        permissions,
        allocation_policy,
    })
}
pub fn required(method: &Method, path: &str) -> Option<&'static str> {
    let read = matches!(*method, Method::GET | Method::HEAD);
    let tail = path.strip_prefix("/api/v1/admin/")?;
    let section = tail.split('/').next()?;
    Some(match section {
        "events" | "permissions" | "deployment" => "console.access",
        "dashboard" | "overview" => "dashboard.read",
        "boards" if tail.contains("/loader-updates") => {
            if read {
                "ota.read"
            } else {
                "ota.manage"
            }
        }
        "boards" if read && tail.split('/').count() == 2 => "devices.manage",
        "boards" | "virtual-devices" => {
            if read {
                "devices.read"
            } else {
                "devices.manage"
            }
        }
        "loader-devices" | "serial-ports" | "network-interfaces" => "devices.read",
        "power-actions" => "devices.power",
        "dtbs" => {
            if read {
                "dtbs.read"
            } else {
                "dtbs.manage"
            }
        }
        "loader-images" => {
            if read {
                "ota.read"
            } else {
                "ota.manage"
            }
        }
        "sessions" | "session-history" => {
            if read {
                "sessions.read"
            } else {
                "sessions.manage"
            }
        }
        "rentals" => {
            if read {
                "rentals.read"
            } else {
                "rentals.manage"
            }
        }
        "rental-requests" => {
            if read {
                "requests.read"
            } else {
                "requests.manage"
            }
        }
        "users" => {
            if read {
                "users.read"
            } else {
                "users.manage"
            }
        }
        "roles" => {
            if read {
                "roles.read"
            } else {
                "roles.manage"
            }
        }
        "site-settings" => {
            if read {
                "site.read"
            } else {
                "site.manage"
            }
        }
        "legal-history" => "site.read",
        "server-config" | "tftp" => {
            if read {
                "server.read"
            } else {
                "server.manage"
            }
        }
        "account-settings" => {
            if read {
                "security.read"
            } else {
                "security.manage"
            }
        }
        "mail-deliveries" => "security.read",
        "audits" if tail.ends_with("/export") => "audits.export",
        "audits" if tail.ends_with("/cleanup") => "audits.manage",
        "audits" => "audits.read",
        _ => return None,
    })
}
pub fn topic_required(topic: &str) -> Option<&'static str> {
    Some(match topic {
        "site" | "device_catalog" | "security_revision" | "deployment" | "permissions" => {
            "console.access"
        }
        "boards" | "board_revision" | "runtimes" | "loaders" | "virtual" | "serial"
        | "serial_manager" | "network" => "devices.read",
        "board_editor" => "devices.manage",
        "dtbs" => "dtbs.read",
        "sessions" | "session_history" => "sessions.read",
        "ota" => "ota.read",
        "power_actions" => "devices.power",
        "users" => "users.read",
        "roles" => "roles.read",
        "rentals" => "rentals.read",
        "rental_requests" => "requests.read",
        "legal_history" => "site.read",
        "account_settings" | "mail_deliveries" => "security.read",
        "server" | "tftp" | "tftp_status" => "server.read",
        "overview" | "dashboard" => "dashboard.read",
        "audit" => "audits.read",
        _ => return None,
    })
}
pub async fn catalog(
    axum::extract::State(_): axum::extract::State<AppState>,
) -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!(CATALOG))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capability_dependencies_are_expanded_but_unknown_codes_and_routes_fail_closed() {
        let list = expand(&["requests.manage".into()]).unwrap();
        for required in [
            "requests.read",
            "rentals.manage",
            "users.read",
            "console.access",
        ] {
            assert!(list.contains(&required.into()));
        }
        assert_eq!(expand(&list).unwrap(), list);
        assert!(expand(&["unknown.manage".into()]).is_err());
        assert_eq!(
            required(&Method::GET, "/api/v1/admin/boards/one"),
            Some("devices.manage")
        );
        assert_eq!(
            required(&Method::POST, "/api/v1/admin/boards"),
            Some("devices.manage")
        );
        assert_eq!(required(&Method::GET, "/api/v1/admin/unclassified"), None);
        assert_eq!(topic_required("unclassified"), None);
    }
    #[test]
    fn instant_limits_are_bounded_and_pool_is_explicit() {
        AllocationPolicy::default().validate().unwrap();
        for policy in [
            AllocationPolicy {
                max_duration_minutes: 0,
                ..Default::default()
            },
            AllocationPolicy {
                max_concurrent_sessions: 17,
                ..Default::default()
            },
            AllocationPolicy {
                min_available_minutes: 121,
                ..Default::default()
            },
            AllocationPolicy {
                board_types: vec!["".into()],
                ..Default::default()
            },
        ] {
            assert!(policy.validate().is_err());
        }
    }
}

#[cfg(test)]
mod delegation_tests {
    use super::*;
    #[test]
    fn role_managers_cannot_expand_instant_limits_or_pool_beyond_their_own_grant() {
        let access = Access {
            role_id: "limited".into(),
            permissions: ["board.use", "board.instant", "roles.manage"]
                .map(str::to_owned)
                .into_iter()
                .collect(),
            allocation_policy: AllocationPolicy {
                max_duration_minutes: 60,
                max_concurrent_sessions: 1,
                min_available_minutes: 10,
                board_types: vec!["arm".into()],
            },
        };
        let codes = vec!["board.instant".into()];
        ensure_delegable(&access, &codes, &access.allocation_policy).unwrap();
        for policy in [
            AllocationPolicy {
                max_duration_minutes: 120,
                ..access.allocation_policy.clone()
            },
            AllocationPolicy {
                max_concurrent_sessions: 2,
                ..access.allocation_policy.clone()
            },
            AllocationPolicy {
                board_types: vec![],
                ..access.allocation_policy.clone()
            },
            AllocationPolicy {
                board_types: vec!["x86".into()],
                ..access.allocation_policy.clone()
            },
        ] {
            assert!(ensure_delegable(&access, &codes, &policy).is_err());
        }
        assert!(
            ensure_delegable(&access, &["system.admin".into()], &access.allocation_policy).is_err()
        );
    }
}
