//! Bounded, in-memory HTTP throughput tests on a dedicated listener.

use std::{
    collections::HashMap,
    io,
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{ConnectInfo, Path, Query, Request, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
    task::AbortHandle,
};
use uuid::Uuid;

use crate::config::NetworkTestConfig;

const CHUNK_SIZE: usize = 64 * 1024;
const MAX_RECORDS: usize = 4096;
const PENDING_TTL: Duration = Duration::from_secs(10 * 60);
const RESULT_TTL: Duration = Duration::from_secs(60 * 60);

#[derive(Clone)]
struct Shared {
    config: NetworkTestConfig,
    slots: Arc<Semaphore>,
    records: Arc<Mutex<HashMap<Uuid, Arc<TestRecord>>>>,
    payload: Bytes,
}

struct TestRecord {
    created: Instant,
    upload_bytes: AtomicU64,
    download_bytes: AtomicU64,
    state: Mutex<TestState>,
}

struct TestState {
    upload: TransferState,
    download: TransferState,
    active_directions: u8,
    permit: Option<OwnedSemaphorePermit>,
    last_finished: Option<Instant>,
}

struct TransferState {
    status: TransferStatus,
    started: Option<Instant>,
    elapsed: Option<Duration>,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum TransferStatus {
    NotStarted,
    Running,
    Completed,
    Canceled,
    TimedOut,
    Failed,
}

#[derive(Clone, Copy)]
enum Direction {
    Upload,
    Download,
}

struct TransferGuard {
    record: Arc<TestRecord>,
    direction: Direction,
    finished: bool,
    watchdog: Option<AbortHandle>,
}

#[derive(Serialize)]
struct CreatedResponse {
    test_id: Uuid,
}

#[derive(Serialize)]
struct TestResponse {
    test_id: Uuid,
    upload: TransferSnapshot,
    download: TransferSnapshot,
}

#[derive(Serialize)]
struct TransferSnapshot {
    status: TransferStatus,
    bytes: u64,
    elapsed_ms: Option<u64>,
    bits_per_second: Option<f64>,
    error: Option<String>,
}

#[derive(Serialize)]
struct ErrorResponse {
    code: &'static str,
    message: String,
}

#[derive(Debug)]
struct TestError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl TestError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", "test not found")
    }

    fn conflict() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            "conflict",
            "this test direction has already started",
        )
    }

    fn busy() -> Self {
        Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            "busy",
            "network test capacity is exhausted",
        )
    }
}

impl IntoResponse for TestError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse {
                code: self.code,
                message: self.message,
            }),
        )
            .into_response()
    }
}

impl Default for TransferState {
    fn default() -> Self {
        Self {
            status: TransferStatus::NotStarted,
            started: None,
            elapsed: None,
            error: None,
        }
    }
}

impl TestRecord {
    fn new() -> Self {
        Self::new_at(Instant::now())
    }

    fn new_at(created: Instant) -> Self {
        Self {
            created,
            upload_bytes: AtomicU64::new(0),
            download_bytes: AtomicU64::new(0),
            state: Mutex::new(TestState {
                upload: TransferState::default(),
                download: TransferState::default(),
                active_directions: 0,
                permit: None,
                last_finished: None,
            }),
        }
    }

    fn is_expired(&self, now: Instant) -> bool {
        let state = self.state.lock().expect("network test state poisoned");
        if state.active_directions != 0 {
            return false;
        }
        let (since, ttl) = match state.last_finished {
            Some(finished) => (finished, RESULT_TTL),
            None => (self.created, PENDING_TTL),
        };
        now.saturating_duration_since(since) >= ttl
    }

    fn eviction_time(&self) -> Option<Instant> {
        let state = self.state.lock().expect("network test state poisoned");
        if state.active_directions != 0 {
            None
        } else {
            state.last_finished
        }
    }

    fn snapshot(&self, id: Uuid) -> TestResponse {
        let now = Instant::now();
        let state = self.state.lock().expect("network test state poisoned");
        TestResponse {
            test_id: id,
            upload: state
                .upload
                .snapshot(self.upload_bytes.load(Ordering::Relaxed), now),
            download: state
                .download
                .snapshot(self.download_bytes.load(Ordering::Relaxed), now),
        }
    }

    fn bytes(&self, direction: Direction) -> &AtomicU64 {
        match direction {
            Direction::Upload => &self.upload_bytes,
            Direction::Download => &self.download_bytes,
        }
    }
}

