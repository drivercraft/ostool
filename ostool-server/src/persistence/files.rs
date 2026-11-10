//! Authoritative, chunked PostgreSQL session artifacts. Disk copies are rebuildable.
use super::Database;
use crate::tftp::files::{self, TftpFileRef};
use anyhow::{Context, ensure};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::path::Path;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

pub const CHUNK_SIZE: usize = 256 * 1024;
#[derive(Clone, Debug)]
pub struct SessionFileStore {
    database: Database,
}
#[derive(Clone, Debug)]
pub struct StoredFile {
    pub id: Uuid,
    pub session_id: String,
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub uploaded_at: DateTime<Utc>,
}
impl StoredFile {
    pub fn reference(&self, root: &Path) -> anyhow::Result<TftpFileRef> {
        Ok(TftpFileRef {
            filename: Path::new(&self.path)
                .file_name()
                .context("missing filename")?
                .to_str()
                .context("invalid filename")?
                .into(),
            disk_path: files::disk_path(root, &self.session_id, &self.path)?,
            relative_path: files::session_relative_path(&self.session_id, &self.path)?,
            size: self.size,
            uploaded_at: self.uploaded_at,
        })
    }
}
impl SessionFileStore {
    pub fn new(database: Database) -> Self {
        Self { database }
    }
    pub async fn get(&self, sid: &str, path: &str) -> anyhow::Result<Option<StoredFile>> {
        let path = files::normalize_relative_path(path)?;
        sqlx::query("SELECT * FROM session_files WHERE session_id=$1 AND path=$2")
            .bind(sid)
            .bind(path)
            .fetch_optional(self.database.pool())
            .await?
            .as_ref()
            .map(from_row)
            .transpose()
    }
    pub async fn get_by_id(&self, id: Uuid) -> anyhow::Result<Option<StoredFile>> {
        sqlx::query("SELECT * FROM session_files WHERE id=$1")
            .bind(id)
            .fetch_optional(self.database.pool())
            .await?
            .as_ref()
            .map(from_row)
            .transpose()
    }
    pub async fn list(&self, sid: &str) -> anyhow::Result<Vec<StoredFile>> {
        sqlx::query(r#"SELECT * FROM session_files WHERE session_id=$1 ORDER BY path COLLATE "C""#)
            .bind(sid)
            .fetch_all(self.database.pool())
            .await?
            .iter()
            .map(from_row)
            .collect()
    }
    pub async fn usage(
        &self,
        sid: &str,
        path: &str,
        user: Option<Uuid>,
    ) -> anyhow::Result<(u64, u64, u64, u64, u64)> {
        let row=sqlx::query("SELECT COALESCE(sum(size),0)::bigint AS total,COALESCE(sum(size) FILTER (WHERE session_id=$1),0)::bigint AS own,count(*) FILTER (WHERE session_id=$1) AS count,COALESCE(max(size) FILTER (WHERE session_id=$1 AND path=$2),0) AS old,COALESCE(sum(size) FILTER (WHERE session_id IN (SELECT session_id FROM session_owners WHERE user_id=$3)),0)::bigint AS user_total FROM session_files")
            .bind(sid).bind(path).bind(user).fetch_one(self.database.pool()).await?;
        Ok((
            row.try_get::<i64, _>("total")?.try_into()?,
            row.try_get::<i64, _>("own")?.try_into()?,
            row.try_get::<i64, _>("count")?.try_into()?,
            row.try_get::<i64, _>("old")?.try_into()?,
            row.try_get::<i64, _>("user_total")?.try_into()?,
        ))
    }
    pub(crate) async fn stage_path(
        &self,
        sid: &str,
        path: &str,
        source: &Path,
    ) -> anyhow::Result<StagedFile> {
        let mut file = tokio::fs::File::open(source).await?;
        self.write(sid, path, &mut file, false).await
    }
    pub async fn write_bytes(
        &self,
        sid: &str,
        path: &str,
        bytes: &[u8],
    ) -> anyhow::Result<StoredFile> {
        self.write(sid, path, &mut &*bytes, true)
            .await?
            .commit()
            .await
    }
    async fn write(
        &self,
        sid: &str,
        path: &str,
        source: &mut (impl AsyncRead + Unpin),
        require_active: bool,
    ) -> anyhow::Result<StagedFile> {
        let path = files::normalize_relative_path(path)?;
        let mut tx = self.database.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended(current_schema()||':ostool-session-artifacts',1109))")
            .execute(&mut *tx)
            .await?;
        lock_session(&mut tx, sid).await?;
        if require_active {
            ensure!(session_active(&mut tx, sid).await?, "session has ended");
        }
        sqlx::query("DELETE FROM session_files WHERE session_id=$1 AND path=$2")
            .bind(sid)
            .bind(&path)
            .execute(&mut *tx)
            .await?;
        let id = Uuid::new_v4();
        let uploaded_at:DateTime<Utc>=sqlx::query_scalar("INSERT INTO session_files(id,session_id,path,size,sha256) VALUES($1,$2,$3,0,'') RETURNING uploaded_at").bind(id).bind(sid).bind(&path).fetch_one(&mut *tx).await?;
        let mut digest = Sha256::new();
        let mut size = 0u64;
        let mut index = 0i64;
        let mut buffer = vec![0; CHUNK_SIZE];
        loop {
            // Fill stable chunk boundaries even when AsyncRead yields short reads.
            let mut filled = 0;
            while filled < buffer.len() {
                let n = source.read(&mut buffer[filled..]).await?;
                if n == 0 {
                    break;
                }
                filled += n;
            }
            if filled == 0 {
                break;
            }
            digest.update(&buffer[..filled]);
            size += filled as u64;
            sqlx::query(
                "INSERT INTO session_file_chunks(file_id,chunk_index,content) VALUES($1,$2,$3)",
            )
            .bind(id)
            .bind(index)
            .bind(&buffer[..filled])
            .execute(&mut *tx)
            .await?;
            index += 1;
        }
        let sha256 = format!("{:x}", digest.finalize());
        sqlx::query("UPDATE session_files SET size=$2,sha256=$3 WHERE id=$1")
            .bind(id)
            .bind(i64::try_from(size)?)
            .bind(&sha256)
            .execute(&mut *tx)
            .await?;
        Ok(StagedFile {
            tx,
            file: StoredFile {
                id,
                session_id: sid.into(),
                path,
                size,
                sha256,
                uploaded_at,
            },
        })
    }
    pub async fn chunk(&self, id: Uuid, index: u64) -> anyhow::Result<Vec<u8>> {
        sqlx::query_scalar(
            "SELECT content FROM session_file_chunks WHERE file_id=$1 AND chunk_index=$2",
        )
        .bind(id)
        .bind(i64::try_from(index)?)
        .fetch_optional(self.database.pool())
        .await?
        .context("session file chunk not found")
    }
    pub async fn read(&self, file: &StoredFile) -> anyhow::Result<Vec<u8>> {
        let mut bytes = Vec::with_capacity(usize::try_from(file.size)?);
        for index in 0..file.size.div_ceil(CHUNK_SIZE as u64) {
            bytes.extend(self.chunk(file.id, index).await?);
        }
        ensure!(
            bytes.len() as u64 == file.size
                && format!("{:x}", Sha256::digest(&bytes)) == file.sha256,
            "session file integrity check failed"
        );
        Ok(bytes)
    }
    pub async fn materialize(&self, file: &StoredFile, root: &Path) -> anyhow::Result<TftpFileRef> {
        let mut tx = self.database.begin().await?;
        lock_session(&mut tx, &file.session_id).await?;
        let present: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM session_files WHERE id=$1)")
                .bind(file.id)
                .fetch_one(&mut *tx)
                .await?;
        ensure!(
            present && session_active(&mut tx, &file.session_id).await?,
            "session file has ended"
        );
        let reference = file.reference(root)?;
        files::prepare_session_parent(root, &file.session_id, &file.path)?;
        let temp = tempfile::NamedTempFile::new_in(
            reference.disk_path.parent().context("invalid cache path")?,
        )?;
        let mut output = tokio::fs::File::from_std(temp.reopen()?);
        let mut digest = Sha256::new();
        let mut size = 0u64;
        for index in 0..file.size.div_ceil(CHUNK_SIZE as u64) {
            let chunk: Vec<u8> = sqlx::query_scalar(
                "SELECT content FROM session_file_chunks WHERE file_id=$1 AND chunk_index=$2",
            )
            .bind(file.id)
            .bind(i64::try_from(index)?)
            .fetch_one(&mut *tx)
            .await?;
            digest.update(&chunk);
            size += chunk.len() as u64;
            output.write_all(&chunk).await?;
        }
        output.flush().await?;
        drop(output);
        ensure!(
            size == file.size && format!("{:x}", digest.finalize()) == file.sha256,
            "session file integrity check failed"
        );
        ensure!(
            session_active(&mut tx, &file.session_id).await?,
            "session has ended"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temp.as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        temp.persist(&reference.disk_path)?;
        tx.commit().await?;
        Ok(reference)
    }
    pub async fn delete(&self, sid: &str, path: &str) -> anyhow::Result<()> {
        let path = files::normalize_relative_path(path)?;
        let mut tx = self.database.begin().await?;
        lock_session(&mut tx, sid).await?;
        sqlx::query("DELETE FROM session_files WHERE session_id=$1 AND path=$2")
            .bind(sid)
            .bind(path)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn delete_session(&self, sid: &str) -> anyhow::Result<()> {
        let mut tx = self.database.begin().await?;
        lock_session(&mut tx, sid).await?;
        sqlx::query("DELETE FROM session_files WHERE session_id=$1")
            .bind(sid)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn cleanup_ended(&self) -> anyhow::Result<()> {
        let ids:Vec<String>=sqlx::query_scalar("SELECT DISTINCT f.session_id FROM session_files f JOIN session_history h ON h.session_id=f.session_id WHERE h.ended_at IS NOT NULL").fetch_all(self.database.pool()).await?;
        for id in ids {
            self.delete_session(&id).await?;
        }
        Ok(())
    }
}
fn from_row(row: &sqlx::postgres::PgRow) -> anyhow::Result<StoredFile> {
    Ok(StoredFile {
        id: row.try_get("id")?,
        session_id: row.try_get("session_id")?,
        path: row.try_get("path")?,
        size: row.try_get::<i64, _>("size")?.try_into()?,
        sha256: row.try_get("sha256")?,
        uploaded_at: row.try_get("uploaded_at")?,
    })
}

/// Unpublished transaction; dropping it restores the previous artifact atomically.
pub(crate) struct StagedFile {
    pub tx: sqlx::Transaction<'static, sqlx::Postgres>,
    pub file: StoredFile,
}
impl StagedFile {
    pub async fn commit(self) -> anyhow::Result<StoredFile> {
        self.tx.commit().await?;
        Ok(self.file)
    }
    pub async fn credential_active(
        &mut self,
        credential: &crate::persistence::postgres::Credential,
    ) -> anyhow::Result<bool> {
        if let Some(id) = credential.session_id {
            Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM oauth_sessions s JOIN users u ON u.id=s.user_id WHERE s.family_id=(SELECT family_id FROM oauth_sessions WHERE id=$1) AND s.revoked_at IS NULL AND s.access_expires_at>clock_timestamp() AND s.absolute_expires_at>clock_timestamp() AND s.last_seen_at>clock_timestamp()-interval '1 day' AND s.security_version=u.security_version AND NOT u.disabled AND EXISTS(SELECT 1 FROM roles r WHERE r.id=u.role_id AND r.enabled))").bind(id).fetch_one(&mut *self.tx).await?)
        } else {
            Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM personal_access_tokens p JOIN users u ON u.id=p.user_id WHERE p.token_hash=$1 AND (p.expires_at IS NULL OR p.expires_at>clock_timestamp()) AND NOT u.disabled AND EXISTS(SELECT 1 FROM roles r WHERE r.id=u.role_id AND r.enabled))").bind(&credential.token_hash).fetch_one(&mut *self.tx).await?)
        }
    }
    pub async fn usage(&mut self, owner: Option<Uuid>) -> anyhow::Result<(u64, u64, u64, u64)> {
        let row=sqlx::query("SELECT COALESCE(sum(size),0)::bigint AS total,COALESCE(sum(size) FILTER (WHERE session_id=$1),0)::bigint AS own,count(*) FILTER (WHERE session_id=$1) AS count,COALESCE(sum(size) FILTER (WHERE session_id IN (SELECT session_id FROM session_owners WHERE user_id=$2)),0)::bigint AS user_total FROM session_files")
            .bind(&self.file.session_id).bind(owner).fetch_one(&mut *self.tx).await?;
        Ok((
            row.try_get::<i64, _>("total")?.try_into()?,
            row.try_get::<i64, _>("own")?.try_into()?,
            row.try_get::<i64, _>("count")?.try_into()?,
            row.try_get::<i64, _>("user_total")?.try_into()?,
        ))
    }
}

async fn lock_session(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    sid: &str,
) -> anyhow::Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended(current_schema()||':ostool-session:'||$1,2999))").bind(sid).execute(&mut **tx).await?;
    Ok(())
}
async fn session_active(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    sid: &str,
) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM session_history WHERE session_id=$1 AND ended_at IS NULL AND stop_requested_at IS NULL AND expires_at>clock_timestamp())").bind(sid).fetch_one(&mut **tx).await?)
}
