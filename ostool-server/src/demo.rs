//! Explicit, loopback-only development fixtures. Never enabled by deployment configuration.
use std::{net::IpAddr, time::Duration};

use anyhow::{Context, bail};
use chrono::Utc;
use sha2::{Digest, Sha256};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::{
    AppState, BoardConfig, BoardNetworkIdentity, BootConfig, BuiltinTftpConfig,
    CustomPowerManagement, PowerManagementConfig, PxeProfile, ServerConfig, TftpConfig,
    UbootProfile, UefiBootArch, UefiHttpProfile, state::BoardLeaseState,
};

pub const DEMO_PASSWORD: &str = "demo-user-password-123";
const BOARD_COUNT: usize = 48;
const DTB_COUNT: usize = 24;
const USER_COUNT: usize = 36;

/// Refuse public listeners/origins before writing fixtures or connecting to hardware.
pub fn prepare(config: &mut ServerConfig) -> anyhow::Result<()> {
    if !config.listen_addr.ip().is_loopback() {
        bail!("--dev-demo requires a loopback management listener");
    }
    if let Some(origin) = &config.auth.public_base_url {
        let origin = url::Url::parse(origin)?;
        let host = origin
            .host_str()
            .unwrap_or_default()
            .trim_matches(['[', ']']);
        if host != "localhost" && !host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback()) {
            bail!("--dev-demo requires a loopback auth.public_base_url");
        }
    }
    config.loader_network.enabled = false;
    config.network_test.enabled = false;
    config.virtual_qemu.enabled = false;
    let mut tftp = BuiltinTftpConfig::default_with_root(config.tftp.root_dir().to_path_buf());
    tftp.enabled = false;
    config.tftp = TftpConfig::Builtin(tftp);
    Ok(())
}

fn board(index: usize) -> BoardConfig {
    let board_type = [
        "rk3588",
        "raspberry-pi-4",
        "visionfive2",
        "ls2k1000",
        "x86_64",
        "aarch64",
    ][index % 6];
    let id = format!("demo-{board_type}-{:02}", index + 1);
    let boot = match index % 3 {
        0 => BootConfig::Uboot(UbootProfile {
            use_tftp: true,
            dtb_name: Some(format!("demo-board-{:02}.dtb", index % DTB_COUNT + 1)),
            kernel_load_addr: Some("0x80200000".into()),
            ..Default::default()
        }),
        1 => BootConfig::Pxe(PxeProfile {
            notes: Some("[演示] PXE 网络启动配置".into()),
        }),
        _ => BootConfig::UefiHttp(UefiHttpProfile {
            boot_arch: Some(
                [
                    UefiBootArch::Riscv64,
                    UefiBootArch::Aarch64,
                    UefiBootArch::X86_64,
                    UefiBootArch::Loongarch64,
                ][index % 4]
                    .clone(),
            ),
            serial_parameters: None,
        }),
    };
    BoardConfig {
        power_management: PowerManagementConfig::Custom(CustomPowerManagement {
            power_on_cmd: format!("true # ostool-demo {id} on"),
            power_off_cmd: format!("true # ostool-demo {id} off"),
        }),
        id,
        board_type: board_type.into(),
        tags: vec!["demo".into(), ["实验室 A", "实验室 B", "远程机房"][index % 3].into(), ["教学", "内核调试", "CI", "驱动开发"][index % 4].into()],
        serial: None,
        boot,
        network_identity: Some(BoardNetworkIdentity { mac_address: format!("02:de:00:00:00:{:02x}", index + 1).parse().expect("demo MAC") }),
        notes: match index % 4 {
            0 => Some("[演示] 用于检查长文本显示、分页、搜索和状态筛选。该板卡没有连接真实硬件，电源操作为空操作。\n机架：A-03 / 负责人：测试团队 / 用途：操作系统兼容性验证。".into()),
            1 => Some("[演示] 日常开发板 / Demo development board".into()),
            2 => None,
            _ => Some("[演示] 维护窗口：每周三 18:00–20:00".into()),
        },
        disabled: index >= 42,
    }
}