impl TestState {
    fn direction(&self, direction: Direction) -> &TransferState {
        match direction {
            Direction::Upload => &self.upload,
            Direction::Download => &self.download,
        }
    }

    fn direction_mut(&mut self, direction: Direction) -> &mut TransferState {
        match direction {
            Direction::Upload => &mut self.upload,
            Direction::Download => &mut self.download,
        }
    }
}

impl TransferState {
    fn snapshot(&self, bytes: u64, now: Instant) -> TransferSnapshot {
        let elapsed = self
            .elapsed
            .or_else(|| self.started.map(|started| now.duration_since(started)));
        TransferSnapshot {
            status: self.status,
            bytes,
            elapsed_ms: elapsed.map(|duration| duration.as_millis() as u64),
            bits_per_second: elapsed
                .filter(|duration| !duration.is_zero())
                .map(|duration| (bytes as f64 * 8.0) / duration.as_secs_f64()),
            error: self.error.clone(),
        }
    }
}

impl TransferGuard {
    fn add_bytes(&self, count: usize) -> Result<(), TestError> {
        let state = self
            .record
            .state
            .lock()
            .expect("network test state poisoned");
        if state.direction(self.direction).status != TransferStatus::Running {
            return Err(TestError::new(
                StatusCode::REQUEST_TIMEOUT,
                "timed_out",
                "test transfer has ended",
            ));
        }
        let counter = self.record.bytes(self.direction);
        counter
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(count as u64)
            })
            .map(|_| ())
            .map_err(|_| TestError::bad_request("test byte counter overflow"))
    }

    fn finish(&mut self, status: TransferStatus, error: Option<String>) {
        if self.finished {
            return;
        }
        if let Some(watchdog) = self.watchdog.take() {
            watchdog.abort();
        }
        finish_record(&self.record, self.direction, status, error);
        self.finished = true;
    }

    fn snapshot(&self) -> TransferSnapshot {
        let state = self
            .record
            .state
            .lock()
            .expect("network test state poisoned");
        state.direction(self.direction).snapshot(
            self.record.bytes(self.direction).load(Ordering::Relaxed),
            Instant::now(),
        )
    }
}

fn finish_record(
    record: &TestRecord,
    direction: Direction,
    status: TransferStatus,
    error: Option<String>,
) {
    let now = Instant::now();
    let mut state = record.state.lock().expect("network test state poisoned");
    let transfer = state.direction_mut(direction);
    if transfer.status != TransferStatus::Running {
        return;
    }
    transfer.status = status;
    transfer.elapsed = transfer.started.map(|started| now.duration_since(started));
    transfer.error = error;
    state.active_directions -= 1;
    state.last_finished = Some(now);
    if state.active_directions == 0 {
        state.permit.take();
    }
}

impl Drop for TransferGuard {
    fn drop(&mut self) {
        self.finish(
            TransferStatus::Canceled,
            Some("transfer canceled before completion".into()),
        );
    }
}

impl Shared {
    fn record(&self, id: Uuid) -> Result<Arc<TestRecord>, TestError> {
        let mut records = self.records.lock().expect("network test registry poisoned");
        let record = records.get(&id).cloned().ok_or_else(TestError::not_found)?;
        if record.is_expired(Instant::now()) {
            records.remove(&id);
            return Err(TestError::not_found());
        }
        Ok(record)
    }

