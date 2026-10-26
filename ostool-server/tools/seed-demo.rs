//! 独立导入界面预置数据；不启动硬件连接或改变正式服务的运行方式。
use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
use chrono::{Duration, Utc};
use clap::Parser;
use ostool_server::{
    BoardConfig, BoardNetworkIdentity, BoardStore, BootConfig, CustomPowerManagement, DtbStore,
    PowerManagementConfig, UbootProfile, UefiBootArch, UefiHttpProfile,
    persistence::postgres::Database,
};
use rand::RngCore;
use std::path::PathBuf;
use uuid::Uuid;

const BOARD_COUNT: usize = 48;
const DTB_COUNT: usize = 24;
const PEOPLE: [(&str, &str); 36] = [
    ("lin_jun", "林峻"),
    ("chen_siyuan", "陈思远"),
    ("zhang_wei", "张伟"),
    ("wang_yu", "王瑜"),
    ("li_ming", "李明"),
    ("zhao_ning", "赵宁"),
    ("liu_yang", "刘洋"),
    ("zhou_hang", "周航"),
    ("wu_tong", "吴桐"),
    ("xu_ran", "徐然"),
    ("sun_yue", "孙悦"),
    ("hu_yifan", "胡一帆"),
    ("gao_yuan", "高远"),
    ("he_jing", "何静"),
    ("luo_chen", "罗晨"),
    ("zheng_kai", "郑凯"),
    ("liang_yue", "梁玥"),
    ("song_yichen", "宋奕辰"),
    ("tang_yun", "唐韵"),
    ("han_xu", "韩旭"),
    ("feng_rui", "冯睿"),
    ("deng_lin", "邓琳"),
    ("cao_yu", "曹宇"),
    ("peng_cheng", "彭程"),
    ("xiao_qing", "肖晴"),
    ("yuan_hao", "袁浩"),
    ("jiang_nan", "蒋楠"),
    ("shen_yi", "沈奕"),
    ("du_hao", "杜昊"),
    ("cheng_xi", "程曦"),
    ("ye_shu", "叶舒"),
    ("su_ning", "苏宁"),
    ("wei_zhe", "魏哲"),
    ("ding_yi", "丁毅"),
    ("ren_xin", "任欣"),
    ("lu_yan", "陆言"),
];
const ROLES: [(&str, &str, &str); 6] = [
    (
        "lab-admin",
        "实验室管理员",
        "负责设备资产、人员权限和实验室运行维护",
    ),
    (
        "kernel-engineer",
        "内核研发工程师",
        "开展操作系统移植、启动链适配和内核调试",
    ),
    (
        "driver-engineer",
        "驱动开发工程师",
        "负责外设驱动开发、设备树维护和接口验证",
    ),
    (
        "quality-engineer",
        "质量工程师",
        "执行版本回归、稳定性验证和性能基准测试",
    ),
    (
        "instructor",
        "课程教师",
        "安排教学设备使用、课程实验和学生项目",
    ),
    (
        "collaborator",
        "项目协作成员",
        "在获批时间段内参与联合研发和设备验证",
    ),
];
// 型号、架构、DTB 命名保持对应；这里只生成设备清单，不操作物理硬件。
const MODELS: [(&str, &str, &str, &str); 6] = [
    ("rk3588", "ROCK 5B", "rk3588-rock-5b", "radxa,rock-5b"),
    (
        "raspberry-pi-4",
        "Raspberry Pi 4 Model B",
        "bcm2711-rpi-4-b",
        "raspberrypi,4-model-b",
    ),
    (
        "visionfive2",
        "VisionFive 2",
        "jh7110-starfive-visionfive-2",
        "starfive,visionfive-2-v1.3b",
    ),
    (
        "ls2k0300",
        "龙芯 2K0300 开发板",
        "loongson-2k0300",
        "loongson,2k0300",
    ),
    (
        "x86_64",
        "研华边缘计算节点",
        "x86-edge-controller",
        "ostool,edge-controller",
    ),
    (
        "aarch64",
        "Ampere Altra 研发节点",
        "ampere-altra",
        "ampere,altra",
    ),
];
const PURPOSES: [&str; 8] = [
    "内核启动链与中断控制器适配",
    "以太网驱动与网络吞吐回归",
    "设备树与外设资源核对",
    "文件系统压力与掉电恢复验证",
    "操作系统课程实验",
    "调度器性能与时延评估",
    "PCIe 设备枚举及 DMA 验证",
    "新版本发布前兼容性验证",
];

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
        let item = board(index);
        item.validate()?;
        if !existing.contains_key(&item.id) {
            boards.write_board(&item).await?;
            sqlx::query("UPDATE board_records SET created_at=$2,updated_at=$3 WHERE id=$1")
                .bind(&item.id)
                .bind(Utc::now() - Duration::days(365 - index as i64))
                .bind(Utc::now() - Duration::days((index % 14) as i64))
                .execute(db.pool())
                .await?;
        }
    }
    for index in 0..DTB_COUNT {
        let name = dtb_name(index);
        if dtbs.get(&name).await?.is_none() {
            dtbs.write(&name, &device_tree(index)).await?;
            sqlx::query("UPDATE dtb_records SET updated_at=$2 WHERE name=$1")
                .bind(&name)
                .bind(Utc::now() - Duration::days((index % 18 + 1) as i64))
                .execute(db.pool())
                .await?;
        }
    }
    if config.edition.is_public() {
        db.bootstrap_admin(&config.auth).await?;
        seed_identity(&db).await?;
    }
    println!("设备、人员、租赁及使用记录已导入；已有记录保持不变。");
    println!("管理员登录信息保持不变。预置人员使用独立随机密码，可在用户管理中设置登录密码。");
    Ok(())
}

