//! Persistent, board-scoped axloader upgrade assignments.

use std::{collections::BTreeMap, sync::Arc};

use anyhow::{Context, ensure};
use httpboot_protocol::{LoaderOtaState, OtaOutcome, OtaSource};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

pub const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const MAX_DELIVERY_ATTEMPTS: u8 = 3;
const DELIVERY_RETRY_SECONDS: i64 = 5;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Image {
    pub sha256: String,
    pub size: u64,
    pub version: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Queued,
    Downloading,
    Staged,
    Confirming,
    Succeeded,
    RolledBack,
    Failed,
    Cancelled,
}

impl Phase {
    pub(crate) fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::RolledBack | Self::Failed | Self::Cancelled
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub board_id: String,
    pub mac_address: httpboot_protocol::MacAddress,
    pub update_id: String,
    pub image: Image,
    pub phase: Phase,
    pub error: Option<String>,
    #[serde(default)]
    pub delivery_attempts: u8,
    #[serde(default)]
    pub next_delivery_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub waiting_for_device_change: bool,
}

impl Job {
    pub fn management_view(mut self) -> Self {
        self.error = self.error.map(|_| "设备升级失败，请检查服务器日志".into());
        self
    }
}

#[derive(Clone, Debug)]
pub enum Decision {
    Idle,
    Update(Job),
    Confirm(String),
    Wait,
}

/// Delivery policy follows device HTTP results, never image architecture.
#[derive(Debug, thiserror::Error)]
pub enum DeliveryFailure {
    #[error("{0}")]
    Rejected(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Unavailable(String),
    #[error("{0}")]
    Retryable(String),
}

#[derive(Debug, thiserror::Error)]
pub enum DeleteImageError {
    #[error("invalid image digest")]
    InvalidDigest,
    #[error("OTA image not found")]
    NotFound,
    #[error("OTA image is referenced by an active assignment")]
    InUse,
    #[error("OTA database operation failed: {0}")]
    Database(#[from] sqlx::Error),
}

#[derive(Clone)]
pub struct OtaStore {
    database: crate::persistence::postgres::Database,
    image_catalog: Arc<Mutex<BTreeMap<String, Image>>>,
    jobs: Arc<Mutex<BTreeMap<String, Job>>>,
    observations: Arc<Mutex<BTreeMap<String, Observation>>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Observation {
    mac_address: httpboot_protocol::MacAddress,
    state: Option<LoaderOtaState>,
    boot_epoch: String,
}

impl OtaStore {
    pub async fn open(database: crate::persistence::postgres::Database) -> anyhow::Result<Self> {
        use sqlx::Row;
        let jobs = database
            .initialize_record("ota-jobs", &BTreeMap::<String, Job>::new())
            .await?;
        let observations = database
            .initialize_record("ota-observations", &BTreeMap::<String, Observation>::new())
            .await?;
        let rows = sqlx::query("SELECT sha256,metadata FROM ota_images ORDER BY sha256")
            .fetch_all(database.pool())
            .await?;
        let images = rows
            .into_iter()
            .map(|row| {
                Ok((
                    row.try_get::<String, _>("sha256")?,
                    serde_json::from_value::<Image>(row.try_get("metadata")?)?,
                ))
            })
            .collect::<anyhow::Result<BTreeMap<_, _>>>()?;
        Ok(Self {
            database,
            jobs: Arc::new(Mutex::new(jobs)),
            observations: Arc::new(Mutex::new(observations)),
            image_catalog: Arc::new(Mutex::new(images)),
        })
    }
    pub async fn image_bytes(&self, digest: &str) -> anyhow::Result<Vec<u8>> {
        ensure!(valid_digest(digest), "invalid image digest");
        sqlx::query_scalar("SELECT content FROM ota_images WHERE sha256=$1")
            .bind(digest)
            .fetch_optional(self.database.pool())
            .await?
            .context("OTA image not found")
    }

    pub async fn images(&self) -> anyhow::Result<Vec<Image>> {
        Ok(self.image_catalog.lock().await.values().cloned().collect())
    }

    pub async fn put_image(&self, bytes: &[u8], version: Option<String>) -> anyhow::Result<Image> {
        ensure!(
            !bytes.is_empty() && bytes.len() <= MAX_IMAGE_BYTES,
            "invalid EFI image size"
        );
        let bytes = bytes.to_vec();
        let db = &self.database;

        let mut images = self.image_catalog.lock().await;
        let validation_bytes = bytes.clone();
        let image = tokio::task::spawn_blocking(move || validate_image(&validation_bytes, version))
            .await??;
        sqlx::query("INSERT INTO ota_images (sha256,metadata,content) VALUES ($1,$2,$3) ON CONFLICT (sha256) DO UPDATE SET metadata=excluded.metadata")
                .bind(&image.sha256).bind(serde_json::to_value(&image)?).bind(bytes).execute(db.pool()).await?;
        images.insert(image.sha256.clone(), image.clone());
        Ok(image)
    }

