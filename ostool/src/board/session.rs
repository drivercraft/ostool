use std::{future::Future, net::IpAddr, sync::Arc, time::Duration};

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use tokio::{
    sync::{RwLock, watch},
    task::JoinHandle,
};
use url::Url;

use crate::board::client::{
    BoardServerClient, BoardServerClientError, SessionCreatedResponse, SharedSessionFileResponse,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoardSessionContext {
    pub session_id: String,
    pub server_ip: IpAddr,
    pub http_base_url: Url,
}

#[derive(Clone, Debug)]
pub struct SharedSessionFile {
    pub filename: String,
    pub relative_path: String,
    pub http_url: Url,
    pub size: u64,
    pub uploaded_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct BoardSession {
    client: BoardServerClient,
    info: SessionCreatedResponse,
    lease_expires_at: Arc<RwLock<DateTime<Utc>>>,
    heartbeat_stop: watch::Sender<bool>,
    heartbeat_task: Option<JoinHandle<anyhow::Result<()>>>,
}

impl BoardSession {
    pub async fn acquire(client: BoardServerClient, board_type: &str) -> anyhow::Result<Self> {
        Self::acquire_inner(client, board_type, None).await
    }

    pub async fn acquire_with_board_id(
        client: BoardServerClient,
        board_type: &str,
        board_id: &str,
    ) -> anyhow::Result<Self> {
        let board_id = board_id.trim();
        if board_id.is_empty() {
            return Err(anyhow!("board_id must not be empty when provided"));
        }
        Self::acquire_inner(client, board_type, Some(board_id)).await
    }

    async fn acquire_inner(
        client: BoardServerClient,
        board_type: &str,
        board_id: Option<&str>,
    ) -> anyhow::Result<Self> {
        let info = match board_id {
            Some(board_id) => {
                acquire_session_with(
                    board_type,
                    Some(board_id),
                    || client.create_session_with_board_id(board_type, board_id),
                    |duration| tokio::time::sleep(duration),
                )
                .await?
            }
            None => {
                acquire_session_with(
                    board_type,
                    None,
                    || client.create_session(board_type),
                    |duration| tokio::time::sleep(duration),
                )
                .await?
            }
        };

        let lease_expires_at = Arc::new(RwLock::new(info.lease_expires_at));
        let (heartbeat_stop, heartbeat_rx) = watch::channel(false);
        let heartbeat_task = Some(tokio::spawn(run_heartbeat_loop(
            client.clone(),
            info.session_id.clone(),
            lease_expires_at.clone(),
            heartbeat_rx,
        )));

        Ok(Self {
            client,
            info,
            lease_expires_at,
            heartbeat_stop,
            heartbeat_task,
        })
    }

    pub fn info(&self) -> &SessionCreatedResponse {
        &self.info
    }

    pub async fn current_lease_expires_at(&self) -> DateTime<Utc> {
        *self.lease_expires_at.read().await
    }

    pub async fn context(&self) -> anyhow::Result<BoardSessionContext> {
        let profile = self
            .client
            .get_boot_profile(&self.info.session_id)
            .await
            .context("failed to get board session network context")?;
        let server_ip = profile
            .server_ip
            .as_deref()
            .ok_or_else(|| anyhow!("board server did not report a board-reachable IP address"))?
            .parse::<IpAddr>()
            .with_context(|| {
                format!(
                    "board server returned invalid board-reachable IP address `{}`",
                    profile.server_ip.as_deref().unwrap_or_default()
                )
            })?;
        let http_base_url = profile.http_base_url.ok_or_else(|| {
            anyhow!("board server did not report a board-reachable shared HTTP URL")
        })?;
        Ok(BoardSessionContext {
            session_id: self.info.session_id.clone(),
            server_ip,
            http_base_url,
        })
    }

    pub async fn upload_shared_file(
        &self,
        relative_path: &str,
        bytes: Vec<u8>,
    ) -> anyhow::Result<SharedSessionFile> {
        let response = self
            .client
            .upload_session_file(&self.info.session_id, relative_path, bytes)
            .await
            .with_context(|| format!("failed to upload shared session file `{relative_path}`"))?;
        shared_session_file_from_response(relative_path, response)
    }

    pub async fn release(mut self) -> anyhow::Result<()> {
        self.stop_heartbeat();
        if let Some(task) = self.heartbeat_task.take() {
            match task.await {
                Ok(Ok(())) => {}
                Ok(Err(err)) => log::warn!("board heartbeat stopped with error: {err:#}"),
                Err(err) if err.is_cancelled() => {}
                Err(err) => log::warn!("board heartbeat task join error: {err}"),
            }
        }

        self.client
            .delete_session(&self.info.session_id)
            .await
            .with_context(|| format!("failed to release session `{}`", self.info.session_id))?;
        Ok(())
    }

    fn stop_heartbeat(&self) {
        let _ = self.heartbeat_stop.send(true);
    }
}

fn shared_session_file_from_response(
    requested_relative_path: &str,
    response: SharedSessionFileResponse,
) -> anyhow::Result<SharedSessionFile> {
    let http_url = response.http_url.ok_or_else(|| {
        anyhow!(
            "board server did not return an HTTP URL for shared session file `{requested_relative_path}`"
        )
    })?;
    Ok(SharedSessionFile {
        filename: response.filename,
        relative_path: requested_relative_path.to_string(),
        http_url,
        size: response.size,
        uploaded_at: response.uploaded_at,
    })
}

impl Drop for BoardSession {
    fn drop(&mut self) {
        self.stop_heartbeat();
        if let Some(task) = &self.heartbeat_task {
            task.abort();
        }
    }
}

async fn run_heartbeat_loop(
    client: BoardServerClient,
    session_id: String,
    lease_expires_at: Arc<RwLock<DateTime<Utc>>>,
    mut stop_rx: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            changed = stop_rx.changed() => {
                if changed.is_err() || *stop_rx.borrow() {
                    break;
                }
                continue;
            }
        }

        let heartbeat = client
            .heartbeat(&session_id)
            .await
            .with_context(|| format!("heartbeat failed for session `{session_id}`"))?;
        log::debug!(
            "Session {} heartbeat extended lease to {}",
            heartbeat.session_id,
            heartbeat.lease_expires_at
        );
        *lease_expires_at.write().await = heartbeat.lease_expires_at;
    }

    Ok(())
}