fn dtb_name(index: usize) -> String {
    format!("{}-rev{}.dtb", MODELS[index % 4].2, index / 4 + 1)
}

fn board(index: usize) -> BoardConfig {
    let model_index = index % MODELS.len();
    let (kind, model, _, _) = MODELS[model_index];
    let number = index / MODELS.len() + 1;
    let zone = ["A", "B", "C"][model_index / 2];
    let id = format!("{kind}-{}-{number:02}", zone.to_lowercase());
    let boot = if model_index < 4 {
        BootConfig::Uboot(UbootProfile {
            use_tftp: true,
            dtb_name: Some(dtb_name(model_index + (number - 1) % 6 * 4)),
            kernel_load_addr: Some(
                [
                    "0x00280000",
                    "0x00080000",
                    "0x80200000",
                    "0x9000000090200000",
                ][model_index]
                    .into(),
            ),
            ..Default::default()
        })
    } else {
        BootConfig::UefiHttp(UefiHttpProfile {
            boot_arch: Some(if model_index == 4 {
                UefiBootArch::X86_64
            } else {
                UefiBootArch::Aarch64
            }),
            serial_parameters: None,
        })
    };
    BoardConfig {
        allow_instant_use: false,
        reset_command: None,
        power_management: PowerManagementConfig::Custom(CustomPowerManagement {
            power_on_cmd: "true".into(),
            power_off_cmd: "true".into(),
        }),
        id,
        board_type: kind.into(),
        serial: None,
        boot,
        tags: vec![
            format!("{zone} 区实验室"),
            ["ARM64", "ARM64", "RISC-V", "LoongArch64", "x86_64", "ARM64"][model_index].into(),
            ["内核研发", "驱动开发", "版本回归", "课程教学"][index % 4].into(),
        ],
        network_identity: Some(BoardNetworkIdentity {
            mac_address: format!(
                "02:74:6c:{:02x}:{:02x}:{:02x}",
                model_index + 1,
                number,
                0x20 + index
            )
            .parse()
            .expect("valid inventory MAC"),
        }),
        notes: Some(format!(
            "型号：{model}\n位置：{zone} 区 / 机架 {:02} / 槽位 {number:02}\n负责人：{}\n用途：{}{}",
            model_index / 2 + 1,
            PEOPLE[(index + 2) % 30].1,
            PURPOSES[index % PURPOSES.len()],
            if index >= 44 {
                "\n维护安排：电源与串口接线检查，完成验收后恢复开放。"
            } else {
                ""
            }
        )),
        disabled: index >= 44,
    }
}

fn record_id(kind: u128, index: usize) -> Uuid {
    Uuid::from_u128(0x4a8736d2_e927_478c_9f04_000000000000 + (kind << 16) + index as u128)
}
fn role(index: usize) -> usize {
    if index < 2 { 0 } else { (index - 2) % 5 + 1 }
}
fn password_hash() -> anyhow::Result<String> {
    let mut random = [0; 32];
    rand::rng().fill_bytes(&mut random);
    let salt = SaltString::encode_b64(&random[..16]).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(Argon2::default()
        .hash_password(&random, &salt)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?
        .to_string())
}

