use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use httpboot_protocol::{
    BootArch, BootOfferResponse, BootOfferState, ImageFormat, KernelPublishResponse,
    LoaderHelloRequest, LoaderHelloResponse,
};
use tokio::sync::RwLock;

use crate::config::{BoardConfig, BootConfig, UefiBootArch, UefiHttpStrategy};

const DEFAULT_WAIT_AFTER_MS: u64 = 1_000;

#[derive(Debug, Clone)]
pub struct LoaderRegistry {
    inner: Arc<RwLock<LoaderRegistryInner>>,
}

impl LoaderRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(LoaderRegistryInner::default())),
        }
    }

    pub async fn register_loader(
        &self,
        boards: &BTreeMap<String, BoardConfig>,
        request: LoaderHelloRequest,
        poll_url: String,
    ) -> Result<LoaderHelloResponse, LoaderRegisterError> {
        if request.protocol_version != 1 {
            return Err(LoaderRegisterError::UnsupportedProtocolVersion(
                request.protocol_version,
            ));
        }
        let mac = normalize_mac(&request.mac)
            .ok_or_else(|| LoaderRegisterError::InvalidMac(request.mac.clone()))?;
        let board = match_httpboot_board(boards, &mac)?;
        if let Some(board) = board.as_ref() {
            ensure_arch_compatible(request.arch, board)?;
        }

        let loader_id = uuid::Uuid::new_v4().to_string();
        let state = LoaderState {
            loader_id: loader_id.clone(),
            board_id: board.as_ref().map(|board| board.id.clone()),
            mac,
            arch: request.arch,
            board_hint: request.board,
            nonce: request.nonce,
            last_seen: Utc::now(),
            registered_at: Instant::now(),
        };
        self.inner
            .write()
            .await
            .loaders
            .insert(loader_id.clone(), state);

        Ok(LoaderHelloResponse {
            loader_id,
            board_id: board.as_ref().map(|board| board.id.clone()),
            board_type: board.as_ref().map(|board| board.board_type.clone()),
            poll_url,
            wait_after_ms: DEFAULT_WAIT_AFTER_MS,
        })
    }

    pub async fn publish_offer(
        &self,
        session_id: String,
        board_id: String,
        kernel_url: String,
        kernel_size: u64,
        kernel_sha256: Option<String>,
        arch: BootArch,
        image_format: ImageFormat,
        entry_symbol: Option<String>,
    ) -> KernelPublishResponse {
        let boot_id = uuid::Uuid::new_v4().to_string();
        let offer = BootOffer {
            boot_id: boot_id.clone(),
            session_id,
            board_id,
            kernel_url: kernel_url.clone(),
            kernel_size,
            kernel_sha256: kernel_sha256.clone(),
            arch,
            image_format,
            entry_symbol,
            published_at: Utc::now(),
        };
        self.inner
            .write()
            .await
            .offers_by_session
            .insert(offer.session_id.clone(), offer);

        KernelPublishResponse {
            boot_id,
            kernel_url,
            kernel_size,
            kernel_sha256,
        }
    }

    pub async fn loader_board_id(&self, loader_id: &str) -> Result<Option<String>, BootOfferError> {
        let inner = self.inner.read().await;
        inner
            .loaders
            .get(loader_id)
            .map(|loader| loader.board_id.clone())
            .ok_or(BootOfferError::UnknownLoader)
    }

    pub async fn boot_offer(
        &self,
        loader_id: &str,
        active_session_id: Option<&str>,
    ) -> Result<BootOfferResponse, BootOfferError> {
        let mut inner = self.inner.write().await;
        let loader = inner
            .loaders
            .get_mut(loader_id)
            .ok_or(BootOfferError::UnknownLoader)?;
        loader.last_seen = Utc::now();
        let Some(loader_board_id) = loader.board_id.clone() else {
            return Ok(waiting("loader is not matched to a board"));
        };
        let Some(session_id) = active_session_id else {
            return Ok(waiting("board has no active session"));
        };
        let Some(offer) = inner.offers_by_session.get(session_id) else {
            return Ok(waiting("session has no published kernel"));
        };
        if offer.board_id != loader_board_id {
            return Err(BootOfferError::BoardSessionMismatch {
                loader_board_id,
                offer_board_id: offer.board_id.clone(),
            });
        }

        Ok(BootOfferResponse {
            state: BootOfferState::Ready,
            wait_after_ms: None,
            boot_id: Some(offer.boot_id.clone()),
            kernel_url: Some(offer.kernel_url.clone()),
            kernel_size: Some(offer.kernel_size),
            kernel_sha256: offer.kernel_sha256.clone(),
            image_format: Some(offer.image_format),
            arch: Some(offer.arch),
            entry_symbol: offer.entry_symbol.clone(),
            message: None,
        })
    }
}

