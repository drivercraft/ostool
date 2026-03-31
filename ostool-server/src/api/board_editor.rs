use schemars::{JsonSchema, Schema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    api::{error::ApiError, models::SerialPortSummary},
    config::{BoardConfig, BootConfig, PxeProfile, SerialConfig, UbootProfile},
};

const DEFAULT_SERIAL_BAUD_RATE: u32 = 115_200;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BoardBootKind {
    #[default]
    Uboot,
    Pxe,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, PartialEq, Eq)]
pub struct BoardEditorUbootData {
    #[serde(default)]
    pub use_tftp: bool,
    #[serde(default)]
    pub kernel_load_addr: String,
    #[serde(default)]
    pub fit_load_addr: String,
    #[serde(default)]
    pub board_reset_cmd: String,
    #[serde(default)]
    pub board_power_off_cmd: String,
    #[serde(default)]
    pub success_regex_text: String,
    #[serde(default)]
    pub fail_regex_text: String,
    #[serde(default)]
    pub uboot_cmd_text: String,
    #[serde(default)]
    pub shell_prefix: String,
    #[serde(default)]
    pub shell_init_cmd: String,
    pub timeout: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, PartialEq, Eq)]
pub struct BoardEditorPxeData {
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct BoardEditorData {
    pub id: String,
    pub name: String,
    pub board_type: String,
    #[serde(default)]
    pub tags_text: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub serial_enabled: bool,
    #[serde(default)]
    pub serial_port: String,
    #[serde(default = "default_serial_baud_rate")]
    pub serial_baud_rate: u32,
    #[serde(default)]
    pub boot_kind: BoardBootKind,
    #[serde(default)]
    pub uboot: BoardEditorUbootData,
    #[serde(default)]
    pub pxe: BoardEditorPxeData,
}

impl Default for BoardEditorData {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            board_type: String::new(),
            tags_text: String::new(),
            notes: String::new(),
            disabled: false,
            serial_enabled: false,
            serial_port: String::new(),
            serial_baud_rate: default_serial_baud_rate(),
            boot_kind: BoardBootKind::default(),
            uboot: BoardEditorUbootData::default(),
            pxe: BoardEditorPxeData::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardEditorDocument {
    pub data: BoardEditorData,
    pub schema: Schema,
}

impl BoardEditorData {
    pub fn from_board_config(board: &BoardConfig) -> Self {
        let mut data = Self {
            id: board.id.clone(),
            name: board.name.clone(),
            board_type: board.board_type.clone(),
            tags_text: join_tags(&board.tags),
            notes: board.notes.clone().unwrap_or_default(),
            disabled: board.disabled,
            serial_enabled: board.serial.is_some(),
            serial_port: board
                .serial
                .as_ref()
                .map(|serial| serial.port.clone())
                .unwrap_or_default(),
            serial_baud_rate: board
                .serial
                .as_ref()
                .map(|serial| serial.baud_rate)
                .unwrap_or(DEFAULT_SERIAL_BAUD_RATE),
            boot_kind: BoardBootKind::default(),
            uboot: BoardEditorUbootData::default(),
            pxe: BoardEditorPxeData::default(),
        };

        match &board.boot {
            BootConfig::Uboot(profile) => {
                data.boot_kind = BoardBootKind::Uboot;
                data.uboot = BoardEditorUbootData {
                    use_tftp: profile.use_tftp,
                    kernel_load_addr: profile.kernel_load_addr.clone().unwrap_or_default(),
                    fit_load_addr: profile.fit_load_addr.clone().unwrap_or_default(),
                    board_reset_cmd: profile.board_reset_cmd.clone().unwrap_or_default(),
                    board_power_off_cmd: profile.board_power_off_cmd.clone().unwrap_or_default(),
                    success_regex_text: join_lines(&profile.success_regex),
                    fail_regex_text: join_lines(&profile.fail_regex),
                    uboot_cmd_text: join_lines(profile.uboot_cmd.as_deref().unwrap_or(&[])),
                    shell_prefix: profile.shell_prefix.clone().unwrap_or_default(),
                    shell_init_cmd: profile.shell_init_cmd.clone().unwrap_or_default(),
                    timeout: profile.timeout,
                };
            }
            BootConfig::Pxe(profile) => {
                data.boot_kind = BoardBootKind::Pxe;
                data.pxe = BoardEditorPxeData {
                    notes: profile.notes.clone().unwrap_or_default(),
                };
            }
        }

        data
    }