/// Owns only the heartbeat task. Releasing a sample session in the UI keeps it released.
pub struct DemoRuntime(JoinHandle<()>);
impl Drop for DemoRuntime {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub async fn seed(state: &AppState) -> anyhow::Result<DemoRuntime> {
    // Defense in depth for library callers as well as the CLI.
    prepare(&mut state.config.read().await.clone())?;
    let _inventory = state.board_inventory_gate.lock().await;
    let mut owned = Vec::new();
    for index in 0..BOARD_COUNT {
        let sample = board(index);
        let existing = state.boards.read().await.get(&sample.id).cloned();
        if existing.is_none() {
            sample.validate()?;
            state.board_store.write_board(&sample).await?;
            state
                .boards
                .write()
                .await
                .insert(sample.id.clone(), sample.clone());
        }
        // Modified fixtures can point at real hardware: never create sessions or
        // artificial runtime states on anything other than the exact safe fixture.
        if existing
            .as_ref()
            .is_none_or(|b| serde_json::to_value(b).ok() == serde_json::to_value(&sample).ok())
        {
            owned.push((index, sample));
        }
    }
    state.sync_board_runtime_states().await;
    drop(_inventory);
    for index in 0..DTB_COUNT {
        let name = format!("demo-board-{:02}.dtb", index + 1);
        if state.dtb_store.get(&name).await?.is_none() {
            state.dtb_store.write(&name, &demo_dtb(index)).await?;
        }
        if let Some(db) = &state.database {
            let bytes = state.dtb_store.read(&name).await?;
            let dir = state.config.read().await.dtb_dir.clone();
            db.sync_dtb(
                &name,
                bytes.len() as u64,
                &dir.join(&name).to_string_lossy(),
                &format!("{:x}", Sha256::digest(&bytes)),
            )
            .await?;
        }
    }
    let admin = if state.config.read().await.edition.is_public() {
        seed_identity(state).await?
    } else {
        None
    };
    let mut sessions = Vec::new();
    for (index, sample) in &owned {
        if *index < 12 && !sample.disabled {
            let session = state
                .create_session_with_board_id(
                    &sample.board_type,
                    &sample.id,
                    &[],
                    Some(format!(
                        "[演示] {} / 工作站 {:02}",
                        ["内核构建", "串口调试", "自动化回归"][index % 3],
                        index + 1
                    )),
                )
                .await;
            if let Ok(session) = session {
                let manager = state.tftp_manager.read().await.clone();
                manager
                    .put_session_file(
                        &session.id,
                        "demo/README.txt",
                        b"ostool UI demo files. No bootable hardware payload.\n",
                    )
                    .await?;
                manager.put_session_file(&session.id, "demo/logs/boot.log", format!("[DEMO] session {}\n[DEMO] waiting for a device; no physical board attached\n", session.id).as_bytes()).await?;
                if let (Some(db), Some(admin)) = (&state.database, admin) {
                    sqlx::query("INSERT INTO session_owners (session_id,user_id) VALUES ($1,$2)")
                        .bind(&session.id)
                        .bind(admin)
                        .execute(db.pool())
                        .await?;
                }
                sessions.push(session.id);
            }
        } else if (36..42).contains(index) {
            let mut runtimes = state.board_runtimes.write().await;
            if let Some(runtime) = runtimes.get_mut(&sample.id)
                && runtime.lease_state == BoardLeaseState::Idle
            {
                runtime.lease_state = if index % 2 == 0 {
                    BoardLeaseState::Error
                } else {
                    BoardLeaseState::Releasing
                };
                runtime.last_release_error = Some(
                    if index % 2 == 0 {
                        "[演示状态] 电源关闭超时；用于查看异常提示"
                    } else {
                        "[演示状态] 等待释放；用于查看过渡状态"
                    }
                    .into(),
                );
                runtime.updated_at = Utc::now();
            }
        }
    }
    state.loader_registry.seed_demo_devices().await;
    state.admin_power.seed_demo();
    state.admin_events.invalidate(&[
        "boards",
        "runtimes",
        "sessions",
        "dtbs",
        "users",
        "roles",
        "rentals",
        "rental_requests",
        "power_actions",
    ]);
    log::warn!(
        "LOCAL DEMO enabled: 48 boards, 24 DTBs, sample sessions/devices; public edition adds 36 users, 6 roles, 72 rentals, 48 rental requests. Fixtures use demo-* identifiers; no startup hardware actions."
    );
    let runtime_state = state.clone();
    Ok(DemoRuntime(tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            for id in &sessions {
                let _ = runtime_state.heartbeat_session(id).await;
            }
            runtime_state.admin_events.invalidate(&["sessions"]);
            runtime_state.loader_registry.refresh_demo_devices().await;
        }
    })))
}

