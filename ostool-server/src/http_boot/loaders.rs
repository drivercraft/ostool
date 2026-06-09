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

use crate::config::{BoardConfig, BootConfig, UefiBootArch};

const DEFAULT_WAIT_AFTER_MS: u64 = 1_000;

#[derive(Debug, Clone)]
pub struct LoaderRegistry {
    inner: Arc<RwLock<LoaderRegistryInner>>,
}

#[derive(Debug, Clone)]
pub struct PublishOfferRequest {
    pub session_id: String,
    pub board_id: String,
    pub kernel_url: String,
    pub kernel_size: u64,
    pub kernel_sha256: Option<String>,
    pub arch: BootArch,
    pub image_format: ImageFormat,
    pub entry_symbol: Option<String>,
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
        active_board_ids: &[String],
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
        let board = match_httpboot_board(boards, active_board_ids, &mac, request.arch)?;
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

    pub async fn publish_offer(&self, request: PublishOfferRequest) -> KernelPublishResponse {
        let boot_id = uuid::Uuid::new_v4().to_string();
        let offer = BootOffer {
            boot_id: boot_id.clone(),
            session_id: request.session_id,
            board_id: request.board_id,
            kernel_url: request.kernel_url.clone(),
            kernel_size: request.kernel_size,
            kernel_sha256: request.kernel_sha256.clone(),
            arch: request.arch,
            image_format: request.image_format,
            entry_symbol: request.entry_symbol,
            published_at: Utc::now(),
        };
        self.inner
            .write()
            .await
            .offers_by_session
            .insert(offer.session_id.clone(), offer);

        KernelPublishResponse {
            boot_id,
            kernel_url: request.kernel_url,
            kernel_size: request.kernel_size,
            kernel_sha256: request.kernel_sha256,
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
    #[error("no active HTTPBoot board can accept loader MAC `{mac}`")]
    UnknownActiveBoard { mac: String },
    #[error("multiple active HTTPBoot boards can accept loader MAC `{mac}`: {board_ids:?}")]
    AmbiguousActiveBoard { mac: String, board_ids: Vec<String> },
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
    active_board_ids: &[String],
    normalized_mac: &str,
    loader_arch: BootArch,
) -> Result<Option<&'a BoardConfig>, LoaderRegisterError> {
    let mut matches = boards
        .values()
        .filter(|board| !board.disabled)
        .filter_map(|board| {
            let BootConfig::UefiHttp(profile) = &board.boot else {
                return None;
            };
            let board_mac = profile.mac.as_deref().and_then(normalize_mac)?;
            (board_mac == normalized_mac).then_some(board)
        })
        .collect::<Vec<_>>();

    match matches.len() {
        0 => match_active_httpboot_board(boards, active_board_ids, normalized_mac, loader_arch),
        1 => Ok(matches.pop()),
        _ => Err(LoaderRegisterError::AmbiguousBoardMac(
            normalized_mac.to_string(),
            matches.iter().map(|board| board.id.clone()).collect(),
        )),
    }
}

fn match_active_httpboot_board<'a>(
    boards: &'a BTreeMap<String, BoardConfig>,
    active_board_ids: &[String],
    normalized_mac: &str,
    loader_arch: BootArch,
) -> Result<Option<&'a BoardConfig>, LoaderRegisterError> {
    let mut matches = active_board_ids
        .iter()
        .filter_map(|board_id| boards.get(board_id))
        .filter(|board| !board.disabled)
        .filter(
            |board| matches!(&board.boot, BootConfig::UefiHttp(profile) if profile.mac.is_none()),
        )
        .filter(|board| is_arch_compatible(loader_arch, board))
        .collect::<Vec<_>>();

    match matches.len() {
        0 => Err(LoaderRegisterError::UnknownActiveBoard {
            mac: normalized_mac.to_string(),
        }),
        1 => Ok(matches.pop()),
        _ => Err(LoaderRegisterError::AmbiguousActiveBoard {
            mac: normalized_mac.to_string(),
            board_ids: matches.iter().map(|board| board.id.clone()).collect(),
        }),
    }
}

fn is_arch_compatible(loader_arch: BootArch, board: &BoardConfig) -> bool {
    let BootConfig::UefiHttp(profile) = &board.boot else {
        return false;
    };
    profile
        .boot_arch
        .as_ref()
        .is_none_or(|board_arch| uefi_arch_to_boot_arch(board_arch) == loader_arch)
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
        UefiHttpProfile,
    };

    use super::{BootOfferError, LoaderRegistry, PublishOfferRequest, normalize_mac};

    fn httpboot_board(id: &str, mac: Option<&str>) -> BoardConfig {
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
                mac: mac.map(str::to_string),
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
            httpboot_board("asus-1", Some("1c:69:7a:dc:f3:47")),
        );

        let response = registry
            .register_loader(&boards, &[], hello("1c:69:7a:dc:f3:47"), "/poll".into())
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
            .publish_offer(PublishOfferRequest {
                session_id: "session-1".into(),
                board_id: "asus-1".into(),
                kernel_url: "http://127.0.0.1/kernel.elf".into(),
                kernel_size: 4096,
                kernel_sha256: Some("00".repeat(32)),
                arch: BootArch::X86_64,
                image_format: ImageFormat::Elf64,
                entry_symbol: Some("httpboot_entry".into()),
            })
            .await;

        let offer = registry
            .boot_offer(&response.loader_id, Some("session-1"))
            .await
            .unwrap();
        assert_eq!(offer.boot_id, Some(published.boot_id));
        assert_eq!(offer.kernel_size, Some(4096));
    }

    #[tokio::test]
    async fn loader_can_match_active_httpboot_board_without_configured_mac() {
        let registry = LoaderRegistry::new();
        let mut boards = BTreeMap::new();
        boards.insert("asus-1".into(), httpboot_board("asus-1", None));

        let response = registry
            .register_loader(
                &boards,
                &["asus-1".into()],
                hello("1c:69:7a:dc:f3:47"),
                "/poll".into(),
            )
            .await
            .unwrap();
        assert_eq!(response.board_id.as_deref(), Some("asus-1"));
    }

    #[tokio::test]
    async fn offer_rejects_mismatched_board() {
        let registry = LoaderRegistry::new();
        let mut boards = BTreeMap::new();
        boards.insert(
            "asus-1".into(),
            httpboot_board("asus-1", Some("1c:69:7a:dc:f3:47")),
        );
        let response = registry
            .register_loader(&boards, &[], hello("1c:69:7a:dc:f3:47"), "/poll".into())
            .await
            .unwrap();
        registry
            .publish_offer(PublishOfferRequest {
                session_id: "session-1".into(),
                board_id: "asus-2".into(),
                kernel_url: "http://127.0.0.1/kernel.elf".into(),
                kernel_size: 4096,
                kernel_sha256: None,
                arch: BootArch::X86_64,
                image_format: ImageFormat::Elf64,
                entry_symbol: None,
            })
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
