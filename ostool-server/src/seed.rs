use anyhow::Context;

use crate::{
    BootConfig, CustomPowerManagement, PowerManagementConfig, PxeProfile, UbootNetworkMode,
    UbootProfile, UefiBootArch, UefiHttpProfile,
    auth::hash_password,
    config::{AdminSeedConfig, BoardConfig, SampleDataConfig},
    storage::{DynStorage, NewUser, UserProfile},
};

pub async fn seed_database(
    storage: &DynStorage,
    sample_data: &SampleDataConfig,
) -> anyhow::Result<()> {
    seed_admin_user(storage, &sample_data.admin).await?;
    if sample_data.enabled {
        seed_sample_boards(storage).await?;
        seed_sample_users(storage).await?;
    }
    Ok(())
}

pub async fn seed_admin_user(storage: &DynStorage, admin: &AdminSeedConfig) -> anyhow::Result<()> {
    if !admin.enabled {
        return Ok(());
    }
    let username = admin.username.trim();
    if username.is_empty() {
        anyhow::bail!("sample_data.admin.username must not be empty when admin seed is enabled");
    }
    if let Some(existing) = storage.find_user_by_username(username).await? {
        if admin.reset_existing_password {
            storage
                .update_password_hash(
                    &existing.id,
                    hash_password(&admin.password)
                        .context("failed to hash seeded admin password")?,
                )
                .await?;
        }
        let admin_role_id = storage
            .list_roles()
            .await?
            .into_iter()
            .find(|role| role.name == "admin")
            .map(|role| role.id)
            .ok_or_else(|| anyhow::anyhow!("admin role does not exist"))?;
        storage
            .set_user_roles(&existing.id, vec![admin_role_id])
            .await?;
        return Ok(());
    }

    let display_name = admin
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(username);
    let email = admin
        .email
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{username}@ostool.local"));

    storage
        .create_user(NewUser {
            username: username.to_string(),
            display_name: display_name.to_string(),
            email,
            password_hash: hash_password(&admin.password)
                .context("failed to hash seeded admin password")?,
            profile: UserProfile::default(),
            role_names: vec!["admin".to_string()],
        })
        .await?;
    Ok(())
}

async fn seed_sample_boards(storage: &DynStorage) -> anyhow::Result<()> {
    if !storage.list_board_configs().await?.is_empty() {
        return Ok(());
    }
    for board in sample_boards() {
        storage.create_board_config(board).await?;
    }
    Ok(())
}

async fn seed_sample_users(storage: &DynStorage) -> anyhow::Result<()> {
    for user in sample_users() {
        if storage
            .find_user_by_username(user.username)
            .await?
            .is_some()
        {
            continue;
        }
        storage
            .create_user(NewUser {
                username: user.username.into(),
                display_name: user.display_name.into(),
                email: user.email.into(),
                password_hash: hash_password(user.password)
                    .context("failed to hash sample user password")?,
                profile: UserProfile::default(),
                role_names: vec!["user".into()],
            })
            .await?;
    }
    Ok(())
}

fn sample_boards() -> Vec<BoardConfig> {
    vec![
        sample_uboot_board(
            "sample-rk3568-01",
            "sample-rk3568",
            &["sample", "arm64", "uboot", "tftp", "lab-a"],
            true,
            UbootNetworkMode::Dhcp,
            None,
        ),
        sample_uboot_board(
            "sample-rk3568-02",
            "sample-rk3568",
            &["sample", "arm64", "uboot", "static-ip", "lab-a"],
            true,
            UbootNetworkMode::StaticIp,
            Some("192.168.10.42"),
        ),
        sample_uboot_board(
            "sample-riscv64-01",
            "sample-riscv64",
            &["sample", "riscv64", "uboot", "tftp", "lab-b"],
            true,
            UbootNetworkMode::Dhcp,
            None,
        ),
        sample_httpboot_board(
            "sample-x86-httpboot-01",
            "sample-x86-httpboot",
            UefiBootArch::X86_64,
            &["sample", "x86_64", "httpboot", "uefi", "lab-c"],
            false,
        ),
        sample_httpboot_board(
            "sample-aarch64-httpboot-01",
            "sample-aarch64-httpboot",
            UefiBootArch::Aarch64,
            &["sample", "aarch64", "httpboot", "uefi", "lab-c"],
            false,
        ),
        sample_httpboot_board(
            "sample-loongarch64-httpboot-01",
            "sample-loongarch64-httpboot",
            UefiBootArch::Loongarch64,
            &["sample", "loongarch64", "httpboot", "uefi", "lab-d"],
            false,
        ),
        sample_pxe_board(
            "sample-pxe-01",
            "sample-pxe",
            &["sample", "pxe", "x86_64", "legacy", "lab-d"],
            false,
        ),
        sample_pxe_board(
            "sample-maintenance-01",
            "sample-maintenance",
            &["sample", "reserved", "maintenance"],
            true,
        ),
    ]
}