fn demo_uuid(kind: u128, index: usize) -> Uuid {
    Uuid::from_u128(0xde000000_0000_4000_8000_000000000000 + (kind << 16) + index as u128)
}

async fn seed_identity(state: &AppState) -> anyhow::Result<Option<Uuid>> {
    let db = state
        .database
        .as_ref()
        .context("public demo requires PostgreSQL")?;
    let mut tx = db.pool().begin().await?;
    for (index, name) in [
        "演示运维管理员",
        "演示开发者",
        "演示测试工程师",
        "演示教学用户",
        "演示访客",
        "演示长期项目成员",
    ]
    .iter()
    .enumerate()
    {
        sqlx::query("INSERT INTO roles (id,name,description,administrator) VALUES ($1,$2,$3,$4) ON CONFLICT DO NOTHING")
            .bind(format!("demo-role-{index}" )).bind(name).bind("[演示] 用于检查角色列表、过滤和编辑；权限仍由管理员标记决定")
            .bind(index == 0).execute(&mut *tx).await?;
    }
    let password = crate::persistence::postgres::hash_password_async(DEMO_PASSWORD.into()).await?;
    for index in 0..USER_COUNT {
        let display_name = match index % 4 {
            0 => Some(format!("演示用户 {:02} · 操作系统研发团队", index + 1)),
            1 => Some(format!("Demo Engineer {:02}", index + 1)),
            2 => None,
            _ => Some(format!("演示用户 {:02}", index + 1)),
        };
        sqlx::query("INSERT INTO users (id,email,password_hash,display_name,is_admin,disabled,role_id,created_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT DO NOTHING")
            .bind(demo_uuid(1, index)).bind(format!("demo{:02}@example.test", index + 1)).bind(&password)
            .bind(display_name).bind(index % 6 == 0).bind(index >= 30).bind(format!("demo-role-{}", index % 6))
            .bind(Utc::now() - chrono::Duration::days(index as i64 * 3)).execute(&mut *tx).await?;
    }
    // Fixed IDs and windows are inserted once. Existing bookings/passwords/edits
    // are retained on restart. A shared lock prevents races with rental creation.
    let _rentals = state.rental_gate.lock().await;
    for index in 0..72 {
        let sample = board(index % 24 + 12);
        let start = Utc::now() + chrono::Duration::days((index / 24) as i64 * 7 - 7);
        let end = start + chrono::Duration::hours(48);
        let cancelled = (index % 5 == 0).then_some(Utc::now() - chrono::Duration::hours(1));
        // Do not introduce overlaps with bookings the developer has added/edited.
        sqlx::query("INSERT INTO board_rentals (id,board_id,user_id,starts_at,ends_at,notes,cancelled_at) SELECT $1,$2,$3,$4,$5,$6,$7 WHERE EXISTS (SELECT 1 FROM users WHERE id=$3) AND ($7::timestamptz IS NOT NULL OR NOT EXISTS (SELECT 1 FROM board_rentals WHERE board_id=$2 AND cancelled_at IS NULL AND starts_at<$5 AND ends_at>$4)) ON CONFLICT DO NOTHING")
            .bind(demo_uuid(2, index)).bind(&sample.id).bind(demo_uuid(1, index % 29 + 1)).bind(start).bind(end)
            .bind(format!("[演示] {} / 项目 {:02}", ["内核适配", "课程实验", "驱动回归", "性能验证"][index % 4], index + 1))
            .bind(cancelled).execute(&mut *tx).await?;
    }
    for index in 0..48 {
        let status = ["pending", "approved", "rejected", "cancelled"][index % 4];
        let sample = board(index % 12);
        let start = Utc::now() + chrono::Duration::days(14 + (index / 12) as i64);
        sqlx::query("INSERT INTO rental_requests (id,user_id,board_id,starts_at,ends_at,notes,status,decision_note,reviewed_at,rental_id) SELECT $1,user_id,CASE WHEN $2='approved' THEN board_id ELSE $3 END,CASE WHEN $2='approved' THEN starts_at ELSE $4 END,CASE WHEN $2='approved' THEN ends_at ELSE $5 END,'[演示] 开发板使用申请',$2,CASE WHEN $2='pending' THEN '' ELSE '[演示] 审核记录' END,CASE WHEN $2 IN ('approved','rejected') THEN now() ELSE NULL END,CASE WHEN $2='approved' THEN id ELSE NULL END FROM board_rentals WHERE id=$6 ON CONFLICT DO NOTHING")
            .bind(demo_uuid(3,index)).bind(status).bind(sample.id).bind(start).bind(start+chrono::Duration::hours(2)).bind(demo_uuid(2,index)).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    // If the demo administrator has been deleted/disabled or its role changed,
    // don't recreate it or create sessions under its identity.
    Ok(db
        .find_user("demo01@example.test")
        .await?
        .filter(|user| user.is_admin)
        .map(|user| user.id))
}

/// Small valid flattened device tree, explicitly marked as a display fixture.
/// It intentionally contains no hardware description usable for booting a board.
fn demo_dtb(index: usize) -> Vec<u8> {
    let model = format!("ostool UI demo {:02} — not a hardware DTB\0", index + 1).into_bytes();
    let mut structure = Vec::new();
    for value in [1u32, 0, 3, model.len() as u32, 0] {
        structure.extend(value.to_be_bytes());
    }
    structure.extend(model);
    while structure.len() % 4 != 0 {
        structure.push(0);
    }
    for value in [2u32, 9] {
        structure.extend(value.to_be_bytes());
    }
    let strings = b"model\0";
    let mut result = Vec::new();
    for value in [
        0xd00dfeed,
        (56 + structure.len() + strings.len()) as u32,
        56,
        (56 + structure.len()) as u32,
        40,
        17,
        16,
        0,
        strings.len() as u32,
        structure.len() as u32,
    ] {
        result.extend(value.to_be_bytes());
    }
    result.extend([0; 16]);
    result.extend(structure);
    result.extend(strings);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{build_fixture_app_state as build_app_state, tftp::service::build_tftp_manager};

    fn test_config(dir: &std::path::Path) -> ServerConfig {
        ServerConfig {
            listen_addr: "127.0.0.1:0".parse().unwrap(),
            data_dir: dir.join("data"),
            board_dir: dir.join("boards"),
            dtb_dir: dir.join("dtbs"),
            tftp: TftpConfig::Builtin(BuiltinTftpConfig::default_with_root(dir.join("tftp"))),
            network: crate::TftpNetworkConfig {
                interface: "lo".into(),
            },
            ..Default::default()
        }
    }

    #[tokio::test]
    #[ignore = "requires a disposable PostgreSQL database via OSTOOL_DEMO_TEST_DATABASE_URL"]
    async fn postgres_fixtures_preserve_accounts_and_bookings() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_config(dir.path());
        config.edition = crate::config::Edition::Public;
        config.database.url =
            Some(std::env::var("OSTOOL_DEMO_TEST_DATABASE_URL").expect("disposable database URL"));
        prepare(&mut config).unwrap();
        let manager = build_tftp_manager(&config.tftp);
        let state = build_app_state(dir.path().join("config.toml"), config, manager)
            .await
            .unwrap();
        state.ensure_data_dirs().await.unwrap();
        let demo = seed(&state).await.unwrap();
        let db = state.database.as_ref().unwrap();
        for (query, expected) in [
            (
                "SELECT count(*) FROM board_records WHERE id LIKE 'demo-%'",
                48i64,
            ),
            (
                "SELECT count(*) FROM dtb_records WHERE name LIKE 'demo-%'",
                24,
            ),
            (
                "SELECT count(*) FROM users WHERE email LIKE 'demo%@example.test'",
                36,
            ),
            ("SELECT count(*) FROM roles WHERE id LIKE 'demo-%'", 6),
            (
                "SELECT count(*) FROM rental_requests WHERE notes LIKE '[演示]%'",
                48,
            ),
            (
                "SELECT count(*) FROM board_rentals WHERE notes LIKE '[演示]%'",
                72,
            ),
            ("SELECT count(*) FROM session_owners", 12),
        ] {
            assert_eq!(
                sqlx::query_scalar::<_, i64>(query)
                    .fetch_one(db.pool())
                    .await
                    .unwrap(),
                expected
            );
        }
        assert!(
            db.authenticate_password("demo02@example.test", DEMO_PASSWORD)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            db.authenticate_password("demo36@example.test", DEMO_PASSWORD)
                .await
                .unwrap()
                .is_none()
        );
        sqlx::query("UPDATE users SET disabled=true,display_name='edited demo admin' WHERE email='demo01@example.test'").execute(db.pool()).await.unwrap();
        sqlx::query("UPDATE board_rentals SET notes='edited booking' WHERE id=$1")
            .bind(demo_uuid(2, 0))
            .execute(db.pool())
            .await
            .unwrap();
        let second = seed(&state).await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM board_rentals")
                .fetch_one(db.pool())
                .await
                .unwrap(),
            72
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT notes FROM board_rentals WHERE id=$1")
                .bind(demo_uuid(2, 0))
                .fetch_one(db.pool())
                .await
                .unwrap(),
            "edited booking"
        );
        assert!(db.find_user("demo01@example.test").await.unwrap().is_none());
        assert_eq!(state.sessions.read().await.len(), 12);
        drop((demo, second));
    }

    #[test]
    fn demo_requires_loopback_and_disables_hardware_services() {
        let mut config = ServerConfig::default();
        assert!(prepare(&mut config).is_err());
        config.listen_addr = "127.0.0.1:2999".parse().unwrap();
        config.auth.public_base_url = Some("https://boards.example.com".into());
        assert!(prepare(&mut config).is_err());
        for origin in [
            "http://localhost:2999",
            "http://[::1]:2999",
            "http://127.0.0.1:2999",
        ] {
            config.auth.public_base_url = Some(origin.into());
            prepare(&mut config).unwrap();
            assert!(!config.loader_network.enabled);
            assert!(!config.network_test.enabled);
            assert!(!config.virtual_qemu.enabled);
            assert!(matches!(config.tftp, TftpConfig::Builtin(ref cfg) if !cfg.enabled));
        }
    }

    #[tokio::test]
    async fn repeated_seed_preserves_edits_and_released_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_config(dir.path());
        prepare(&mut config).unwrap();
        let manager = build_tftp_manager(&config.tftp);
        let state = build_app_state(dir.path().join("config.toml"), config, manager)
            .await
            .unwrap();
        state.ensure_data_dirs().await.unwrap();
        let demo = seed(&state).await.unwrap();
        assert_eq!(state.boards.read().await.len(), BOARD_COUNT);
        assert_eq!(state.sessions.read().await.len(), 12);
        assert_eq!(state.dtb_store.list_all().await.unwrap().len(), DTB_COUNT);
        assert_eq!(state.loader_registry.snapshots().await.len(), 18);
        let boards = state.boards.read().await;
        assert_eq!(
            state
                .loader_registry
                .snapshots()
                .await
                .iter()
                .filter(|device| !boards.values().any(|board| board
                    .network_identity
                    .as_ref()
                    .is_some_and(|network| network.mac_address == device.mac_address)))
                .count(),
            6
        );
        drop(boards);
        assert_eq!(
            state
                .loader_registry
                .snapshots()
                .await
                .iter()
                .filter(|d| d.online)
                .count(),
            12
        );
        assert!(
            state
                .admin_power
                .snapshots()
                .iter()
                .any(|r| r.state == "failed")
        );
        let id = board(0).id;
        let mut edited = state.boards.read().await[&id].clone();
        edited.notes = Some("developer's edited note".into());
        state.board_store.write_board(&edited).await.unwrap();
        state.boards.write().await.insert(id.clone(), edited);
        state
            .dtb_store
            .write("demo-board-01.dtb", b"developer's replacement")
            .await
            .unwrap();
        let second = seed(&state).await.unwrap();
        assert_eq!(state.sessions.read().await.len(), 12);
        assert_eq!(
            state.board_store.load_all().await.unwrap()[&id]
                .notes
                .as_deref(),
            Some("developer's edited note")
        );
        assert_eq!(
            state.dtb_store.read("demo-board-01.dtb").await.unwrap(),
            b"developer's replacement"
        );
        let session_id = state.sessions.read().await.keys().next().unwrap().clone();
        state.remove_session(&session_id).await.unwrap();
        tokio::time::sleep(Duration::from_secs(4)).await;
        assert!(state.get_session(&session_id).await.is_none());
        assert_eq!(state.sessions.read().await.len(), 11);
        assert!(
            state
                .loader_registry
                .snapshots()
                .await
                .iter()
                .any(|d| d.online)
        );
        // Live sessions are kept alive past their ordinary 10 second TTL.
        tokio::time::sleep(Duration::from_secs(7)).await;
        assert!(state.cleanup_expired_sessions().await.unwrap().is_empty());
        drop((demo, second));
    }
}