async fn seed_identity(db: &Database) -> anyhow::Result<()> {
    let now = Utc::now();
    // 北京时间 09:00，日历中的预约从工作日常用时段开始。
    let day = (now + Duration::hours(8))
        .date_naive()
        .and_hms_opt(1, 0, 0)
        .unwrap()
        .and_utc();
    let mut tx = db.pool().begin().await?;
    for (index, (id, name, description)) in ROLES.iter().enumerate() {
        sqlx::query("INSERT INTO roles(id,code,name,description,administrator,permissions,created_at,updated_at) VALUES($1,$1,$2,$3,$4,CASE WHEN $4 THEN ARRAY['system.admin'] ELSE ARRAY['board.use','rental.request'] END,now()-interval '200 days',now()-interval '30 days') ON CONFLICT DO NOTHING")
            .bind(id).bind(name).bind(description).bind(index == 0).execute(&mut *tx).await?;
    }
    for (index, (username, name)) in PEOPLE.iter().enumerate() {
        let age = if (30..34).contains(&index) {
            (index - 29) as i64
        } else {
            180 - index as i64 * 4
        };
        let created = now - Duration::days(age);
        let login_age = if index >= 30 {
            (index - 30) * 4 + 2
        } else {
            index * 7 + 1
        };
        let last_login = (index < 32).then_some(now - Duration::hours(login_age as i64));
        sqlx::query("INSERT INTO users(id,username,email,password_hash,display_name,is_admin,disabled,role_id,created_at,updated_at,password_changed_at,email_verified_at,last_login_at,last_login_ip) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$9,$11,$12,$13) ON CONFLICT DO NOTHING")
            .bind(record_id(1,index)).bind(username).bind(format!("{username}@ostool.internal")).bind(password_hash()?)
            .bind(name).bind(role(index)==0).bind(index>=34).bind(ROLES[role(index)].0)
            .bind(created).bind(last_login.unwrap_or(created)).bind((index<32).then_some(created+Duration::minutes(30)))
            .bind(last_login).bind(last_login.map(|_|format!("10.20.{}.{}",index/20+1,index%20+21)))
            .execute(&mut *tx).await?;
    }
    for index in 0..72 {
        let item = board(index % 24 + 12);
        let start =
            day + Duration::days((index / 24) as i64 * 7 - 7) + Duration::hours((index % 3) as i64);
        let end = start + Duration::hours([4, 8, 24][index % 3]);
        let created = (start - Duration::days(3))
            .min(now - Duration::hours(12) - Duration::minutes(index as i64 * 17));
        let cancelled = (index % 9 == 0).then_some(created + Duration::hours(2));
        sqlx::query("INSERT INTO board_rentals(id,board_id,user_id,starts_at,ends_at,notes,cancelled_at,created_at) SELECT $1,$2,$3,$4,$5,$6,$7,$8 WHERE EXISTS(SELECT 1 FROM users WHERE id=$3) AND ($7::timestamptz IS NOT NULL OR NOT EXISTS(SELECT 1 FROM board_rentals WHERE board_id=$2 AND cancelled_at IS NULL AND starts_at<$5 AND ends_at>$4)) ON CONFLICT DO NOTHING")
            .bind(record_id(2,index)).bind(&item.id).bind(record_id(1,index%28+2)).bind(start).bind(end)
            .bind(PURPOSES[index%PURPOSES.len()]).bind(cancelled).bind(created).execute(&mut *tx).await?;
    }
    for index in 0..48 {
        let status = ["pending", "approved", "rejected", "cancelled"][index % 4];
        let item = board(index % 12);
        let start =
            day + Duration::days(14 + (index / 12) as i64) + Duration::hours((index % 4) as i64);
        let created = now - Duration::hours(24 + (index * 3) as i64);
        let decision = match status {
            "approved" => "设备时间段已核对，按申请安排使用。",
            "rejected" => "该设备正在安排维护，请调整使用时间或选择同类型设备。",
            _ => "",
        };
        sqlx::query("INSERT INTO rental_requests(id,user_id,board_id,starts_at,ends_at,notes,status,decision_note,reviewed_at,reviewed_by,rental_id,created_at) SELECT $1,user_id,CASE WHEN $2='approved' THEN board_id ELSE $3 END,CASE WHEN $2='approved' THEN starts_at ELSE $4 END,CASE WHEN $2='approved' THEN ends_at ELSE $5 END,$6,$2,$7,CASE WHEN $2='approved' THEN created_at WHEN $2='rejected' THEN $8 ELSE NULL END,CASE WHEN $2 IN ('approved','rejected') THEN $9 ELSE NULL END,CASE WHEN $2='approved' THEN id ELSE NULL END,CASE WHEN $2='approved' THEN created_at-interval '1 hour' ELSE $10 END FROM board_rentals WHERE id=$11 ON CONFLICT DO NOTHING")
            .bind(record_id(3,index)).bind(status).bind(item.id).bind(start).bind(start+Duration::hours(4))
            .bind(PURPOSES[index%8]).bind(decision).bind(created+Duration::hours(4)).bind(record_id(1,index%2))
            .bind(created).bind(record_id(2,index)).execute(&mut *tx).await?;
    }
    for index in 0..60 {
        let booking = [
            1, 2, 3, 4, 5, 6, 7, 8, 10, 11, 12, 13, 14, 15, 16, 17, 19, 20, 21, 22, 23,
        ][index % 21];
        let item = board(booking + 12);
        let person = booking + 2;
        let start = day - Duration::days(7)
            + Duration::hours((booking % 3) as i64)
            + Duration::minutes((index / 21 * 70 + 10) as i64);
        let ended = start + Duration::minutes(20 + (index % 3) as i64 * 10);
        let reason = [
            "client_released",
            "serial_closed",
            "lease_expired",
            "client_released",
        ][index % 4];
        sqlx::query("INSERT INTO session_history(session_id,board_id,board_type,user_id,username,client_name,created_at,last_heartbeat_at,expires_at,stop_requested_at,ended_at,stop_reason,end_reason,rental_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$10,$11,$11,$12) ON CONFLICT DO NOTHING")
            .bind(record_id(4,index).to_string()).bind(item.id).bind(item.board_type).bind(record_id(1,person)).bind(PEOPLE[person].0)
            .bind(["ostool / Linux", "ostool / Windows", "cargo-osrun / Linux"][index%3])
            .bind(start).bind(ended-Duration::seconds(if reason=="lease_expired" {10}else{2})).bind(ended+Duration::seconds(if reason=="lease_expired" {0}else{8}))
            .bind(ended).bind(reason).bind(record_id(2,booking)).execute(&mut *tx).await?;
    }
    for index in 0..156 {
        let (query, target, action, summary, resource, method, route) = if index < 48 {
            (
                "SELECT updated_at AS at FROM board_records WHERE id=$1",
                board(index).id,
                "board.update",
                "更新设备配置",
                "boards",
                "PUT",
                "/api/v1/admin/boards",
            )
        } else if index < 84 {
            (
                "SELECT created_at AS at FROM users WHERE id::text=$1",
                record_id(1, index - 48).to_string(),
                "user.create",
                "创建用户账号",
                "users",
                "POST",
                "/api/v1/admin/users",
            )
        } else if index < 96 {
            (
                "SELECT reviewed_at AS at FROM rental_requests WHERE id::text=$1",
                record_id(3, (index - 84) * 4 + 1).to_string(),
                "rental_request.approve",
                "批准租赁申请",
                "rental-requests",
                "POST",
                "/api/v1/admin/rental-requests",
            )
        } else {
            (
                "SELECT ended_at AS at FROM session_history WHERE session_id=$1",
                record_id(4, index - 96).to_string(),
                "session.release",
                "释放设备会话",
                "sessions",
                "DELETE",
                "/api/v1/admin/sessions",
            )
        };
        let at: chrono::DateTime<Utc> = sqlx::query_scalar(query)
            .bind(&target)
            .fetch_one(&mut *tx)
            .await?;
        let system = index >= 96 || index == 48 || index == 49;
        sqlx::query("INSERT INTO audit_events(id,request_id,created_at,finished_at,actor_id,actor_name,actor_kind,source,action,summary,resource,target_id,method,route,ip,user_agent,result,status_code,duration_ms,details) VALUES($1,$2,$3,$3+interval '80 milliseconds',$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,'Mozilla/5.0','success',200,$15,'{}') ON CONFLICT DO NOTHING")
            .bind(record_id(5,index)).bind(record_id(6,index)).bind(at)
            .bind((!system).then_some(record_id(1,index%2))).bind(if system {"系统"} else {PEOPLE[index%2].1})
            .bind(if system {"system"}else{"admin"}).bind(if system {"system"}else{"admin"})
            .bind(action).bind(summary).bind(resource).bind(target).bind(method).bind(route)
            .bind((!system).then(||format!("10.20.1.{}",index%2+11))).bind((index%41+12) as i64)
            .execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

fn device_tree(index: usize) -> Vec<u8> {
    let model = format!("{} Rev {}\0", MODELS[index % 4].1, index / 4 + 1).into_bytes();
    let compatible = format!("{}\0", MODELS[index % 4].3).into_bytes();
    let mut structure = Vec::new();
    for value in [1u32, 0] {
        structure.extend(value.to_be_bytes());
    }
    for (offset, value) in [(0, model), (6, compatible)] {
        for word in [3u32, value.len() as u32, offset] {
            structure.extend(word.to_be_bytes());
        }
        structure.extend(value);
        while structure.len() % 4 != 0 {
            structure.push(0);
        }
    }
    for value in [2u32, 9] {
        structure.extend(value.to_be_bytes());
    }
    let strings = b"model\0compatible\0";
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