    pub async fn delete_image(&self, digest: &str) -> Result<(), DeleteImageError> {
        if !valid_digest(digest) {
            return Err(DeleteImageError::InvalidDigest);
        }
        let mut images = self.image_catalog.lock().await;
        {
            let jobs = self.jobs.lock().await;
            if jobs
                .values()
                .any(|job| job.image.sha256 == digest && !job.phase.is_terminal())
            {
                return Err(DeleteImageError::InUse);
            }
        }
        let db = &self.database;

        let count = sqlx::query("DELETE FROM ota_images WHERE sha256=$1")
            .bind(digest)
            .execute(db.pool())
            .await?
            .rows_affected();
        images.remove(digest);
        if count == 0 {
            Err(DeleteImageError::NotFound)
        } else {
            Ok(())
        }
    }

    pub async fn jobs(&self) -> Vec<Job> {
        self.jobs.lock().await.values().cloned().collect()
    }

    pub async fn job(&self, board_id: &str) -> Option<Job> {
        self.jobs.lock().await.get(board_id).cloned()
    }

    pub async fn queue(
        &self,
        board_id: String,
        mac_address: httpboot_protocol::MacAddress,
        digest: &str,
    ) -> anyhow::Result<Job> {
        // Keep deletion and assignment mutually exclusive while the cached
        // catalog entry is promoted into a persistent job.
        let images = self.image_catalog.lock().await;
        let image = images.get(digest).context("missing OTA image")?.clone();
        let observations = self.observations.lock().await;
        let mut jobs = self.jobs.lock().await;
        ensure!(
            jobs.get(&board_id)
                .is_none_or(|job| job.phase.is_terminal()),
            "board already has an OTA job"
        );
        if let Some(device) = observations
            .get(&board_id)
            .filter(|device| device.mac_address == mac_address)
        {
            ensure!(
                device
                    .state
                    .as_ref()
                    .is_none_or(|state| !state.trial && state.pending_update_id.is_none()),
                "device has an unconfirmed OTA trial"
            );
            ensure!(
                device
                    .state
                    .as_ref()
                    .is_none_or(|state| state.active_sha256 != image.sha256),
                "this image is already the active loader"
            );
        }
        let job = Job {
            board_id: board_id.clone(),
            mac_address,
            update_id: uuid::Uuid::new_v4().to_string(),
            image,
            phase: Phase::Queued,
            error: None,
            delivery_attempts: 0,
            next_delivery_at: None,
            waiting_for_device_change: false,
        };
        let mut updated = jobs.clone();
        updated.insert(board_id, job.clone());
        self.save(&updated).await?;
        *jobs = updated;
        drop(observations);
        Ok(job)
    }

    pub async fn cancel(&self, board_id: &str, update_id: &str) -> anyhow::Result<Job> {
        let mut observations = self.observations.lock().await;
        let mut jobs = self.jobs.lock().await;
        let mut updated_jobs = jobs.clone();
        let job = updated_jobs
            .get_mut(board_id)
            .context("no OTA job for board")?;
        ensure!(
            job.update_id == update_id && !job.phase.is_terminal(),
            "OTA job already finished or superseded"
        );
        job.phase = Phase::Cancelled;
        job.error = None;
        job.next_delivery_at = None;
        job.waiting_for_device_change = false;
        let result = job.clone();

        let mut updated_observations = observations.clone();
        updated_observations.remove(board_id);
        {
            let db = &self.database;
            let mut tx = db.begin().await?;
            for (name, payload) in [
                (
                    "ota-observations",
                    serde_json::to_value(&updated_observations)?,
                ),
                ("ota-jobs", serde_json::to_value(&updated_jobs)?),
            ] {
                sqlx::query("UPDATE server_records SET payload=$2,updated_at=now() WHERE name=$1")
                    .bind(name)
                    .bind(payload)
                    .execute(&mut *tx)
                    .await?;
            }
            write_job_audit(&mut tx, jobs.get(board_id), &result).await?;
            tx.commit().await?;
        }
        *observations = updated_observations;
        *jobs = updated_jobs;
        Ok(result)
    }

    pub async fn decide(
        &self,
        board_id: &str,
        mac_address: httpboot_protocol::MacAddress,
        ota: Option<&LoaderOtaState>,
        board_idle: bool,
        boot_epoch: &str,
    ) -> anyhow::Result<Decision> {
        self.decide_at(
            board_id,
            mac_address,
            ota,
            board_idle,
            boot_epoch,
            chrono::Utc::now(),
        )
        .await
    }

