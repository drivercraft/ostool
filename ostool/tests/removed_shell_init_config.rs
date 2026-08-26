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
fn qemu_rejects_legacy_top_level_shell_fields() {
    assert_legacy_root_keys_are_rejected::<QemuConfig>(QEMU_CONFIG, false);
}

#[test]
fn uboot_rejects_legacy_top_level_shell_fields() {
    assert_legacy_root_keys_are_rejected::<UbootConfig>(UBOOT_CONFIG, true);
}

#[test]
fn board_rejects_legacy_top_level_shell_fields() {
    assert_legacy_root_keys_are_rejected::<BoardRunConfig>(BOARD_CONFIG, true);
}

#[test]
fn qemu_rejects_removed_top_level_success_regex() {
    for field in ["success_regex = [\"PASS\"]", "success_regex = []"] {
        let input = format!("{QEMU_CONFIG}\n{field}\n");
        let error =
            toml::from_str::<QemuConfig>(&input).expect_err("QEMU success_regex was accepted");
        assert!(
            error.to_string().contains("success_regex"),
            "error did not name removed key `success_regex`: {error}"
        );
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
fn qemu_rejects_removed_shell_init_steps() {
    assert_removed_shell_init_steps_is_rejected::<QemuConfig>(QEMU_CONFIG);
}

#[test]
fn uboot_rejects_removed_shell_init_steps() {
    assert_removed_shell_init_steps_is_rejected::<UbootConfig>(UBOOT_CONFIG);
}

#[test]
fn board_rejects_removed_shell_init_steps() {
    assert_removed_shell_init_steps_is_rejected::<BoardRunConfig>(BOARD_CONFIG);
}

fn assert_legacy_root_keys_are_rejected<T>(minimal_config: &str, include_success_regex: bool)
where
    T: DeserializeOwned,
{
    let mut legacy_fields = vec!["shell_prefix = \"root#\"", "shell_init_cmd = \"echo pass\""];
    if include_success_regex {
        legacy_fields.extend(["success_regex = [\"PASS\"]", "success_regex = []"]);
    }
    for legacy_field in legacy_fields {
        let input = format!("{minimal_config}\n{legacy_field}\n");
        let error = toml::from_str::<T>(&input)
            .err()
            .expect("legacy root key was accepted");
        let key = legacy_field.split_once(' ').unwrap().0;
        assert!(
            error.to_string().contains(key),
            "error did not name removed key `{key}`: {error}"
        );
    }
}

fn assert_removed_shell_init_steps_is_rejected<T>(minimal_config: &str)
where
    T: DeserializeOwned,
{
    let input = format!("{minimal_config}\nshell_init_steps = []\n");
    let error = toml::from_str::<T>(&input)
        .err()
        .expect("removed root key `shell_init_steps` was accepted");
    let message = error.to_string();

    assert!(
        message.contains("shell_init_steps"),
        "error did not name removed key `shell_init_steps`: {message}"
    );
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