    pub fn to_board_config(&self) -> BoardConfig {
        let serial = self.serial_enabled.then(|| SerialConfig {
            port: self.serial_port.trim().to_string(),
            baud_rate: self.serial_baud_rate,
        });

        let boot = match self.boot_kind {
            BoardBootKind::Uboot => BootConfig::Uboot(UbootProfile {
                use_tftp: self.uboot.use_tftp,
                kernel_load_addr: empty_to_none(&self.uboot.kernel_load_addr),
                fit_load_addr: empty_to_none(&self.uboot.fit_load_addr),
                board_reset_cmd: empty_to_none(&self.uboot.board_reset_cmd),
                board_power_off_cmd: empty_to_none(&self.uboot.board_power_off_cmd),
                success_regex: parse_lines(&self.uboot.success_regex_text),
                fail_regex: parse_lines(&self.uboot.fail_regex_text),
                uboot_cmd: parse_optional_lines(&self.uboot.uboot_cmd_text),
                shell_prefix: empty_to_none(&self.uboot.shell_prefix),
                shell_init_cmd: empty_to_none(&self.uboot.shell_init_cmd),
                timeout: self.uboot.timeout,
            }),
            BoardBootKind::Pxe => BootConfig::Pxe(PxeProfile {
                notes: empty_to_none(&self.pxe.notes),
            }),
        };

        BoardConfig {
            id: self.id.trim().to_string(),
            name: self.name.trim().to_string(),
            board_type: self.board_type.trim().to_string(),
            tags: parse_tags(&self.tags_text),
            serial,
            boot,
            notes: empty_to_none(&self.notes),
            disabled: self.disabled,
        }
    }

