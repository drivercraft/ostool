//! Revisioned management projection. Business owners invalidate affected topics;
//! one projector serializes reads and publication. Subscribers never read owners.
use crate::AppState;
use axum::{
    extract::{Query, State},
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::{Stream, stream};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    convert::Infallible,
    sync::{Arc, Mutex},
};
use tokio::sync::{Notify, watch};

pub const TOPICS: &[&str] = &[
    "boards",
    "quarantined_boards",
    "runtimes",
    "sessions",
    "loaders",
    "ota",
    "virtual",
    "dtbs",
    "serial",
    "serial_manager",
    "network",
    "server",
    "tftp",
    "tftp_status",
    "overview",
    "power_actions",
    "users",
    "roles",
    "deployment",
    "rentals",
    "rental_requests",
    "dashboard",
    "audit",
];
const HISTORY_LIMIT: usize = 256;

#[derive(Clone, Debug, Serialize)]
pub struct Update {
    pub epoch: String,
    pub revision: u64,
    pub kind: &'static str,
    pub data: BTreeMap<String, Value>,
}
#[derive(Debug)]
struct Projection {
    epoch: String,
    revision: u64,
    initialized: bool,
    data: BTreeMap<String, Value>,
    history: VecDeque<Update>,
}
#[derive(Clone, Debug)]
pub struct AdminEvents {
    projection: Arc<Mutex<Projection>>,
    dirty: Arc<Mutex<BTreeSet<&'static str>>>,
    wake: Arc<Notify>,
    changed: watch::Sender<u64>,
}
impl Default for AdminEvents {
    fn default() -> Self {
        Self {
            projection: Arc::new(Mutex::new(Projection {
                epoch: uuid::Uuid::new_v4().to_string(),
                revision: 0,
                initialized: false,
                data: BTreeMap::new(),
                history: VecDeque::new(),
            })),
            dirty: Arc::new(Mutex::new(BTreeSet::new())),
            wake: Arc::new(Notify::new()),
            changed: watch::channel(0).0,
        }
    }
}
impl AdminEvents {
    pub fn invalidate(&self, topics: &[&'static str]) {
        let mut dirty = self.dirty.lock().unwrap();
        for topic in topics {
            dirty.insert(topic);
            match *topic {
                "boards" => {
                    dirty.extend(["runtimes", "loaders", "overview"]);
                }
                "sessions" | "runtimes" => {
                    dirty.extend(["sessions", "runtimes", "overview"]);
                }
                "network" | "server" | "tftp" => {
                    dirty.extend(["tftp_status", "overview"]);
                }
                "tftp_status" => {
                    dirty.insert("overview");
                }
                "serial" => {
                    dirty.insert("boards");
                }
                _ => {}
            }
            if !matches!(*topic, "dashboard" | "audit") {
                dirty.insert("dashboard");
            }
            if *topic == "ota" {
                dirty.insert("audit");
            }
        }
        drop(dirty);
        self.wake.notify_one();
    }
    fn publish(&self, values: BTreeMap<String, Value>) {
        let mut p = self.projection.lock().unwrap();
        let changes: BTreeMap<_, _> = values
            .into_iter()
            .filter(|(k, v)| p.data.get(k) != Some(v))
            .collect();
        if changes.is_empty() && p.initialized {
            return;
        }
        p.initialized = true;
        p.revision += 1;
        p.data.extend(changes.clone());
        let update = Update {
            epoch: p.epoch.clone(),
            revision: p.revision,
            kind: "update",
            data: changes,
        };
        p.history.push_back(update);
        while p.history.len() > HISTORY_LIMIT {
            p.history.pop_front();
        }
        self.changed.send_replace(p.revision);
    }
    fn after(&self, cursor: Option<&str>) -> VecDeque<Update> {
        let p = self.projection.lock().unwrap();
        if let Some((epoch, revision)) = cursor.and_then(|c| c.rsplit_once(':'))
            && epoch == p.epoch
            && let Ok(revision) = revision.parse::<u64>()
            && revision <= p.revision
            && p.history
                .front()
                .is_some_and(|first| revision >= first.revision.saturating_sub(1))
        {
            return p
                .history
                .iter()
                .filter(|e| e.revision > revision)
                .cloned()
                .collect();
        }
        VecDeque::from([Update {
            epoch: p.epoch.clone(),
            revision: p.revision,
            kind: "snapshot",
            data: p.data.clone(),
        }])
    }
    pub(crate) fn start(&self, state: AppState) {
        let events = self.clone();
        events.invalidate(TOPICS);
        tokio::spawn(async move {
            loop {
                events.wake.notified().await;
                let mut topics = std::mem::take(&mut *events.dirty.lock().unwrap());
                let overview = topics.remove("overview");
                let mut values = BTreeMap::new();
                for topic in topics {
                    let value = match crate::api::router::admin_topic(&state, topic).await {
                        Ok(value) => value,
                        Err(error) => serde_json::json!({"error": error.message}),
                    };
                    values.insert(topic.to_string(), value);
                }
                if overview {
                    let status = values
                        .get("tftp_status")
                        .cloned()
                        .or_else(|| {
                            events
                                .projection
                                .lock()
                                .unwrap()
                                .data
                                .get("tftp_status")
                                .cloned()
                        })
                        .unwrap_or(Value::Null);
                    let value = crate::api::router::projected_overview(&state, status)
                        .await
                        .unwrap_or_else(|error| serde_json::json!({"error":error.message}));
                    values.insert("overview".into(), value);
                }
                events.publish(values);
            }
        });
    }
}

pub async fn subscribe(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<BTreeMap<String, String>>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, crate::api::error::ApiError> {
    subscribe_scoped(state, headers, query, None).await
}

pub async fn subscribe_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<BTreeMap<String, String>>,
    axum::extract::Extension(crate::api::auth::CurrentUser(user)): axum::extract::Extension<
        crate::api::auth::CurrentUser,
    >,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, crate::api::error::ApiError> {
    subscribe_scoped(state, headers, query, Some(user.id)).await
}

async fn subscribe_scoped(
    state: AppState,
    headers: HeaderMap,
    query: BTreeMap<String, String>,
    account: Option<uuid::Uuid>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, crate::api::error::ApiError> {
    let events = state.admin_events.clone();
    let mut changed = events.changed.subscribe();
    // Subscribe before checking readiness; publication and its cursor share a lock.
    while !events.projection.lock().unwrap().initialized {
        if changed.changed().await.is_err() {
            break;
        }
    }
    let cursor = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let credential = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_owned)
        .or_else(|| query.get("access_token").cloned());
    let database = if state.config.read().await.edition.is_public() {
        state.database.clone()
    } else {
        None
    };
    let audit_filters =
        if account.is_none() && query.get("audit_view").is_some_and(|value| value == "1") {
            Some(crate::audit::Filters::from_stream(&query)?)
        } else {
            None
        };
    let guard = state.monitor.stream();
    let initial = Subscription {
        events,
        changed,
        cursor,
        queue: VecDeque::new(),
        database,
        credential,
        check_at: tokio::time::Instant::now(),
        finished: false,
        account,
        state,
        audit_filters,
        _guard: guard,
    };
    let stream = stream::unfold(initial, |mut subscription| async move {
        if subscription.finished {
            return None;
        }
        loop {
            if tokio::time::Instant::now() >= subscription.check_at {
                if let Some(db) = &subscription.database {
                    let valid = if let Some(token) = &subscription.credential {
                        db.authenticate_token(token)
                            .await
                            .ok()
                            .flatten()
                            .is_some_and(|user| {
                                subscription
                                    .account
                                    .map_or(user.is_admin, |id| user.id == id)
                            })
                    } else {
                        false
                    };
                    if !valid {
                        subscription.finished = true;
                        return Some((
                            Ok(Event::default()
                                .event("auth-expired")
                                .data("reauthenticate")),
                            subscription,
                        ));
                    }
                }
                subscription.check_at =
                    tokio::time::Instant::now() + std::time::Duration::from_secs(15);
            }
            if let Some(mut update) = subscription.queue.pop_front() {
                if let Some(user) = subscription.account {
                    let relevant = update.kind == "snapshot"
                        || ["users", "rentals", "rental_requests", "boards"]
                            .iter()
                            .any(|key| update.data.contains_key(*key));
                    update.data.clear();
                    if relevant {
                        match crate::api::account::projection(&subscription.state, user).await {
                            Ok(Value::Object(data)) => update.data.extend(data),
                            Ok(_) => unreachable!("object projection"),
                            Err(error) => {
                                update.data.insert(
                                    "profile".into(),
                                    serde_json::json!({"error":error.message}),
                                );
                            }
                        }
                    }
                } else if update.data.contains_key("audit")
                    && let Some(filters) = &subscription.audit_filters
                {
                    let data = crate::audit::page(&subscription.state, filters)
                        .await
                        .unwrap_or_else(|error| serde_json::json!({"error":error.message}));
                    update.data.insert("audit".into(), data);
                }
                let id = format!("{}:{}", update.epoch, update.revision);
                subscription.cursor = Some(id.clone());
                let event = Event::default()
                    .id(id)
                    .event(update.kind)
                    .json_data(update)
                    .expect("serializable projection");
                return Some((Ok(event), subscription));
            }
            subscription.queue = subscription.events.after(subscription.cursor.as_deref());
            if !subscription.queue.is_empty() {
                continue;
            }
            tokio::select! {result=subscription.changed.changed()=>{if result.is_err(){return None;}},_=tokio::time::sleep_until(subscription.check_at)=>{}}
        }
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

struct Subscription {
    events: AdminEvents,
    changed: watch::Receiver<u64>,
    cursor: Option<String>,
    queue: VecDeque<Update>,
    database: Option<crate::persistence::postgres::Database>,
    credential: Option<String>,
    check_at: tokio::time::Instant,
    finished: bool,
    account: Option<uuid::Uuid>,
    state: AppState,
    audit_filters: Option<crate::audit::Filters>,
    _guard: crate::dashboard::StreamGuard,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replay_gap_restart_and_snapshot_cursor_are_consistent() {
        let hub = AdminEvents::default();
        hub.publish(BTreeMap::from([("boards".into(), serde_json::json!([]))]));
        let first = hub.after(None).pop_front().unwrap();
        let cursor = format!("{}:{}", first.epoch, first.revision);
        assert!(hub.after(Some(&cursor)).is_empty());
        hub.publish(BTreeMap::from([(
            "boards".into(),
            serde_json::json!([{"id":"one"}]),
        )]));
        let replay = hub.after(Some(&cursor));
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].kind, "update");
        assert_eq!(replay[0].data["boards"][0]["id"], "one");
        for n in 0..=HISTORY_LIMIT {
            hub.publish(BTreeMap::from([("boards".into(), serde_json::json!([n]))]));
        }
        assert_eq!(hub.after(Some(&cursor))[0].kind, "snapshot");
        assert_eq!(hub.after(Some("other:1"))[0].kind, "snapshot");
        assert_eq!(hub.after(Some("malformed"))[0].kind, "snapshot");
    }
}
