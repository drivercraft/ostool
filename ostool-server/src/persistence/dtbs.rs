//! DTB repository. Metadata and binary contents are stored in PostgreSQL.
use super::{legacy::dtbs::DtbFiles, postgres::Database};
use anyhow::{Context, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::{
    ffi::OsStr,
    path::{Component, Path, PathBuf},
};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DtbFile {
    pub name: String,
    pub size: u64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct DtbStore {
    files: DtbFiles,
    database: Option<Database>,
}
impl DtbStore {
    pub async fn open_postgres(database: Database, legacy_dir: PathBuf) -> anyhow::Result<Self> {
        let store = Self::file_fixture(legacy_dir);
        store.ensure_dir().await?;
        store.with_database(database).await
    }
    /// File-backed fixtures are explicit and independent of edition.
    #[doc(hidden)]
    pub fn file_fixture(dtb_dir: PathBuf) -> Self {
        Self {
            files: DtbFiles::new(dtb_dir),
            database: None,
        }
    }
    /// Compatibility constructor for earlier library callers.
    #[doc(hidden)]
    pub fn new(dtb_dir: PathBuf) -> Self {
        Self::file_fixture(dtb_dir)
    }
    pub async fn with_database(mut self, database: Database) -> anyhow::Result<Self> {
        let mut tx = database.pool().begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('dtbs-content',2999))")
            .execute(&mut *tx)
            .await?;
        let imported: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM storage_imports WHERE name='dtbs-content')",
        )
        .fetch_one(&mut *tx)
        .await?;
        if !imported {
            for file in self.files.list_all().await? {
                let bytes = self.files.read(&file.name).await?;
                sqlx::query("INSERT INTO dtb_records (name,size,sha256,storage_path,content) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (name) DO UPDATE SET content=excluded.content,size=excluded.size,sha256=excluded.sha256 WHERE dtb_records.content IS NULL")
                    .bind(&file.name).bind(bytes.len() as i64).bind(format!("{:x}",Sha256::digest(&bytes))).bind(self.path_for_name(&file.name).to_string_lossy().as_ref()).bind(bytes).execute(&mut *tx).await?;
            }
            let missing: Option<String> =
                sqlx::query_scalar("SELECT name FROM dtb_records WHERE content IS NULL LIMIT 1")
                    .fetch_optional(&mut *tx)
                    .await?;
            if let Some(name) = missing {
                bail!(
                    "cannot migrate DTB {name}: source file is missing from {}",
                    self.files.directory().display()
                );
            }
            sqlx::query("INSERT INTO storage_imports (name) VALUES ('dtbs-content')")
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        self.database = Some(database);
        Ok(self)
    }

    pub fn path_for_name(&self, name: &str) -> PathBuf {
        self.files.path_for_name(name)
    }
    pub async fn ensure_dir(&self) -> anyhow::Result<()> {
        self.files.ensure_dir().await
    }
    pub async fn list_all(&self) -> anyhow::Result<Vec<DtbFile>> {
        if let Some(db) = &self.database {
            return sqlx::query("SELECT name,size,updated_at FROM dtb_records WHERE content IS NOT NULL ORDER BY name").fetch_all(db.pool()).await?.iter().map(dtb_from_row).collect();
        }
        self.files.list_all().await
    }
    pub async fn get(&self, name: &str) -> anyhow::Result<Option<DtbFile>> {
        let name = normalize_dtb_name(name)?;
        if let Some(db) = &self.database {
            return sqlx::query("SELECT name,size,updated_at FROM dtb_records WHERE name=$1 AND content IS NOT NULL").bind(name).fetch_optional(db.pool()).await?.as_ref().map(dtb_from_row).transpose();
        }
        self.files.get(&name).await
    }
    pub async fn read(&self, name: &str) -> anyhow::Result<Vec<u8>> {
        let name = normalize_dtb_name(name)?;
        if let Some(db) = &self.database {
            return sqlx::query_scalar(
                "SELECT content FROM dtb_records WHERE name=$1 AND content IS NOT NULL",
            )
            .bind(&name)
            .fetch_optional(db.pool())
            .await?
            .with_context(|| format!("DTB {name} not found"));
        }
        self.files.read(&name).await
    }
    pub async fn write(&self, name: &str, bytes: &[u8]) -> anyhow::Result<DtbFile> {
        let name = normalize_dtb_name(name)?;
        if let Some(db) = &self.database {
            sqlx::query("INSERT INTO dtb_records (name,size,sha256,storage_path,content) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (name) DO UPDATE SET size=excluded.size,sha256=excluded.sha256,content=excluded.content,updated_at=now()")
                .bind(&name).bind(bytes.len() as i64).bind(format!("{:x}",Sha256::digest(bytes))).bind(self.path_for_name(&name).to_string_lossy().as_ref()).bind(bytes).execute(db.pool()).await?;
            return self.get(&name).await?.context("saved DTB missing");
        }
        self.files.write(&name, bytes).await
    }
    pub async fn rename(&self, current_name: &str, new_name: &str) -> anyhow::Result<DtbFile> {
        let current_name = normalize_dtb_name(current_name)?;
        let new_name = normalize_dtb_name(new_name)?;
        if let Some(db) = &self.database {
            if current_name != new_name {
                let count =
                    sqlx::query("UPDATE dtb_records SET name=$2,updated_at=now() WHERE name=$1")
                        .bind(&current_name)
                        .bind(&new_name)
                        .execute(db.pool())
                        .await?
                        .rows_affected();
                if count == 0 {
                    bail!("DTB {current_name} not found");
                }
            }
            return self.get(&new_name).await?.context("renamed DTB missing");
        }
        self.files.rename(&current_name, &new_name).await
    }
    pub async fn delete(&self, name: &str) -> anyhow::Result<()> {
        let name = normalize_dtb_name(name)?;
        if let Some(db) = &self.database {
            sqlx::query("DELETE FROM dtb_records WHERE name=$1")
                .bind(name)
                .execute(db.pool())
                .await?;
            return Ok(());
        }
        self.files.delete(&name).await
    }
}
fn dtb_from_row(row: &sqlx::postgres::PgRow) -> anyhow::Result<DtbFile> {
    Ok(DtbFile {
        name: row.try_get("name")?,
        size: row.try_get::<i64, _>("size")?.try_into()?,
        updated_at: row.try_get("updated_at")?,
    })
}

pub fn normalize_dtb_name(name: &str) -> anyhow::Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        bail!("DTB name must not be empty");
    }
    if Path::new(trimmed).extension() != Some(OsStr::new("dtb")) {
        bail!("DTB name must end with `.dtb`");
    }

    let path = Path::new(trimmed);
    let mut normal_count = 0usize;
    for component in path.components() {
        match component {
            Component::Normal(_) => normal_count += 1,
            _ => bail!("DTB name must be a plain filename"),
        }
    }
    if normal_count != 1 {
        bail!("DTB name must be a plain filename");
    }

    Ok(trimmed.to_string())
}
