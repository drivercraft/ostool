use anyhow::Context;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{MySqlPool, Row};
use uuid::Uuid;

use crate::BoardConfig;
use crate::storage::{
    AuditLog, AuditLogRepository, AuthSession, AuthSessionRepository, BoardConfigRepository,
    DtbMetadata, DtbMetadataRepository, Lease, LeaseRepository, LeaseState, NewAuditLog, NewLease,
    NewRole, NewUser, Permission, RbacRepository, Role, SiteSettingValue, SiteSettings,
    SiteSettingsRepository, UpsertDtbMetadata, User, UserProfile, UserRepository,
    default_site_setting_rows, parse_time, site_settings_from_values, site_settings_to_values,
};

const MIGRATION_RBAC_PLATFORM: &str = "0001_rbac_platform";
const MIGRATION_BOARD_CONFIGS: &str = "0002_board_configs";
const MIGRATION_DTB_AUDIT: &str = "0003_dtb_audit";
const MIGRATION_STANDARD_FIELDS: &str = "0004_standard_profile_fields";
const MIGRATION_SITE_SETTINGS: &str = "0005_site_settings";

const BUILTIN_PERMISSIONS: &[(&str, &str, &str)] = &[
    ("overview.read", "查看概览", "查看站点运行情况和统计数据"),
    (
        "resources.manage",
        "管理资源",
        "管理开发板、DTB 和 TFTP 配置",
    ),
    ("rentals.manage", "管理租赁", "查看和释放租赁、会话租约"),
    ("users.manage", "管理用户", "创建、禁用、更新用户和重置密码"),
    (
        "roles.manage",
        "管理角色权限",
        "创建、修改和删除角色权限配置",
    ),
    ("settings.manage", "管理系统设置", "修改服务器运行配置"),
];

#[derive(Clone)]
pub struct MysqlStorage {
    pool: MySqlPool,
}

