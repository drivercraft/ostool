//! Bounded streaming uploads. A session's file lock orders commits and release cleanup.
use crate::{
    AppState,
    api::error::ApiError,
    security::web::CurrentCredential,
    tftp::files::{self, TftpFileRef},
};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{io::AsyncWriteExt, sync::Mutex};

#[derive(Clone)]
pub(crate) struct Uploads {
    active: Arc<AtomicUsize>,
    commits: Arc<Mutex<()>>,
    users: Arc<StdMutex<BTreeMap<uuid::Uuid, u32>>>,
    pub(crate) downloads: Arc<tokio::sync::Semaphore>,
}
impl Default for Uploads {
    fn default() -> Self {
        Self {
            active: Arc::default(),
            commits: Arc::default(),
            users: Arc::default(),
            downloads: Arc::new(tokio::sync::Semaphore::new(16)),
        }
    }
}
struct Slot {
    active: Arc<AtomicUsize>,
    users: Arc<StdMutex<BTreeMap<uuid::Uuid, u32>>>,
    owner: Option<uuid::Uuid>,
}
impl Drop for Slot {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
        if let Some(owner) = self.owner {
            let mut users = self.users.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(count) = users.get_mut(&owner) {
                *count -= 1;
                if *count == 0 {
                    users.remove(&owner);
                }
            }
        }
    }
}