    async fn decide_at(
        &self,
        board_id: &str,
        mac_address: httpboot_protocol::MacAddress,
        ota: Option<&LoaderOtaState>,
        board_idle: bool,
        boot_epoch: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<Decision> {
        let mut observations = self.observations.lock().await;
        let changed_observation = observations.get(board_id).is_none_or(|old| {
            old.mac_address != mac_address
                || old.boot_epoch != boot_epoch
                || old.state.as_ref() != ota
        });
        if changed_observation {
            let mut updated = observations.clone();
            updated.insert(
                board_id.into(),
                Observation {
                    mac_address,
                    state: ota.cloned(),
                    boot_epoch: boot_epoch.into(),
                },
            );
            self.save_observations(&updated).await?;
            *observations = updated;
        }
        let mut jobs = self.jobs.lock().await;
        let Some(previous) = jobs.get(board_id) else {
            return Ok(
                if ota.is_some_and(|ota| ota.trial || ota.pending_update_id.is_some()) {
                    Decision::Wait
                } else {
                    Decision::Idle
                },
            );
        };
        if previous.mac_address != mac_address {
            return Ok(Decision::Wait);
        }
        // Device reports from another boot must never revive a terminal assignment.
        if previous.phase.is_terminal() {
            return Ok(
                if ota.is_some_and(|ota| ota.trial || ota.pending_update_id.is_some()) {
                    Decision::Wait
                } else {
                    Decision::Idle
                },
            );
        }
        let mut updated = jobs.clone();
        let job = updated.get_mut(board_id).unwrap();
        let decision = if let Some(ota) = ota {
            if changed_observation {
                job.waiting_for_device_change = false;
            }
            if ota.last_update_id.as_deref() == Some(&job.update_id) {
                match ota.last_outcome {
                    Some(OtaOutcome::Confirmed)
                        if job.phase == Phase::Confirming
                            && ota.active_sha256 == job.image.sha256 =>
                    {
                        job.phase = Phase::Succeeded;
                    }
                    Some(OtaOutcome::RolledBack) => job.phase = Phase::RolledBack,
                    Some(OtaOutcome::Failed) => job.phase = Phase::Failed,
                    _ => {}
                }
            }
            if job.phase.is_terminal() {
                job.next_delivery_at = None;
                job.waiting_for_device_change = false;
                if ota.trial || ota.pending_update_id.is_some() {
                    Decision::Wait
                } else {
                    Decision::Idle
                }
            } else if ota.trial {
                if ota.source == Some(OtaSource::Server)
                    && ota.pending_update_id.as_deref() == Some(&job.update_id)
                    && ota.running_sha256 == job.image.sha256
                {
                    job.phase = Phase::Confirming;
                    job.error = None;
                    job.next_delivery_at = None;
                    job.waiting_for_device_change = false;
                    Decision::Confirm(job.update_id.clone())
                } else {
                    Decision::Wait
                }
            } else if ota.pending_update_id.is_some() {
                // Source is optional until firmware starts the pending slot.
                if ota.pending_update_id.as_deref() == Some(&job.update_id) {
                    job.phase = Phase::Staged;
                    job.error = None;
                    job.next_delivery_at = None;
                    job.waiting_for_device_change = false;
                }
                Decision::Wait
            } else if !board_idle {
                Decision::Idle
            } else if ota.active_sha256 == job.image.sha256 {
                job.phase = Phase::Failed;
                job.error =
                    Some("image already active without this assignment's confirmation".into());
                job.next_delivery_at = None;
                job.waiting_for_device_change = false;
                Decision::Idle
            } else if (job.waiting_for_device_change && !changed_observation)
                || job.next_delivery_at.is_some_and(|deadline| now < deadline)
            {
                Decision::Wait
            } else if matches!(job.phase, Phase::Queued | Phase::Downloading) {
                job.phase = Phase::Downloading;
                job.error = None;
                job.next_delivery_at = None;
                job.waiting_for_device_change = false;
                Decision::Update(job.clone())
            } else {
                Decision::Wait
            }
        } else {
            // Missing upgrade support must not block ordinary kernel HTTP Boot.
            if job.error.is_none() {
                job.error =
                    Some("loader did not report OTA state; waiting for OTA availability".into());
            }
            job.waiting_for_device_change = true;
            job.next_delivery_at = None;
            Decision::Idle
        };
        let current = jobs.get(board_id).unwrap();
        if job.phase != current.phase
            || job.error != current.error
            || job.next_delivery_at != current.next_delivery_at
            || job.waiting_for_device_change != current.waiting_for_device_change
        {
            self.save(&updated).await?;
            *jobs = updated;
        }
        Ok(decision)
    }

    pub async fn report(
        &self,
        board_id: &str,
        boot_epoch: &str,
        update_id: &str,
        phase: Phase,
        error: Option<String>,
        active_sha256: Option<&str>,
    ) -> anyhow::Result<()> {
        let observations = self.observations.lock().await;
        let mut jobs = self.jobs.lock().await;
        let mut updated = jobs.clone();
        let job = updated.get_mut(board_id).context("unknown OTA job")?;
        ensure!(job.update_id == update_id, "stale OTA report");
        let observed = observations
            .get(board_id)
            .context("OTA status not observed")?;
        ensure!(
            observed.mac_address == job.mac_address
                && observed.boot_epoch == boot_epoch
                && observed.state.is_some(),
            "stale OTA report boot epoch or capability"
        );
        ensure!(
            matches!(
                phase,
                Phase::Downloading | Phase::Staged | Phase::Failed | Phase::Succeeded
            ),
            "invalid loader OTA phase"
        );
        if job.phase.is_terminal() {
            ensure!(job.phase == phase, "OTA job already finished");
            return Ok(());
        }
        if job.phase == phase {
            if job.error != error {
                job.error = error;
                self.save(&updated).await?;
                *jobs = updated;
            }
            return Ok(());
        }
        ensure!(!job.phase.is_terminal(), "OTA job already finished");
        ensure!(
            matches!(
                (job.phase, phase),
                (
                    Phase::Queued | Phase::Downloading,
                    Phase::Downloading | Phase::Failed
                ) | (Phase::Downloading, Phase::Staged)
                    | (Phase::Staged | Phase::Confirming, Phase::Failed)
                    | (Phase::Confirming, Phase::Succeeded)
            ),
            "invalid OTA phase transition"
        );
        if phase == Phase::Succeeded {
            ensure!(
                observed.state.as_ref().is_some_and(|state| {
                    state.source == Some(OtaSource::Server)
                        && state.trial
                        && state.pending_update_id.as_deref() == Some(update_id)
                        && state.running_sha256 == job.image.sha256
                }) && active_sha256 == Some(job.image.sha256.as_str()),
                "confirmed OTA result does not match registered trial"
            );
        }
        if job.phase != phase || job.error != error {
            job.phase = phase;
            job.error = error;
            if phase != Phase::Downloading {
                job.next_delivery_at = None;
                job.waiting_for_device_change = false;
            }
            self.save(&updated).await?;
            *jobs = updated;
        }
        Ok(())
    }

    pub async fn record_delivery_failure(
        &self,
        board_id: &str,
        mac_address: httpboot_protocol::MacAddress,
        update_id: &str,
        boot_epoch: &str,
        failure: DeliveryFailure,
    ) -> anyhow::Result<Job> {
        let observations = self.observations.lock().await;
        ensure!(
            observations.get(board_id).is_some_and(|observed| {
                observed.mac_address == mac_address && observed.boot_epoch == boot_epoch
            }),
            "stale OTA delivery boot epoch"
        );
        let mut jobs = self.jobs.lock().await;
        let mut updated = jobs.clone();
        let job = updated.get_mut(board_id).context("unknown OTA job")?;
        ensure!(
            job.mac_address == mac_address && job.update_id == update_id,
            "stale OTA delivery failure"
        );
        if job.phase.is_terminal() {
            return Ok(job.clone());
        }
        ensure!(
            matches!(job.phase, Phase::Queued | Phase::Downloading),
            "OTA job is not accepting an image"
        );
        job.next_delivery_at = None;
        job.waiting_for_device_change = false;
        match failure {
            DeliveryFailure::Rejected(error) => {
                job.phase = Phase::Failed;
                job.error = Some(error);
            }
            DeliveryFailure::Conflict(error) => {
                job.phase = Phase::Downloading;
                job.error = Some(error);
                job.next_delivery_at =
                    Some(chrono::Utc::now() + chrono::Duration::seconds(DELIVERY_RETRY_SECONDS));
            }
            DeliveryFailure::Unavailable(error) => {
                job.phase = Phase::Downloading;
                job.error = Some(error);
                job.waiting_for_device_change = true;
            }
            DeliveryFailure::Retryable(error) => {
                job.delivery_attempts = job.delivery_attempts.saturating_add(1);
                job.error = Some(error);
                if job.delivery_attempts >= MAX_DELIVERY_ATTEMPTS {
                    job.phase = Phase::Failed;
                } else {
                    job.phase = Phase::Downloading;
                    job.next_delivery_at = Some(
                        chrono::Utc::now()
                            + chrono::Duration::seconds(
                                DELIVERY_RETRY_SECONDS * i64::from(job.delivery_attempts),
                            ),
                    );
                }
            }
        }
        let result = job.clone();
        self.save(&updated).await?;
        *jobs = updated;
        Ok(result)
    }

    async fn save(&self, jobs: &BTreeMap<String, Job>) -> anyhow::Result<()> {
        let db = &self.database;

        let mut tx = db.begin().await?;
        let old: Option<serde_json::Value> = sqlx::query_scalar(
            "SELECT payload FROM server_records WHERE name='ota-jobs' FOR UPDATE",
        )
        .fetch_optional(&mut *tx)
        .await?;
        let old = old
            .map(serde_json::from_value::<BTreeMap<String, Job>>)
            .transpose()?
            .unwrap_or_default();
        for (key, job) in jobs {
            write_job_audit(&mut tx, old.get(key), job).await?;
        }
        sqlx::query("INSERT INTO server_records(name,payload) VALUES ('ota-jobs',$1) ON CONFLICT(name) DO UPDATE SET payload=excluded.payload,updated_at=now()").bind(serde_json::to_value(jobs)?).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn save_observations(
        &self,
        observations: &BTreeMap<String, Observation>,
    ) -> anyhow::Result<()> {
        let db = &self.database;

        db.save_record("ota-observations", observations).await
    }
}

async fn write_job_audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    old: Option<&Job>,
    job: &Job,
) -> anyhow::Result<()> {
    let transition = old.is_none_or(|old| old.phase != job.phase || old.update_id != job.update_id);
    let failed_delivery = old.is_some_and(|old| job.delivery_attempts > old.delivery_attempts);
    if !transition && !failed_delivery {
        return Ok(());
    }
    let action = if failed_delivery {
        "ota.delivery_failed"
    } else {
        "ota.phase_changed"
    };
    let result = if failed_delivery || matches!(job.phase, Phase::Failed | Phase::RolledBack) {
        "failed"
    } else if job.phase.is_terminal() {
        "success"
    } else {
        "accepted"
    };
    let id = uuid::Uuid::new_v4();
    let details = serde_json::json!({"board_id":job.board_id,"update_id":job.update_id,"previous_phase":old.map(|old|old.phase),"phase":job.phase,"image_sha256":job.image.sha256,"delivery_attempts":job.delivery_attempts,"has_error":job.error.is_some()});
    sqlx::query("INSERT INTO audit_events(id,request_id,finished_at,actor_name,actor_kind,source,action,summary,resource,target_id,result,duration_ms,details) VALUES ($1,$1,now(),'系统','system','system',$2,$3,'ota',$4,$5,0,$6)").bind(id).bind(action).bind(if failed_delivery{"OTA 镜像投递失败"}else{"OTA 任务状态变化"}).bind(&job.update_id).bind(result).bind(details).execute(&mut **tx).await?;
    Ok(())
}

fn validate_image(bytes: &[u8], version: Option<String>) -> anyhow::Result<Image> {
    if let Some(label) = &version {
        ensure!(
            label.len() <= 96 && label.bytes().all(|byte| byte.is_ascii_graphic()),
            "invalid image version"
        );
    }
    let sha256 = format!("{:x}", Sha256::digest(bytes));
    let image = Image {
        sha256: sha256.clone(),
        size: bytes.len() as u64,
        version,
    };
    Ok(image)
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn test_store(path: &std::path::Path) -> OtaStore {
        OtaStore::open(crate::test_support::database(path).await.unwrap())
            .await
            .unwrap()
    }
    use httpboot_protocol::{LoaderOtaState, OtaOutcome, OtaSource};

    fn efi_image() -> Vec<u8> {
        let mut bytes = vec![0; 512];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&(0x80_u32).to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&[0x64, 0x86]);
        bytes[0x98..0x9a].copy_from_slice(&[0x0b, 0x02]);
        bytes[0xdc..0xde].copy_from_slice(&[10, 0]);
        bytes
    }