    pub fn validate(&self) -> Result<(), ApiError> {
        if self.id.trim().is_empty() {
            return Err(ApiError::bad_request("board id must not be empty"));
        }
        if self.id.contains('/') || self.id.contains('\\') {
            return Err(ApiError::bad_request(
                "board id must not contain path separators",
            ));
        }
        if self.name.trim().is_empty() {
            return Err(ApiError::bad_request("board name must not be empty"));
        }
        if self.board_type.trim().is_empty() {
            return Err(ApiError::bad_request("board_type must not be empty"));
        }
        if self.serial_enabled && self.serial_port.trim().is_empty() {
            return Err(ApiError::bad_request(
                "serial_port must not be empty when serial is enabled",
            ));
        }
        if self.serial_enabled && self.serial_baud_rate == 0 {
            return Err(ApiError::bad_request(
                "serial_baud_rate must be > 0 when serial is enabled",
            ));
        }
        Ok(())
    }
}

pub fn build_board_editor_document(
    data: BoardEditorData,
    serial_ports: &[SerialPortSummary],
    current_serial_port: Option<&str>,
) -> BoardEditorDocument {
    BoardEditorDocument {
        schema: build_board_editor_schema(serial_ports, current_serial_port),
        data,
    }
}

pub fn build_board_editor_schema(
    serial_ports: &[SerialPortSummary],
    current_serial_port: Option<&str>,
) -> Schema {
    let mut schema = schema_for!(BoardEditorData).to_value();

    set_property_value(
        &mut schema,
        &["properties", "id"],
        json!({ "minLength": 1 }),
    );
    set_property_value(
        &mut schema,
        &["properties", "name"],
        json!({ "minLength": 1 }),
    );
    set_property_value(
        &mut schema,
        &["properties", "board_type"],
        json!({ "minLength": 1 }),
    );
    set_property_value(
        &mut schema,
        &["properties", "tags_text"],
        json!({ "default": "" }),
    );
    set_property_value(
        &mut schema,
        &["properties", "notes"],
        json!({ "default": "" }),
    );
    set_property_value(
        &mut schema,
        &["properties", "disabled"],
        json!({ "default": false }),
    );
    set_property_value(
        &mut schema,
        &["properties", "serial_enabled"],
        json!({ "default": false }),
    );
    set_property_value(
        &mut schema,
        &["properties", "serial_baud_rate"],
        json!({ "default": DEFAULT_SERIAL_BAUD_RATE, "minimum": 1 }),
    );
    set_property_value(
        &mut schema,
        &["properties", "boot_kind"],
        json!({ "default": "uboot" }),
    );
    set_property_value(
        &mut schema,
        &["properties", "uboot", "properties", "success_regex_text"],
        json!({ "default": "" }),
    );
    set_property_value(
        &mut schema,
        &["properties", "uboot", "properties", "fail_regex_text"],
        json!({ "default": "" }),
    );
    set_property_value(
        &mut schema,
        &["properties", "uboot", "properties", "uboot_cmd_text"],
        json!({ "default": "" }),
    );
    set_property_value(
        &mut schema,
        &["properties", "pxe", "properties", "notes"],
        json!({ "default": "" }),
    );

    let serial_options = collect_serial_options(serial_ports, current_serial_port);
    if !serial_options.is_empty() {
        set_property_value(
            &mut schema,
            &["properties", "serial_port"],
            json!({
                "oneOf": serial_options
                    .into_iter()
                    .map(|(value, title)| json!({ "const": value, "title": title }))
                    .collect::<Vec<_>>(),
            }),
        );
    }

    Schema::try_from(schema).expect("generated board editor schema must be valid")
}

fn set_property_value(schema: &mut Value, path: &[&str], patch: Value) {
    let Some(pointer) = resolve_schema_pointer(schema, path) else {
        return;
    };
    let Some(target) = schema.pointer_mut(&pointer) else {
        return;
    };
    merge_value(target, patch);
}

fn resolve_schema_pointer(schema: &Value, path: &[&str]) -> Option<String> {
    let mut pointer = String::new();
    let mut index = 0;

    loop {
        let node = if pointer.is_empty() {
            schema
        } else {
            schema.pointer(&pointer)?
        };

        if let Some(reference) = node.get("$ref").and_then(Value::as_str) {
            pointer = reference.strip_prefix('#')?.to_string();
            continue;
        }

        if index == path.len() {
            return Some(pointer);
        }

        pointer.push('/');
        pointer.push_str(&escape_json_pointer(path[index]));
        index += 1;
    }
}

fn escape_json_pointer(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

fn merge_value(target: &mut Value, patch: Value) {
    match (target, patch) {
        (Value::Object(target), Value::Object(patch)) => {
            for (key, value) in patch {
                match target.get_mut(&key) {
                    Some(existing) => merge_value(existing, value),
                    None => {
                        target.insert(key, value);
                    }
                }
            }
        }
        (target, patch) => *target = patch,
    }
}

fn collect_serial_options(
    serial_ports: &[SerialPortSummary],
    current_serial_port: Option<&str>,
) -> Vec<(String, String)> {
    let mut options = serial_ports
        .iter()
        .map(|port| (port.port_name.clone(), port.label.clone()))
        .collect::<Vec<_>>();

    if let Some(current_serial_port) = current_serial_port
        && !current_serial_port.trim().is_empty()
        && !options
            .iter()
            .any(|(value, _)| value == current_serial_port)
    {
        options.insert(
            0,
            (
                current_serial_port.to_string(),
                format!("{current_serial_port} (当前配置，未检测到)"),
            ),
        );
    }

    options
}

fn default_serial_baud_rate() -> u32 {
    DEFAULT_SERIAL_BAUD_RATE
}

fn empty_to_none(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn parse_tags(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn join_tags(tags: &[String]) -> String {
    tags.join(", ")
}

fn parse_lines(value: &str) -> Vec<String> {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_optional_lines(value: &str) -> Option<Vec<String>> {
    let lines = parse_lines(value);
    (!lines.is_empty()).then_some(lines)
}

fn join_lines(lines: &[String]) -> String {
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::{BoardBootKind, BoardEditorData, BoardEditorUbootData, build_board_editor_schema};
    use crate::{
        api::models::SerialPortSummary,
        config::{BoardConfig, BootConfig, SerialConfig, UbootProfile},
    };

    #[test]
    fn board_editor_round_trip_preserves_serial_and_multiline_fields() {
        let board = BoardConfig {
            id: "demo-board".into(),
            name: "Demo Board".into(),
            board_type: "rk3568".into(),
            tags: vec!["lab".into(), "usb".into()],
            serial: Some(SerialConfig {
                port: "/dev/ttyUSB0".into(),
                baud_rate: 1_500_000,
            }),
            boot: BootConfig::Uboot(UbootProfile {
                use_tftp: true,
                kernel_load_addr: Some("0x80200000".into()),
                fit_load_addr: None,
                board_reset_cmd: Some("reboot".into()),
                board_power_off_cmd: None,
                success_regex: vec!["booted".into(), "login:".into()],
                fail_regex: vec!["panic".into()],
                uboot_cmd: Some(vec!["setenv foo bar".into(), "bootm".into()]),
                shell_prefix: Some("=>".into()),
                shell_init_cmd: None,
                timeout: Some(30),
            }),
            notes: Some("rack-1".into()),
            disabled: false,
        };

        let data = BoardEditorData::from_board_config(&board);
        assert_eq!(data.tags_text, "lab, usb");
        assert!(data.serial_enabled);
        assert_eq!(data.serial_port, "/dev/ttyUSB0");
        assert_eq!(data.serial_baud_rate, 1_500_000);
        assert_eq!(data.boot_kind, BoardBootKind::Uboot);
        assert_eq!(data.uboot.success_regex_text, "booted\nlogin:");
        assert_eq!(data.uboot.uboot_cmd_text, "setenv foo bar\nbootm");

        let round_trip = data.to_board_config();
        assert_eq!(round_trip.tags, vec!["lab", "usb"]);
        assert_eq!(round_trip.notes.as_deref(), Some("rack-1"));
        assert_eq!(
            round_trip.serial.as_ref().map(|item| item.port.as_str()),
            Some("/dev/ttyUSB0")
        );

        let BootConfig::Uboot(profile) = round_trip.boot else {
            panic!("expected uboot profile");
        };
        assert_eq!(profile.success_regex, vec!["booted", "login:"]);
        assert_eq!(
            profile.uboot_cmd,
            Some(vec!["setenv foo bar".into(), "bootm".into()])
        );
        assert_eq!(profile.timeout, Some(30));
    }

    #[test]
    fn empty_text_fields_become_none_and_serial_can_be_disabled() {
        let mut data = BoardEditorData {
            id: "demo-board".into(),
            name: "Demo Board".into(),
            board_type: "rk3568".into(),
            serial_enabled: false,
            boot_kind: BoardBootKind::Uboot,
            uboot: BoardEditorUbootData {
                use_tftp: false,
                kernel_load_addr: " ".into(),
                fit_load_addr: String::new(),
                board_reset_cmd: String::new(),
                board_power_off_cmd: String::new(),
                success_regex_text: String::new(),
                fail_regex_text: "panic\n".into(),
                uboot_cmd_text: "\n".into(),
                shell_prefix: " ".into(),
                shell_init_cmd: String::new(),
                timeout: None,
            },
            ..BoardEditorData::default()
        };
        data.notes = " ".into();
        data.pxe.notes = " ".into();

        let board = data.to_board_config();
        assert!(board.serial.is_none());
        assert!(board.notes.is_none());

        let BootConfig::Uboot(profile) = board.boot else {
            panic!("expected uboot profile");
        };
        assert!(profile.kernel_load_addr.is_none());
        assert_eq!(profile.fail_regex, vec!["panic"]);
        assert!(profile.uboot_cmd.is_none());
        assert!(profile.shell_prefix.is_none());
    }

    #[test]
    fn schema_includes_current_serial_port_when_not_detected() {
        let schema = build_board_editor_schema(
            &[SerialPortSummary {
                port_name: "/dev/ttyUSB1".into(),
                port_type: "usb".into(),
                label: "USB Serial".into(),
                usb_vendor_id: None,
                usb_product_id: None,
                manufacturer: None,
                product: None,
                serial_number: None,
            }],
            Some("/dev/ttyUSB9"),
        );

        let one_of = schema
            .as_value()
            .pointer("/properties/serial_port/oneOf")
            .and_then(Value::as_array)
            .expect("serial_port oneOf");

        assert_eq!(one_of[0]["const"], "/dev/ttyUSB9");
        assert_eq!(one_of[0]["title"], "/dev/ttyUSB9 (当前配置，未检测到)");
    }
}
