use std::time::Duration;

use anyhow::Context;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_modbus::{
    Slave,
    client::{Client, Writer, rtu},
};
use tokio_serial::{DataBits, Parity, SerialPortBuilderExt, StopBits};

use crate::{
    config::{BoardConfig, CustomPowerManagement, PowerManagementConfig},
    process::run_shell_command,
};

const ZHONGSHENG_RELAY_BAUD_RATE: u32 = 38_400;
const ZHONGSHENG_RELAY_SLAVE_ID: u8 = 1;
const ZHONGSHENG_RELAY_COIL_ADDRESS: u16 = 0;
const ZHONGSHENG_RELAY_TIMEOUT: Duration = Duration::from_secs(1);

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

pub async fn execute_power_action_for_board(
    board: &BoardConfig,
    action: PowerAction,
) -> Result<String, PowerActionError> {
    execute_power_action(&board.power_management, action).await
}

pub async fn execute_power_action(
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
            run_shell_command(command).await?;
            Ok(format!("executed `{command}`"))
        }
        PowerManagementConfig::ZhongshengRelay(relay) => {
            if relay.serial_port.trim().is_empty() {
                return Err(PowerActionError::InvalidConfig(
                    "board power management relay serial port is not configured".to_string(),
                ));
            }
            run_zhongsheng_relay_action(&relay.serial_port, action).await?;
            Ok(format!(
                "executed Zhongsheng relay {} via {}",
                action.label(),
                relay.serial_port
            ))
        }
    }
}

async fn run_zhongsheng_relay_action(serial_port: &str, action: PowerAction) -> anyhow::Result<()> {
    let transport = tokio_serial::new(serial_port, ZHONGSHENG_RELAY_BAUD_RATE)
        .data_bits(DataBits::Eight)
        .exclusive(false)
        .parity(Parity::None)
        .stop_bits(StopBits::One)
        .timeout(ZHONGSHENG_RELAY_TIMEOUT)
        .open_native_async()
        .with_context(|| format!("failed to open relay serial port {serial_port}"))?;

    write_zhongsheng_relay_action(transport, action).await
}

async fn write_zhongsheng_relay_action<T>(transport: T, action: PowerAction) -> anyhow::Result<()>
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut context = rtu::attach_slave(transport, Slave(ZHONGSHENG_RELAY_SLAVE_ID));
    let coil = matches!(action, PowerAction::On);

    tokio::time::timeout(
        ZHONGSHENG_RELAY_TIMEOUT,
        context.write_single_coil(ZHONGSHENG_RELAY_COIL_ADDRESS, coil),
    )
    .await
    .context("timed out writing Zhongsheng relay coil")?
    .context("failed to write Zhongsheng relay coil")?
    .context("Zhongsheng relay rejected coil write")?;

    if let Err(err) = context.disconnect().await {
        log::debug!("failed to close Zhongsheng relay Modbus session: {err}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{future, time::Duration};

    use tokio::sync::{mpsc, oneshot};
    use tokio_modbus::{
        ExceptionCode, Request, Response, SlaveRequest,
        server::{Service, rtu::Server},
    };

    use super::{
        PowerAction, ZHONGSHENG_RELAY_COIL_ADDRESS, ZHONGSHENG_RELAY_SLAVE_ID,
        execute_power_action_for_board, write_zhongsheng_relay_action,
    };
    use crate::config::{
        BoardConfig, BootConfig, CustomPowerManagement, PowerManagementConfig, PxeProfile,
    };

    fn board_with_power_management(power_management: PowerManagementConfig) -> BoardConfig {
        BoardConfig {
            id: "demo".into(),
            board_type: "demo".into(),
            tags: vec![],
            serial: None,
            power_management,
            boot: BootConfig::Pxe(PxeProfile::default()),
            notes: None,
            disabled: false,
        }
    }

    #[tokio::test]
    async fn custom_power_management_executes_commands() {
        let board =
            board_with_power_management(PowerManagementConfig::Custom(CustomPowerManagement {
                power_on_cmd: "printf power-on >/dev/null".into(),
                power_off_cmd: "printf power-off >/dev/null".into(),
            }));

        let message = execute_power_action_for_board(&board, PowerAction::On)
            .await
            .unwrap();
        assert_eq!(message, "executed `printf power-on >/dev/null`");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn relay_power_management_writes_single_coil_for_power_off() {
        let (client, server, mut requests, stop_tx) = spawn_relay_test_server();

        write_zhongsheng_relay_action(client, PowerAction::Off)
            .await
            .unwrap();

        let request = tokio::time::timeout(Duration::from_secs(1), requests.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            request,
            (
                ZHONGSHENG_RELAY_SLAVE_ID,
                ZHONGSHENG_RELAY_COIL_ADDRESS,
                false
            )
        );

        let _ = stop_tx.send(());
        let _ = server.await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn relay_power_management_writes_single_coil_for_power_on() {
        let (client, server, mut requests, stop_tx) = spawn_relay_test_server();

        write_zhongsheng_relay_action(client, PowerAction::On)
            .await
            .unwrap();

        let request = tokio::time::timeout(Duration::from_secs(1), requests.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            request,
            (
                ZHONGSHENG_RELAY_SLAVE_ID,
                ZHONGSHENG_RELAY_COIL_ADDRESS,
                true
            )
        );

        let _ = stop_tx.send(());
        let _ = server.await.unwrap();
    }

    #[derive(Clone)]
    struct RecordingRelayService {
        requests: mpsc::UnboundedSender<(u8, u16, bool)>,
    }

    impl Service for RecordingRelayService {
        type Request = SlaveRequest<'static>;
        type Response = Response;
        type Exception = ExceptionCode;
        type Future = future::Ready<std::result::Result<Self::Response, Self::Exception>>;

        fn call(&self, req: Self::Request) -> Self::Future {
            match req.request {
                Request::WriteSingleCoil(address, coil) => {
                    self.requests.send((req.slave, address, coil)).unwrap();
                    future::ready(Ok(Response::WriteSingleCoil(address, coil)))
                }
                _ => future::ready(Err(ExceptionCode::IllegalFunction)),
            }
        }
    }

    #[cfg(unix)]
    fn spawn_relay_test_server() -> (
        tokio_serial::SerialStream,
        tokio::task::JoinHandle<std::io::Result<tokio_modbus::server::Terminated>>,
        mpsc::UnboundedReceiver<(u8, u16, bool)>,
        oneshot::Sender<()>,
    ) {
        let (client, server) = tokio_serial::SerialStream::pair().unwrap();
        let (request_tx, request_rx) = mpsc::unbounded_channel();
        let (stop_tx, stop_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            Server::new(server)
                .serve_until(
                    RecordingRelayService {
                        requests: request_tx,
                    },
                    async move {
                        let _ = stop_rx.await;
                    },
                )
                .await
        });

        (client, task, request_rx, stop_tx)
    }
}
