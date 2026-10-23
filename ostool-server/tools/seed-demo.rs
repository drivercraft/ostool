//! 独立导入持久化演示数据；正式服务不包含演示运行模式。
use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
use chrono::Utc;
use clap::Parser;
use ostool_server::{
    BoardConfig, BoardNetworkIdentity, BoardStore, BootConfig, CustomPowerManagement, DtbStore,
    PowerManagementConfig, PxeProfile, UbootProfile, UefiBootArch, UefiHttpProfile,
    persistence::postgres::Database,
};
use rand::RngCore;
use std::path::PathBuf;
use uuid::Uuid;
const DEMO_PASSWORD: &str = "demo-user-password-123";
const BOARD_COUNT: usize = 48;
const DTB_COUNT: usize = 24;
const USER_COUNT: usize = 36;

#[derive(Parser)]
struct Cli {
    #[arg(long)]
    env_file: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = ostool_server::environment::load(cli.env_file.as_deref())?;
    let db = Database::connect(&config.database).await?;
    let boards = BoardStore::new(db.clone());
    let dtbs = DtbStore::new(db.clone());
    let existing = boards.load_all().await?;
    for index in 0..BOARD_COUNT {
        let sample = board(index);
        if !existing.contains_key(&sample.id) {
            boards.write_board(&sample).await?;
        }
    }
    for index in 0..DTB_COUNT {
        let name = format!("demo-board-{:02}.dtb", index + 1);
        if dtbs.get(&name).await?.is_none() {
            dtbs.write(&name, &demo_dtb(index)).await?;
        }
    }
    if config.edition.is_public() {
        seed_identity(&db).await?;
    }
    println!("已导入演示数据，保留已有记录；重启服务器以刷新设备清单。");
    println!(
        "演示账号 demo01@example.test / demo02@example.test，密码：{DEMO_PASSWORD}（仅 public 版创建）"
    );
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

fn demo_uuid(kind: u128, index: usize) -> Uuid {
    Uuid::from_u128(0xde000000_0000_4000_8000_000000000000 + (kind << 16) + index as u128)
}

async fn seed_identity(db: &Database) -> anyhow::Result<()> {
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
    let mut salt = [0; 16];
    rand::rng().fill_bytes(&mut salt);
    let salt = SaltString::encode_b64(&salt).map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let password = Argon2::default()
        .hash_password(DEMO_PASSWORD.as_bytes(), &salt)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?
        .to_string();
    for index in 0..USER_COUNT {
        let display_name = match index % 4 {
            0 => Some(format!("演示用户 {:02} · 操作系统研发团队", index + 1)),
            1 => Some(format!("Demo Engineer {:02}", index + 1)),
            2 => None,
            _ => Some(format!("演示用户 {:02}", index + 1)),
        };
        sqlx::query("INSERT INTO users (id,email,password_hash,display_name,is_admin,disabled,role_id,created_at,username) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT DO NOTHING")
            .bind(demo_uuid(1, index)).bind(format!("demo{:02}@example.test", index + 1)).bind(&password)
            .bind(display_name).bind(index % 6 == 0).bind(index >= 30).bind(format!("demo-role-{}", index % 6))
            .bind(Utc::now() - chrono::Duration::days(index as i64 * 3)).bind(format!("demo{:02}",index+1)).execute(&mut *tx).await?;
    }
    // Fixed IDs and windows are inserted once. Existing bookings/passwords/edits
    // are retained on repeated imports. Run while the server is stopped.
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
    Ok(())
}

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