    #[tokio::test]
    async fn duplicate_phase_reports_are_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path()).await;
        let image = store
            .put_image(&efi_image(), Some("first".into()))
            .await
            .unwrap();
        let mac = "02:00:00:00:00:01".parse().unwrap();
        let job = store
            .queue("board-1".into(), mac, &image.sha256)
            .await
            .unwrap();

        let initial = LoaderOtaState {
            active_sha256: "11".repeat(32),
            running_sha256: "11".repeat(32),
            pending_update_id: None,
            trial: false,
            source: None,
            last_update_id: None,
            last_outcome: None,
        };
        store
            .decide("board-1", mac, Some(&initial), false, "epoch-1")
            .await
            .unwrap();

        for phase in [
            Phase::Downloading,
            Phase::Downloading,
            Phase::Staged,
            Phase::Staged,
            Phase::Failed,
            Phase::Failed,
        ] {
            store
                .report(
                    "board-1",
                    "epoch-1",
                    &job.update_id,
                    phase,
                    (phase == Phase::Failed).then(|| "device rejected image".into()),
                    None,
                )
                .await
                .unwrap();
        }

        store
            .report(
                "board-1",
                "epoch-1",
                &job.update_id,
                Phase::Failed,
                Some("late duplicate must not overwrite the refusal".into()),
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            store.job("board-1").await.unwrap().error.as_deref(),
            Some("device rejected image")
        );

