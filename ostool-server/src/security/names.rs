//! Editable reserved-name policy for user-facing account writes.
use crate::api::error::ApiError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct NamePolicy {
    pub username_keywords: Vec<String>,
    pub display_name_keywords: Vec<String>,
}
impl Default for NamePolicy {
    fn default() -> Self {
        Self {
            username_keywords: [
                "admin",
                "administrator",
                "root",
                "system",
                "official",
                "support",
                "moderator",
            ]
            .map(str::to_owned)
            .into(),
            display_name_keywords: [
                "管理员",
                "官方",
                "客服",
                "admin",
                "administrator",
                "moderator",
            ]
            .map(str::to_owned)
            .into(),
        }
    }
}
// Comparison only: the chosen display spelling is never rewritten. Removing separators
// also prevents splitting protected words with punctuation, whitespace or zero-width marks.
fn comparison_key(value: &str) -> String {
    value
        .nfkc()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric())
        .collect()
}
impl NamePolicy {
    pub fn validate(&self) -> anyhow::Result<()> {
        for (label, keywords) in [
            ("用户名", &self.username_keywords),
            ("昵称", &self.display_name_keywords),
        ] {
            anyhow::ensure!(keywords.len() <= 128, "{label}禁用关键词最多 128 个");
            let mut keys = std::collections::BTreeSet::new();
            for word in keywords {
                anyhow::ensure!(
                    word.chars().count() <= 64 && !word.chars().any(char::is_control),
                    "{label}关键词不超过 64 个字符，且不能包含控制字符"
                );
                let key = comparison_key(word);
                anyhow::ensure!(!key.is_empty(), "{label}关键词必须包含文字或数字");
                anyhow::ensure!(
                    keys.insert(key),
                    "{label}禁用关键词不能重复（忽略大小写、全角和分隔符）"
                );
            }
        }
        Ok(())
    }

    /// Existing names are allowed unchanged, including provisioned administrator accounts.
    /// Internal bootstrap/import tools deliberately do not use this user-facing policy.
    pub fn check_change(
        &self,
        username: &str,
        display_name: Option<&str>,
        previous: Option<(&str, Option<&str>)>,
    ) -> Result<(), ApiError> {
        let username = crate::persistence::postgres::normalize_username(username)
            .map_err(|e| ApiError::bad_request(e.to_string()))?;
        let display_name = normalize_display_name(display_name)?;
        for (value, words, unchanged, code, message) in [
            (
                username.as_str(),
                &self.username_keywords,
                previous.is_some_and(|(old, _)| old == username),
                "reserved_username",
                "用户名包含系统保留关键词，请更换用户名",
            ),
            (
                display_name.as_deref().unwrap_or_default(),
                &self.display_name_keywords,
                previous.is_some_and(|(_, old)| {
                    old.map(str::trim).filter(|s| !s.is_empty()) == display_name.as_deref()
                }),
                "reserved_display_name",
                "昵称包含系统保留关键词，请更换昵称",
            ),
        ] {
            if !unchanged {
                let key = comparison_key(value);
                if words.iter().any(|word| key.contains(&comparison_key(word))) {
                    return Err(ApiError::new(
                        axum::http::StatusCode::BAD_REQUEST,
                        code,
                        message,
                    ));
                }
            }
        }
        Ok(())
    }
}

pub fn normalize_display_name(value: Option<&str>) -> Result<Option<String>, ApiError> {
    let value = value.map(str::trim).filter(|s| !s.is_empty());
    if value.is_some_and(|s| s.chars().count() > 120 || s.chars().any(char::is_control)) {
        return Err(ApiError::bad_request(
            "昵称不超过 120 个字符，且不能包含控制字符",
        ));
    }
    Ok(value.map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blocks_reserved_fragments_and_obfuscated_display_names() {
        let policy = NamePolicy::default();
        for name in [
            "admin",
            "ADMIN_01",
            "administrator",
            "my_root",
            "system-user",
            "a_d-m_i_n",
        ] {
            assert!(
                policy.check_change(name, Some("工程师"), None).is_err(),
                "{name}"
            );
        }
        for name in [
            "平台管理员",
            "管 理 员",
            "管-理_员",
            "管\u{200b}理\u{2060}员",
            "Ａｄｍｉｎ",
            "官方支持",
            "客服小王",
        ] {
            assert_eq!(
                policy
                    .check_change("engineer", Some(name), None)
                    .unwrap_err()
                    .code,
                "reserved_display_name",
                "{name}"
            );
        }
        policy
            .check_change("lin_jun", Some("操作系统研发工程师"), None)
            .unwrap();
        policy.check_change("chen_siyuan", None, None).unwrap();
    }
    #[test]
    fn allows_existing_names_but_rechecks_any_changed_field() {
        let policy = NamePolicy::default();
        let old = Some(("admin", Some("平台管理员")));
        policy
            .check_change("ADMIN", Some("平台管理员"), old)
            .unwrap();
        assert!(
            policy
                .check_change("new_admin", Some("平台管理员"), old)
                .is_err()
        );
        assert!(
            policy
                .check_change("admin", Some("超级管理员"), old)
                .is_err()
        );
        policy
            .check_change("engineer", Some("研发工程师"), old)
            .unwrap();
    }
    #[test]
    fn configuration_is_bounded_and_can_override_defaults() {
        let mut policy = NamePolicy {
            username_keywords: vec!["operator".into()],
            display_name_keywords: vec![],
        };
        policy.validate().unwrap();
        policy.check_change("admin", Some("管理员"), None).unwrap();
        assert!(policy.check_change("operator01", None, None).is_err());
        for invalid in [
            vec![" ".into()],
            vec!["\u{200b}".into()],
            vec!["x".repeat(65)],
            vec!["admin".into(), "ＡＤＭＩＮ".into()],
            vec!["word".into(); 129],
        ] {
            policy.username_keywords = invalid;
            assert!(policy.validate().is_err());
        }
    }
}
