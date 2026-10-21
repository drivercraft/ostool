//! PostgreSQL DTB metadata and binary-content repository.
use super::postgres::Database;
use anyhow::{Context, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::{
    ffi::OsStr,
    path::{Component, Path},
};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DtbFile {
    pub name: String,
    pub size: u64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct DtbStore {
    database: Database,
}
impl DtbStore {
    pub fn new(database: Database) -> Self {
        Self { database }
    }
    pub async fn list_all(&self) -> anyhow::Result<Vec<DtbFile>> {
        let db = &self.database;
        sqlx::query("SELECT name,size,updated_at FROM dtb_records ORDER BY name")
            .fetch_all(db.pool())
            .await?
            .iter()
            .map(dtb_from_row)
            .collect()
    }
    pub async fn get(&self, name: &str) -> anyhow::Result<Option<DtbFile>> {
        let name = normalize_dtb_name(name)?;
        let db = &self.database;
        sqlx::query("SELECT name,size,updated_at FROM dtb_records WHERE name=$1")
            .bind(name)
            .fetch_optional(db.pool())
            .await?
            .as_ref()
            .map(dtb_from_row)
            .transpose()
    }
    pub async fn read(&self, name: &str) -> anyhow::Result<Vec<u8>> {
        let name = normalize_dtb_name(name)?;
        let db = &self.database;
        sqlx::query_scalar("SELECT content FROM dtb_records WHERE name=$1")
            .bind(&name)
            .fetch_optional(db.pool())
            .await?
            .with_context(|| format!("DTB {name} not found"))
    }
    pub async fn write(&self, name: &str, bytes: &[u8]) -> anyhow::Result<DtbFile> {
        let name = normalize_dtb_name(name)?;
        let db = &self.database;
        sqlx::query("INSERT INTO dtb_records (name,size,sha256,content) VALUES ($1,$2,$3,$4) ON CONFLICT (name) DO UPDATE SET size=excluded.size,sha256=excluded.sha256,content=excluded.content,updated_at=now()")
                .bind(&name).bind(bytes.len() as i64).bind(format!("{:x}",Sha256::digest(bytes))).bind(bytes).execute(db.pool()).await?;
        self.get(&name).await?.context("saved DTB missing")
    }
    pub async fn rename(&self, current_name: &str, new_name: &str) -> anyhow::Result<DtbFile> {
        let current_name = normalize_dtb_name(current_name)?;
        let new_name = normalize_dtb_name(new_name)?;
        let db = &self.database;
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
        self.get(&new_name).await?.context("renamed DTB missing")
    }
    pub async fn delete(&self, name: &str) -> anyhow::Result<()> {
        let name = normalize_dtb_name(name)?;
        let db = &self.database;
        sqlx::query("DELETE FROM dtb_records WHERE name=$1")
            .bind(name)
            .execute(db.pool())
            .await?;
        Ok(())
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
