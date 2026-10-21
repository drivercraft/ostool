//! Server-owned runtime sampling. Browsers consume the shared SSE projection.
use crate::{AppState, BoardLeaseState};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::Row;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Serialize)]
struct Task {
    id: String,
    name: String,
    state: String,
    interval_secs: u64,
    runs: u64,
    last_run: Option<DateTime<Utc>>,
    next_run: Option<DateTime<Utc>>,
    last_duration_ms: Option<u64>,
    last_ok: Option<bool>,
}
struct Inner {
    started_at: DateTime<Utc>,
    started: Instant,
    sample: Mutex<Value>,
    history: Mutex<VecDeque<Value>>,
    tasks: Mutex<BTreeMap<String, Task>>,
    requests: AtomicU64,
    errors: AtomicU64,
    inflight: AtomicU64,
    latency_us: AtomicU64,
    streams: AtomicU64,
}
#[derive(Clone)]
pub struct Monitor(Arc<Inner>);
impl Default for Monitor {
    fn default() -> Self {
        Self(Arc::new(Inner {
            started_at: Utc::now(),
            started: Instant::now(),
            sample: Mutex::new(json!({"ready":false})),
            history: Mutex::new(VecDeque::new()),
            tasks: Mutex::new(BTreeMap::new()),
            requests: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            inflight: AtomicU64::new(0),
            latency_us: AtomicU64::new(0),
            streams: AtomicU64::new(0),
        }))
    }
}
pub struct StreamGuard(Monitor);
pub struct RequestGuard {
    monitor: Monitor,
    start: Instant,
    completed: bool,
}
impl RequestGuard {
    pub fn finish(mut self, status: u16) {
        self.monitor.request_finished(status, self.start.elapsed());
        self.completed = true;
    }
}
impl Drop for RequestGuard {
    fn drop(&mut self) {
        if !self.completed {
            self.monitor.0.inflight.fetch_sub(1, Ordering::Relaxed);
        }
    }
}
impl Drop for StreamGuard {
    fn drop(&mut self) {
        self.0.0.streams.fetch_sub(1, Ordering::Relaxed);
    }
}
impl Monitor {
    pub fn stream(&self) -> StreamGuard {
        self.0.streams.fetch_add(1, Ordering::Relaxed);
        StreamGuard(self.clone())
    }
    pub fn request(&self) -> RequestGuard {
        self.0.inflight.fetch_add(1, Ordering::Relaxed);
        RequestGuard {
            monitor: self.clone(),
            start: Instant::now(),
            completed: false,
        }
    }
    fn request_finished(&self, status: u16, duration: Duration) {
        self.0.inflight.fetch_sub(1, Ordering::Relaxed);
        self.0.requests.fetch_add(1, Ordering::Relaxed);
        self.0
            .latency_us
            .fetch_add(duration.as_micros() as u64, Ordering::Relaxed);
        if status >= 400 {
            self.0.errors.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub fn register(&self, id: &str, name: &str, seconds: u64) {
        self.0
            .tasks
            .lock()
            .unwrap()
            .entry(id.into())
            .or_insert(Task {
                id: id.into(),
                name: name.into(),
                state: "scheduled".into(),
                interval_secs: seconds,
                runs: 0,
                last_run: None,
                next_run: None,
                last_duration_ms: None,
                last_ok: None,
            });
    }
    pub fn task_started(&self, id: &str) {
        if let Some(task) = self.0.tasks.lock().unwrap().get_mut(id) {
            task.state = "running".into();
        }
    }
    pub fn task_finished(&self, id: &str, ok: bool, duration: Duration) {
        if let Some(task) = self.0.tasks.lock().unwrap().get_mut(id) {
            task.state = if ok { "scheduled" } else { "error" }.into();
            task.last_run = Some(Utc::now());
            task.next_run = Some(Utc::now() + chrono::Duration::seconds(task.interval_secs as i64));
            task.last_duration_ms = Some(duration.as_millis() as u64);
            task.last_ok = Some(ok);
            task.runs += 1;
        }
    }
    pub fn snapshot(&self) -> Value {
        let requests = self.0.requests.load(Ordering::Relaxed);
        json!({"started_at":self.0.started_at,"uptime_secs":self.0.started.elapsed().as_secs(),"version":env!("CARGO_PKG_VERSION"),"platform":std::env::consts::OS,"arch":std::env::consts::ARCH,"pid":std::process::id(),"logical_cpus":std::thread::available_parallelism().map(|v|v.get()).ok(),"sample":*self.0.sample.lock().unwrap(),"history":*self.0.history.lock().unwrap(),"tasks":self.0.tasks.lock().unwrap().values().cloned().collect::<Vec<_>>(),"http":{"requests":requests,"errors":self.0.errors.load(Ordering::Relaxed),"inflight":self.0.inflight.load(Ordering::Relaxed),"mean_latency_ms":if requests==0{0.0}else{self.0.latency_us.load(Ordering::Relaxed) as f64 /requests as f64/1000.0},"sse_connections":self.0.streams.load(Ordering::Relaxed)}})
    }
    pub fn start(&self, state: AppState) {
        self.register("runtime", "服务器运行采样", 5);
        let monitor = self.clone();
        tokio::spawn(async move {
            let mut previous: Option<Raw> = None;
            loop {
                let started = Instant::now();
                monitor.task_started("runtime");
                let config = state.config.read().await.clone();
                let raw = tokio::task::spawn_blocking(move || {
                    sample_host(&config.data_dir, &config.network.interface)
                })
                .await
                .unwrap_or_default();
                let elapsed = previous
                    .as_ref()
                    .map(|previous| raw.at.duration_since(previous.at).as_secs_f64())
                    .filter(|s| *s > 0.0);
                let cpu = previous
                    .as_ref()
                    .and_then(|previous| previous.cpu.zip(raw.cpu))
                    .and_then(|((total, idle), (next_total, next_idle))| {
                        let total = next_total.checked_sub(total)?;
                        let idle = next_idle.checked_sub(idle)?;
                        (total > 0).then_some(
                            (100.0 * (total.saturating_sub(idle)) as f64 / total.max(1) as f64)
                                .clamp(0.0, 100.0),
                        )
                    });
                let rate = |current: Option<u64>, old: Option<u64>| {
                    current
                        .zip(old)
                        .and_then(|(current, old)| current.checked_sub(old))
                        .zip(elapsed)
                        .map(|(delta, elapsed)| delta as f64 / elapsed)
                };
                let rx = rate(raw.rx, previous.as_ref().and_then(|v| v.rx));
                let tx = rate(raw.tx, previous.as_ref().and_then(|v| v.tx));
                let ping = Instant::now();
                let database_ok = {
                    let db = &state.database;
                    matches!(
                        tokio::time::timeout(
                            Duration::from_secs(2),
                            sqlx::query("SELECT 1").execute(db.pool())
                        )
                        .await,
                        Ok(Ok(_))
                    )
                };
                let sample = json!({"ready":true,"sampled_at":Utc::now(),"hostname":raw.hostname,"cpu_percent":cpu,"load_average":raw.load,"memory_total":raw.memory_total,"memory_available":raw.memory_available,"process_rss":raw.rss,"disk_total":raw.disk_total,"disk_available":raw.disk_available,"interface":raw.interface,"rx_bytes":raw.rx,"tx_bytes":raw.tx,"rx_bytes_per_second":rx,"tx_bytes_per_second":tx,"database_ok":database_ok,"database_latency_ms":ping.elapsed().as_millis(),"pool_size":state.database.pool().size(),"pool_idle":state.database.pool().num_idle()});
                *monitor.0.sample.lock().unwrap() = sample;
                {
                    let mut history = monitor.0.history.lock().unwrap();
                    history.push_back(json!({"at":Utc::now(),"cpu_percent":cpu,"memory_percent":raw.memory_total.zip(raw.memory_available).filter(|(total,_)|*total>0).map(|(total,available)|100.0*(total.saturating_sub(available))as f64/total as f64),"rx":rx,"tx":tx}));
                    while history.len() > 60 {
                        history.pop_front();
                    }
                }
                previous = Some(raw);
                monitor.task_finished("runtime", database_ok, started.elapsed());
                state.admin_events.invalidate(&["dashboard"]);
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
    }
}
struct Raw {
    at: Instant,
    cpu: Option<(u64, u64)>,
    load: Option<Vec<f64>>,
    memory_total: Option<u64>,
    memory_available: Option<u64>,
    rss: Option<u64>,
    disk_total: Option<u64>,
    disk_available: Option<u64>,
    hostname: Option<String>,
    interface: String,
    rx: Option<u64>,
    tx: Option<u64>,
}
impl Default for Raw {
    fn default() -> Self {
        Self {
            at: Instant::now(),
            cpu: None,
            load: None,
            memory_total: None,
            memory_available: None,
            rss: None,
            disk_total: None,
            disk_available: None,
            hostname: None,
            interface: String::new(),
            rx: None,
            tx: None,
        }
    }
}
fn parse_kib(source: &str, key: &str) -> Option<u64> {
    source.lines().find_map(|line| {
        let (label, value) = line.split_once(':')?;
        (label == key)
            .then(|| {
                value
                    .split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()?
                    .checked_mul(1024)
            })
            .flatten()
    })
}
fn parse_cpu(source: &str) -> Option<(u64, u64)> {
    let line = source.lines().find(|line| line.starts_with("cpu "))?;
    let values = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if values.len() < 4 {
        return None;
    }
    Some((
        values.iter().sum(),
        values[3] + values.get(4).copied().unwrap_or(0),
    ))
}
fn sample_host(path: &std::path::Path, interface: &str) -> Raw {
    let read = |path: &str| std::fs::read_to_string(path).ok();
    let memory = read("/proc/meminfo").unwrap_or_default();
    let process = read("/proc/self/status").unwrap_or_default();
    let network = |name: &str| {
        if interface
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.:".contains(&c))
        {
            read(&format!("/sys/class/net/{interface}/statistics/{name}"))
                .and_then(|v| v.trim().parse().ok())
        } else {
            None
        }
    };
    Raw {
        at: Instant::now(),
        cpu: read("/proc/stat").and_then(|v| parse_cpu(&v)),
        load: read("/proc/loadavg").and_then(|value| {
            value
                .split_whitespace()
                .take(3)
                .map(str::parse::<f64>)
                .collect::<Result<Vec<_>, _>>()
                .ok()
        }),
        memory_total: parse_kib(&memory, "MemTotal"),
        memory_available: parse_kib(&memory, "MemAvailable"),
        rss: parse_kib(&process, "VmRSS"),
        disk_total: fs4::total_space(path).ok(),
        disk_available: fs4::available_space(path).ok(),
        hostname: read("/proc/sys/kernel/hostname").map(|s| s.trim().to_string()),
        interface: interface.into(),
        rx: network("rx_bytes"),
        tx: network("tx_bytes"),
    }
}

pub async fn projection(state: &AppState) -> Value {
    let boards = state.boards.read().await;
    let runtimes = state.board_runtimes.read().await;
    let mut counts = BTreeMap::<&str, usize>::from([
        ("idle", 0),
        ("using", 0),
        ("releasing", 0),
        ("error", 0),
        ("disabled", 0),
    ]);
    for board in boards.values() {
        let key = if board.disabled {
            "disabled"
        } else {
            match runtimes
                .get(&board.id)
                .map(|v| v.lease_state)
                .unwrap_or(BoardLeaseState::Idle)
            {
                BoardLeaseState::Idle => "idle",
                BoardLeaseState::Using => "using",
                BoardLeaseState::Releasing => "releasing",
                BoardLeaseState::Error => "error",
            }
        };
        *counts.get_mut(key).expect("known state") += 1;
    }
    let total = boards.len();
    drop(runtimes);
    drop(boards);
    let public = state.config.read().await.edition.is_public();
    let devices = state.loader_registry.snapshots().await;
    let mut operations = json!({"boards":total,"board_states":counts,"sessions":state.sessions.read().await.len(),"devices":devices.len(),"online_devices":devices.iter().filter(|d|d.online).count(),"conflicting_devices":devices.iter().filter(|d|d.conflict).count(),"dtbs":state.dtb_store.list_all().await.map(|v|v.len()).ok(),"public":public});
    {
        let db = &state.database;
        let result=tokio::time::timeout(Duration::from_secs(2),sqlx::query("SELECT (SELECT count(*) FROM users) users,(SELECT count(*) FROM users WHERE NOT disabled) enabled_users,(SELECT count(*) FROM board_rentals WHERE cancelled_at IS NULL AND starts_at<=now() AND ends_at>now()) active_rentals,(SELECT count(*) FROM board_rentals WHERE cancelled_at IS NULL AND starts_at>now()) scheduled_rentals,(SELECT count(*) FROM board_rentals WHERE cancelled_at IS NOT NULL) cancelled_rentals,(SELECT count(*) FROM rental_requests WHERE status='pending') pending_requests,(SELECT count(*) FROM audit_events WHERE created_at>now()-interval '24 hours') recent_audits,(SELECT count(*) FROM audit_events WHERE created_at>now()-interval '24 hours' AND result IN ('failed','denied')) recent_failures").fetch_one(db.pool())).await;
        if let Ok(Ok(row)) = result {
            operations["database_available"] = json!(true);
            for key in ["recent_audits", "recent_failures"] {
                operations[key] = json!(row.get::<i64, _>(key));
            }
            if public {
                for key in [
                    "users",
                    "enabled_users",
                    "active_rentals",
                    "scheduled_rentals",
                    "cancelled_rentals",
                    "pending_requests",
                ] {
                    operations[key] = json!(row.get::<i64, _>(key));
                }
            }
        } else {
            operations["database_available"] = json!(false);
        }
    }
    json!({"runtime":state.monitor.snapshot(),"operations":operations,"audit":state.audit.snapshot(),"power_actions":state.admin_power.snapshots(),"ota_jobs":state.ota.jobs().await,"audit_retention_days":state.config.read().await.audit.retention_days})
}
pub async fn get(axum::extract::State(state): axum::extract::State<AppState>) -> axum::Json<Value> {
    axum::Json(projection(&state).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_cancellation_releases_inflight_without_inventing_a_response() {
        let monitor = Monitor::default();
        let request = monitor.request();
        assert_eq!(monitor.snapshot()["http"]["inflight"], 1);
        drop(request);
        assert_eq!(monitor.snapshot()["http"]["inflight"], 0);
        assert_eq!(monitor.snapshot()["http"]["requests"], 0);
        monitor.request().finish(403);
        let snapshot = monitor.snapshot();
        assert_eq!(snapshot["http"]["requests"], 1);
        assert_eq!(snapshot["http"]["errors"], 1);
    }
    #[test]
    fn parses_kernel_metrics_without_inventing_missing_values() {
        assert_eq!(
            parse_kib("MemTotal: 100 kB\nMemAvailable: 20 kB", "MemTotal"),
            Some(102400)
        );
        assert_eq!(parse_kib("", "MemTotal"), None);
        assert_eq!(
            parse_cpu("cpu  10 20 30 40 5 1 2 3 100 200"),
            Some((111, 45))
        );
        assert_eq!(parse_cpu("broken"), None);
    }
}