/// Called before serving requests; interrupted uploads are never published session files.
pub async fn cleanup_interrupted(state: &AppState) -> anyhow::Result<()> {
    let root = state
        .tftp_manager
        .read()
        .await
        .root_dir()
        .join("ostool/.incoming");
    tokio::fs::create_dir_all(&root).await?;
    let mut entries = tokio::fs::read_dir(&root).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_file()
            && entry.file_name().to_string_lossy().starts_with("upload-")
        {
            tokio::fs::remove_file(entry.path()).await?;
        }
    }
    let sessions = state
        .tftp_manager
        .read()
        .await
        .root_dir()
        .join("ostool/sessions");
    if let Ok(mut entries) = tokio::fs::read_dir(&sessions).await {
        let mut ids = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let id = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type().await?.is_dir() && uuid::Uuid::parse_str(&id).is_ok() {
                ids.push(id);
            }
        }
        let ended:Vec<String>=sqlx::query_scalar("SELECT session_id FROM session_history WHERE session_id=ANY($1) AND ended_at IS NOT NULL").bind(ids).fetch_all(state.database.pool()).await?;
        for id in ended {
            tokio::fs::remove_dir_all(sessions.join(id)).await?;
        }
    }
    Ok(())
}
pub async fn store(
    state: &AppState,
    sid: &str,
    path: &str,
    request: Request<Body>,
    label: &str,
) -> Result<(TftpFileRef, u64, String), ApiError> {
    let limits = state.config.read().await.upload_limits.clone();
    if request
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|n| n > limits.session_file_max_mib as u64 * 1024 * 1024)
    {
        return Err(ApiError::payload_too_large(format!(
            "{label} upload body exceeds limit of {} MiB",
            limits.session_file_max_mib
        )));
    }
    let mut active = state.uploads.active.load(Ordering::Acquire);
    loop {
        if active >= limits.max_concurrent_uploads as usize {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "upload_busy",
                "上传并发已满，请稍后重试",
            )
            .with_retry_after(2));
        }
        match state.uploads.active.compare_exchange_weak(
            active,
            active + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => break,
            Err(current) => active = current,
        }
    }
    let credential = request.extensions().get::<CurrentCredential>().cloned();
    let mut slot = Slot {
        active: state.uploads.active.clone(),
        users: state.uploads.users.clone(),
        owner: None,
    };
    if let Some(CurrentCredential(credential)) = &credential {
        let mut users = state
            .uploads
            .users
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let count = users.entry(credential.user.id).or_default();
        if *count >= limits.user_max_concurrent_uploads {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "upload_busy",
                "该账号上传并发已满",
            )
            .with_retry_after(2));
        }
        *count += 1;
        slot.owner = Some(credential.user.id);
    }
    let _slot = slot;
    let session = state
        .session_state(sid)
        .await
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    let _upload = session
        .upload_gate
        .clone()
        .try_lock_owned()
        .map_err(|_| ApiError::conflict("该会话已有上传正在进行"))?;
    let manager = state.tftp_manager.read().await.clone();
    tokio::fs::create_dir_all(manager.root_dir())
        .await
        .map_err(anyhow::Error::from)?;
    let reserved = state.uploads.active.load(Ordering::Acquire) as u64
        * limits.session_file_max_mib as u64
        * 1024
        * 1024;
    if fs4::available_space(manager.root_dir()).map_err(anyhow::Error::from)?
        < reserved + 64 * 1024 * 1024
    {
        return Err(ApiError::service_unavailable("文件存储可用空间不足"));
    }
    let incoming = manager.root_dir().join("ostool/.incoming");
    tokio::fs::create_dir_all(&incoming)
        .await
        .map_err(anyhow::Error::from)?;
    let temp = tempfile::Builder::new()
        .prefix(&format!("upload-{}-", uuid::Uuid::new_v4()))
        .tempfile_in(&incoming)
        .map_err(anyhow::Error::from)?;
    let mut file = tokio::fs::File::from_std(temp.reopen().map_err(anyhow::Error::from)?);
    let mut body = request.into_body().into_data_stream();
    let mut size = 0u64;
    let mut digest = Sha256::new();
    let max = limits.session_file_max_mib as u64 * 1024 * 1024;
    let mut shutdown = session.subscribe_shutdown();
    let receive = async {
        loop {
            let chunk = tokio::time::timeout(Duration::from_secs(15), body.next())
                .await
                .map_err(|_| {
                    ApiError::new(
                        StatusCode::REQUEST_TIMEOUT,
                        "upload_timeout",
                        "上传数据等待超时",
                    )
                })?;
            let Some(chunk) = chunk else { break };
            let chunk = chunk.map_err(|_| ApiError::bad_request("读取上传数据失败"))?;
            size = size
                .checked_add(chunk.len() as u64)
                .ok_or_else(|| ApiError::payload_too_large("上传文件过大"))?;
            if size > max {
                return Err(ApiError::payload_too_large(format!(
                    "{label} upload body exceeds limit of {} MiB",
                    limits.session_file_max_mib
                )));
            }
            digest.update(&chunk);
            file.write_all(&chunk).await.map_err(anyhow::Error::from)?;
        }
        file.flush().await.map_err(anyhow::Error::from)?;
        Ok::<_, ApiError>(())
    };
    tokio::select! {
        biased;
        _=shutdown.wait_for(|stopped|*stopped)=>return Err(ApiError::conflict("会话已结束，上传已取消")),
        result=tokio::time::timeout(Duration::from_secs(limits.upload_timeout_secs as u64),receive)=>{
            result.map_err(|_|ApiError::new(StatusCode::REQUEST_TIMEOUT,"upload_timeout","上传总时间超出限制"))??;
        }
    }
    drop(file);
    let _commit = state.uploads.commits.lock().await;
    let _rental = state.rental_gate.lock().await;
    let _files = session.file_gate.lock().await;
    let snapshot = session.snapshot().await;
    if session.is_stop_requested()
        || session.is_releasing()
        || snapshot.expires_at <= chrono::Utc::now()
    {
        return Err(ApiError::conflict("会话已结束，上传已取消"));
    }
    if let Some(CurrentCredential(credential)) = &credential {
        if !state.database.credential_active(credential).await? {
            return Err(ApiError::unauthorized("登录已失效，上传已取消"));
        }
        if !credential.user.is_admin {
            let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM session_owners s JOIN board_rentals r ON r.id=s.rental_id JOIN users u ON u.id=s.user_id WHERE s.session_id=$1 AND s.user_id=$2 AND NOT u.disabled AND r.cancelled_at IS NULL AND r.starts_at<=now() AND r.ends_at>now())")
                .bind(sid).bind(credential.user.id).fetch_one(state.database.pool()).await?;
            if !valid {
                return Err(ApiError::forbidden("租赁已结束，上传已取消"));
            }
        }
    }
    let destination = files::disk_path(manager.root_dir(), sid, path)?;
    let root = manager.root_dir().to_path_buf();
    let session_root = root.join("ostool/sessions").join(sid);
    let target = destination.clone();
    let owned: Vec<String> = if let Some(CurrentCredential(credential)) = &credential {
        sqlx::query_scalar("SELECT session_id FROM session_owners WHERE user_id=$1")
            .bind(credential.user.id)
            .fetch_all(state.database.pool())
            .await?
    } else {
        Vec::new()
    };
    let owner_root = root.clone();
    let (total, own, count, old, user_total) =
        tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            let total = usage(&root.join("ostool/sessions"))?.0;
            let (own, count) = usage(&session_root)?;
            let old = std::fs::symlink_metadata(target)
                .ok()
                .filter(|m| m.is_file())
                .map(|m| m.len());
            let mut user_total = 0u64;
            for id in owned {
                user_total = user_total
                    .saturating_add(usage(&owner_root.join("ostool/sessions").join(id))?.0);
            }
            Ok((total, own, count, old, user_total))
        })
        .await
        .map_err(anyhow::Error::from)??;
    if own.saturating_sub(old.unwrap_or(0)) + size
        > limits.session_total_max_mib as u64 * 1024 * 1024
        || count + u64::from(old.is_none()) > limits.session_max_files as u64
    {
        return Err(ApiError::payload_too_large("会话文件数量或总容量超出限制"));
    }
    if credential.is_some()
        && user_total.saturating_sub(old.unwrap_or(0)) + size
            > limits.user_total_max_mib as u64 * 1024 * 1024
    {
        return Err(ApiError::payload_too_large("账号文件总容量超出限制"));
    }
    if total.saturating_sub(old.unwrap_or(0)) + size > limits.storage_max_mib as u64 * 1024 * 1024 {
        return Err(ApiError::payload_too_large("会话文件存储总容量超出限制"));
    }
    if fs4::available_space(manager.root_dir()).map_err(anyhow::Error::from)? < 64 * 1024 * 1024 {
        return Err(ApiError::service_unavailable("文件存储可用空间不足"));
    }
    files::prepare_session_parent(manager.root_dir(), sid, path)?;
    if session.is_stop_requested() || session.snapshot().await.expires_at <= chrono::Utc::now() {
        return Err(ApiError::conflict("会话已结束，上传已取消"));
    }
    // The synchronous rename is atomic and stays inside both commit/cleanup locks.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o644))
            .map_err(anyhow::Error::from)?;
    }
    temp.persist(&destination)
        .map_err(|error| ApiError::from(anyhow::Error::from(error.error)))?;
    let stored = files::get_session_file(manager.root_dir(), sid, path)?
        .ok_or_else(|| ApiError::internal("上传文件提交失败"))?;
    Ok((stored, size, format!("{:x}", digest.finalize())))
}

fn usage(root: &Path) -> anyhow::Result<(u64, u64)> {
    let mut pending = vec![root.to_path_buf()];
    let (mut bytes, mut count) = (0u64, 0u64);
    while let Some(dir) = pending.pop() {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let entry = entry?;
            let meta = match entry.path().symlink_metadata() {
                Ok(meta) => meta,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            if meta.is_symlink() {
                anyhow::bail!("session storage must not contain symbolic links");
            }
            if meta.is_dir() {
                pending.push(entry.path());
            } else if meta.is_file() {
                bytes = bytes.saturating_add(meta.len());
                count += 1;
            }
        }
    }
    Ok((bytes, count))
}
