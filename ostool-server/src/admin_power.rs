//! Unbound, explicit power commands. No board/session is created by this API.
use crate::{
    AppState, BoardLeaseState, PowerManagementConfig,
    api::error::ApiError,
    power::{PowerAction, execute_power_action},
};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PowerRequest {
    pub request_id: String,
    pub action: Action,
    pub power_management: PowerManagementConfig,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    On,
    Off,
}
#[derive(Clone, Debug, Serialize)]
pub struct PowerResult {
    pub id: String,
    pub action: Action,
    pub state: &'static str,
    pub message: Option<String>,
}
#[derive(Default)]
struct Actions {
    records: BTreeMap<String, (PowerRequest, PowerResult)>,
    busy: BTreeSet<String>,
}
#[derive(Clone, Default)]
pub struct AdminPower(Arc<Mutex<Actions>>);

// A stable key plus its resolved device path catches different stable aliases
// referring to the same relay. Custom commands are identifiable only by config.
fn keys(config: &PowerManagementConfig) -> BTreeSet<String> {
    let mut result =
        BTreeSet::from([serde_json::to_string(config).expect("power config serializes")]);
    if let PowerManagementConfig::Custom(custom) = config {
        for command in [&custom.power_on_cmd, &custom.power_off_cmd] {
            if !command.trim().is_empty() {
                result.insert(format!("command:{}", command.trim()));
            }
        }
    }
    if let PowerManagementConfig::ZhongshengRelay(relay) = config
        && let Ok(serial) = crate::serial::discovery::resolve_serial_key(&relay.key)
    {
        result.insert(format!("relay:{}", serial.current_device_path));
    }
    result
}
impl AdminPower {
    pub fn is_busy(&self, config: &PowerManagementConfig) -> bool {
        !self.0.lock().unwrap().busy.is_disjoint(&keys(config))
    }
    pub fn snapshots(&self) -> Vec<PowerResult> {
        self.0
            .lock()
            .unwrap()
            .records
            .values()
            .map(|(_, r)| r.clone())
            .collect()
    }
}
pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<PowerResult>, ApiError> {
    state
        .admin_power
        .0
        .lock()
        .unwrap()
        .records
        .get(&id)
        .map(|(_, r)| Json(r.clone()))
        .ok_or_else(|| ApiError::not_found("power action not found"))
}
pub async fn create(
    State(state): State<AppState>,
    Json(request): Json<PowerRequest>,
) -> Result<(StatusCode, Json<PowerResult>), ApiError> {
    if request.request_id.trim().is_empty() || request.request_id.len() > 128 {
        return Err(ApiError::bad_request(
            "request_id must contain 1–128 characters",
        ));
    }
    let _inventory = state.board_inventory_gate.lock().await;
    {
        let actions = state.admin_power.0.lock().unwrap();
        if let Some((previous, result)) = actions.records.get(&request.request_id) {
            if serde_json::to_value(previous).unwrap() != serde_json::to_value(&request).unwrap() {
                return Err(ApiError::conflict(
                    "request_id was already used for a different action",
                ));
            }
            return Ok((StatusCode::ACCEPTED, Json(result.clone())));
        }
        // Never forget an idempotency key within a server epoch. Bounded storage
        // rejects new work explicitly instead of silently replaying old commands.
        if actions.records.len() >= 4096 {
            return Err(ApiError::service_unavailable(
                "power action history is full; restart during a maintenance window",
            ));
        }
    }
    match &request.power_management {
        PowerManagementConfig::Custom(config) => {
            let command = match request.action {
                Action::On => &config.power_on_cmd,
                Action::Off => &config.power_off_cmd,
            };
            if command.trim().is_empty() {
                return Err(ApiError::bad_request("power command is empty"));
            }
        }
        PowerManagementConfig::ZhongshengRelay(relay) => {
            crate::serial::discovery::resolve_serial_key(&relay.key)
                .map_err(|e| ApiError::bad_request(e.to_string()))?;
        }
        PowerManagementConfig::Qemu { virtual_device_id } => {
            if state
                .virtual_boards
                .snapshot(virtual_device_id)
                .await
                .is_none()
            {
                return Err(ApiError::not_found("virtual device not found"));
            }
        }
    }
    let resource_keys = keys(&request.power_management);
    let boards = state.boards.read().await;
    let runtimes = state.board_runtimes.read().await;
    for board in boards.values() {
        if !resource_keys.is_disjoint(&keys(&board.power_management))
            && runtimes
                .get(&board.id)
                .is_some_and(|r| r.lease_state != BoardLeaseState::Idle)
        {
            return Err(ApiError::conflict(format!(
                "power resource belongs to busy board `{}`",
                board.id
            )));
        }
    }
    drop(runtimes);
    drop(boards);
    let result = PowerResult {
        id: request.request_id.clone(),
        action: request.action,
        state: "running",
        message: None,
    };
    {
        let mut actions = state.admin_power.0.lock().unwrap();
        if !actions.busy.is_disjoint(&resource_keys) {
            return Err(ApiError::conflict("power resource has a running action"));
        }
        actions.busy.extend(resource_keys.clone());
        actions.records.insert(
            request.request_id.clone(),
            (request.clone(), result.clone()),
        );
    }
    state.admin_events.invalidate(&["power_actions"]);
    let job_state = state.clone();
    tokio::spawn(async move {
        let outcome = run(&job_state, &request).await;
        {
            let mut actions = job_state.admin_power.0.lock().unwrap();
            let (_, result) = actions
                .records
                .get_mut(&request.request_id)
                .expect("reserved action exists");
            result.state = if outcome.is_ok() {
                "succeeded"
            } else {
                "failed"
            };
            result.message = Some(match outcome {
                Ok(message) => message,
                Err(error) => format!("{error:#}"),
            });
            for key in resource_keys {
                actions.busy.remove(&key);
            }
        }
        job_state
            .admin_events
            .invalidate(&["power_actions", "virtual"]);
    });
    Ok((StatusCode::ACCEPTED, Json(result)))
}
async fn run(state: &AppState, request: &PowerRequest) -> anyhow::Result<String> {
    let action = match request.action {
        Action::On => PowerAction::On,
        Action::Off => PowerAction::Off,
    };
    let _serial_reservation = state
        .reserve_power_serial(&request.power_management)
        .await?;
    match &request.power_management {
        #[cfg(target_os = "linux")]
        PowerManagementConfig::Custom(config) => {
            let command = match action {
                PowerAction::On => &config.power_on_cmd,
                PowerAction::Off => &config.power_off_cmd,
            };
            crate::process::run_admin_shell_command(command).await?;
            Ok("power command completed".into())
        }
        PowerManagementConfig::Qemu { virtual_device_id } => match action {
            PowerAction::On => {
                let device = state
                    .virtual_boards
                    .snapshot(virtual_device_id)
                    .await
                    .ok_or_else(|| anyhow::anyhow!("virtual device disappeared"))?;
                state
                    .virtual_boards
                    .power_on(virtual_device_id, device.mac_address)
                    .await
            }
            PowerAction::Off => state.virtual_boards.power_off(virtual_device_id).await,
        },
        _ => execute_power_action(&request.power_management, action)
            .await
            .map_err(Into::into),
    }
}
