//! Session-owned UART runtime. Only this task reads a transferred lease.
use crate::{
    config::{AxloaderSerialParameters, BootConfig},
    session::{SessionState, SessionStopReason},
    state::AppState,
};
use anyhow::Context;
use httpboot_protocol::{LoaderDeviceStatus, SerialBinding, SerialBindingMode, SerialParameters};
use ostool_serial::{BindRequest, SerialLease};
use serde::{Deserialize, Serialize};
use std::{
    future::{Future, pending},
    pin::Pin,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    sync::{Mutex, mpsc, oneshot, watch},
    time::Instant,
};

const SERIAL_BIND_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SerialRuntimeStatus {
    pub phase: SerialRuntimePhase,
    pub port: Option<String>,
    pub parameters: Option<SerialParameters>,
    pub boot_epoch: Option<String>,
    pub binding_id: Option<String>,
    pub error: Option<String>,
    pub warning: Option<String>,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SerialRuntimePhase {
    #[default]
    WaitingDevice,
    Verifying,
    Discovering,
    Bound,
    Recovering,
    Failed,
    Closed,
}
enum Command {
    Bind(
        Box<LoaderDeviceStatus>,
        oneshot::Sender<anyhow::Result<SerialBinding>>,
    ),
    Confirm(String, oneshot::Sender<anyhow::Result<()>>),
    Fail(String),
    Restart(bool, oneshot::Sender<()>),
}
#[derive(Debug)]
pub struct SessionSerialRuntime {
    commands: mpsc::UnboundedSender<Command>,
    receiver: Mutex<Option<mpsc::UnboundedReceiver<Command>>>,
    status: watch::Sender<SerialRuntimeStatus>,
    retired_epoch: std::sync::Mutex<Option<String>>,
}
impl Default for SessionSerialRuntime {
    fn default() -> Self {
        let (commands, receiver) = mpsc::unbounded_channel();
        Self {
            commands,
            receiver: Mutex::new(Some(receiver)),
            status: watch::channel(SerialRuntimeStatus::default()).0,
            retired_epoch: std::sync::Mutex::new(None),
        }
    }
}
impl SessionSerialRuntime {
    pub fn accepts_epoch(&self, epoch: &str) -> bool {
        self.retired_epoch
            .lock()
            .expect("serial epoch lock")
            .as_deref()
            != Some(epoch)
    }
    pub async fn restart(&self, powered: bool) {
        if self.receiver.lock().await.is_some() {
            return;
        }
        let epoch = self.snapshot().boot_epoch;
        if epoch.is_some() {
            *self.retired_epoch.lock().expect("serial epoch lock") = epoch;
        }
        let (reply, rx) = oneshot::channel();
        if self.commands.send(Command::Restart(powered, reply)).is_ok() {
            let _ = rx.await;
        }
    }
    pub fn snapshot(&self) -> SerialRuntimeStatus {
        self.status.borrow().clone()
    }
    pub async fn bind(&self, device: LoaderDeviceStatus) -> anyhow::Result<SerialBinding> {
        let (reply, rx) = oneshot::channel();
        self.commands
            .send(Command::Bind(Box::new(device), reply))
            .map_err(|_| anyhow::anyhow!("serial runtime closed"))?;
        rx.await.context("serial binding cancelled")?
    }
    pub async fn confirm(&self, binding_id: &str) -> anyhow::Result<()> {
        let (reply, rx) = oneshot::channel();
        self.commands
            .send(Command::Confirm(binding_id.into(), reply))
            .map_err(|_| anyhow::anyhow!("serial runtime closed"))?;
        rx.await.context("serial confirmation cancelled")?
    }
    pub fn fail(&self, error: String) {
        let _ = self.commands.send(Command::Fail(error));
    }
    pub async fn attach(
        self: &Arc<Self>,
        state: AppState,
        session: Arc<SessionState>,
    ) -> anyhow::Result<DuplexStream> {
        let commands = self
            .receiver
            .lock()
            .await
            .take()
            .context("serial runtime already attached")?;
        let (client, io) = tokio::io::duplex(256 * 1024);
        let runtime = self.clone();
        tokio::spawn(async move {
            let result = runtime.run(&state, &session, commands, io).await;
            let mut status = runtime.snapshot();
            status.phase = if result.is_err() {
                SerialRuntimePhase::Failed
            } else {
                SerialRuntimePhase::Closed
            };
            if let Err(error) = &result {
                status.error = Some(format!("{error:#}"));
            }
            runtime.status.send_replace(status);
            state.admin_events.invalidate(&["sessions"]);
            if result.is_err() {
                session.request_stop(SessionStopReason::SerialClosed);
            }
        });
        Ok(client)
    }
    async fn run(
        &self,
        state: &AppState,
        session: &Arc<SessionState>,
        mut commands: mpsc::UnboundedReceiver<Command>,
        io: DuplexStream,
    ) -> anyhow::Result<()> {
        let (mut read_client, mut write_client) = tokio::io::split(io);
        let (input_tx, mut input) = mpsc::channel::<Vec<u8>>(64);
        let (output, mut output_rx) = mpsc::channel::<Vec<u8>>(64);
        let input_worker = async {
            let mut bytes = [0; 4096];
            loop {
                let n = read_client.read(&mut bytes).await?;
                if n == 0 {
                    return Ok::<(), anyhow::Error>(());
                }
                input_tx
                    .send(bytes[..n].to_vec())
                    .await
                    .context("serial input closed")?;
            }
        };
        let (result_tx, result_rx) = oneshot::channel();
        let owner_worker = async {
            let result = self
                .run_owner(state, session, &mut commands, &mut input, output)
                .await;
            let _ = result_tx.send(result);
            pending::<anyhow::Result<()>>().await
        };
        let output_worker = async {
            while let Some(bytes) = output_rx.recv().await {
                write_client.write_all(&bytes).await?;
            }
            result_rx
                .await
                .context("serial owner stopped without result")?
        };
        tokio::select! { result=owner_worker=>result, result=input_worker=>result, result=output_worker=>result }
    }
    async fn run_owner(
        &self,
        state: &AppState,
        session: &Arc<SessionState>,
        commands: &mut mpsc::UnboundedReceiver<Command>,
        input: &mut mpsc::Receiver<Vec<u8>>,
        output: mpsc::Sender<Vec<u8>>,
    ) -> anyhow::Result<()> {
        type BindingFuture =
            Pin<Box<dyn Future<Output = Result<SerialLease, ostool_serial::BindError>> + Send>>;
        let owner = session.snapshot().await.id;
        let mut shutdown = session.subscribe_shutdown();
        let mut lease: Option<SerialLease> = None;
        let mut opening: Option<BindingFuture> = None;
        let mut current: Option<BindingAttempt> = None;
        let mut generation = 0_u64;
        let mut deadline = Instant::now() + Duration::from_secs(60);
        let mut power_wait = true;
        let mut verifying_until: Option<Instant> = None;
        let mut timer = tokio::time::interval(Duration::from_millis(25));
        let mut bytes = [0; 4096];
        loop {
            let event = tokio::select! {
                _=async {let _=shutdown.wait_for(|s|*s).await;}=>RuntimeEvent::Shutdown,
                command=commands.recv()=>RuntimeEvent::Command(command),
                result=async {match opening.as_mut(){Some(f)=>f.await,None=>pending().await}}=>RuntimeEvent::Opened(Box::new(result)),
                result=async {match lease.as_mut(){Some(l)=>l.read(&mut bytes).await,None=>pending().await}}=>RuntimeEvent::Read(result),
                Some(payload)=input.recv(),if lease.is_some()=>RuntimeEvent::Input(payload),
                _=timer.tick()=>RuntimeEvent::Tick,
            };
            let event = match event {
                RuntimeEvent::Input(payload) => {
                    match tokio::time::timeout(
                        Duration::from_secs(1),
                        lease.as_mut().expect("live lease").write_all(&payload),
                    )
                    .await
                    {
                        Ok(Ok(())) => continue,
                        Ok(Err(error)) => RuntimeEvent::Lost(error.to_string()),
                        Err(_) => RuntimeEvent::Lost("serial write timed out".into()),
                    }
                }
                RuntimeEvent::Read(Ok(0)) => RuntimeEvent::Lost("serial reader closed".into()),
                RuntimeEvent::Read(Err(error)) => RuntimeEvent::Lost(error.to_string()),
                other => other,
            };
            match event {
                RuntimeEvent::Shutdown => return Ok(()),
                RuntimeEvent::Command(command) => match command {
                    None => return Ok(()),
                    Some(Command::Fail(error)) => anyhow::bail!(error),
                    Some(Command::Restart(powered, reply)) => {
                        current = None;
                        opening = None;
                        verifying_until = None;
                        power_wait = powered;
                        deadline = Instant::now() + SERIAL_BIND_TIMEOUT;
                        self.publish(
                            state,
                            SerialRuntimeStatus {
                                port: lease.as_ref().map(|l| l.locator().name.clone()),
                                ..Default::default()
                            },
                        );
                        let _ = reply.send(());
                    }
                    Some(Command::Confirm(id, reply)) => {
                        let valid = current
                            .as_ref()
                            .is_some_and(|c| c.binding.binding_id == id && c.reply.is_none())
                            && lease.is_some();
                        if valid {
                            lease.as_ref().expect("checked lease").confirm();
                            let mut status = self.snapshot();
                            status.phase = SerialRuntimePhase::Bound;
                            self.publish(state, status);
                        }
                        let _ = reply.send(if valid {
                            Ok(())
                        } else {
                            Err(anyhow::anyhow!("stale serial confirmation"))
                        });
                    }
                    Some(Command::Bind(device, reply)) => {
                        let device = *device;
                        if !self.accepts_epoch(&device.boot_epoch) {
                            let _ =
                                reply.send(Err(anyhow::anyhow!("waiting for a new boot epoch")));
                            continue;
                        }
                        power_wait = true;
                        // Each valid device report is progress.  Refresh the wait
                        // window before handling recoverable reports such as
                        // `ready = false`, so repeated diagnostics do not expire
                        // the session while axloader is still booting.
                        deadline = Instant::now() + SERIAL_BIND_TIMEOUT;
                        let Some(serial) = device.serial.as_ref() else {
                            self.reject_bind(
                                state,
                                &device,
                                reply,
                                anyhow::anyhow!("device has no v6 serial parameters"),
                            );
                            continue;
                        };
                        if !serial.ready {
                            self.reject_bind(
                                state,
                                &device,
                                reply,
                                anyhow::anyhow!("automatic serial unavailable: {:?}", serial.error),
                            );
                            continue;
                        }
                        // A saved Web UI profile is an explicit host-side override. Otherwise
                        // use the current boot's report, falling back to the conventional UART
                        // profile when firmware has a usable port but cannot report its mode.
                        let parameters = effective_parameters(
                            session
                                .board()
                                .boot
                                .as_uefi_http()
                                .and_then(|profile| profile.serial_parameters),
                            serial.parameters,
                        );
                        let warning = serial.error.clone();
                        if let Err(error) = parameters.validate() {
                            self.reject_bind(state, &device, reply, anyhow::anyhow!("{error}"));
                            continue;
                        }
                        if !matches!(
                            session.board().power_management,
                            crate::config::PowerManagementConfig::Qemu { .. }
                        ) && let Err(error) = ostool_serial::validate_host_parameters(parameters)
                        {
                            self.reject_bind(state, &device, reply, error.into());
                            continue;
                        }
                        if let Some(c) = current.as_ref()
                            && c.device.boot_epoch == device.boot_epoch
                            && c.binding.serial_id == serial.serial_id
                            && c.reply.is_none()
                            && lease.is_some()
                        {
                            let _ = reply.send(Ok(c.binding.clone()));
                            continue;
                        }
                        generation = generation
                            .checked_add(1)
                            .context("serial generation exhausted")?;
                        opening = None;
                        let binding = SerialBinding {
                            serial_id: serial.serial_id.clone(),
                            binding_id: uuid::Uuid::new_v4().simple().to_string(),
                            mode: SerialBindingMode::Bound,
                        };
                        current = Some(BindingAttempt {
                            device,
                            parameters,
                            binding,
                            warning,
                            reply: Some(reply),
                        });
                        deadline = Instant::now() + SERIAL_BIND_TIMEOUT;
                        let c = current.as_ref().expect("current attempt");
                        if let Some(io) = lease.as_mut() {
                            if io.configure(parameters).is_err() {
                                state.serial_manager.invalidate(c.device.mac_address);
                                lease = None;
                            } else {
                                verifying_until = Some(Instant::now() + Duration::from_secs(1));
                            }
                        }
                        self.publish(
                            state,
                            SerialRuntimeStatus {
                                phase: if lease.is_some() {
                                    SerialRuntimePhase::Verifying
                                } else {
                                    SerialRuntimePhase::Discovering
                                },
                                parameters: Some(parameters),
                                boot_epoch: Some(c.device.boot_epoch.clone()),
                                port: lease.as_ref().map(|l| l.locator().name.clone()),
                                warning: c.warning.clone(),
                                ..Default::default()
                            },
                        );
                        if lease.is_none() {
                            opening = Some(bind_future(state, &owner, generation, c, deadline));
                        }
                    }
                },
                RuntimeEvent::Opened(result) => {
                    opening = None;
                    let io = (*result)?;
                    let c = current.as_mut().context("missing binding attempt")?;
                    let status = SerialRuntimeStatus {
                        phase: SerialRuntimePhase::Verifying,
                        port: Some(io.locator().name.clone()),
                        parameters: Some(c.parameters),
                        boot_epoch: Some(c.device.boot_epoch.clone()),
                        binding_id: Some(c.binding.binding_id.clone()),
                        error: None,
                        warning: c.warning.clone(),
                    };
                    lease = Some(io);
                    if let Some(reply) = c.reply.take() {
                        let _ = reply.send(Ok(c.binding.clone()));
                    }
                    self.publish(state, status);
                }
                RuntimeEvent::Lost(error) => {
                    if let Some(c) = current.as_ref() {
                        state.serial_manager.invalidate(c.device.mac_address);
                    }
                    lease = None;
                    verifying_until = None;
                    let mut status = self.snapshot();
                    status.phase = SerialRuntimePhase::Recovering;
                    status.error = Some(format!(
                        "serial IO lost: {error}; waiting for axloader identity"
                    ));
                    self.publish(state, status);
                    if let Some(c) = current.as_ref().filter(|c| c.reply.is_some()) {
                        opening = Some(bind_future(state, &owner, generation, c, deadline));
                    } else {
                        deadline = Instant::now() + Duration::from_secs(5);
                    }
                }
                RuntimeEvent::Read(Ok(size)) => {
                    if let Some(c) = current.as_mut()
                        && c.reply.is_some()
                        && lease
                            .as_mut()
                            .and_then(|l| l.take_observed_id())
                            .is_some_and(|id| id.as_slice() == c.binding.serial_id.as_bytes())
                    {
                        verifying_until = None;
                        let _ = c
                            .reply
                            .take()
                            .expect("checked reply")
                            .send(Ok(c.binding.clone()));
                        let mut status = self.snapshot();
                        status.binding_id = Some(c.binding.binding_id.clone());
                        self.publish(state, status);
                    }
                    output
                        .try_send(bytes[..size].to_vec())
                        .context("serial output buffer full or closed")?;
                }
                RuntimeEvent::Read(Err(_)) | RuntimeEvent::Input(_) => {
                    unreachable!("normalized event")
                }
                RuntimeEvent::Tick => {
                    if current
                        .as_ref()
                        .is_some_and(|c| c.reply.as_ref().is_some_and(|r| r.is_closed()))
                    {
                        current = None;
                        opening = None;
                        verifying_until = None;
                    }
                    if verifying_until.is_some_and(|time| time <= Instant::now()) {
                        verifying_until = None;
                        let c = current.as_ref().context("missing verification")?;
                        state.serial_manager.invalidate(c.device.mac_address);
                        lease = None;
                        opening = Some(bind_future(state, &owner, generation, c, deadline));
                        let mut status = self.snapshot();
                        status.phase = SerialRuntimePhase::Discovering;
                        self.publish(state, status);
                    }
                    if power_wait
                        && Instant::now() >= deadline
                        && (lease.is_none() || current.as_ref().is_some_and(|c| c.reply.is_some()))
                    {
                        anyhow::bail!(
                            "axloader serial identity did not become available before deadline"
                        );
                    }
                }
            }
        }
    }
    fn reject_bind(
        &self,
        state: &AppState,
        device: &LoaderDeviceStatus,
        reply: oneshot::Sender<anyhow::Result<SerialBinding>>,
        error: anyhow::Error,
    ) {
        let mut status = self.snapshot();
        status.phase = SerialRuntimePhase::Recovering;
        status.boot_epoch = Some(device.boot_epoch.clone());
        status.parameters = device.serial.as_ref().and_then(|serial| serial.parameters);
        status.error = Some(format!("{error:#}"));
        self.publish(state, status);
        let _ = reply.send(Err(error));
    }
    fn publish(&self, state: &AppState, status: SerialRuntimeStatus) {
        self.status.send_replace(status);
        state.admin_events.invalidate(&["sessions"]);
    }
}

trait BootConfigExt {
    fn as_uefi_http(&self) -> Option<&crate::config::UefiHttpProfile>;
}

impl BootConfigExt for BootConfig {
    fn as_uefi_http(&self) -> Option<&crate::config::UefiHttpProfile> {
        match self {
            Self::UefiHttp(profile) => Some(profile),
            _ => None,
        }
    }
}

enum RuntimeEvent {
    Shutdown,
    Command(Option<Command>),
    Opened(Box<Result<SerialLease, ostool_serial::BindError>>),
    Read(std::io::Result<usize>),
    Lost(String),
    Input(Vec<u8>),
    Tick,
}

struct BindingAttempt {
    device: LoaderDeviceStatus,
    parameters: SerialParameters,
    binding: SerialBinding,
    warning: Option<String>,
    reply: Option<oneshot::Sender<anyhow::Result<SerialBinding>>>,
}

fn effective_parameters(
    override_parameters: Option<AxloaderSerialParameters>,
    reported: Option<SerialParameters>,
) -> SerialParameters {
    override_parameters
        .map(Into::into)
        .or(reported)
        .unwrap_or_default()
}

fn bind_future(
    state: &AppState,
    owner: &str,
    generation: u64,
    c: &BindingAttempt,
    deadline: Instant,
) -> Pin<Box<dyn Future<Output = Result<SerialLease, ostool_serial::BindError>> + Send>> {
    let manager = state.serial_manager.clone();
    let request = BindRequest {
        owner: owner.into(),
        generation,
        mac_address: c.device.mac_address,
        boot_epoch: c.device.boot_epoch.clone(),
        serial_id: c.binding.serial_id.clone(),
        parameters: c.parameters,
        deadline,
    };
    Box::pin(async move { manager.bind(request).await })
}

#[cfg(test)]
mod tests {
    use super::effective_parameters;
    use crate::config::{
        AxloaderSerialFlowControl, AxloaderSerialParameters, AxloaderSerialParity,
        AxloaderSerialStopBits,
    };
    use httpboot_protocol::{SerialFlowControl, SerialParameters, SerialParity, SerialStopBits};

    fn reported(baud_rate: u64) -> SerialParameters {
        SerialParameters {
            baud_rate,
            data_bits: 8,
            parity: SerialParity::None,
            stop_bits: SerialStopBits::One,
            flow_control: SerialFlowControl::None,
        }
    }

    #[test]
    fn missing_firmware_parameters_use_common_uart_defaults() {
        assert_eq!(
            effective_parameters(None, None),
            SerialParameters::default()
        );
    }

    #[test]
    fn configured_parameters_override_firmware_report() {
        let configured = AxloaderSerialParameters {
            baud_rate: 921_600,
            data_bits: 7,
            parity: AxloaderSerialParity::Even,
            stop_bits: AxloaderSerialStopBits::Two,
            flow_control: AxloaderSerialFlowControl::RtsCts,
        };
        assert_eq!(
            effective_parameters(Some(configured), Some(reported(115_200))),
            configured.into_protocol()
        );
    }
}