fn sample_uboot_board(
    id: &str,
    board_type: &str,
    tags: &[&str],
    use_tftp: bool,
    network_mode: UbootNetworkMode,
    board_ip: Option<&str>,
) -> BoardConfig {
    BoardConfig {
        id: id.into(),
        board_type: board_type.into(),
        tags: tags.iter().map(|tag| (*tag).into()).collect(),
        serial: None,
        power_management: sample_power_management(),
        boot: BootConfig::Uboot(UbootProfile {
            use_tftp,
            dtb_name: None,
            network_mode,
            board_ip: board_ip.map(str::to_string),
            server_ip: Some("192.168.10.1".into()),
            netmask: Some("255.255.255.0".into()),
            gatewayip: Some("192.168.10.1".into()),
            ..Default::default()
        }),
        notes: Some("示例开发板，可在管理后台复制后调整为真实硬件参数。".into()),
        disabled: false,
    }
}

fn sample_httpboot_board(
    id: &str,
    board_type: &str,
    boot_arch: UefiBootArch,
    tags: &[&str],
    disabled: bool,
) -> BoardConfig {
    BoardConfig {
        id: id.into(),
        board_type: board_type.into(),
        tags: tags.iter().map(|tag| (*tag).into()).collect(),
        serial: None,
        power_management: sample_power_management(),
        boot: BootConfig::UefiHttp(UefiHttpProfile {
            boot_arch: Some(boot_arch),
            mac: None,
        }),
        notes: Some("UEFI HTTP Boot 示例资源。".into()),
        disabled,
    }
}

fn sample_pxe_board(id: &str, board_type: &str, tags: &[&str], disabled: bool) -> BoardConfig {
    BoardConfig {
        id: id.into(),
        board_type: board_type.into(),
        tags: tags.iter().map(|tag| (*tag).into()).collect(),
        serial: None,
        power_management: sample_power_management(),
        boot: BootConfig::Pxe(PxeProfile {
            notes: Some("PXE 启动示例".into()),
        }),
        notes: Some("PXE 示例资源。".into()),
        disabled,
    }
}

fn sample_power_management() -> PowerManagementConfig {
    PowerManagementConfig::Custom(CustomPowerManagement {
        power_on_cmd: "true".into(),
        power_off_cmd: "true".into(),
    })
}

struct SampleUser {
    username: &'static str,
    password: &'static str,
    display_name: &'static str,
    email: &'static str,
}

fn sample_users() -> Vec<SampleUser> {
    vec![
        SampleUser {
            username: "alice",
            password: "ostool123",
            display_name: "Alice Chen",
            email: "alice@ostool.local",
        },
        SampleUser {
            username: "bob",
            password: "ostool123",
            display_name: "Bob Li",
            email: "bob@ostool.local",
        },
        SampleUser {
            username: "carol",
            password: "ostool123",
            display_name: "Carol Wang",
            email: "carol@ostool.local",
        },
        SampleUser {
            username: "dave",
            password: "ostool123",
            display_name: "Dave Zhang",
            email: "dave@ostool.local",
        },
    ]
}
