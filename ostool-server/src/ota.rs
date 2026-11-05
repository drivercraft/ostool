//! Persistent, board-scoped axloader upgrade assignments.

use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

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
    #[error("OTA image storage is inconsistent")]
    Inconsistent,
    #[error("failed to update OTA image storage: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone)]
pub struct OtaStore {
    root: PathBuf,
    image_catalog: Arc<Mutex<BTreeMap<String, Image>>>,
    jobs: Arc<Mutex<BTreeMap<String, Job>>>,
    observations: Arc<Mutex<BTreeMap<String, Observation>>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Observation {
    mac_address: httpboot_protocol::MacAddress,
    state: Option<LoaderOtaState>,
    #[serde(default)]
    epoch: String,
}

impl OtaStore {
    pub fn open(data_dir: &Path) -> anyhow::Result<Self> {
        let root = data_dir.join("loader-ota");
        fs::create_dir_all(root.join("images"))?;
        let jobs = match fs::read(root.join("jobs.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("invalid persistent OTA jobs")?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error.into()),
        };
        let observations = match fs::read(root.join("devices.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .context("invalid persistent OTA device observations")?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error.into()),
        };
        let images = read_images(&root)?
            .into_iter()
            .map(|image| (image.sha256.clone(), image))
            .collect();
        Ok(Self {
            root,
            image_catalog: Arc::new(Mutex::new(images)),
            jobs: Arc::new(Mutex::new(jobs)),
            observations: Arc::new(Mutex::new(observations)),
        })
    }

    pub fn image_path(&self, digest: &str) -> anyhow::Result<PathBuf> {
        ensure!(valid_digest(digest), "invalid image digest");
        Ok(self.root.join("images").join(format!("{digest}.efi")))
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
        let root = self.root.clone();
        let mut images = self.image_catalog.lock().await;
        let image = tokio::task::spawn_blocking(move || write_image(&root, &bytes, version))
            .await
            .context("OTA image write task failed")??;
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
        let root = self.root.clone();
        let digest = digest.to_owned();
        let remove_digest = digest.clone();
        let (result, still_valid) =
            tokio::task::spawn_blocking(move || remove_image(&root, &remove_digest))
                .await
                .map_err(|_| DeleteImageError::Inconsistent)?;
        if !still_valid {
            images.remove(digest.as_str());
        }
        result
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
        self.save(&updated)?;
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
        self.save_observations(&updated_observations)?;
        self.save(&updated_jobs)?;
        *observations = updated_observations;
        *jobs = updated_jobs;
        Ok(result)
    }

    pub async fn can_download(
        &self,
        board_id: &str,
        mac: httpboot_protocol::MacAddress,
        update_id: &str,
        epoch: &str,
    ) -> bool {
        let observations = self.observations.lock().await;
        let eligible = observations.get(board_id).is_some_and(|observation| {
            observation.mac_address == mac
                && observation.epoch == epoch
                && observation
                    .state
                    .as_ref()
                    .is_some_and(|ota| !ota.trial && ota.pending_update_id.is_none())
        });
        eligible
            && self.jobs.lock().await.get(board_id).is_some_and(|job| {
                job.mac_address == mac
                    && job.update_id == update_id
                    && job.phase == Phase::Downloading
            })
    }

    pub async fn decide(
        &self,
        board_id: &str,
        mac_address: httpboot_protocol::MacAddress,
        ota: Option<&LoaderOtaState>,
        board_idle: bool,
        epoch: &str,
    ) -> anyhow::Result<Decision> {
        self.decide_at(
            board_id,
            mac_address,
            ota,
            board_idle,
            epoch,
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
        epoch: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<Decision> {
        let mut observations = self.observations.lock().await;
        let changed_observation = observations.get(board_id).is_none_or(|old| {
            old.mac_address != mac_address || old.epoch != epoch || old.state.as_ref() != ota
        });
        if changed_observation {
            let mut updated = observations.clone();
            updated.insert(
                board_id.into(),
                Observation {
                    mac_address,
                    state: ota.cloned(),
                    epoch: epoch.into(),
                },
            );
            self.save_observations(&updated)?;
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
            self.save(&updated)?;
            *jobs = updated;
        }
        Ok(decision)
    }

    pub async fn report(
        &self,
        board_id: &str,
        epoch: &str,
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
                && observed.epoch == epoch
                && observed.state.is_some(),
            "stale OTA report epoch or capability"
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
                self.save(&updated)?;
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
            self.save(&updated)?;
            *jobs = updated;
        }
        Ok(())
    }

    pub async fn record_delivery_failure(
        &self,
        board_id: &str,
        mac_address: httpboot_protocol::MacAddress,
        update_id: &str,
        epoch: &str,
        failure: DeliveryFailure,
    ) -> anyhow::Result<Job> {
        let observations = self.observations.lock().await;
        ensure!(
            observations.get(board_id).is_some_and(|observed| {
                observed.mac_address == mac_address && observed.epoch == epoch
            }),
            "stale OTA delivery epoch"
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
        self.save(&updated)?;
        *jobs = updated;
        Ok(result)
    }

    fn save(&self, jobs: &BTreeMap<String, Job>) -> anyhow::Result<()> {
        atomic_write(&self.root.join("jobs.json"), &serde_json::to_vec(jobs)?)
    }

    fn save_observations(
        &self,
        observations: &BTreeMap<String, Observation>,
    ) -> anyhow::Result<()> {
        atomic_write(
            &self.root.join("devices.json"),
            &serde_json::to_vec(observations)?,
        )
    }
}

fn image_path(root: &Path, digest: &str) -> PathBuf {
    root.join("images").join(format!("{digest}.efi"))
}

fn read_image(root: &Path, digest: &str) -> anyhow::Result<Image> {
    ensure!(valid_digest(digest), "invalid image digest");
    let bytes = fs::read(root.join("images").join(format!("{digest}.json")))?;
    let image: Image = serde_json::from_slice(&bytes)?;
    ensure!(
        image.sha256 == digest && image_path(root, digest).is_file(),
        "missing OTA image"
    );
    Ok(image)
}

fn read_images(root: &Path) -> anyhow::Result<Vec<Image>> {
    let mut images = Vec::new();
    for entry in fs::read_dir(root.join("images"))? {
        let entry = entry?;
        if let Some(name) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.strip_suffix(".json"))
        {
            match read_image(root, name) {
                Ok(image) => images.push(image),
                Err(error) => {
                    log::warn!("ignoring inconsistent OTA image metadata `{name}`: {error:#}")
                }
            }
        }
    }
    images.sort_by(|a, b| a.sha256.cmp(&b.sha256));
    Ok(images)
}

fn write_image(root: &Path, bytes: &[u8], version: Option<String>) -> anyhow::Result<Image> {
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
    let path = image_path(root, &sha256);
    if !path.exists() {
        atomic_write(&path, bytes)?;
    } else {
        ensure!(
            format!("{:x}", Sha256::digest(fs::read(&path)?)) == sha256,
            "existing image digest mismatch"
        );
    }
    atomic_write(
        &root.join("images").join(format!("{sha256}.json")),
        &serde_json::to_vec(&image)?,
    )?;
    Ok(image)
}

fn remove_image(root: &Path, digest: &str) -> (Result<(), DeleteImageError>, bool) {
    let result = remove_image_files(root, digest);
    let still_valid = read_image(root, digest).is_ok();
    (result, still_valid)
}

fn remove_image_files(root: &Path, digest: &str) -> Result<(), DeleteImageError> {
    let directory = root.join("images");
    let metadata = directory.join(format!("{digest}.json"));
    let image = image_path(root, digest);
    if !metadata.exists() && !image.exists() {
        return Err(DeleteImageError::NotFound);
    }
    if metadata.is_file() {
        fs::remove_file(&metadata)?;
        fs::File::open(&directory)?.sync_all()?;
    } else if metadata.exists() {
        return Err(DeleteImageError::Inconsistent);
    }
    if image.is_file() {
        fs::remove_file(&image)?;
        fs::File::open(&directory)?.sync_all()?;
    } else if image.exists() {
        return Err(DeleteImageError::Inconsistent);
    }
    Ok(())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    let result = (|| -> anyhow::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        fs::File::open(path.parent().context("OTA path without parent")?)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let store = OtaStore::open(dir.path()).unwrap();
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
        let store = OtaStore::open(dir.path()).unwrap();
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
        let store = OtaStore::open(dir.path()).unwrap();
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

        let store = OtaStore::open(dir.path()).unwrap();
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
        let store = OtaStore::open(dir.path()).unwrap();
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
        drop(store);
        let job_path = dir.path().join("loader-ota/jobs.json");
        let mut legacy: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&job_path).unwrap()).unwrap();
        let old_job = legacy["board-1"].as_object_mut().unwrap();
        old_job.remove("next_delivery_at");
        old_job.remove("waiting_for_device_change");
        std::fs::write(&job_path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let legacy_ota = LoaderOtaState {
            active_sha256: "11".repeat(32),
            running_sha256: "11".repeat(32),
            pending_update_id: None,
            trial: false,
            source: None,
            last_update_id: None,
            last_outcome: None,
        };
        std::fs::write(
            dir.path().join("loader-ota/devices.json"),
            serde_json::to_vec(&serde_json::json!({
                "board-1": { "mac_address": mac, "state": legacy_ota }
            }))
            .unwrap(),
        )
        .unwrap();
        let store = OtaStore::open(dir.path()).unwrap();
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
        let store = OtaStore::open(dir.path()).unwrap();
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
        let store = OtaStore::open(dir.path()).unwrap();
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
        let store = OtaStore::open(dir.path()).unwrap();
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
        let store = OtaStore::open(dir.path()).unwrap();
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
        let store = OtaStore::open(dir.path()).unwrap();
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
        let image_path = store.image_path(&image.sha256).unwrap();
        let metadata_path = store
            .root
            .join("images")
            .join(format!("{}.json", image.sha256));
        assert!(image_path.is_file());
        assert!(metadata_path.is_file());

        assert!(matches!(
            store.delete_image(&image.sha256).await,
            Err(DeleteImageError::InUse)
        ));
        store.cancel("board-1", &job.update_id).await.unwrap();
        store.delete_image(&image.sha256).await.unwrap();
        assert!(store.images().await.unwrap().is_empty());
        assert!(!image_path.exists());
        assert!(!metadata_path.exists());
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
        fs::remove_file(&metadata_path).unwrap();
        store.delete_image(&image.sha256).await.unwrap();
        assert!(!image_path.exists());
        assert!(!metadata_path.exists());

        drop(store);
        let store = OtaStore::open(dir.path()).unwrap();
        assert!(store.images().await.unwrap().is_empty());
        let retained = store.job("board-1").await.unwrap();
        assert_eq!(retained.phase, Phase::Cancelled);
        assert_eq!(retained.image.sha256, image.sha256);
        assert_eq!(retained.image.size, image.size);
        assert_eq!(retained.image.version, image.version);
        assert!(!image_path.exists());
        assert!(!metadata_path.exists());
    }

    #[tokio::test]
    async fn inconsistent_delete_evicts_the_cached_image_and_does_not_block_restart() {
        let dir = tempfile::tempdir().unwrap();
        let store = OtaStore::open(dir.path()).unwrap();
        let image = store.put_image(&efi_image(), None).await.unwrap();
        let image_path = store.image_path(&image.sha256).unwrap();
        fs::remove_file(&image_path).unwrap();
        fs::create_dir(&image_path).unwrap();

        assert!(matches!(
            store.delete_image(&image.sha256).await,
            Err(DeleteImageError::Inconsistent)
        ));
        assert!(store.images().await.unwrap().is_empty());

        drop(store);
        let store = OtaStore::open(dir.path()).unwrap();
        assert!(store.images().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn inconsistent_image_metadata_does_not_prevent_restart() {
        let dir = tempfile::tempdir().unwrap();
        let store = OtaStore::open(dir.path()).unwrap();
        let image = store.put_image(&efi_image(), None).await.unwrap();
        fs::remove_file(store.image_path(&image.sha256).unwrap()).unwrap();
        drop(store);

        let store = OtaStore::open(dir.path()).unwrap();
        assert!(store.images().await.unwrap().is_empty());
    }
}
