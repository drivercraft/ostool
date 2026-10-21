use std::{
    ffi::OsStr,
    path::{Component, Path, PathBuf},
    time::SystemTime,
};

use anyhow::{Context, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use tokio::fs;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DtbFile {
    pub name: String,
    pub size: u64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct DtbStore {
    dtb_dir: PathBuf,
    database: Option<crate::database::Database>,
}

impl DtbStore {
    pub fn new(dtb_dir: PathBuf) -> Self {
        Self {
            dtb_dir,
            database: None,
        }
    }

    pub async fn with_database(
        mut self,
        database: crate::database::Database,
    ) -> anyhow::Result<Self> {
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
            for file in self.list_all().await? {
                let bytes = self.read(&file.name).await?;
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
                    self.dtb_dir.display()
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
        self.dtb_dir.join(name)
    }

    pub async fn ensure_dir(&self) -> anyhow::Result<()> {
        fs::create_dir_all(&self.dtb_dir).await?;
        Ok(())
    }

    pub async fn list_all(&self) -> anyhow::Result<Vec<DtbFile>> {
        if let Some(db) = &self.database {
            return sqlx::query("SELECT name,size,updated_at FROM dtb_records WHERE content IS NOT NULL ORDER BY name").fetch_all(db.pool()).await?.iter().map(dtb_from_row).collect();
        }
        self.ensure_dir().await?;
        let mut files = Vec::new();
        let mut dir = fs::read_dir(&self.dtb_dir).await?;

        while let Some(entry) = dir.next_entry().await? {
            let path = entry.path();
            if path.extension() != Some(OsStr::new("dtb")) || !path.is_file() {
                continue;
            }

            files.push(file_from_metadata(&path, &entry.metadata().await?)?);
        }

        files.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(files)
    }

    pub async fn get(&self, name: &str) -> anyhow::Result<Option<DtbFile>> {
        let name = normalize_dtb_name(name)?;
        if let Some(db) = &self.database {
            return sqlx::query("SELECT name,size,updated_at FROM dtb_records WHERE name=$1 AND content IS NOT NULL").bind(name).fetch_optional(db.pool()).await?.as_ref().map(dtb_from_row).transpose();
        }
        let path = self.path_for_name(&name);
        let metadata = match fs::metadata(&path).await {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        Ok(Some(file_from_metadata(&path, &metadata)?))
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
        let path = self.path_for_name(&name);
        fs::read(&path)
            .await
            .with_context(|| format!("failed to read {}", path.display()))
    }

    pub async fn write(&self, name: &str, bytes: &[u8]) -> anyhow::Result<DtbFile> {
        let name = normalize_dtb_name(name)?;
        if let Some(db) = &self.database {
            sqlx::query("INSERT INTO dtb_records (name,size,sha256,storage_path,content) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (name) DO UPDATE SET size=excluded.size,sha256=excluded.sha256,content=excluded.content,updated_at=now()")
                .bind(&name).bind(bytes.len() as i64).bind(format!("{:x}",Sha256::digest(bytes))).bind(self.path_for_name(&name).to_string_lossy().as_ref()).bind(bytes).execute(db.pool()).await?;
            return self.get(&name).await?.context("saved DTB missing");
        }
        self.ensure_dir().await?;
        let path = self.path_for_name(&name);
        let temp_path = path.with_extension("dtb.tmp");
        fs::write(&temp_path, bytes)
            .await
            .with_context(|| format!("failed to write {}", temp_path.display()))?;
        fs::rename(&temp_path, &path).await.with_context(|| {
            format!(
                "failed to rename {} to {}",
                temp_path.display(),
                path.display()
            )
        })?;
        self.get(&name)
            .await?
            .ok_or_else(|| anyhow::anyhow!("DTB `{name}` disappeared after write"))
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
        let current_path = self.path_for_name(&current_name);
        let new_path = self.path_for_name(&new_name);

        if current_name == new_name {
            return self
                .get(&current_name)
                .await?
                .ok_or_else(|| anyhow::anyhow!("DTB `{current_name}` not found"));
        }
        if fs::try_exists(&new_path).await? {
            bail!("DTB `{new_name}` already exists");
        }

        fs::rename(&current_path, &new_path)
            .await
            .with_context(|| {
                format!(
                    "failed to rename {} to {}",
                    current_path.display(),
                    new_path.display()
                )
            })?;
        self.get(&new_name)
            .await?
            .ok_or_else(|| anyhow::anyhow!("DTB `{new_name}` disappeared after rename"))
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
        let path = self.path_for_name(&name);
        match fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err.into()),
        }
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

fn file_from_metadata(path: &Path, metadata: &std::fs::Metadata) -> anyhow::Result<DtbFile> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("invalid DTB filename: {}", path.display()))?;
    let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);

    Ok(DtbFile {
        name: name.to_string(),
        size: metadata.len(),
        updated_at: DateTime::<Utc>::from(modified),
    })
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{DtbStore, normalize_dtb_name};

    #[tokio::test]
    async fn dtb_store_round_trip_and_rename() {
        let dir = tempdir().unwrap();
        let store = DtbStore::new(dir.path().to_path_buf());

        let saved = store.write("board.dtb", b"dtb").await.unwrap();
        assert_eq!(saved.name, "board.dtb");

        let renamed = store.rename("board.dtb", "board-v2.dtb").await.unwrap();
        assert_eq!(renamed.name, "board-v2.dtb");
        assert_eq!(store.read("board-v2.dtb").await.unwrap(), b"dtb");
        assert!(store.get("board.dtb").await.unwrap().is_none());
    }

    #[test]
    fn normalize_dtb_name_rejects_nested_paths() {
        assert!(normalize_dtb_name("board.dtb").is_ok());
        assert!(normalize_dtb_name("../board.dtb").is_err());
        assert!(normalize_dtb_name("nested/board.dtb").is_err());
        assert!(normalize_dtb_name("board.bin").is_err());
    }
}
