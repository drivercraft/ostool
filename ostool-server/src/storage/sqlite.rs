use std::{collections::BTreeMap, path::Path};

use anyhow::Context;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool, sqlite::SqliteConnectOptions};
use uuid::Uuid;

use crate::BoardConfig;
use crate::storage::{
    AuditLog, AuditLogRepository, AuthSession, AuthSessionRepository, BUILTIN_PERMISSIONS,
    BoardConfigRepository, DtbMetadata, DtbMetadataRepository, Lease, LeaseRepository, LeaseState,
    NewAuditLog, NewLease, NewRole, NewSessionRecord, NewUser, Permission, RbacRepository, Role,
    SessionRecord, SessionRecordRepository, SiteSettingValue, SiteSettings, SiteSettingsRepository,
    UpsertDtbMetadata, User, UserProfile, UserRepository, default_site_setting_rows,
    default_user_permission, parse_time, site_settings_from_values, site_settings_to_values,
};

const MIGRATION_RBAC_PLATFORM: &str = "0001_rbac_platform";
const MIGRATION_BOARD_CONFIGS: &str = "0002_board_configs";
const MIGRATION_DTB_AUDIT: &str = "0003_dtb_audit";
const MIGRATION_STANDARD_FIELDS: &str = "0004_standard_profile_fields";
const MIGRATION_SITE_SETTINGS: &str = "0005_site_settings";
const MIGRATION_PERFORMANCE_INDEXES: &str = "0006_performance_indexes";

#[derive(Clone)]
pub struct SqliteStorage {
    pool: SqlitePool,
}