impl MysqlStorage {
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        ensure_mysql_database_url(url)?;
        let pool = MySqlPool::connect(url)
            .await
            .with_context(|| format!("failed to connect MySQL database `{url}`"))?;
        let storage = Self { pool };
        storage.migrate().await?;
        Ok(storage)
    }

    async fn migrate(&self) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS schema_migrations (
                version VARCHAR(255) PRIMARY KEY NOT NULL,
                applied_at VARCHAR(255) NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        if !self.is_migration_applied(MIGRATION_RBAC_PLATFORM).await? {
            self.migrate_schema().await?;
            self.mark_migration_applied(MIGRATION_RBAC_PLATFORM).await?;
        }
        if !self.is_migration_applied(MIGRATION_BOARD_CONFIGS).await? {
            self.migrate_board_config_tables().await?;
            self.mark_migration_applied(MIGRATION_BOARD_CONFIGS).await?;
        }
        if !self.is_migration_applied(MIGRATION_DTB_AUDIT).await? {
            self.migrate_dtb_audit_tables().await?;
            self.mark_migration_applied(MIGRATION_DTB_AUDIT).await?;
        }
        if !self.is_migration_applied(MIGRATION_STANDARD_FIELDS).await? {
            self.migrate_standard_fields().await?;
            self.mark_migration_applied(MIGRATION_STANDARD_FIELDS)
                .await?;
        }
        if !self.is_migration_applied(MIGRATION_SITE_SETTINGS).await? {
            self.migrate_site_settings().await?;
            self.mark_migration_applied(MIGRATION_SITE_SETTINGS).await?;
        }
        Ok(())
    }

    async fn is_migration_applied(&self, version: &str) -> anyhow::Result<bool> {
        let applied = sqlx::query("SELECT version FROM schema_migrations WHERE version = ?")
            .bind(version)
            .fetch_optional(&self.pool)
            .await?;
        Ok(applied.is_some())
    }

    async fn mark_migration_applied(&self, version: &str) -> anyhow::Result<()> {
        sqlx::query("INSERT IGNORE INTO schema_migrations (version, applied_at) VALUES (?, ?)")
            .bind(version)
            .bind(Utc::now().to_rfc3339())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn migrate_schema(&self) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS users (
                id VARCHAR(255) PRIMARY KEY NOT NULL,
                username VARCHAR(255) NOT NULL UNIQUE,
                display_name VARCHAR(255) NOT NULL,
                email VARCHAR(255) NOT NULL,
                password_hash VARCHAR(255) NOT NULL,
                disabled TINYINT NOT NULL DEFAULT 0,
                created_at VARCHAR(255) NOT NULL,
                updated_at VARCHAR(255) NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        self.migrate_rbac_tables().await?;
        self.migrate_board_config_tables().await?;
        self.migrate_dtb_audit_tables().await?;
        self.migrate_site_settings().await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS auth_sessions (
                id VARCHAR(255) PRIMARY KEY NOT NULL,
                user_id VARCHAR(255) NOT NULL,
                token_hash VARCHAR(255) NOT NULL UNIQUE,
                expires_at VARCHAR(255) NOT NULL,
                created_at VARCHAR(255) NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS leases (
                id VARCHAR(255) PRIMARY KEY NOT NULL,
                user_id VARCHAR(255) NOT NULL,
                session_id VARCHAR(255) NOT NULL UNIQUE,
                board_id VARCHAR(255) NOT NULL,
                board_type VARCHAR(255) NOT NULL,
                required_tags_json TEXT NOT NULL,
                state VARCHAR(255) NOT NULL,
                created_at VARCHAR(255) NOT NULL,
                expires_at VARCHAR(255) NOT NULL,
                released_at VARCHAR(255),
                failure_message TEXT,
                INDEX idx_leases_user_id (user_id),
                FOREIGN KEY(user_id) REFERENCES users(id)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn migrate_board_config_tables(&self) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS board_configs (
                id VARCHAR(255) PRIMARY KEY NOT NULL,
                board_type VARCHAR(255) NOT NULL,
                config_json TEXT NOT NULL,
                created_at VARCHAR(255) NOT NULL,
                updated_at VARCHAR(255) NOT NULL,
                INDEX idx_board_configs_board_type (board_type)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn migrate_rbac_tables(&self) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS permissions (
                id VARCHAR(255) PRIMARY KEY NOT NULL,
                code VARCHAR(255) NOT NULL UNIQUE,
                name VARCHAR(255) NOT NULL,
                description VARCHAR(255) NOT NULL,
                created_at VARCHAR(255) NOT NULL,
                updated_at VARCHAR(255) NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS roles (
                id VARCHAR(255) PRIMARY KEY NOT NULL,
                name VARCHAR(255) NOT NULL UNIQUE,
                display_name VARCHAR(255) NOT NULL,
                description VARCHAR(255) NOT NULL,
                `system` TINYINT NOT NULL DEFAULT 0,
                created_at VARCHAR(255) NOT NULL,
                updated_at VARCHAR(255) NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS role_permissions (
                role_id VARCHAR(255) NOT NULL,
                permission_id VARCHAR(255) NOT NULL,
                PRIMARY KEY(role_id, permission_id),
                FOREIGN KEY(role_id) REFERENCES roles(id) ON DELETE CASCADE,
                FOREIGN KEY(permission_id) REFERENCES permissions(id) ON DELETE CASCADE
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS user_roles (
                user_id VARCHAR(255) NOT NULL,
                role_id VARCHAR(255) NOT NULL,
                PRIMARY KEY(user_id, role_id),
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE,
                FOREIGN KEY(role_id) REFERENCES roles(id) ON DELETE CASCADE
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        self.seed_builtin_rbac().await?;
        Ok(())
    }

    async fn migrate_dtb_audit_tables(&self) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS dtb_files (
                id VARCHAR(255) PRIMARY KEY NOT NULL,
                name VARCHAR(255) NOT NULL UNIQUE,
                storage_path VARCHAR(1024) NOT NULL,
                size_bytes BIGINT NOT NULL,
                sha256 VARCHAR(255) NOT NULL,
                description TEXT,
                uploaded_by VARCHAR(255),
                created_at VARCHAR(255) NOT NULL,
                updated_at VARCHAR(255) NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS audit_logs (
                id VARCHAR(255) PRIMARY KEY NOT NULL,
                actor_user_id VARCHAR(255),
                actor_username VARCHAR(255),
                action VARCHAR(255) NOT NULL,
                target_type VARCHAR(255) NOT NULL,
                target_id VARCHAR(255),
                outcome VARCHAR(255) NOT NULL,
                ip_address VARCHAR(255),
                user_agent VARCHAR(512),
                request_id VARCHAR(255),
                metadata_json TEXT NOT NULL,
                created_at VARCHAR(255) NOT NULL,
                INDEX idx_audit_logs_created_at (created_at),
                INDEX idx_audit_logs_target (target_type, target_id)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn migrate_standard_fields(&self) -> anyhow::Result<()> {
        self.add_column_if_missing("users", "nickname", "VARCHAR(255)")
            .await?;
        self.add_column_if_missing("users", "avatar_url", "VARCHAR(1024)")
            .await?;
        self.add_column_if_missing("users", "phone", "VARCHAR(255)")
            .await?;
        self.add_column_if_missing("users", "department", "VARCHAR(255)")
            .await?;
        self.add_column_if_missing("users", "title", "VARCHAR(255)")
            .await?;
        self.add_column_if_missing("users", "last_login_at", "VARCHAR(255)")
            .await?;
        self.add_column_if_missing("auth_sessions", "ip_address", "VARCHAR(255)")
            .await?;
        self.add_column_if_missing("auth_sessions", "user_agent", "VARCHAR(512)")
            .await?;
        self.add_column_if_missing("auth_sessions", "last_seen_at", "VARCHAR(255)")
            .await?;
        self.add_column_if_missing("auth_sessions", "revoked_at", "VARCHAR(255)")
            .await?;
        self.add_column_if_missing("leases", "updated_at", "VARCHAR(255)")
            .await?;
        sqlx::query("UPDATE leases SET updated_at = created_at WHERE updated_at IS NULL")
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn migrate_site_settings(&self) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS site_settings (
                `key` VARCHAR(255) PRIMARY KEY NOT NULL,
                value_json TEXT NOT NULL,
                value_type VARCHAR(255) NOT NULL,
                group_name VARCHAR(255) NOT NULL,
                name VARCHAR(255) NOT NULL,
                description VARCHAR(1024) NOT NULL,
                readonly TINYINT NOT NULL DEFAULT 0,
                `sensitive` TINYINT NOT NULL DEFAULT 0,
                updated_by VARCHAR(255),
                created_at VARCHAR(255) NOT NULL,
                updated_at VARCHAR(255) NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        self.seed_site_settings().await?;
        Ok(())
    }

    async fn seed_site_settings(&self) -> anyhow::Result<()> {
        for setting in default_site_setting_rows() {
            sqlx::query(
                r#"
                INSERT IGNORE INTO site_settings
                    (`key`, value_json, value_type, group_name, name, description, readonly, `sensitive`, created_at, updated_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(setting.key)
            .bind(setting.value_json)
            .bind(setting.value_type)
            .bind(setting.group_name)
            .bind(setting.name)
            .bind(setting.description)
            .bind(if setting.readonly { 1 } else { 0 })
            .bind(if setting.sensitive { 1 } else { 0 })
            .bind(setting.created_at)
            .bind(setting.updated_at)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    async fn add_column_if_missing(
        &self,
        table: &str,
        column: &str,
        definition: &str,
    ) -> anyhow::Result<()> {
        let row = sqlx::query(
            r#"
            SELECT COUNT(*) AS count
            FROM INFORMATION_SCHEMA.COLUMNS
            WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ? AND COLUMN_NAME = ?
            "#,
        )
        .bind(table)
        .bind(column)
        .fetch_one(&self.pool)
        .await?;
        let count: i64 = row.try_get("count")?;
        if count == 0 {
            sqlx::query(&format!(
                "ALTER TABLE `{table}` ADD COLUMN `{column}` {definition}"
            ))
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    async fn seed_builtin_rbac(&self) -> anyhow::Result<()> {
        let now = Utc::now().to_rfc3339();
        for (code, name, description) in BUILTIN_PERMISSIONS {
            sqlx::query(
                r#"
                INSERT IGNORE INTO permissions (id, code, name, description, created_at, updated_at)
                VALUES (?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(Uuid::new_v4().to_string())
            .bind(code)
            .bind(name)
            .bind(description)
            .bind(&now)
            .bind(&now)
            .execute(&self.pool)
            .await?;
        }

        for (name, display_name, description, system) in [
            ("admin", "管理员", "拥有平台全部管理权限", 1),
            ("user", "普通用户", "可登录平台并租赁开发板", 1),
        ] {
            sqlx::query(
                r#"
                INSERT IGNORE INTO roles (id, name, display_name, description, `system`, created_at, updated_at)
                VALUES (?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(Uuid::new_v4().to_string())
            .bind(name)
            .bind(display_name)
            .bind(description)
            .bind(system)
            .bind(&now)
            .bind(&now)
            .execute(&self.pool)
            .await?;
        }

        let admin_role_id: String = sqlx::query("SELECT id FROM roles WHERE name = 'admin'")
            .fetch_one(&self.pool)
            .await?
            .try_get("id")?;
        let user_role_id: String = sqlx::query("SELECT id FROM roles WHERE name = 'user'")
            .fetch_one(&self.pool)
            .await?
            .try_get("id")?;
        let permissions = sqlx::query("SELECT id, code FROM permissions")
            .fetch_all(&self.pool)
            .await?;
        for row in &permissions {
            let permission_id: String = row.try_get("id")?;
            sqlx::query(
                "INSERT IGNORE INTO role_permissions (role_id, permission_id) VALUES (?, ?)",
            )
            .bind(&admin_role_id)
            .bind(&permission_id)
            .execute(&self.pool)
            .await?;
            let code: String = row.try_get("code")?;
            if code == "overview.read" {
                sqlx::query(
                    "INSERT IGNORE INTO role_permissions (role_id, permission_id) VALUES (?, ?)",
                )
                .bind(&user_role_id)
                .bind(&permission_id)
                .execute(&self.pool)
                .await?;
            }
        }

        Ok(())
    }
}

#[async_trait]
impl SiteSettingsRepository for MysqlStorage {
    async fn get_site_settings(&self) -> anyhow::Result<SiteSettings> {
        self.seed_site_settings().await?;
        let rows = sqlx::query("SELECT `key`, value_json, updated_at FROM site_settings")
            .fetch_all(&self.pool)
            .await?;
        let values = rows
            .into_iter()
            .map(|row| {
                Ok(SiteSettingValue {
                    key: row.try_get("key")?,
                    value_json: row.try_get("value_json")?,
                    updated_at: row.try_get("updated_at")?,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        site_settings_from_values(values)
    }

    async fn update_site_settings(
        &self,
        mut settings: SiteSettings,
        updated_by: Option<String>,
    ) -> anyhow::Result<SiteSettings> {
        settings.validate()?;
        settings.updated_at = Utc::now();
        self.seed_site_settings().await?;
        let updated_at = settings.updated_at.to_rfc3339();
        for (key, value_json) in site_settings_to_values(&settings)? {
            sqlx::query(
                r#"
                UPDATE site_settings
                SET value_json = ?, updated_by = ?, updated_at = ?
                WHERE `key` = ?
                "#,
            )
            .bind(value_json)
            .bind(&updated_by)
            .bind(&updated_at)
            .bind(key)
            .execute(&self.pool)
            .await?;
        }
        self.get_site_settings().await
    }
}

#[async_trait]
impl DtbMetadataRepository for MysqlStorage {
    async fn upsert_dtb_metadata(
        &self,
        metadata: UpsertDtbMetadata,
    ) -> anyhow::Result<DtbMetadata> {
        let existing = self.find_dtb_metadata_by_name(&metadata.name).await?;
        let now = Utc::now().to_rfc3339();
        if let Some(existing) = existing {
            sqlx::query(
                r#"
                UPDATE dtb_files
                SET storage_path = ?, size_bytes = ?, sha256 = ?, description = ?, uploaded_by = ?, updated_at = ?
                WHERE id = ?
                "#,
            )
            .bind(metadata.storage_path)
            .bind(metadata.size_bytes)
            .bind(metadata.sha256)
            .bind(metadata.description)
            .bind(metadata.uploaded_by)
            .bind(&now)
            .bind(&existing.id)
            .execute(&self.pool)
            .await?;
            self.find_dtb_metadata_by_name(&existing.name)
                .await?
                .ok_or_else(|| {
                    anyhow::anyhow!("updated DTB metadata `{}` disappeared", existing.name)
                })
        } else {
            let id = Uuid::new_v4().to_string();
            sqlx::query(
                r#"
                INSERT INTO dtb_files
                    (id, name, storage_path, size_bytes, sha256, description, uploaded_by, created_at, updated_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&id)
            .bind(metadata.name)
            .bind(metadata.storage_path)
            .bind(metadata.size_bytes)
            .bind(metadata.sha256)
            .bind(metadata.description)
            .bind(metadata.uploaded_by)
            .bind(&now)
            .bind(&now)
            .execute(&self.pool)
            .await?;
            let row = sqlx::query("SELECT * FROM dtb_files WHERE id = ?")
                .bind(&id)
                .fetch_one(&self.pool)
                .await?;
            dtb_metadata_from_row(&row)
        }
    }

    async fn list_dtb_metadata(&self) -> anyhow::Result<Vec<DtbMetadata>> {
        let rows = sqlx::query("SELECT * FROM dtb_files ORDER BY name ASC")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(dtb_metadata_from_row).collect()
    }

    async fn find_dtb_metadata_by_name(&self, name: &str) -> anyhow::Result<Option<DtbMetadata>> {
        sqlx::query("SELECT * FROM dtb_files WHERE name = ?")
            .bind(name)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(dtb_metadata_from_row)
            .transpose()
    }

    async fn rename_dtb_metadata(&self, current_name: &str, new_name: &str) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE dtb_files SET name = ?, storage_path = ?, updated_at = ? WHERE name = ?",
        )
        .bind(new_name)
        .bind(new_name)
        .bind(Utc::now().to_rfc3339())
        .bind(current_name)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_dtb_metadata_by_name(&self, name: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM dtb_files WHERE name = ?")
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[async_trait]
impl AuditLogRepository for MysqlStorage {
    async fn create_audit_log(&self, log: NewAuditLog) -> anyhow::Result<AuditLog> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"
            INSERT INTO audit_logs
                (id, actor_user_id, actor_username, action, target_type, target_id, outcome, ip_address, user_agent, request_id, metadata_json, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(log.actor_user_id)
        .bind(log.actor_username)
        .bind(log.action)
        .bind(log.target_type)
        .bind(log.target_id)
        .bind(log.outcome)
        .bind(log.ip_address)
        .bind(log.user_agent)
        .bind(log.request_id)
        .bind(log.metadata_json)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        let row = sqlx::query("SELECT * FROM audit_logs WHERE id = ?")
            .bind(&id)
            .fetch_one(&self.pool)
            .await?;
        audit_log_from_row(&row)
    }

    async fn list_audit_logs(&self, limit: i64) -> anyhow::Result<Vec<AuditLog>> {
        let rows = sqlx::query("SELECT * FROM audit_logs ORDER BY created_at DESC LIMIT ?")
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(audit_log_from_row).collect()
    }
}

fn ensure_mysql_database_url(url: &str) -> anyhow::Result<()> {
    if !url.starts_with("mysql://") {
        anyhow::bail!("MySQL database URL must start with `mysql://`");
    }
    Ok(())
}

fn user_from_row(row: &sqlx::mysql::MySqlRow) -> anyhow::Result<User> {
    Ok(User {
        id: row.try_get("id")?,
        username: row.try_get("username")?,
        display_name: row.try_get("display_name")?,
        nickname: row.try_get("nickname")?,
        avatar_url: row.try_get("avatar_url")?,
        email: row.try_get("email")?,
        phone: row.try_get("phone")?,
        department: row.try_get("department")?,
        title: row.try_get("title")?,
        password_hash: row.try_get("password_hash")?,
        disabled: row.try_get::<i64, _>("disabled")? != 0,
        last_login_at: row
            .try_get::<Option<String>, _>("last_login_at")?
            .map(|value| parse_time(&value))
            .transpose()?,
        created_at: parse_time(row.try_get::<String, _>("created_at")?.as_str())?,
        updated_at: parse_time(row.try_get::<String, _>("updated_at")?.as_str())?,
    })
}

fn auth_session_from_row(row: &sqlx::mysql::MySqlRow) -> anyhow::Result<AuthSession> {
    Ok(AuthSession {
        id: row.try_get("id")?,
        user_id: row.try_get("user_id")?,
        token_hash: row.try_get("token_hash")?,
        ip_address: row.try_get("ip_address")?,
        user_agent: row.try_get("user_agent")?,
        expires_at: parse_time(row.try_get::<String, _>("expires_at")?.as_str())?,
        last_seen_at: row
            .try_get::<Option<String>, _>("last_seen_at")?
            .map(|value| parse_time(&value))
            .transpose()?,
        revoked_at: row
            .try_get::<Option<String>, _>("revoked_at")?
            .map(|value| parse_time(&value))
            .transpose()?,
        created_at: parse_time(row.try_get::<String, _>("created_at")?.as_str())?,
    })
}

fn lease_from_row(row: &sqlx::mysql::MySqlRow) -> anyhow::Result<Lease> {
    let required_tags_json: String = row.try_get("required_tags_json")?;
    let created_at = parse_time(row.try_get::<String, _>("created_at")?.as_str())?;
    let updated_at = row
        .try_get::<Option<String>, _>("updated_at")?
        .map(|value| parse_time(&value))
        .transpose()?
        .unwrap_or(created_at);
    Ok(Lease {
        id: row.try_get("id")?,
        user_id: row.try_get("user_id")?,
        session_id: row.try_get("session_id")?,
        board_id: row.try_get("board_id")?,
        board_type: row.try_get("board_type")?,
        required_tags: serde_json::from_str(&required_tags_json)
            .context("failed to parse lease required_tags_json")?,
        state: LeaseState::from_str(row.try_get::<String, _>("state")?.as_str())?,
        created_at,
        updated_at,
        expires_at: parse_time(row.try_get::<String, _>("expires_at")?.as_str())?,
        released_at: row
            .try_get::<Option<String>, _>("released_at")?
            .map(|value| parse_time(&value))
            .transpose()?,
        failure_message: row.try_get("failure_message")?,
    })
}

fn board_config_from_row(row: &sqlx::mysql::MySqlRow) -> anyhow::Result<BoardConfig> {
    let config_json: String = row.try_get("config_json")?;
    let board: BoardConfig =
        serde_json::from_str(&config_json).context("failed to parse board config_json")?;
    Ok(board)
}

fn dtb_metadata_from_row(row: &sqlx::mysql::MySqlRow) -> anyhow::Result<DtbMetadata> {
    Ok(DtbMetadata {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        storage_path: row.try_get("storage_path")?,
        size_bytes: row.try_get("size_bytes")?,
        sha256: row.try_get("sha256")?,
        description: row.try_get("description")?,
        uploaded_by: row.try_get("uploaded_by")?,
        created_at: parse_time(row.try_get::<String, _>("created_at")?.as_str())?,
        updated_at: parse_time(row.try_get::<String, _>("updated_at")?.as_str())?,
    })
}

fn audit_log_from_row(row: &sqlx::mysql::MySqlRow) -> anyhow::Result<AuditLog> {
    Ok(AuditLog {
        id: row.try_get("id")?,
        actor_user_id: row.try_get("actor_user_id")?,
        actor_username: row.try_get("actor_username")?,
        action: row.try_get("action")?,
        target_type: row.try_get("target_type")?,
        target_id: row.try_get("target_id")?,
        outcome: row.try_get("outcome")?,
        ip_address: row.try_get("ip_address")?,
        user_agent: row.try_get("user_agent")?,
        request_id: row.try_get("request_id")?,
        metadata_json: row.try_get("metadata_json")?,
        created_at: parse_time(row.try_get::<String, _>("created_at")?.as_str())?,
    })
}

fn permission_from_row(row: &sqlx::mysql::MySqlRow) -> anyhow::Result<Permission> {
    Ok(Permission {
        id: row.try_get("id")?,
        code: row.try_get("code")?,
        name: row.try_get("name")?,
        description: row.try_get("description")?,
        created_at: parse_time(row.try_get::<String, _>("created_at")?.as_str())?,
        updated_at: parse_time(row.try_get::<String, _>("updated_at")?.as_str())?,
    })
}

fn role_from_row(row: &sqlx::mysql::MySqlRow) -> anyhow::Result<Role> {
    Ok(Role {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        display_name: row.try_get("display_name")?,
        description: row.try_get("description")?,
        system: row.try_get::<i64, _>("system")? != 0,
        created_at: parse_time(row.try_get::<String, _>("created_at")?.as_str())?,
        updated_at: parse_time(row.try_get::<String, _>("updated_at")?.as_str())?,
    })
}

#[async_trait]
impl UserRepository for MysqlStorage {
    async fn create_user(&self, user: NewUser) -> anyhow::Result<User> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let role_names = if user.role_names.is_empty() {
            vec!["user".to_string()]
        } else {
            user.role_names
        };
        sqlx::query(
            r#"
            INSERT INTO users
                (id, username, display_name, nickname, avatar_url, email, phone, department, title, password_hash, disabled, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(user.username)
        .bind(user.display_name)
        .bind(user.profile.nickname)
        .bind(user.profile.avatar_url)
        .bind(user.email)
        .bind(user.profile.phone)
        .bind(user.profile.department)
        .bind(user.profile.title)
        .bind(user.password_hash)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;
        self.set_user_role_names(&id, &role_names).await?;
        self.find_user_by_id(&id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("created user `{id}` disappeared"))
    }

    async fn list_users(&self) -> anyhow::Result<Vec<User>> {
        let rows = sqlx::query("SELECT * FROM users ORDER BY username ASC")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(user_from_row).collect()
    }

    async fn find_user_by_id(&self, user_id: &str) -> anyhow::Result<Option<User>> {
        sqlx::query("SELECT * FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(user_from_row)
            .transpose()
    }

    async fn find_user_by_username(&self, username: &str) -> anyhow::Result<Option<User>> {
        sqlx::query("SELECT * FROM users WHERE username = ?")
            .bind(username)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(user_from_row)
            .transpose()
    }

    async fn update_user(
        &self,
        user_id: &str,
        display_name: String,
        email: String,
        profile: UserProfile,
        disabled: bool,
    ) -> anyhow::Result<Option<User>> {
        sqlx::query(
            r#"
            UPDATE users
            SET display_name = ?, email = ?, nickname = ?, avatar_url = ?, phone = ?, department = ?, title = ?, disabled = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(display_name)
        .bind(email)
        .bind(profile.nickname)
        .bind(profile.avatar_url)
        .bind(profile.phone)
        .bind(profile.department)
        .bind(profile.title)
        .bind(if disabled { 1 } else { 0 })
        .bind(Utc::now().to_rfc3339())
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        self.find_user_by_id(user_id).await
    }

    async fn mark_user_login(&self, user_id: &str, at: DateTime<Utc>) -> anyhow::Result<()> {
        sqlx::query("UPDATE users SET last_login_at = ?, updated_at = ? WHERE id = ?")
            .bind(at.to_rfc3339())
            .bind(at.to_rfc3339())
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn update_password_hash(
        &self,
        user_id: &str,
        password_hash: String,
    ) -> anyhow::Result<()> {
        sqlx::query("UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?")
            .bind(password_hash)
            .bind(Utc::now().to_rfc3339())
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn set_user_disabled(&self, user_id: &str, disabled: bool) -> anyhow::Result<()> {
        sqlx::query("UPDATE users SET disabled = ?, updated_at = ? WHERE id = ?")
            .bind(if disabled { 1 } else { 0 })
            .bind(Utc::now().to_rfc3339())
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn user_count(&self) -> anyhow::Result<i64> {
        let row = sqlx::query("SELECT COUNT(*) AS count FROM users")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.try_get("count")?)
    }
}

#[async_trait]
impl AuthSessionRepository for MysqlStorage {
    async fn create_auth_session(&self, session: AuthSession) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO auth_sessions
                (id, user_id, token_hash, ip_address, user_agent, expires_at, last_seen_at, revoked_at, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(session.id)
        .bind(session.user_id)
        .bind(session.token_hash)
        .bind(session.ip_address)
        .bind(session.user_agent)
        .bind(session.expires_at.to_rfc3339())
        .bind(session.last_seen_at.map(|value| value.to_rfc3339()))
        .bind(session.revoked_at.map(|value| value.to_rfc3339()))
        .bind(session.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn find_auth_session_by_token_hash(
        &self,
        token_hash: &str,
    ) -> anyhow::Result<Option<AuthSession>> {
        sqlx::query("SELECT * FROM auth_sessions WHERE token_hash = ?")
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(auth_session_from_row)
            .transpose()
    }

    async fn delete_auth_session_by_token_hash(&self, token_hash: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM auth_sessions WHERE token_hash = ?")
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn delete_expired_auth_sessions(&self, now: DateTime<Utc>) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM auth_sessions WHERE expires_at <= ?")
            .bind(now.to_rfc3339())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[async_trait]
impl LeaseRepository for MysqlStorage {
    async fn create_lease(&self, lease: NewLease) -> anyhow::Result<Lease> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        sqlx::query(
            r#"
            INSERT INTO leases
                (id, user_id, session_id, board_id, board_type, required_tags_json, state, created_at, updated_at, expires_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(lease.user_id)
        .bind(lease.session_id)
        .bind(lease.board_id)
        .bind(lease.board_type)
        .bind(serde_json::to_string(&lease.required_tags)?)
        .bind(LeaseState::Active.as_str())
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(lease.expires_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        self.find_lease(&id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("created lease `{id}` disappeared"))
    }

    async fn list_leases(&self) -> anyhow::Result<Vec<Lease>> {
        let rows = sqlx::query("SELECT * FROM leases ORDER BY created_at DESC")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(lease_from_row).collect()
    }

    async fn list_leases_for_user(&self, user_id: &str) -> anyhow::Result<Vec<Lease>> {
        let rows = sqlx::query("SELECT * FROM leases WHERE user_id = ? ORDER BY created_at DESC")
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(lease_from_row).collect()
    }

    async fn find_lease(&self, lease_id: &str) -> anyhow::Result<Option<Lease>> {
        sqlx::query("SELECT * FROM leases WHERE id = ?")
            .bind(lease_id)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(lease_from_row)
            .transpose()
    }

    async fn mark_lease_state(
        &self,
        lease_id: &str,
        state: LeaseState,
        released_at: Option<DateTime<Utc>>,
        failure_message: Option<String>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE leases SET state = ?, released_at = ?, failure_message = ?, updated_at = ? WHERE id = ?",
        )
        .bind(state.as_str())
        .bind(released_at.map(|value| value.to_rfc3339()))
        .bind(failure_message)
        .bind(Utc::now().to_rfc3339())
        .bind(lease_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn update_lease_expiry(
        &self,
        lease_id: &str,
        expires_at: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        sqlx::query("UPDATE leases SET expires_at = ?, updated_at = ? WHERE id = ?")
            .bind(expires_at.to_rfc3339())
            .bind(Utc::now().to_rfc3339())
            .bind(lease_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[async_trait]
impl BoardConfigRepository for MysqlStorage {
    async fn create_board_config(&self, board: BoardConfig) -> anyhow::Result<BoardConfig> {
        let now = Utc::now().to_rfc3339();
        let config_json = serde_json::to_string(&board)?;
        sqlx::query(
            r#"
            INSERT INTO board_configs (id, board_type, config_json, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(&board.id)
        .bind(&board.board_type)
        .bind(config_json)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(board)
    }

    async fn list_board_configs(&self) -> anyhow::Result<Vec<BoardConfig>> {
        let rows = sqlx::query("SELECT * FROM board_configs ORDER BY id ASC")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(board_config_from_row).collect()
    }

    async fn update_board_config(
        &self,
        current_board_id: &str,
        board: BoardConfig,
    ) -> anyhow::Result<BoardConfig> {
        let now = Utc::now().to_rfc3339();
        let config_json = serde_json::to_string(&board)?;
        if current_board_id == board.id {
            sqlx::query(
                r#"
                UPDATE board_configs
                SET board_type = ?, config_json = ?, updated_at = ?
                WHERE id = ?
                "#,
            )
            .bind(&board.board_type)
            .bind(config_json)
            .bind(&now)
            .bind(current_board_id)
            .execute(&self.pool)
            .await?;
        } else {
            let mut tx = self.pool.begin().await?;
            sqlx::query("DELETE FROM board_configs WHERE id = ?")
                .bind(current_board_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                r#"
                INSERT INTO board_configs (id, board_type, config_json, created_at, updated_at)
                VALUES (?, ?, ?, ?, ?)
                "#,
            )
            .bind(&board.id)
            .bind(&board.board_type)
            .bind(config_json)
            .bind(&now)
            .bind(&now)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
        }
        Ok(board)
    }

    async fn delete_board_config(&self, board_id: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM board_configs WHERE id = ?")
            .bind(board_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[async_trait]
impl RbacRepository for MysqlStorage {
    async fn list_permissions(&self) -> anyhow::Result<Vec<Permission>> {
        let rows = sqlx::query("SELECT * FROM permissions ORDER BY code ASC")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(permission_from_row).collect()
    }

    async fn list_roles(&self) -> anyhow::Result<Vec<Role>> {
        let rows = sqlx::query("SELECT * FROM roles ORDER BY `system` DESC, name ASC")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(role_from_row).collect()
    }

    async fn create_role(&self, role: NewRole) -> anyhow::Result<Role> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"
            INSERT INTO roles (id, name, display_name, description, `system`, created_at, updated_at)
            VALUES (?, ?, ?, ?, 0, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(role.name)
        .bind(role.display_name)
        .bind(role.description)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        self.replace_role_permissions(&id, &role.permission_ids)
            .await?;
        self.find_role_by_id(&id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("created role `{id}` disappeared"))
    }

    async fn update_role(
        &self,
        role_id: &str,
        display_name: String,
        description: String,
        permission_ids: Vec<String>,
    ) -> anyhow::Result<Option<Role>> {
        sqlx::query(
            r#"
            UPDATE roles
            SET display_name = ?, description = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(display_name)
        .bind(description)
        .bind(Utc::now().to_rfc3339())
        .bind(role_id)
        .execute(&self.pool)
        .await?;
        self.replace_role_permissions(role_id, &permission_ids)
            .await?;
        self.find_role_by_id(role_id).await
    }

    async fn delete_role(&self, role_id: &str) -> anyhow::Result<()> {
        let role = self.find_role_by_id(role_id).await?;
        if role.as_ref().map(|item| item.system).unwrap_or(false) {
            anyhow::bail!("system role cannot be deleted");
        }
        sqlx::query("DELETE FROM roles WHERE id = ?")
            .bind(role_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn role_permissions(&self, role_id: &str) -> anyhow::Result<Vec<Permission>> {
        let rows = sqlx::query(
            r#"
            SELECT permissions.*
            FROM permissions
            INNER JOIN role_permissions ON role_permissions.permission_id = permissions.id
            WHERE role_permissions.role_id = ?
            ORDER BY permissions.code ASC
            "#,
        )
        .bind(role_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(permission_from_row).collect()
    }

    async fn user_roles(&self, user_id: &str) -> anyhow::Result<Vec<Role>> {
        let rows = sqlx::query(
            r#"
            SELECT roles.*
            FROM roles
            INNER JOIN user_roles ON user_roles.role_id = roles.id
            WHERE user_roles.user_id = ?
            ORDER BY roles.`system` DESC, roles.name ASC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(role_from_row).collect()
    }

    async fn set_user_roles(&self, user_id: &str, role_ids: Vec<String>) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM user_roles WHERE user_id = ?")
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        for role_id in role_ids {
            sqlx::query("INSERT IGNORE INTO user_roles (user_id, role_id) VALUES (?, ?)")
                .bind(user_id)
                .bind(role_id)
                .execute(&self.pool)
                .await?;
        }
        sqlx::query("UPDATE users SET updated_at = ? WHERE id = ?")
            .bind(Utc::now().to_rfc3339())
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn user_permissions(&self, user_id: &str) -> anyhow::Result<Vec<Permission>> {
        let rows = sqlx::query(
            r#"
            SELECT DISTINCT permissions.*
            FROM permissions
            INNER JOIN role_permissions ON role_permissions.permission_id = permissions.id
            INNER JOIN user_roles ON user_roles.role_id = role_permissions.role_id
            WHERE user_roles.user_id = ?
            ORDER BY permissions.code ASC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(permission_from_row).collect()
    }
}

impl MysqlStorage {
    async fn set_user_role_names(
        &self,
        user_id: &str,
        role_names: &[String],
    ) -> anyhow::Result<()> {
        let mut role_ids = Vec::new();
        for role_name in role_names {
            let role_id: Option<String> = sqlx::query("SELECT id FROM roles WHERE name = ?")
                .bind(role_name)
                .fetch_optional(&self.pool)
                .await?
                .map(|row| row.try_get("id"))
                .transpose()?;
            if let Some(role_id) = role_id {
                role_ids.push(role_id);
            }
        }
        self.set_user_roles(user_id, role_ids).await
    }

    async fn find_role_by_id(&self, role_id: &str) -> anyhow::Result<Option<Role>> {
        sqlx::query("SELECT * FROM roles WHERE id = ?")
            .bind(role_id)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(role_from_row)
            .transpose()
    }

    async fn replace_role_permissions(
        &self,
        role_id: &str,
        permission_ids: &[String],
    ) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM role_permissions WHERE role_id = ?")
            .bind(role_id)
            .execute(&self.pool)
            .await?;
        for permission_id in permission_ids {
            sqlx::query(
                "INSERT IGNORE INTO role_permissions (role_id, permission_id) VALUES (?, ?)",
            )
            .bind(role_id)
            .bind(permission_id)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }
}