    fn start(&self, id: Uuid, direction: Direction) -> Result<TransferGuard, TestError> {
        let mut records = self.records.lock().expect("network test registry poisoned");
        let record = records.get(&id).cloned().ok_or_else(TestError::not_found)?;
        if record.is_expired(Instant::now()) {
            records.remove(&id);
            return Err(TestError::not_found());
        }
        let mut state = record.state.lock().expect("network test state poisoned");
        if state.direction(direction).status != TransferStatus::NotStarted {
            return Err(TestError::conflict());
        }
        if state.active_directions == 0 {
            state.permit = Some(
                self.slots
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| TestError::busy())?,
            );
        }
        let transfer = state.direction_mut(direction);
        transfer.status = TransferStatus::Running;
        transfer.started = Some(Instant::now());
        state.active_directions += 1;
        drop(state);
        drop(records);
        Ok(TransferGuard {
            record,
            direction,
            finished: false,
            watchdog: None,
        })
    }

    fn create(&self) -> Result<Uuid, TestError> {
        let mut records = self.records.lock().expect("network test registry poisoned");
        let now = Instant::now();
        records.retain(|_, record| !record.is_expired(now));
        if records.len() >= MAX_RECORDS {
            let oldest = records
                .iter()
                .filter_map(|(id, record)| record.eviction_time().map(|time| (*id, time)))
                .min_by_key(|(_, time)| *time)
                .map(|(id, _)| id);
            if let Some(id) = oldest {
                records.remove(&id);
            } else {
                return Err(TestError::busy());
            }
        }
        let id = Uuid::new_v4();
        records.insert(id, Arc::new(TestRecord::new()));
        Ok(id)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DownloadQuery {
    bytes: Option<u64>,
    duration_secs: Option<u64>,
}

struct DownloadStream {
    guard: TransferGuard,
    remaining: Option<u64>,
    end: Instant,
    time_mode: bool,
    payload: Bytes,
    ended: bool,
}

/// Build the router mounted only on the configured network-test listener.
pub fn build_router(config: NetworkTestConfig) -> Router {
    let mut payload = vec![0_u8; CHUNK_SIZE];
    for (index, byte) in payload.iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(73).wrapping_add(19);
    }
    let shared = Shared {
        slots: Arc::new(Semaphore::new(config.max_active_tests)),
        config,
        records: Arc::new(Mutex::new(HashMap::new())),
        payload: Bytes::from(payload),
    };
    Router::new()
        .route("/healthz", get(health))
        .route("/v1/tests", post(create_test))
        .route("/v1/tests/{id}", get(get_test))
        .route("/v1/tests/{id}/upload", put(upload))
        .route("/v1/tests/{id}/download", get(download))
        .with_state(shared)
}

async fn health() -> StatusCode {
    StatusCode::OK
}

async fn create_test(
    State(shared): State<Shared>,
) -> Result<(StatusCode, Json<CreatedResponse>), TestError> {
    Ok((
        StatusCode::CREATED,
        Json(CreatedResponse {
            test_id: shared.create()?,
        }),
    ))
}

async fn get_test(
    Path(id): Path<Uuid>,
    State(shared): State<Shared>,
) -> Result<Json<TestResponse>, TestError> {
    Ok(Json(shared.record(id)?.snapshot(id)))
}

async fn upload(
    Path(id): Path<Uuid>,
    State(shared): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
) -> Result<Json<TransferSnapshot>, TestError> {
    let mut guard = shared.start(id, Direction::Upload)?;
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(shared.config.max_duration_secs))
        .ok_or_else(|| TestError::bad_request("configured test duration is too large"))?;
    let body = request.into_body().into_data_stream();
    tokio::pin!(body);
    loop {
        let next = tokio::time::timeout_at(deadline.into(), body.next()).await;
        match next {
            Ok(Some(Ok(chunk))) => {
                if let Err(error) = guard.add_bytes(chunk.len()) {
                    guard.finish(TransferStatus::Failed, Some(error.message.clone()));
                    return Err(error);
                }
            }
            Ok(Some(Err(error))) => {
                let message = format!("upload body read failed: {error}");
                guard.finish(TransferStatus::Failed, Some(message.clone()));
                return Err(TestError::bad_request(message));
            }
            Ok(None) => {
                guard.finish(TransferStatus::Completed, None);
                let result = guard.snapshot();
                log::info!(
                    "network test {id} upload from {peer}: {} bytes, {:?}",
                    result.bytes,
                    result.status
                );
                return Ok(Json(result));
            }
            Err(_) => {
                guard.finish(
                    TransferStatus::TimedOut,
                    Some("maximum test duration reached".into()),
                );
                return Err(TestError::new(
                    StatusCode::REQUEST_TIMEOUT,
                    "timed_out",
                    "maximum test duration reached",
                ));
            }
        }
    }
}