impl SqliteStorage {
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        ensure_sqlite_parent_dir(url)?;
        let options = url
            .parse::<SqliteConnectOptions>()
            .with_context(|| format!("failed to parse SQLite database URL `{url}`"))?
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(options)
            .await
            .with_context(|| format!("failed to connect SQLite database `{url}`"))?;
        let storage = Self { pool };
        storage.migrate().await?;
        Ok(storage)
    }

    async fn migrate(&self) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS schema_migrations (
                version TEXT PRIMARY KEY NOT NULL,
                applied_at TEXT NOT NULL
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
        if !self
            .is_migration_applied(MIGRATION_PERFORMANCE_INDEXES)
            .await?
        {
            self.migrate_performance_indexes().await?;
            self.mark_migration_applied(MIGRATION_PERFORMANCE_INDEXES)
                .await?;
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
        sqlx::query("INSERT OR IGNORE INTO schema_migrations (version, applied_at) VALUES (?, ?)")
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
                id TEXT PRIMARY KEY NOT NULL CHECK(length(id) BETWEEN 1 AND 64),
                username TEXT NOT NULL UNIQUE CHECK(length(username) BETWEEN 3 AND 64),
                display_name TEXT NOT NULL CHECK(length(display_name) BETWEEN 1 AND 64),
                email TEXT NOT NULL CHECK(length(email) BETWEEN 5 AND 254),
                password_hash TEXT NOT NULL CHECK(length(password_hash) BETWEEN 1 AND 255),
                disabled INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL CHECK(length(created_at) BETWEEN 1 AND 64),
                updated_at TEXT NOT NULL CHECK(length(updated_at) BETWEEN 1 AND 64)
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
                id TEXT PRIMARY KEY NOT NULL CHECK(length(id) BETWEEN 1 AND 64),
                user_id TEXT NOT NULL CHECK(length(user_id) BETWEEN 1 AND 64),
                token_hash TEXT NOT NULL UNIQUE CHECK(length(token_hash) BETWEEN 1 AND 255),
                expires_at TEXT NOT NULL CHECK(length(expires_at) BETWEEN 1 AND 64),
                created_at TEXT NOT NULL CHECK(length(created_at) BETWEEN 1 AND 64),
                FOREIGN KEY(user_id) REFERENCES users(id)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS leases (
                id TEXT PRIMARY KEY NOT NULL CHECK(length(id) BETWEEN 1 AND 64),
                user_id TEXT NOT NULL CHECK(length(user_id) BETWEEN 1 AND 64),
                session_id TEXT UNIQUE CHECK(session_id IS NULL OR length(session_id) BETWEEN 1 AND 64),
                board_id TEXT NOT NULL CHECK(length(board_id) BETWEEN 1 AND 64),
                board_type TEXT NOT NULL CHECK(length(board_type) BETWEEN 1 AND 64),
                required_tags_json TEXT NOT NULL,
                state TEXT NOT NULL CHECK(length(state) BETWEEN 1 AND 32),
                created_at TEXT NOT NULL CHECK(length(created_at) BETWEEN 1 AND 64),
                starts_at TEXT NOT NULL CHECK(length(starts_at) BETWEEN 1 AND 64),
                expires_at TEXT NOT NULL CHECK(length(expires_at) BETWEEN 1 AND 64),
                released_at TEXT CHECK(released_at IS NULL OR length(released_at) BETWEEN 1 AND 64),
                failure_message TEXT CHECK(failure_message IS NULL OR length(failure_message) <= 500),
                FOREIGN KEY(user_id) REFERENCES users(id)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS session_records (
                id TEXT PRIMARY KEY NOT NULL CHECK(length(id) BETWEEN 1 AND 64),
                board_id TEXT NOT NULL CHECK(length(board_id) BETWEEN 1 AND 64),
                client_name TEXT CHECK(client_name IS NULL OR length(client_name) <= 128),
                source_ip TEXT CHECK(source_ip IS NULL OR length(source_ip) <= 45),
                state TEXT NOT NULL CHECK(length(state) BETWEEN 1 AND 32),
                created_at TEXT NOT NULL CHECK(length(created_at) BETWEEN 1 AND 64),
                last_heartbeat_at TEXT NOT NULL CHECK(length(last_heartbeat_at) BETWEEN 1 AND 64),
                expires_at TEXT NOT NULL CHECK(length(expires_at) BETWEEN 1 AND 64),
                ended_at TEXT CHECK(ended_at IS NULL OR length(ended_at) BETWEEN 1 AND 64),
                failure_message TEXT CHECK(failure_message IS NULL OR length(failure_message) <= 500)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_session_records_board_id ON session_records(board_id)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_session_records_created_at ON session_records(created_at)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_leases_user_id ON leases(user_id)")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_auth_sessions_token ON auth_sessions(token_hash)",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn migrate_board_config_tables(&self) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS board_configs (
                id TEXT PRIMARY KEY NOT NULL CHECK(length(id) BETWEEN 1 AND 64),
                board_type TEXT NOT NULL CHECK(length(board_type) BETWEEN 1 AND 64),
                config_json TEXT NOT NULL,
                created_at TEXT NOT NULL CHECK(length(created_at) BETWEEN 1 AND 64),
                updated_at TEXT NOT NULL CHECK(length(updated_at) BETWEEN 1 AND 64)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_board_configs_board_type ON board_configs(board_type)",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn migrate_rbac_tables(&self) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS permissions (
                id TEXT PRIMARY KEY NOT NULL CHECK(length(id) BETWEEN 1 AND 64),
                code TEXT NOT NULL UNIQUE CHECK(length(code) BETWEEN 1 AND 128),
                name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 64),
                description TEXT NOT NULL CHECK(length(description) <= 255),
                created_at TEXT NOT NULL CHECK(length(created_at) BETWEEN 1 AND 64),
                updated_at TEXT NOT NULL CHECK(length(updated_at) BETWEEN 1 AND 64)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS roles (
                id TEXT PRIMARY KEY NOT NULL CHECK(length(id) BETWEEN 1 AND 64),
                name TEXT NOT NULL UNIQUE CHECK(length(name) BETWEEN 2 AND 64),
                display_name TEXT NOT NULL CHECK(length(display_name) BETWEEN 1 AND 64),
                description TEXT NOT NULL CHECK(length(description) <= 255),
                `system` INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL CHECK(length(created_at) BETWEEN 1 AND 64),
                updated_at TEXT NOT NULL CHECK(length(updated_at) BETWEEN 1 AND 64)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS role_permissions (
                role_id TEXT NOT NULL CHECK(length(role_id) BETWEEN 1 AND 64),
                permission_id TEXT NOT NULL CHECK(length(permission_id) BETWEEN 1 AND 64),
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
                user_id TEXT NOT NULL CHECK(length(user_id) BETWEEN 1 AND 64),
                role_id TEXT NOT NULL CHECK(length(role_id) BETWEEN 1 AND 64),
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
                id TEXT PRIMARY KEY NOT NULL CHECK(length(id) BETWEEN 1 AND 64),
                name TEXT NOT NULL UNIQUE CHECK(length(name) BETWEEN 1 AND 128),
                storage_path TEXT NOT NULL CHECK(length(storage_path) BETWEEN 1 AND 255),
                size_bytes INTEGER NOT NULL,
                sha256 TEXT NOT NULL CHECK(length(sha256) = 64),
                boot_architecture TEXT CHECK(boot_architecture IS NULL OR length(boot_architecture) <= 64),
                compatible TEXT CHECK(compatible IS NULL OR length(compatible) <= 255),
                description TEXT CHECK(description IS NULL OR length(description) <= 500),
                disabled INTEGER NOT NULL DEFAULT 0,
                uploaded_by TEXT CHECK(uploaded_by IS NULL OR length(uploaded_by) <= 64),
                created_at TEXT NOT NULL CHECK(length(created_at) BETWEEN 1 AND 64),
                updated_at TEXT NOT NULL CHECK(length(updated_at) BETWEEN 1 AND 64)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS audit_logs (
                id TEXT PRIMARY KEY NOT NULL CHECK(length(id) BETWEEN 1 AND 64),
                actor_user_id TEXT CHECK(actor_user_id IS NULL OR length(actor_user_id) <= 64),
                actor_username TEXT CHECK(actor_username IS NULL OR length(actor_username) <= 64),
                action TEXT NOT NULL CHECK(length(action) BETWEEN 1 AND 64),
                target_type TEXT NOT NULL CHECK(length(target_type) BETWEEN 1 AND 64),
                target_id TEXT CHECK(target_id IS NULL OR length(target_id) <= 128),
                outcome TEXT NOT NULL CHECK(length(outcome) BETWEEN 1 AND 32),
                ip_address TEXT CHECK(ip_address IS NULL OR length(ip_address) <= 45),
                user_agent TEXT CHECK(user_agent IS NULL OR length(user_agent) <= 512),
                request_id TEXT CHECK(request_id IS NULL OR length(request_id) <= 128),
                metadata_json TEXT NOT NULL,
                created_at TEXT NOT NULL CHECK(length(created_at) BETWEEN 1 AND 64)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_audit_logs_created_at ON audit_logs(created_at)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_audit_logs_target ON audit_logs(target_type, target_id)",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn migrate_standard_fields(&self) -> anyhow::Result<()> {
        self.add_column_if_missing(
            "users",
            "nickname",
            "TEXT CHECK(nickname IS NULL OR length(nickname) <= 64)",
        )
        .await?;
        self.add_column_if_missing(
            "users",
            "avatar_url",
            "TEXT CHECK(avatar_url IS NULL OR length(avatar_url) <= 512)",
        )
        .await?;
        self.add_column_if_missing(
            "users",
            "phone",
            "TEXT CHECK(phone IS NULL OR length(phone) <= 32)",
        )
        .await?;
        self.add_column_if_missing(
            "users",
            "department",
            "TEXT CHECK(department IS NULL OR length(department) <= 64)",
        )
        .await?;
        self.add_column_if_missing(
            "users",
            "title",
            "TEXT CHECK(title IS NULL OR length(title) <= 64)",
        )
        .await?;
        self.add_column_if_missing(
            "users",
            "last_login_at",
            "TEXT CHECK(last_login_at IS NULL OR length(last_login_at) BETWEEN 1 AND 64)",
        )
        .await?;
        self.add_column_if_missing(
            "auth_sessions",
            "ip_address",
            "TEXT CHECK(ip_address IS NULL OR length(ip_address) <= 45)",
        )
        .await?;
        self.add_column_if_missing(
            "auth_sessions",
            "user_agent",
            "TEXT CHECK(user_agent IS NULL OR length(user_agent) <= 512)",
        )
        .await?;
        self.add_column_if_missing(
            "auth_sessions",
            "last_seen_at",
            "TEXT CHECK(last_seen_at IS NULL OR length(last_seen_at) BETWEEN 1 AND 64)",
        )
        .await?;
        self.add_column_if_missing(
            "auth_sessions",
            "revoked_at",
            "TEXT CHECK(revoked_at IS NULL OR length(revoked_at) BETWEEN 1 AND 64)",
        )
        .await?;
        self.add_column_if_missing(
            "leases",
            "updated_at",
            "TEXT CHECK(updated_at IS NULL OR length(updated_at) BETWEEN 1 AND 64)",
        )
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
                key TEXT PRIMARY KEY NOT NULL CHECK(length(key) BETWEEN 1 AND 128),
                value_json TEXT NOT NULL,
                value_type TEXT NOT NULL CHECK(length(value_type) BETWEEN 1 AND 64),
                group_name TEXT NOT NULL CHECK(length(group_name) BETWEEN 1 AND 64),
                name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 64),
                description TEXT NOT NULL CHECK(length(description) <= 255),
                readonly INTEGER NOT NULL DEFAULT 0,
                sensitive INTEGER NOT NULL DEFAULT 0,
                updated_by TEXT CHECK(updated_by IS NULL OR length(updated_by) <= 64),
                created_at TEXT NOT NULL CHECK(length(created_at) BETWEEN 1 AND 64),
                updated_at TEXT NOT NULL CHECK(length(updated_at) BETWEEN 1 AND 64)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        self.seed_site_settings().await?;
        Ok(())
    }

    async fn migrate_performance_indexes(&self) -> anyhow::Result<()> {
        for statement in [
            "CREATE INDEX IF NOT EXISTS idx_auth_sessions_user_id ON auth_sessions(user_id)",
            "CREATE INDEX IF NOT EXISTS idx_auth_sessions_expires_at ON auth_sessions(expires_at)",
            "CREATE INDEX IF NOT EXISTS idx_auth_sessions_revoked_at ON auth_sessions(revoked_at)",
            "CREATE INDEX IF NOT EXISTS idx_leases_state ON leases(state)",
            "CREATE INDEX IF NOT EXISTS idx_leases_expires_at ON leases(expires_at)",
            "CREATE INDEX IF NOT EXISTS idx_leases_board_id ON leases(board_id)",
            "CREATE INDEX IF NOT EXISTS idx_leases_user_state ON leases(user_id, state)",
            "CREATE INDEX IF NOT EXISTS idx_user_roles_role_id ON user_roles(role_id)",
            "CREATE INDEX IF NOT EXISTS idx_role_permissions_permission_id ON role_permissions(permission_id)",
            "CREATE INDEX IF NOT EXISTS idx_roles_system_name ON roles(`system`, name)",
            "CREATE INDEX IF NOT EXISTS idx_dtb_files_uploaded_by ON dtb_files(uploaded_by)",
            "CREATE INDEX IF NOT EXISTS idx_dtb_files_sha256 ON dtb_files(sha256)",
            "CREATE INDEX IF NOT EXISTS idx_audit_logs_actor_user_id ON audit_logs(actor_user_id)",
            "CREATE INDEX IF NOT EXISTS idx_audit_logs_created_at ON audit_logs(created_at)",
            "CREATE INDEX IF NOT EXISTS idx_audit_logs_target ON audit_logs(target_type, target_id)",
            "CREATE INDEX IF NOT EXISTS idx_site_settings_group_name ON site_settings(group_name)",
        ] {
            sqlx::query(statement).execute(&self.pool).await?;
        }
        Ok(())
    }

    async fn seed_site_settings(&self) -> anyhow::Result<()> {
        for setting in default_site_setting_rows() {
            sqlx::query(
                r#"
                INSERT OR IGNORE INTO site_settings
                    (key, value_json, value_type, group_name, name, description, readonly, sensitive, created_at, updated_at)
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
        let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
            .fetch_all(&self.pool)
            .await?;
        let exists = rows.iter().any(|row| {
            row.try_get::<String, _>("name")
                .is_ok_and(|name| name == column)
        });
        if !exists {
            sqlx::query(&format!(
                "ALTER TABLE {table} ADD COLUMN {column} {definition}"
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
                INSERT OR IGNORE INTO permissions (id, code, name, description, created_at, updated_at)
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
                INSERT OR IGNORE INTO roles (id, name, display_name, description, `system`, created_at, updated_at)
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
            let code: String = row.try_get("code")?;
            sqlx::query(
                "INSERT OR IGNORE INTO role_permissions (role_id, permission_id) VALUES (?, ?)",
            )
            .bind(&admin_role_id)
            .bind(&permission_id)
            .execute(&self.pool)
            .await?;
            if default_user_permission(&code) {
                sqlx::query(
                    "INSERT OR IGNORE INTO role_permissions (role_id, permission_id) VALUES (?, ?)",
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
impl SiteSettingsRepository for SqliteStorage {
    async fn get_site_settings(&self) -> anyhow::Result<SiteSettings> {
        self.seed_site_settings().await?;
        let rows = sqlx::query("SELECT key, value_json, updated_at FROM site_settings")
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
                WHERE key = ?
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
impl DtbMetadataRepository for SqliteStorage {
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
                SET storage_path = ?, size_bytes = ?, sha256 = ?, boot_architecture = ?, compatible = ?, description = ?, disabled = ?, uploaded_by = ?, updated_at = ?
                WHERE id = ?
                "#,
            )
            .bind(metadata.storage_path)
            .bind(metadata.size_bytes)
            .bind(metadata.sha256)
            .bind(metadata.boot_architecture)
            .bind(metadata.compatible)
            .bind(metadata.description)
            .bind(metadata.disabled)
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
                    (id, name, storage_path, size_bytes, sha256, boot_architecture, compatible, description, disabled, uploaded_by, created_at, updated_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&id)
            .bind(metadata.name)
            .bind(metadata.storage_path)
            .bind(metadata.size_bytes)
            .bind(metadata.sha256)
            .bind(metadata.boot_architecture)
            .bind(metadata.compatible)
            .bind(metadata.description)
            .bind(metadata.disabled)
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
impl AuditLogRepository for SqliteStorage {
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

fn ensure_sqlite_parent_dir(url: &str) -> anyhow::Result<()> {
    let Some(path) = url.strip_prefix("sqlite:") else {
        anyhow::bail!("SQLite database URL must start with `sqlite:`");
    };
    if path == ":memory:" || path.trim().is_empty() {
        return Ok(());
    }
    let path = Path::new(path);
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    Ok(())
}

fn user_from_row(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<User> {
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

fn auth_session_from_row(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<AuthSession> {
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

fn lease_from_row(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<Lease> {
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
        starts_at: parse_time(row.try_get::<String, _>("starts_at")?.as_str())?,
        expires_at: parse_time(row.try_get::<String, _>("expires_at")?.as_str())?,
        released_at: row
            .try_get::<Option<String>, _>("released_at")?
            .map(|value| parse_time(&value))
            .transpose()?,
        failure_message: row.try_get("failure_message")?,
    })
}

fn session_record_from_row(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<SessionRecord> {
    Ok(SessionRecord {
        id: row.try_get("id")?,
        board_id: row.try_get("board_id")?,
        client_name: row.try_get("client_name")?,
        source_ip: row.try_get("source_ip")?,
        state: row.try_get("state")?,
        created_at: parse_time(row.try_get::<String, _>("created_at")?.as_str())?,
        last_heartbeat_at: parse_time(row.try_get::<String, _>("last_heartbeat_at")?.as_str())?,
        expires_at: parse_time(row.try_get::<String, _>("expires_at")?.as_str())?,
        ended_at: row
            .try_get::<Option<String>, _>("ended_at")?
            .map(|value| parse_time(&value))
            .transpose()?,
        failure_message: row.try_get("failure_message")?,
    })
}

fn board_config_from_row(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<BoardConfig> {
    let config_json: String = row.try_get("config_json")?;
    let board: BoardConfig =
        serde_json::from_str(&config_json).context("failed to parse board config_json")?;
    Ok(board)
}

fn dtb_metadata_from_row(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<DtbMetadata> {
    Ok(DtbMetadata {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        storage_path: row.try_get("storage_path")?,
        size_bytes: row.try_get("size_bytes")?,
        sha256: row.try_get("sha256")?,
        boot_architecture: row.try_get("boot_architecture")?,
        compatible: row.try_get("compatible")?,
        description: row.try_get("description")?,
        disabled: row.try_get("disabled")?,
        uploaded_by: row.try_get("uploaded_by")?,
        created_at: parse_time(row.try_get::<String, _>("created_at")?.as_str())?,
        updated_at: parse_time(row.try_get::<String, _>("updated_at")?.as_str())?,
    })
}

fn audit_log_from_row(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<AuditLog> {
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

fn permission_from_row(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<Permission> {
    Ok(Permission {
        id: row.try_get("id")?,
        code: row.try_get("code")?,
        name: row.try_get("name")?,
        description: row.try_get("description")?,
        created_at: parse_time(row.try_get::<String, _>("created_at")?.as_str())?,
        updated_at: parse_time(row.try_get::<String, _>("updated_at")?.as_str())?,
    })
}

fn role_from_row(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<Role> {
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
impl UserRepository for SqliteStorage {
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
impl AuthSessionRepository for SqliteStorage {
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

    async fn find_user_by_auth_token_hash(
        &self,
        token_hash: &str,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Option<User>> {
        sqlx::query(
            r#"
            SELECT users.*
            FROM users
            INNER JOIN auth_sessions ON auth_sessions.user_id = users.id
            WHERE auth_sessions.token_hash = ?
                AND auth_sessions.expires_at > ?
                AND auth_sessions.revoked_at IS NULL
            "#,
        )
        .bind(token_hash)
        .bind(now.to_rfc3339())
        .fetch_optional(&self.pool)
        .await?
        .as_ref()
        .map(user_from_row)
        .transpose()
    }

    async fn delete_auth_session_by_token_hash(&self, token_hash: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM auth_sessions WHERE token_hash = ?")
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn delete_auth_sessions_for_user_except(
        &self,
        user_id: &str,
        token_hash: &str,
    ) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM auth_sessions WHERE user_id = ? AND token_hash <> ?")
            .bind(user_id)
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
impl LeaseRepository for SqliteStorage {
    async fn create_lease(&self, lease: NewLease) -> anyhow::Result<Lease> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        sqlx::query(
            r#"
            INSERT INTO leases
                (id, user_id, session_id, board_id, board_type, required_tags_json, state, created_at, updated_at, starts_at, expires_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
        .bind(lease.starts_at.to_rfc3339())
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

    async fn bind_lease_session(&self, lease_id: &str, session_id: &str) -> anyhow::Result<()> {
        sqlx::query("UPDATE leases SET session_id = ?, updated_at = ? WHERE id = ?")
            .bind(session_id)
            .bind(Utc::now().to_rfc3339())
            .bind(lease_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn delete_lease(&self, lease_id: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM leases WHERE id = ?")
            .bind(lease_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn update_lease(
        &self,
        lease_id: &str,
        starts_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
        failure_message: Option<String>,
    ) -> anyhow::Result<Option<Lease>> {
        sqlx::query(
            "UPDATE leases SET starts_at = ?, expires_at = ?, failure_message = ?, updated_at = ? WHERE id = ?",
        )
        .bind(starts_at.to_rfc3339())
        .bind(expires_at.to_rfc3339())
        .bind(failure_message)
        .bind(Utc::now().to_rfc3339())
        .bind(lease_id)
        .execute(&self.pool)
        .await?;
        self.find_lease(lease_id).await
    }
}

#[async_trait]
impl SessionRecordRepository for SqliteStorage {
    async fn create_session_record(
        &self,
        record: NewSessionRecord,
    ) -> anyhow::Result<SessionRecord> {
        sqlx::query(
            r#"
            INSERT INTO session_records
                (id, board_id, client_name, source_ip, state, created_at, last_heartbeat_at, expires_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&record.id)
        .bind(record.board_id)
        .bind(record.client_name)
        .bind(record.source_ip)
        .bind(record.state)
        .bind(record.created_at.to_rfc3339())
        .bind(record.last_heartbeat_at.to_rfc3339())
        .bind(record.expires_at.to_rfc3339())
        .execute(&self.pool)
        .await?;

        let row = sqlx::query("SELECT * FROM session_records WHERE id = ?")
            .bind(&record.id)
            .fetch_one(&self.pool)
            .await?;
        session_record_from_row(&row)
    }

    async fn list_session_records(&self) -> anyhow::Result<Vec<SessionRecord>> {
        let rows = sqlx::query("SELECT * FROM session_records ORDER BY created_at DESC")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(session_record_from_row).collect()
    }

    async fn update_session_record_runtime(
        &self,
        session_id: &str,
        state: String,
        last_heartbeat_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE session_records SET state = ?, last_heartbeat_at = ?, expires_at = ? WHERE id = ?",
        )
        .bind(state)
        .bind(last_heartbeat_at.to_rfc3339())
        .bind(expires_at.to_rfc3339())
        .bind(session_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn finish_session_record(
        &self,
        session_id: &str,
        state: String,
        ended_at: DateTime<Utc>,
        failure_message: Option<String>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE session_records SET state = ?, ended_at = ?, failure_message = ? WHERE id = ?",
        )
        .bind(state)
        .bind(ended_at.to_rfc3339())
        .bind(failure_message)
        .bind(session_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_session_record(&self, session_id: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM session_records WHERE id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[async_trait]
impl BoardConfigRepository for SqliteStorage {
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
impl RbacRepository for SqliteStorage {
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

    async fn find_role_by_id(&self, role_id: &str) -> anyhow::Result<Option<Role>> {
        self.find_role_by_id_inner(role_id).await
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
        self.find_role_by_id_inner(&id)
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
        self.find_role_by_id_inner(role_id).await
    }

    async fn delete_role(&self, role_id: &str) -> anyhow::Result<()> {
        let role = self.find_role_by_id_inner(role_id).await?;
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

    async fn role_user_counts(&self) -> anyhow::Result<BTreeMap<String, u64>> {
        let rows = sqlx::query(
            r#"
            SELECT role_id, COUNT(*) AS user_count
            FROM user_roles
            GROUP BY role_id
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        let mut counts = BTreeMap::new();
        for row in rows {
            let role_id: String = row.try_get("role_id")?;
            let user_count: i64 = row.try_get("user_count")?;
            counts.insert(role_id, user_count.max(0) as u64);
        }
        Ok(counts)
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
            sqlx::query("INSERT OR IGNORE INTO user_roles (user_id, role_id) VALUES (?, ?)")
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

impl SqliteStorage {
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

    async fn find_role_by_id_inner(&self, role_id: &str) -> anyhow::Result<Option<Role>> {
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
                "INSERT OR IGNORE INTO role_permissions (role_id, permission_id) VALUES (?, ?)",
            )
            .bind(role_id)
            .bind(permission_id)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }
}
