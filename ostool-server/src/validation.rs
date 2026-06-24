pub const ID_MAX_LEN: usize = 64;
pub const USERNAME_MIN_LEN: usize = 3;
pub const USERNAME_MAX_LEN: usize = 64;
pub const DISPLAY_NAME_MIN_LEN: usize = 1;
pub const DISPLAY_NAME_MAX_LEN: usize = 64;
pub const EMAIL_MIN_LEN: usize = 5;
pub const EMAIL_MAX_LEN: usize = 254;
pub const PASSWORD_MIN_LEN: usize = 8;
pub const PASSWORD_MAX_LEN: usize = 128;
pub const ROLE_NAME_MIN_LEN: usize = 2;
pub const ROLE_NAME_MAX_LEN: usize = 64;
pub const DESCRIPTION_MAX_LEN: usize = 255;
pub const LONG_DESCRIPTION_MAX_LEN: usize = 500;
pub const URL_MAX_LEN: usize = 512;
pub const PHONE_MAX_LEN: usize = 32;
pub const BOARD_TYPE_MAX_LEN: usize = 64;
pub const TAG_MAX_LEN: usize = 32;
pub const TAGS_TEXT_MAX_LEN: usize = 256;
pub const SERIAL_KEY_MAX_LEN: usize = 128;
pub const COMMAND_MAX_LEN: usize = 255;
pub const IP_MAX_LEN: usize = 45;
pub const LOAD_ADDR_MAX_LEN: usize = 32;
pub const DTB_NAME_MAX_LEN: usize = 128;
pub const BOOT_ARCH_MAX_LEN: usize = 64;
pub const COMPATIBLE_MAX_LEN: usize = 255;
pub const CLIENT_NAME_MAX_LEN: usize = 128;
pub const STATE_MAX_LEN: usize = 32;
pub const HASH_MAX_LEN: usize = 255;
pub const SHA256_LEN: usize = 64;
pub const STORAGE_PATH_MAX_LEN: usize = 255;
pub const AUDIT_ACTION_MAX_LEN: usize = 64;
pub const AUDIT_TARGET_TYPE_MAX_LEN: usize = 64;
pub const AUDIT_TARGET_ID_MAX_LEN: usize = 128;
pub const AUDIT_OUTCOME_MAX_LEN: usize = 32;
pub const USER_AGENT_MAX_LEN: usize = 512;
pub const REQUEST_ID_MAX_LEN: usize = 128;
pub const SETTING_KEY_MAX_LEN: usize = 128;
pub const SETTING_TYPE_MAX_LEN: usize = 64;
pub const SETTING_NAME_MAX_LEN: usize = 64;

pub fn char_len(value: &str) -> usize {
    value.chars().count()
}

pub fn valid_username(value: &str) -> bool {
    value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

pub fn valid_role_name(value: &str) -> bool {
    value
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
}