        let mut replacement = efi_image();
        replacement.push(1);
        let image = store
            .put_image(&replacement, Some("second".into()))
            .await
            .unwrap();
        let job = store
            .queue("board-1".into(), mac, &image.sha256)
            .await
            .unwrap();
        let trial = LoaderOtaState {
            active_sha256: "11".repeat(32),
            running_sha256: image.sha256.clone(),
            pending_update_id: Some(job.update_id.clone()),
            trial: true,
            source: Some(OtaSource::Server),
            last_update_id: None,
            last_outcome: None,
        };
        assert!(matches!(
            store.decide("board-1", mac, Some(&trial), true, "epoch-1").await.unwrap(),
            Decision::Confirm(id) if id == job.update_id
        ));
        for _ in 0..2 {
            store
                .report(
                    "board-1",
                    "epoch-1",
                    &job.update_id,
                    Phase::Succeeded,
                    None,
                    Some(&image.sha256),
                )
                .await
                .unwrap();
        }
        assert_eq!(store.job("board-1").await.unwrap().phase, Phase::Succeeded);
    }

    #[tokio::test]
    async fn delivery_failures_are_persisted_and_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path()).await;
        let image = store.put_image(&efi_image(), None).await.unwrap();
        let mac = "02:00:00:00:00:01".parse().unwrap();
        let job = store
            .queue("board-1".into(), mac, &image.sha256)
            .await
            .unwrap();