impl Default for LoaderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Default)]
struct LoaderRegistryInner {
    loaders: BTreeMap<String, LoaderState>,
    offers_by_session: BTreeMap<String, BootOffer>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct LoaderState {
    loader_id: String,
    board_id: Option<String>,
    mac: String,
    arch: BootArch,
    board_hint: Option<String>,
    nonce: String,
    last_seen: DateTime<Utc>,
    registered_at: Instant,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct BootOffer {
    boot_id: String,
    session_id: String,
    board_id: String,
    kernel_url: String,
    kernel_size: u64,
    kernel_sha256: Option<String>,
    arch: BootArch,
    image_format: ImageFormat,
    entry_symbol: Option<String>,
    published_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LoaderRegisterError {
    #[error("unsupported HTTPBoot loader protocol version `{0}`")]
    UnsupportedProtocolVersion(u16),
    #[error("invalid loader MAC address `{0}`")]
    InvalidMac(String),
    #[error("no HTTPBoot board is configured for MAC `{0}`")]
    UnknownBoardMac(String),
    #[error("multiple HTTPBoot boards are configured for MAC `{0}`: {1:?}")]
    AmbiguousBoardMac(String, Vec<String>),
    #[error("board `{board_id}` boot arch is incompatible with loader arch `{loader_arch:?}`")]
    IncompatibleArch {
        board_id: String,
        loader_arch: BootArch,
    },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BootOfferError {
    #[error("unknown loader")]
    UnknownLoader,
    #[error("loader board `{loader_board_id}` does not match offer board `{offer_board_id}`")]
    BoardSessionMismatch {
        loader_board_id: String,
        offer_board_id: String,
    },
}

pub fn normalize_mac(input: &str) -> Option<String> {
    let hex = input
        .trim()
        .chars()
        .filter(|ch| *ch != ':' && *ch != '-')
        .map(|ch| ch.to_ascii_lowercase())
        .collect::<String>();
    if hex.len() != 12 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }

    let mut normalized = String::with_capacity(17);
    for (index, ch) in hex.chars().enumerate() {
        if index > 0 && index % 2 == 0 {
            normalized.push(':');
        }
        normalized.push(ch);
    }
    Some(normalized)
}

fn match_httpboot_board<'a>(
    boards: &'a BTreeMap<String, BoardConfig>,
    normalized_mac: &str,
) -> Result<Option<&'a BoardConfig>, LoaderRegisterError> {
    let mut matches = boards
        .values()
        .filter(|board| !board.disabled)
        .filter_map(|board| {
            let BootConfig::UefiHttp(profile) = &board.boot else {
                return None;
            };
            if profile.strategy != UefiHttpStrategy::LoaderDiscovery {
                return None;
            }
            let board_mac = profile.mac.as_deref().and_then(normalize_mac)?;
            (board_mac == normalized_mac).then_some(board)
        })
        .collect::<Vec<_>>();

    match matches.len() {
        0 => Err(LoaderRegisterError::UnknownBoardMac(
            normalized_mac.to_string(),
        )),
        1 => Ok(matches.pop()),
        _ => Err(LoaderRegisterError::AmbiguousBoardMac(
            normalized_mac.to_string(),
            matches.iter().map(|board| board.id.clone()).collect(),
        )),
    }
}

fn ensure_arch_compatible(
    loader_arch: BootArch,
    board: &BoardConfig,
) -> Result<(), LoaderRegisterError> {
    let BootConfig::UefiHttp(profile) = &board.boot else {
        return Ok(());
    };
    let Some(board_arch) = profile.boot_arch.as_ref() else {
        return Ok(());
    };
    if uefi_arch_to_boot_arch(board_arch) == loader_arch {
        Ok(())
    } else {
        Err(LoaderRegisterError::IncompatibleArch {
            board_id: board.id.clone(),
            loader_arch,
        })
    }
}

pub fn uefi_arch_to_boot_arch(arch: &UefiBootArch) -> BootArch {
    match arch {
        UefiBootArch::X86_64 => BootArch::X86_64,
        UefiBootArch::Aarch64 => BootArch::Aarch64,
        UefiBootArch::Loongarch64 => BootArch::Loongarch64,
        UefiBootArch::Riscv64 => BootArch::Riscv64,
        UefiBootArch::Other => BootArch::Other,
    }
}

fn waiting(message: impl Into<String>) -> BootOfferResponse {
    BootOfferResponse {
        state: BootOfferState::Waiting,
        wait_after_ms: Some(DEFAULT_WAIT_AFTER_MS),
        boot_id: None,
        kernel_url: None,
        kernel_size: None,
        kernel_sha256: None,
        image_format: None,
        arch: None,
        entry_symbol: None,
        message: Some(message.into()),
    }
}

#[allow(dead_code)]
fn prune_stale_loaders(inner: &mut LoaderRegistryInner, max_age: Duration) {
    inner
        .loaders
        .retain(|_, loader| loader.registered_at.elapsed() <= max_age);
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use httpboot_protocol::{BootArch, ImageFormat, LoaderCapabilities, LoaderHelloRequest};

    use crate::config::{
        BoardConfig, BootConfig, CustomPowerManagement, PowerManagementConfig, UefiBootArch,
        UefiHttpProfile, UefiHttpStrategy,
    };

    use super::{BootOfferError, LoaderRegistry, normalize_mac};

    fn httpboot_board(id: &str, mac: &str) -> BoardConfig {
        BoardConfig {
            id: id.into(),
            board_type: "Asus-nuc15-x86_64-vmx".into(),
            tags: vec!["httpboot".into()],
            serial: None,
            power_management: PowerManagementConfig::Custom(CustomPowerManagement {
                power_on_cmd: "true".into(),
                power_off_cmd: "true".into(),
            }),
            boot: BootConfig::UefiHttp(UefiHttpProfile {
                boot_arch: Some(UefiBootArch::X86_64),
                strategy: UefiHttpStrategy::LoaderDiscovery,
                mac: Some(mac.into()),
            }),
            notes: None,
            disabled: false,
        }
    }

    fn hello(mac: &str) -> LoaderHelloRequest {
        LoaderHelloRequest {
            protocol_version: 1,
            nonce: "nonce".into(),
            arch: BootArch::X86_64,
            board: Some("asus-nuc15crh".into()),
            mac: mac.into(),
            firmware_vendor: None,
            loader_version: None,
            capabilities: LoaderCapabilities {
                image_formats: vec![ImageFormat::Elf64],
                range_get: true,
                sha256: true,
            },
        }
    }

    #[test]
    fn normalizes_mac_address() {
        assert_eq!(
            normalize_mac("1C-69-7A-DC-F3-47").as_deref(),
            Some("1c:69:7a:dc:f3:47")
        );
        assert!(normalize_mac("not-a-mac").is_none());
    }

    #[tokio::test]
    async fn loader_offer_is_bound_to_matching_board_session() {
        let registry = LoaderRegistry::new();
        let mut boards = BTreeMap::new();
        boards.insert(
            "asus-1".into(),
            httpboot_board("asus-1", "1c:69:7a:dc:f3:47"),
        );

        let response = registry
            .register_loader(&boards, hello("1c:69:7a:dc:f3:47"), "/poll".into())
            .await
            .unwrap();
        assert_eq!(response.board_id.as_deref(), Some("asus-1"));

        let waiting = registry
            .boot_offer(&response.loader_id, Some("session-1"))
            .await
            .unwrap();
        assert_eq!(
            waiting.message.as_deref(),
            Some("session has no published kernel")
        );

        let published = registry
            .publish_offer(
                "session-1".into(),
                "asus-1".into(),
                "http://127.0.0.1/kernel.elf".into(),
                4096,
                Some("00".repeat(32)),
                BootArch::X86_64,
                ImageFormat::Elf64,
                Some("httpboot_entry".into()),
            )
            .await;

        let offer = registry
            .boot_offer(&response.loader_id, Some("session-1"))
            .await
            .unwrap();
        assert_eq!(offer.boot_id, Some(published.boot_id));
        assert_eq!(offer.kernel_size, Some(4096));
    }

    #[tokio::test]
    async fn offer_rejects_mismatched_board() {
        let registry = LoaderRegistry::new();
        let mut boards = BTreeMap::new();
        boards.insert(
            "asus-1".into(),
            httpboot_board("asus-1", "1c:69:7a:dc:f3:47"),
        );
        let response = registry
            .register_loader(&boards, hello("1c:69:7a:dc:f3:47"), "/poll".into())
            .await
            .unwrap();
        registry
            .publish_offer(
                "session-1".into(),
                "asus-2".into(),
                "http://127.0.0.1/kernel.elf".into(),
                4096,
                None,
                BootArch::X86_64,
                ImageFormat::Elf64,
                None,
            )
            .await;

        let err = registry
            .boot_offer(&response.loader_id, Some("session-1"))
            .await
            .unwrap_err();
        assert_eq!(
            err,
            BootOfferError::BoardSessionMismatch {
                loader_board_id: "asus-1".into(),
                offer_board_id: "asus-2".into()
            }
        );
    }
}
