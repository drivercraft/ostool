//! Bounded streaming uploads. A session's file lock orders commits and release cleanup.
use crate::{
    AppState, api::error::ApiError, security::web::CurrentCredential, tftp::files::TftpFileRef,
};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
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
    state.session_files.cleanup_ended().await?;
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
    receive(state, sid, request, label)
        .await?
        .commit(state, sid, path)
        .await
}

/// Holds admission slots and the session upload gate until publication or drop.
/// A rejected or cancelled upload never becomes a published session file.
pub(crate) struct PendingUpload {
    temp: tempfile::NamedTempFile,
    deadline: tokio::time::Instant,
    size: u64,
    sha256: String,
    credential: Option<CurrentCredential>,
    limits: crate::config::UploadLimitsConfig,
    session: Arc<crate::session::SessionState>,
    manager: Arc<dyn crate::tftp::service::TftpManager>,
    _slot: Slot,
    _upload: tokio::sync::OwnedMutexGuard<()>,
}
impl PendingUpload {
    pub async fn within_deadline<F: std::future::Future>(
        &self,
        future: F,
    ) -> Result<F::Output, ApiError> {
        if self.session.is_stop_requested() || self.session.is_releasing() {
            return Err(ApiError::conflict("会话已结束，上传已取消"));
        }
        if tokio::time::Instant::now() >= self.deadline {
            return Err(upload_timeout());
        }
        let mut shutdown = self.session.subscribe_shutdown();
        tokio::select! {biased;
            _=shutdown.wait_for(|value|*value)=>Err(ApiError::conflict("会话已结束，上传已取消")),
            result=tokio::time::timeout_at(self.deadline,future)=>result.map_err(|_|upload_timeout()),
        }
    }
    pub async fn bytes(&self) -> Result<Vec<u8>, ApiError> {
        self.within_deadline(tokio::fs::read(self.temp.path()))
            .await?
            .map_err(|e| ApiError::from(anyhow::Error::from(e)))
    }
    pub async fn commit(
        self,
        state: &AppState,
        sid: &str,
        path: &str,
    ) -> Result<(TftpFileRef, u64, String), ApiError> {
        let deadline = self.deadline;
        if self.session.is_stop_requested() || self.session.is_releasing() {
            return Err(ApiError::conflict("会话已结束，上传已取消"));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(upload_timeout());
        }
        let mut shutdown = self.session.subscribe_shutdown();
        tokio::select! {biased;
            _=shutdown.wait_for(|value|*value)=>Err(ApiError::conflict("会话已结束，上传已取消")),
            result=tokio::time::timeout_at(deadline,self.publish(state,sid,path))=>result.map_err(|_|upload_timeout())?,
        }
    }
    async fn publish(
        self,
        state: &AppState,
        sid: &str,
        path: &str,
    ) -> Result<(TftpFileRef, u64, String), ApiError> {
        let Self {
            temp,
            deadline: _,
            size,
            sha256,
            credential,
            limits,
            session,
            manager,
            _slot,
            _upload,
        } = self;
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
        let owner = credential
            .as_ref()
            .map(|CurrentCredential(value)| value.user.id);
        let exists = state.session_files.get(sid, path).await?.is_some();
        let (total, own, count, old, user_total) =
            state.session_files.usage(sid, path, owner).await?;
        if own.saturating_sub(old) + size > limits.session_total_max_mib as u64 * 1024 * 1024
            || count + u64::from(!exists) > limits.session_max_files as u64
        {
            return Err(ApiError::payload_too_large("会话文件数量或总容量超出限制"));
        }
        if credential.is_some()
            && user_total.saturating_sub(old) + size
                > limits.user_total_max_mib as u64 * 1024 * 1024
        {
            return Err(ApiError::payload_too_large("账号文件总容量超出限制"));
        }
        if total.saturating_sub(old) + size > limits.storage_max_mib as u64 * 1024 * 1024 {
            return Err(ApiError::payload_too_large("会话文件存储总容量超出限制"));
        }
        if fs4::available_space(manager.root_dir()).map_err(anyhow::Error::from)? < 64 * 1024 * 1024
        {
            return Err(ApiError::service_unavailable("文件存储可用空间不足"));
        }
        if session.is_stop_requested() || session.snapshot().await.expires_at <= chrono::Utc::now()
        {
            return Err(ApiError::conflict("会话已结束，上传已取消"));
        }
        let mut staged = state
            .session_files
            .stage_path(sid, path, temp.path())
            .await?;
        if staged.file.size != size || staged.file.sha256 != sha256 {
            return Err(ApiError::internal("上传内容在提交期间发生变化"));
        }
        // Recheck against the same transaction after writing content, using the
        // database's real clock rather than the transaction start timestamp.
        if let Some(CurrentCredential(credential)) = &credential {
            if !staged.credential_active(credential).await? {
                return Err(ApiError::unauthorized("登录已失效，上传已取消"));
            }
            if !credential.user.is_admin {
                let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM session_owners s JOIN board_rentals r ON r.id=s.rental_id JOIN users u ON u.id=s.user_id WHERE s.session_id=$1 AND s.user_id=$2 AND NOT u.disabled AND r.cancelled_at IS NULL AND r.starts_at<=clock_timestamp() AND r.ends_at>clock_timestamp())")
                    .bind(sid).bind(credential.user.id).fetch_one(&mut *staged.tx).await?;
                if !valid {
                    return Err(ApiError::forbidden("租赁已结束，上传已取消"));
                }
            }
        }
        let (total, own, count, user_total) = staged.usage(owner).await?;
        if own > limits.session_total_max_mib as u64 * 1024 * 1024
            || count > limits.session_max_files as u64
            || total > limits.storage_max_mib as u64 * 1024 * 1024
            || (owner.is_some() && user_total > limits.user_total_max_mib as u64 * 1024 * 1024)
        {
            return Err(ApiError::payload_too_large("文件存储数量或容量超出限制"));
        }
        if session.is_stop_requested()
            || session.is_releasing()
            || session.snapshot().await.expires_at <= chrono::Utc::now()
        {
            return Err(ApiError::conflict("会话已结束，上传已取消"));
        }
        let stored = staged.commit().await?;
        // UART caches are materialized from PostgreSQL only when a transport
        // needs a path. A cache failure cannot change the committed upload.
        Ok((stored.reference(manager.root_dir())?, size, sha256))
    }
}

pub(crate) async fn receive(
    state: &AppState,
    sid: &str,
    request: Request<Body>,
    label: &str,
) -> Result<PendingUpload, ApiError> {
    let limits = state.config.read().await.upload_limits.clone();
    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(limits.upload_timeout_secs as u64);
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
        result=tokio::time::timeout_at(deadline,receive)=>{
            result.map_err(|_|ApiError::new(StatusCode::REQUEST_TIMEOUT,"upload_timeout","上传总时间超出限制"))??;
        }
    }
    drop(file);
    Ok(PendingUpload {
        temp,
        deadline,
        size,
        sha256: format!("{:x}", digest.finalize()),
        credential,
        limits,
        session,
        manager,
        _slot: slot,
        _upload,
    })
}

fn upload_timeout() -> ApiError {
    ApiError::new(
        StatusCode::REQUEST_TIMEOUT,
        "upload_timeout",
        "上传总时间超出限制",
    )
}