        let idle = LoaderOtaState {
            active_sha256: "11".repeat(32),
            running_sha256: "11".repeat(32),
            pending_update_id: None,
            trial: false,
            source: None,
            last_update_id: None,
            last_outcome: None,
        };
        store
            .decide("board-1", mac, Some(&idle), true, "epoch-1")
            .await
            .unwrap();
        store
            .record_delivery_failure(
                "board-1",
                mac,
                &job.update_id,
                "epoch-1",
                DeliveryFailure::Unavailable("device HTTP 503: no OTA storage".into()),
            )
            .await
            .unwrap();
        drop(store);
        let store = test_store(dir.path()).await;
        assert!(matches!(
            store
                .decide("board-1", mac, Some(&idle), true, "epoch-1")
                .await
                .unwrap(),
            Decision::Wait
        ));
        assert!(matches!(
            store
                .decide_at(
                    "board-1",
                    mac,
                    Some(&idle),
                    true,
                    "epoch-1",
                    chrono::Utc::now() + chrono::Duration::hours(1)
                )
                .await
                .unwrap(),
            Decision::Wait
        ));
        // Observe recovery while a normal kernel session owns the board, then upgrade when idle.
        assert!(matches!(
            store
                .decide("board-1", mac, Some(&idle), false, "epoch-2")
                .await
                .unwrap(),
            Decision::Idle
        ));
        assert!(matches!(
            store
                .decide("board-1", mac, Some(&idle), true, "epoch-2")
                .await
                .unwrap(),
            Decision::Update(_)
        ));
        store
            .record_delivery_failure(
                "board-1",
                mac,
                &job.update_id,
                "epoch-2",
                DeliveryFailure::Conflict("device HTTP 409: temporarily busy".into()),
            )
            .await
            .unwrap();
        let conflict = store.job("board-1").await.unwrap();
        assert_eq!(conflict.delivery_attempts, 0);
        let deadline = conflict.next_delivery_at.unwrap();
        assert!(matches!(
            store
                .decide_at(
                    "board-1",
                    mac,
                    Some(&idle),
                    true,
                    "epoch-2",
                    deadline - chrono::Duration::microseconds(1)
                )
                .await
                .unwrap(),
            Decision::Wait
        ));
        assert!(matches!(
            store
                .decide_at("board-1", mac, Some(&idle), true, "epoch-2", deadline)
                .await
                .unwrap(),
            Decision::Update(_)
        ));
        let first = store
            .record_delivery_failure(
                "board-1",
                mac,
                &job.update_id,
                "epoch-2",
                DeliveryFailure::Retryable("device HTTP 500 Internal Server Error".into()),
            )
            .await
            .unwrap();
        assert_eq!(first.phase, Phase::Downloading);
        assert_eq!(first.delivery_attempts, 1);
        drop(store);

