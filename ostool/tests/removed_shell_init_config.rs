use ostool::{
    board::config::BoardRunConfig,
    run::{qemu::QemuConfig, uboot::UbootConfig},
};
use serde::de::DeserializeOwned;

const QEMU_CONFIG: &str = r#"
args = []
uefi = false
fail_regex = []
"#;

const UBOOT_CONFIG: &str = r#"
fail_regex = []
"#;

const BOARD_CONFIG: &str = r#"
board_type = "orangepi-5-plus"
"#;

#[test]
fn qemu_rejects_removed_top_level_shell_fields() {
    assert_removed_root_keys_are_rejected::<QemuConfig>(QEMU_CONFIG, false);
}

#[test]
fn uboot_rejects_removed_top_level_shell_fields() {
    assert_removed_root_keys_are_rejected::<UbootConfig>(UBOOT_CONFIG, true);
}

#[test]
fn board_rejects_removed_top_level_shell_fields() {
    assert_removed_root_keys_are_rejected::<BoardRunConfig>(BOARD_CONFIG, true);
}

#[test]
fn qemu_ignores_removed_top_level_success_regex() {
    for field in ["success_regex = [\"PASS\"]", "success_regex = []"] {
        let input = format!("{QEMU_CONFIG}\n{field}\n");
        toml::from_str::<QemuConfig>(&input)
            .unwrap_or_else(|error| panic!("QEMU success_regex must be ignored: {error}"));
    }
}

#[test]
fn qemu_rejects_mixed_new_and_removed_step_command() {
    assert_mixed_step_command_is_rejected::<QemuConfig>(QEMU_CONFIG);
}

#[test]
fn uboot_rejects_mixed_new_and_removed_step_command() {
    assert_mixed_step_command_is_rejected::<UbootConfig>(UBOOT_CONFIG);
}

#[test]
fn board_rejects_mixed_new_and_removed_step_command() {
    assert_mixed_step_command_is_rejected::<BoardRunConfig>(BOARD_CONFIG);
}

#[test]
fn qemu_still_tolerates_axbuild_extension_fields() {
    let input = format!(
        r#"{QEMU_CONFIG}
test_commands = ["echo pass"]

[host_http_server]
port = 8080
"#
    );

    toml::from_str::<QemuConfig>(&input).expect("axbuild extensions must remain tolerated");
}

fn assert_removed_root_keys_are_rejected<T>(minimal_config: &str, reject_success_regex: bool)
where
    T: DeserializeOwned,
{
    let mut removed_fields = vec![
        ("shell_prefix", "shell_prefix = \"root#\""),
        ("shell_init_cmd", "shell_init_cmd = \"echo pass\""),
        ("shell_init_steps", "shell_init_steps = []"),
    ];
    if reject_success_regex {
        removed_fields.extend([
            ("success_regex", "success_regex = [\"PASS\"]"),
            ("success_regex", "success_regex = []"),
        ]);
    }
    for (removed_key, removed_field) in removed_fields {
        let input = format!("{minimal_config}\n{removed_field}\n");
        let error = toml::from_str::<T>(&input)
            .err()
            .unwrap_or_else(|| panic!("removed root key `{removed_key}` was accepted"));
        let message = error.to_string();

        assert!(
            message.contains(removed_key),
            "error did not name removed key `{removed_key}`: {message}"
        );
        assert!(
            message.contains("shell_check_steps"),
            "error did not direct the user to `shell_check_steps`: {message}"
        );
    }
}

fn assert_mixed_step_command_is_rejected<T>(minimal_config: &str)
where
    T: DeserializeOwned,
{
    let input = format!(
        r#"{minimal_config}
shell_check_steps = [
  {{ shell_prefix = "root#", shell_cmd = "new", shell_init_cmd = "old" }},
]
"#
    );
    let error = toml::from_str::<T>(&input)
        .err()
        .expect("mixed new and removed step command was accepted");
    let message = error.to_string();

    assert!(
        message.contains("shell_init_cmd"),
        "error did not name removed step key `shell_init_cmd`: {message}"
    );
}