async fn download(
    Path(id): Path<Uuid>,
    Query(query): Query<DownloadQuery>,
    State(shared): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Result<Response, TestError> {
    let (remaining, duration, time_mode) = match (query.bytes, query.duration_secs) {
        (Some(bytes), None) if bytes > 0 => (Some(bytes), shared.config.max_duration_secs, false),
        (None, Some(seconds)) if seconds > 0 && seconds <= shared.config.max_duration_secs => {
            (None, seconds, true)
        }
        _ => {
            return Err(TestError::bad_request(
                "provide exactly one positive bytes or duration_secs value within the configured duration",
            ));
        }
    };
    let mut guard = shared.start(id, Direction::Download)?;
    let end = Instant::now()
        .checked_add(Duration::from_secs(duration))
        .ok_or_else(|| TestError::bad_request("configured test duration is too large"))?;
    let watchdog_record = guard.record.clone();
    let watchdog = tokio::spawn(async move {
        tokio::time::sleep_until(end.into()).await;
        let (status, error) = if time_mode {
            (TransferStatus::Completed, None)
        } else {
            (
                TransferStatus::TimedOut,
                Some("maximum test duration reached".into()),
            )
        };
        finish_record(&watchdog_record, Direction::Download, status, error);
    });
    guard.watchdog = Some(watchdog.abort_handle());
    log::info!("network test {id} download to {peer} started");
    let state = DownloadStream {
        guard,
        remaining,
        end,
        time_mode,
        payload: shared.payload.clone(),
        ended: false,
    };
    let stream = stream::unfold(state, |mut state| async move {
        if state.ended {
            return None;
        }
        if state
            .guard
            .record
            .state
            .lock()
            .expect("network test state poisoned")
            .download
            .status
            != TransferStatus::Running
        {
            return None;
        }
        if let Some(0) = state.remaining {
            state.guard.finish(TransferStatus::Completed, None);
            return None;
        }
        if Instant::now() >= state.end {
            if state.time_mode {
                state.guard.finish(TransferStatus::Completed, None);
                return None;
            }
            state.guard.finish(
                TransferStatus::TimedOut,
                Some("maximum test duration reached".into()),
            );
            state.ended = true;
            return Some((
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "maximum test duration reached",
                )),
                state,
            ));
        }
        let size = state
            .remaining
            .map(|remaining| remaining.min(CHUNK_SIZE as u64) as usize)
            .unwrap_or(CHUNK_SIZE);
        if let Err(error) = state.guard.add_bytes(size) {
            state
                .guard
                .finish(TransferStatus::Failed, Some(error.message.clone()));
            state.ended = true;
            return Some((Err(io::Error::other(error.message)), state));
        }
        if let Some(remaining) = &mut state.remaining {
            *remaining -= size as u64;
            if *remaining == 0 {
                state.guard.finish(TransferStatus::Completed, None);
            }
        }
        let chunk = state.payload.slice(..size);
        Some((Ok::<Bytes, io::Error>(chunk), state))
    });
    let mut response = Body::from_stream(stream).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Some(bytes) = query.bytes {
        response.headers_mut().insert(
            header::CONTENT_LENGTH,
            HeaderValue::from_str(&bytes.to_string()).expect("u64 is a valid header value"),
        );
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_expire_only_after_their_respective_idle_period() {
        let now = Instant::now();
        let pending = TestRecord::new_at(now - PENDING_TTL);
        assert!(pending.is_expired(now));
        let active = TestRecord::new_at(now - PENDING_TTL);
        active.state.lock().unwrap().active_directions = 1;
        assert!(!active.is_expired(now));
        let finished = TestRecord::new_at(now - RESULT_TTL);
        finished.state.lock().unwrap().last_finished =
            Some(now - RESULT_TTL + Duration::from_secs(1));
        assert!(!finished.is_expired(now));
        assert!(finished.is_expired(now + Duration::from_secs(1)));
    }

    #[test]
    fn expired_records_disappear_and_capacity_evicts_the_oldest_finished_record() {
        let shared = Shared {
            config: NetworkTestConfig::default(),
            slots: Arc::new(Semaphore::new(64)),
            records: Arc::new(Mutex::new(HashMap::new())),
            payload: Bytes::new(),
        };
        let pending_id = Uuid::new_v4();
        let expired_pending = Arc::new(TestRecord::new_at(Instant::now() - PENDING_TTL));
        shared
            .records
            .lock()
            .unwrap()
            .insert(pending_id, expired_pending);
        assert_eq!(
            shared.record(pending_id).err().unwrap().status,
            StatusCode::NOT_FOUND
        );

        let oldest_id = Uuid::new_v4();
        let oldest = Arc::new(TestRecord::new());
        oldest.state.lock().unwrap().last_finished = Some(Instant::now() - Duration::from_secs(30));
        shared.records.lock().unwrap().insert(oldest_id, oldest);
        for _ in 1..MAX_RECORDS {
            let record = Arc::new(TestRecord::new());
            record.state.lock().unwrap().last_finished = Some(Instant::now());
            shared
                .records
                .lock()
                .unwrap()
                .insert(Uuid::new_v4(), record);
        }
        let new_id = shared.create().unwrap();
        assert!(shared.record(new_id).is_ok());
        assert_eq!(
            shared.record(oldest_id).err().unwrap().status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(shared.records.lock().unwrap().len(), MAX_RECORDS);
    }
}
