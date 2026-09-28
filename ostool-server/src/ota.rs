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
    fn is_terminal(self) -> bool {
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
}

#[derive(Clone, Debug)]
pub enum Decision {
    Idle,
    Update(Job),
    Confirm(String),
    Wait,
}

#[derive(Clone)]
pub struct OtaStore {
    root: PathBuf,
    jobs: Arc<Mutex<BTreeMap<String, Job>>>,
    observations: Arc<Mutex<BTreeMap<String, Observation>>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Observation {
    mac_address: httpboot_protocol::MacAddress,
    state: LoaderOtaState,
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
        Ok(Self {
            root,
            jobs: Arc::new(Mutex::new(jobs)),
            observations: Arc::new(Mutex::new(observations)),
        })
    }

    pub fn image_path(&self, digest: &str) -> anyhow::Result<PathBuf> {
        ensure!(valid_digest(digest), "invalid image digest");
        Ok(self.root.join("images").join(format!("{digest}.efi")))
    }

    pub fn image(&self, digest: &str) -> anyhow::Result<Image> {
        ensure!(valid_digest(digest), "invalid image digest");
        let bytes = fs::read(self.root.join("images").join(format!("{digest}.json")))?;
        let image: Image = serde_json::from_slice(&bytes)?;
        ensure!(
            image.sha256 == digest && self.image_path(digest)?.is_file(),
            "missing OTA image"
        );
        Ok(image)
    }