async fn acquire_session_with<CreateFn, CreateFut, SleepFn, SleepFut>(
    board_type: &str,
    board_id: Option<&str>,
    mut create: CreateFn,
    mut sleep: SleepFn,
) -> Result<SessionCreatedResponse, BoardServerClientError>
where
    CreateFn: FnMut() -> CreateFut,
    CreateFut: Future<Output = Result<SessionCreatedResponse, BoardServerClientError>>,
    SleepFn: FnMut(Duration) -> SleepFut,
    SleepFut: Future<Output = ()>,
{
    let board_id = board_id.map(str::trim);
    loop {
        match create().await {
            Ok(session) => return Ok(session),
            Err(err)
                if board_id
                    .map(|board_id| err.is_no_available_board_id(board_id))
                    .unwrap_or(false) =>
            {
                println!(
                    "Board `{}` is not available, retrying in 1s...",
                    board_id.unwrap()
                );
                sleep(Duration::from_secs(1)).await;
            }
            Err(err) if err.is_no_available_board_for(board_type) => {
                println!("No available board for type `{board_type}`, retrying in 1s...");
                sleep(Duration::from_secs(1)).await;
            }
            Err(err) if err.is_board_type_not_found_for(board_type) => {
                return Err(err);
            }
            Err(err) => return Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use chrono::Utc;
    use reqwest::StatusCode;

    use super::{acquire_session_with, shared_session_file_from_response};
    use crate::board::client::{
        BoardServerClientError, SessionCreatedResponse, SharedSessionFileResponse,
    };

    fn created_session(id: &str) -> SessionCreatedResponse {
        SessionCreatedResponse {
            session_id: id.to_string(),
            board_id: "demo-01".to_string(),
            lease_expires_at: Utc::now(),
            serial_available: true,
            boot_mode: "uboot".to_string(),
            ws_url: Some("/api/v1/sessions/demo/serial/ws".to_string()),
        }
    }

    fn no_board_error(board_type: &str) -> BoardServerClientError {
        BoardServerClientError {
            status: StatusCode::CONFLICT,
            code: Some("conflict".to_string()),
            message: format!("no available board for type `{board_type}`"),
        }
    }

    fn no_board_id_error(board_id: &str) -> BoardServerClientError {
        BoardServerClientError {
            status: StatusCode::CONFLICT,
            code: Some("conflict".to_string()),
            message: format!("board `{board_id}` is not available"),
        }
    }

    #[test]
    fn shared_file_keeps_the_requested_relative_path() {
        let uploaded_at = Utc::now();
        let response = SharedSessionFileResponse {
            filename: "probe.sh".to_string(),
            relative_path: "ostool/sessions/demo/tools/probe.sh".to_string(),
            tftp_url: None,
            http_url: Some(
                "http://192.168.1.2:2999/share/sessions/demo/tools/probe.sh"
                    .parse()
                    .unwrap(),
            ),
            size: 5,
            uploaded_at,
        };

        let shared = shared_session_file_from_response("tools/probe.sh", response).unwrap();

        assert_eq!(shared.relative_path, "tools/probe.sh");
    }

    #[tokio::test]
    async fn acquire_session_retries_until_a_board_is_available() {
        let responses = Arc::new(Mutex::new(vec![
            Ok(created_session("demo-session")),
            Err(no_board_error("rk3568")),
            Err(no_board_error("rk3568")),
        ]));
        let sleeps = Arc::new(Mutex::new(Vec::new()));

        let session = acquire_session_with(
            "rk3568",
            None,
            {
                let responses = responses.clone();
                move || {
                    let responses = responses.clone();
                    async move { responses.lock().unwrap().pop().unwrap() }
                }
            },
            {
                let sleeps = sleeps.clone();
                move |duration| {
                    let sleeps = sleeps.clone();
                    async move {
                        sleeps.lock().unwrap().push(duration);
                    }
                }
            },
        )
        .await
        .unwrap();

        assert_eq!(session.session_id, "demo-session");
        assert_eq!(sleeps.lock().unwrap().len(), 2);
        assert!(
            sleeps
                .lock()
                .unwrap()
                .iter()
                .all(|duration| *duration == Duration::from_secs(1))
        );
    }

    #[tokio::test]
    async fn acquire_session_retries_until_requested_board_id_is_available() {
        let responses = Arc::new(Mutex::new(vec![
            Ok(created_session("demo-session")),
            Err(no_board_id_error("demo-02")),
        ]));
        let sleeps = Arc::new(Mutex::new(Vec::new()));

        let session = acquire_session_with(
            "rk3568",
            Some(" demo-02 "),
            {
                let responses = responses.clone();
                move || {
                    let responses = responses.clone();
                    async move { responses.lock().unwrap().pop().unwrap() }
                }
            },
            {
                let sleeps = sleeps.clone();
                move |duration| {
                    let sleeps = sleeps.clone();
                    async move {
                        sleeps.lock().unwrap().push(duration);
                    }
                }
            },
        )
        .await
        .unwrap();

        assert_eq!(session.session_id, "demo-session");
        assert_eq!(*sleeps.lock().unwrap(), vec![Duration::from_secs(1)]);
    }

    #[tokio::test]
    async fn acquire_session_stops_retrying_on_non_conflict_error() {
        let error = BoardServerClientError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: Some("request_failed".to_string()),
            message: "boom".to_string(),
        };

        let result = acquire_session_with(
            "rk3568",
            None,
            || {
                let error = error.clone();
                async move { Err(error) }
            },
            |_| async {},
        )
        .await;

        assert_eq!(result.unwrap_err().message, "boom");
    }

    #[tokio::test]
    async fn acquire_session_stops_retrying_when_board_type_is_missing() {
        let error = BoardServerClientError {
            status: StatusCode::NOT_FOUND,
            code: Some("not_found".to_string()),
            message: "board type `rk3568` not found".to_string(),
        };
        let sleeps = Arc::new(Mutex::new(Vec::new()));

        let result = acquire_session_with(
            "rk3568",
            None,
            || {
                let error = error.clone();
                async move { Err(error) }
            },
            {
                let sleeps = sleeps.clone();
                move |duration| {
                    let sleeps = sleeps.clone();
                    async move {
                        sleeps.lock().unwrap().push(duration);
                    }
                }
            },
        )
        .await;

        assert_eq!(result.unwrap_err().message, "board type `rk3568` not found");
        assert!(sleeps.lock().unwrap().is_empty());
    }
}