        let store = test_store(dir.path()).await;
        for attempt in 2..=MAX_DELIVERY_ATTEMPTS {
            let updated = store
                .record_delivery_failure(
                    "board-1",
                    mac,
                    &job.update_id,
                    "epoch-2",
                    DeliveryFailure::Retryable(format!("delivery attempt {attempt} failed")),
                )
                .await
                .unwrap();
            assert_eq!(updated.delivery_attempts, attempt);
        }
        let failed = store.job("board-1").await.unwrap();
        assert_eq!(failed.phase, Phase::Failed);
        assert_eq!(failed.error.as_deref(), Some("delivery attempt 3 failed"));
        let late = LoaderOtaState {
            active_sha256: "11".repeat(32),
            running_sha256: "11".repeat(32),
            pending_update_id: None,
            trial: false,
            source: None,
            last_update_id: Some(job.update_id.clone()),
            last_outcome: Some(OtaOutcome::RolledBack),
        };
        store
            .decide("board-1", mac, Some(&late), true, "epoch-3")
            .await
            .unwrap();
        assert_eq!(store.job("board-1").await.unwrap().phase, Phase::Failed);
        assert_eq!(store.job("board-1").await.unwrap().error, failed.error);
        let idle = LoaderOtaState {
            active_sha256: "11".repeat(32),
            running_sha256: "11".repeat(32),
            pending_update_id: None,
            trial: false,
            source: None,
            last_update_id: None,
            last_outcome: None,
        };
        assert!(matches!(
            store
                .decide("board-1", mac, Some(&idle), true, "epoch-2")
                .await
                .unwrap(),
            Decision::Idle
        ));
    }

    #[tokio::test]
    async fn update_survives_restart_and_confirms_its_own_trial_while_board_is_busy() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path()).await;
        let image = store
            .put_image(&efi_image(), Some("test".into()))
            .await
            .unwrap();
        let mac = "02:00:00:00:00:01".parse().unwrap();
        let job = store
            .queue("board-1".into(), mac, &image.sha256)
            .await
            .unwrap();
        assert!(
            store
                .queue("board-1".into(), mac, &image.sha256)
                .await
                .is_err()
        );
        let mut legacy: serde_json::Value =
            store.database.record("ota-jobs").await.unwrap().unwrap();
        let old_job = legacy["board-1"].as_object_mut().unwrap();
        old_job.remove("next_delivery_at");
        old_job.remove("waiting_for_device_change");
        store
            .database
            .save_record("ota-jobs", &legacy)
            .await
            .unwrap();
        let legacy_ota = LoaderOtaState {
            active_sha256: "11".repeat(32),
            running_sha256: "11".repeat(32),
            pending_update_id: None,
            trial: false,
            source: None,
            last_update_id: None,
            last_outcome: None,
        };
        store
            .database
            .save_record(
                "ota-observations",
                &serde_json::json!({
                    "board-1": { "mac_address": mac, "state": legacy_ota, "boot_epoch": "epoch-1" }
                }),
            )
            .await
            .unwrap();
        drop(store);
        let store = test_store(dir.path()).await;
        let mut state = LoaderOtaState {
            active_sha256: "11".repeat(32),
            running_sha256: "11".repeat(32),
            pending_update_id: None,
            trial: false,
            source: None,
            last_update_id: None,
            last_outcome: None,
        };
        assert!(matches!(
            store
                .decide("board-1", mac, Some(&state), false, "epoch-1")
                .await
                .unwrap(),
            Decision::Idle
        ));
        assert!(matches!(
            store
                .decide("board-1", mac, Some(&state), true, "epoch-1")
                .await
                .unwrap(),
            Decision::Update(_)
        ));
        state.pending_update_id = Some(job.update_id.clone());
        state.trial = true;
        state.running_sha256 = image.sha256.clone();
        state.source = Some(OtaSource::Direct);
        assert!(matches!(
            store
                .decide("board-1", mac, Some(&state), true, "epoch-1")
                .await
                .unwrap(),
            Decision::Wait
        ));
        state.source = Some(OtaSource::Server);
        assert!(
            matches!(store.decide("board-1", mac, Some(&state), false, "epoch-1").await.unwrap(), Decision::Confirm(id) if id == job.update_id)
        );
        drop(store);
        let store = test_store(dir.path()).await;
        assert_eq!(store.job("board-1").await.unwrap().phase, Phase::Confirming);
        // The prior assignment can finish while a newer direct trial is already running.
        state.trial = true;
        state.pending_update_id = Some("01234567-89ab-cdef-0123-456789abcdef".into());
        state.source = Some(OtaSource::Direct);
        state.active_sha256 = image.sha256;
        state.running_sha256 = "22".repeat(32);
        state.last_update_id = Some(job.update_id);
        state.last_outcome = Some(OtaOutcome::Confirmed);
        assert!(matches!(
            store
                .decide("board-1", mac, Some(&state), true, "epoch-1")
                .await
                .unwrap(),
            Decision::Wait
        ));
        assert_eq!(store.job("board-1").await.unwrap().phase, Phase::Succeeded);
        let mut newer = efi_image();
        newer.push(1);
        let candidate = store.put_image(&newer, None).await.unwrap();
        state.trial = true;
        state.pending_update_id = Some("01234567-89ab-cdef-0123-456789abcdef".into());
        state.source = Some(OtaSource::Direct);
        assert!(matches!(
            store
                .decide("board-1", mac, Some(&state), true, "epoch-1")
                .await
                .unwrap(),
            Decision::Wait
        ));
        drop(store);
        let store = test_store(dir.path()).await;
        assert!(
            store
                .queue("board-1".into(), mac, &candidate.sha256)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn active_update_can_be_cancelled_and_replaced_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path()).await;
        let image = store
            .put_image(&efi_image(), Some("first".into()))
            .await
            .unwrap();
        let mac = "02:00:00:00:00:01".parse().unwrap();
        let job = store
            .queue("board-1".into(), mac, &image.sha256)
            .await
            .unwrap();
        let state = LoaderOtaState {
            active_sha256: "11".repeat(32),
            running_sha256: "11".repeat(32),
            pending_update_id: None,
            trial: false,
            source: None,
            last_update_id: None,
            last_outcome: None,
        };
        assert!(matches!(
            store
                .decide("board-1", mac, Some(&state), true, "epoch-1")
                .await
                .unwrap(),
            Decision::Update(_)
        ));
        assert_eq!(
            store.job("board-1").await.unwrap().phase,
            Phase::Downloading
        );

        let cancelled = store.cancel("board-1", &job.update_id).await.unwrap();
        assert_eq!(cancelled.phase, Phase::Cancelled);
        assert!(!store.observations.lock().await.contains_key("board-1"));
        assert!(store.cancel("board-1", &job.update_id).await.is_err());

        drop(store);
        let store = test_store(dir.path()).await;
        assert_eq!(store.job("board-1").await.unwrap().phase, Phase::Cancelled);
        assert!(!store.observations.lock().await.contains_key("board-1"));
        let mut replacement = efi_image();
        replacement.push(1);
        let replacement = store
            .put_image(&replacement, Some("second".into()))
            .await
            .unwrap();
        assert!(
            store
                .queue("board-1".into(), mac, &replacement.sha256)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn image_delete_rejects_active_assignment_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path()).await;
        let image = store
            .put_image(&efi_image(), Some("delete-me".into()))
            .await
            .unwrap();
        assert_eq!(store.images().await.unwrap()[0].sha256, image.sha256);
        let mac = "02:00:00:00:00:01".parse().unwrap();
        let job = store
            .queue("board-1".into(), mac, &image.sha256)
            .await
            .unwrap();
        let digest = image.sha256.clone();
        assert!(store.image_bytes(&digest).await.is_ok());

        assert!(matches!(
            store.delete_image(&image.sha256).await,
            Err(DeleteImageError::InUse)
        ));
        store.cancel("board-1", &job.update_id).await.unwrap();
        store.delete_image(&image.sha256).await.unwrap();
        assert!(store.images().await.unwrap().is_empty());
        assert!(store.image_bytes(&digest).await.is_err());

        let retained = store.job("board-1").await.unwrap();
        assert_eq!(retained.image.sha256, image.sha256);
        assert_eq!(retained.image.size, image.size);
        assert_eq!(retained.image.version, image.version);
        assert!(matches!(
            store.delete_image(&image.sha256).await,
            Err(DeleteImageError::NotFound)
        ));

        store
            .put_image(&efi_image(), Some("delete-me".into()))
            .await
            .unwrap();

        store.delete_image(&image.sha256).await.unwrap();
        assert!(store.image_bytes(&digest).await.is_err());

        drop(store);
        let store = test_store(dir.path()).await;
        assert!(store.images().await.unwrap().is_empty());
        let retained = store.job("board-1").await.unwrap();
        assert_eq!(retained.phase, Phase::Cancelled);
        assert_eq!(retained.image.sha256, image.sha256);
        assert_eq!(retained.image.size, image.size);
        assert_eq!(retained.image.version, image.version);
        assert!(store.image_bytes(&digest).await.is_err());
    }
}