    pub fn images(&self) -> anyhow::Result<Vec<Image>> {
        let mut images = Vec::new();
        for entry in fs::read_dir(self.root.join("images"))? {
            let entry = entry?;
            if let Some(name) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_suffix(".json"))
            {
                images.push(self.image(name)?);
            }
        }
        images.sort_by(|a, b| a.sha256.cmp(&b.sha256));
        Ok(images)
    }

    pub fn put_image(&self, bytes: &[u8], version: Option<String>) -> anyhow::Result<Image> {
        ensure!(
            !bytes.is_empty() && bytes.len() <= MAX_IMAGE_BYTES,
            "invalid EFI image size"
        );
        // DOS header, PE signature, AMD64 machine and EFI application subsystem.
        ensure!(
            bytes.len() >= 0x40 && &bytes[..2] == b"MZ",
            "invalid DOS header"
        );
        let pe_offset = u32::from_le_bytes(bytes[0x3c..0x40].try_into()?) as usize;
        ensure!(
            pe_offset
                .checked_add(94)
                .is_some_and(|end| end <= bytes.len()),
            "invalid PE offset"
        );
        ensure!(
            &bytes[pe_offset..pe_offset + 4] == b"PE\0\0",
            "invalid PE header"
        );
        ensure!(
            bytes[pe_offset + 4..pe_offset + 6] == [0x64, 0x86],
            "unsupported EFI architecture"
        );
        let optional = pe_offset + 24;
        ensure!(
            bytes[optional..optional + 2] == [0x0b, 0x02],
            "EFI image must be PE32+"
        );
        ensure!(
            bytes[optional + 68..optional + 70] == [10, 0],
            "EFI image must be an application"
        );
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
        let path = self.image_path(&sha256)?;
        if !path.exists() {
            atomic_write(&path, bytes)?;
        } else {
            ensure!(
                Sha256::digest(fs::read(&path)?).as_slice() == Sha256::digest(bytes).as_slice(),
                "existing image digest mismatch"
            );
        }
        atomic_write(
            &self.root.join("images").join(format!("{sha256}.json")),
            &serde_json::to_vec(&image)?,
        )?;
        Ok(image)
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
        let image = self.image(digest)?;
        let observations = self.observations.lock().await;
        if let Some(device) = observations
            .get(&board_id)
            .filter(|device| device.mac_address == mac_address)
        {
            ensure!(
                !device.state.trial && device.state.pending_update_id.is_none(),
                "device has an unconfirmed OTA trial"
            );
            ensure!(
                device.state.active_sha256 != image.sha256,
                "this image is already the active loader"
            );
        }
        let mut jobs = self.jobs.lock().await;
        ensure!(
            jobs.get(&board_id)
                .is_none_or(|job| job.phase.is_terminal()),
            "board already has an OTA job"
        );
        let job = Job {
            board_id: board_id.clone(),
            mac_address,
            update_id: uuid::Uuid::new_v4().to_string(),
            image,
            phase: Phase::Queued,
            error: None,
        };
        let mut updated = jobs.clone();
        updated.insert(board_id, job.clone());
        self.save(&updated)?;
        *jobs = updated;
        drop(observations);
        Ok(job)
    }

    pub async fn cancel(&self, board_id: &str, update_id: &str) -> anyhow::Result<Job> {
        let mut jobs = self.jobs.lock().await;
        let mut updated = jobs.clone();
        let job = updated.get_mut(board_id).context("no OTA job for board")?;
        ensure!(
            job.update_id == update_id && job.phase == Phase::Queued,
            "OTA job already activated or superseded"
        );
        job.phase = Phase::Cancelled;
        let result = job.clone();
        self.save(&updated)?;
        *jobs = updated;
        Ok(result)
    }

    pub async fn decide(
        &self,
        board_id: &str,
        mac_address: httpboot_protocol::MacAddress,
        ota: &LoaderOtaState,
        board_idle: bool,
    ) -> anyhow::Result<Decision> {
        let mut observations = self.observations.lock().await;
        let changed_observation = observations
            .get(board_id)
            .is_none_or(|old| old.mac_address != mac_address || old.state != *ota);
        if changed_observation {
            let mut updated = observations.clone();
            updated.insert(
                board_id.into(),
                Observation {
                    mac_address,
                    state: ota.clone(),
                },
            );
            atomic_write(
                &self.root.join("devices.json"),
                &serde_json::to_vec(&updated)?,
            )?;
            *observations = updated;
        }
        let mut jobs = self.jobs.lock().await;
        let Some(job) = jobs.get(board_id) else {
            return Ok(if ota.trial {
                Decision::Wait
            } else {
                Decision::Idle
            });
        };
        if job.mac_address != mac_address {
            return Ok(Decision::Wait);
        }
        let mut changed = jobs.clone();
        let job = changed.get_mut(board_id).unwrap();
        if ota.last_update_id.as_deref() == Some(&job.update_id) {
            match ota.last_outcome {
                Some(OtaOutcome::Confirmed)
                    if job.phase == Phase::Confirming && ota.active_sha256 == job.image.sha256 =>
                {
                    job.phase = Phase::Succeeded
                }
                Some(OtaOutcome::RolledBack) => job.phase = Phase::RolledBack,
                Some(OtaOutcome::Failed) => job.phase = Phase::Failed,
                _ => {}
            }
        }
        let decision = if ota.trial {
            if board_idle
                && ota.source == Some(OtaSource::Server)
                && ota.pending_update_id.as_deref() == Some(&job.update_id)
                && ota.running_sha256 == job.image.sha256
                && !job.phase.is_terminal()
            {
                job.phase = Phase::Confirming;
                Decision::Confirm(job.update_id.clone())
            } else {
                Decision::Wait
            }
        } else if job.phase.is_terminal() || !board_idle {
            Decision::Idle
        } else if ota.active_sha256 == job.image.sha256 {
            job.phase = Phase::Failed;
            job.error = Some("image already active without this assignment's confirmation".into());
            Decision::Idle
        } else if matches!(job.phase, Phase::Queued | Phase::Downloading) {
            job.phase = Phase::Downloading;
            Decision::Update(job.clone())
        } else {
            Decision::Wait
        };
        if changed.get(board_id).unwrap().phase != jobs.get(board_id).unwrap().phase {
            self.save(&changed)?;
            *jobs = changed;
        }
        Ok(decision)
    }

    pub async fn report(
        &self,
        board_id: &str,
        mac_address: httpboot_protocol::MacAddress,
        update_id: &str,
        phase: Phase,
        error: Option<String>,
        active_sha256: Option<&str>,
    ) -> anyhow::Result<()> {
        let observations = self.observations.lock().await;
        let mut jobs = self.jobs.lock().await;
        let mut updated = jobs.clone();
        let job = updated.get_mut(board_id).context("unknown OTA job")?;
        ensure!(
            job.mac_address == mac_address && job.update_id == update_id,
            "stale OTA report"
        );
        ensure!(
            matches!(
                phase,
                Phase::Downloading | Phase::Staged | Phase::Failed | Phase::Succeeded
            ),
            "invalid loader OTA phase"
        );
        ensure!(
            !job.phase.is_terminal() || job.phase == phase,
            "OTA job already finished"
        );
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
            let observed = observations
                .get(board_id)
                .context("OTA trial not observed")?;
            ensure!(
                observed.mac_address == mac_address
                    && observed.state.source == Some(OtaSource::Server)
                    && observed.state.trial
                    && observed.state.pending_update_id.as_deref() == Some(update_id)
                    && observed.state.running_sha256 == job.image.sha256
                    && active_sha256 == Some(job.image.sha256.as_str()),
                "confirmed OTA result does not match registered trial"
            );
        }
        if job.phase != phase || job.error != error {
            job.phase = phase;
            job.error = error;
            self.save(&updated)?;
            *jobs = updated;
        }
        Ok(())
    }

    fn save(&self, jobs: &BTreeMap<String, Job>) -> anyhow::Result<()> {
        atomic_write(&self.root.join("jobs.json"), &serde_json::to_vec(jobs)?)
    }
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
    async fn update_survives_server_restart_and_only_confirms_its_own_trial() {
        let dir = tempfile::tempdir().unwrap();
        let store = OtaStore::open(dir.path()).unwrap();
        let image = store.put_image(&efi_image(), Some("test".into())).unwrap();
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
            store.decide("board-1", mac, &state, false).await.unwrap(),
            Decision::Idle
        ));
        assert!(matches!(
            store.decide("board-1", mac, &state, true).await.unwrap(),
            Decision::Update(_)
        ));
        state.pending_update_id = Some(job.update_id.clone());
        state.trial = true;
        state.running_sha256 = image.sha256.clone();
        state.source = Some(OtaSource::Direct);
        assert!(matches!(
            store.decide("board-1", mac, &state, true).await.unwrap(),
            Decision::Wait
        ));
        state.source = Some(OtaSource::Server);
        assert!(
            matches!(store.decide("board-1", mac, &state, true).await.unwrap(), Decision::Confirm(id) if id == job.update_id)
        );
        drop(store);
        let store = OtaStore::open(dir.path()).unwrap();
        assert_eq!(store.job("board-1").await.unwrap().phase, Phase::Confirming);
        state.trial = false;
        state.pending_update_id = None;
        state.active_sha256 = image.sha256;
        state.last_update_id = Some(job.update_id);
        state.last_outcome = Some(OtaOutcome::Confirmed);
        assert!(matches!(
            store.decide("board-1", mac, &state, true).await.unwrap(),
            Decision::Idle
        ));
        assert_eq!(store.job("board-1").await.unwrap().phase, Phase::Succeeded);
        let mut newer = efi_image();
        newer.push(1);
        let candidate = store.put_image(&newer, None).unwrap();
        state.trial = true;
        state.pending_update_id = Some("01234567-89ab-cdef-0123-456789abcdef".into());
        state.source = Some(OtaSource::Direct);
        assert!(matches!(
            store.decide("board-1", mac, &state, true).await.unwrap(),
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
}
