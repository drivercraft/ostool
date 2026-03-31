use thiserror::Error;

use crate::{
    config::{BoardConfig, CustomPowerManagement, PowerManagementConfig},
    process::{run_program_command, run_shell_command},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerAction {
    On,
    Off,
}

impl PowerAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::On => "power-on",
            Self::Off => "power-off",
        }
    }
}

#[derive(Debug, Error)]
pub enum PowerActionError {
    #[error("board has no power management configured")]
    NotConfigured,
    #[error("{0}")]
    InvalidConfig(String),
    #[error(transparent)]
    Execution(#[from] anyhow::Error),
}

pub fn execute_power_action_for_board(
    board: &BoardConfig,
    action: PowerAction,
) -> Result<String, PowerActionError> {
    let power_management = board
        .power_management
        .as_ref()
        .ok_or(PowerActionError::NotConfigured)?;
    execute_power_action(power_management, action)
}

pub fn execute_power_action(
    power_management: &PowerManagementConfig,
    action: PowerAction,
) -> Result<String, PowerActionError> {
    match power_management {
        PowerManagementConfig::Custom(CustomPowerManagement {
            power_on_cmd,
            power_off_cmd,
        }) => {
            let command = match action {
                PowerAction::On => power_on_cmd,
                PowerAction::Off => power_off_cmd,
            };
            if command.trim().is_empty() {
                return Err(PowerActionError::InvalidConfig(format!(
                    "board power management `{}` command is not configured",
                    action.label()
                )));
            }
            run_shell_command(command)?;
            Ok(format!("executed `{command}`"))
        }
        PowerManagementConfig::ZhongshengRelay(relay) => {
            if relay.serial_port.trim().is_empty() {
                return Err(PowerActionError::InvalidConfig(
                    "board power management relay serial port is not configured".to_string(),
                ));
            }
            run_zhongsheng_relay_action(&relay.serial_port, action)?;
            Ok(format!(
                "executed Zhongsheng relay {} via {}",
                action.label(),
                relay.serial_port
            ))
        }
    }
}

fn run_zhongsheng_relay_action(serial_port: &str, action: PowerAction) -> anyhow::Result<()> {
    let value = match action {
        PowerAction::On => "1",
        PowerAction::Off => "0",
    };
    let program = std::env::var("OSTOOL_MBPOLL_BIN").unwrap_or_else(|_| "mbpoll".to_string());
    run_program_command(
        &program,
        &[
            "-m",
            "rtu",
            "-a",
            "1",
            "-r",
            "1",
            "-t",
            "0",
            "-b",
            "38400",
            "-P",
            "none",
            "-v",
            serial_port,
            value,
        ],
    )
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::{Mutex, OnceLock},
    };

    use tempfile::tempdir;

    use super::{PowerAction, execute_power_action_for_board};
    use crate::config::{
        BoardConfig, BootConfig, CustomPowerManagement, PowerManagementConfig, PxeProfile,
        ZhongshengRelayPowerManagement,
    };

    fn board_with_power_management(power_management: PowerManagementConfig) -> BoardConfig {
        BoardConfig {
            id: "demo".into(),
            name: "Demo".into(),
            board_type: "demo".into(),
            tags: vec![],
            serial: None,
            power_management: Some(power_management),
            boot: BootConfig::Pxe(PxeProfile::default()),
            notes: None,
            disabled: false,
        }
    }

    #[test]
    fn custom_power_management_executes_commands() {
        let board =
            board_with_power_management(PowerManagementConfig::Custom(CustomPowerManagement {
                power_on_cmd: "printf power-on >/dev/null".into(),
                power_off_cmd: "printf power-off >/dev/null".into(),
            }));

        let message = execute_power_action_for_board(&board, PowerAction::On).unwrap();
        assert_eq!(message, "executed `printf power-on >/dev/null`");
    }

    #[test]
    fn relay_power_management_executes_mbpoll_override() {
        static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let _guard = ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("lock env");

        let dir = tempdir().unwrap();
        let output_path = dir.path().join("mbpoll.log");
        let script_path = dir.path().join("mbpoll-mock.sh");
        fs::write(
            &script_path,
            format!(
                "#!/bin/sh\nprintf '%s' \"$*\" > {}\n",
                output_path.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&script_path).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&script_path, permissions).unwrap();
        }

        let board = board_with_power_management(PowerManagementConfig::ZhongshengRelay(
            ZhongshengRelayPowerManagement {
                serial_port: "/dev/ttyUSB7".into(),
            },
        ));

        unsafe {
            std::env::set_var("OSTOOL_MBPOLL_BIN", &script_path);
        }
        let message = execute_power_action_for_board(&board, PowerAction::Off).unwrap();
        unsafe {
            std::env::remove_var("OSTOOL_MBPOLL_BIN");
        }

        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "-m rtu -a 1 -r 1 -t 0 -b 38400 -P none -v /dev/ttyUSB7 0"
        );
        assert_eq!(
            message,
            "executed Zhongsheng relay power-off via /dev/ttyUSB7"
        );
    }
}
