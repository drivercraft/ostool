use std::{
    collections::{BTreeMap, VecDeque},
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, bail};
use httpboot_protocol::MacAddress;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    net::{TcpListener, UnixStream},
    process::{Child, Command},
    sync::{Mutex, RwLock, broadcast, mpsc, watch},
    task::JoinHandle,
    time::{Instant, sleep, timeout},
};

use crate::config::VirtualQemuConfig;

const SERIAL_HISTORY_LIMIT: usize = 64 * 1024;
const QEMU_START_TIMEOUT: Duration = Duration::from_secs(10);
const QEMU_STOP_TIMEOUT: Duration = Duration::from_secs(3);
const VIRTUAL_SERVER_IP: Ipv4Addr = Ipv4Addr::new(10, 77, 0, 1);

#[derive(Debug, Clone)]
pub struct VirtualDeviceSnapshot {
    pub id: String,
    pub mac_address: MacAddress,
    pub tap: String,
    pub powered: bool,
    pub serial_connected: bool,
    pub generation: u64,
}

#[derive(Clone)]
pub struct VirtualBoardManager {
    events: crate::admin_events::AdminEvents,
    config: Arc<VirtualQemuConfig>,
    devices: Arc<RwLock<BTreeMap<String, Arc<VirtualDevice>>>>,
}

struct VirtualDevice {
    id: String,
    mac_address: MacAddress,
    tap: String,
    hub: SerialHub,
    runtime: Mutex<VirtualRuntime>,
}

struct VirtualRuntime {
    child: Option<Child>,
    deleting: bool,
    generation: u64,
    run_dir: PathBuf,
    qmp_path: PathBuf,
}

#[derive(Clone)]
struct SerialHub {
    address: SocketAddr,
    history: Arc<Mutex<VecDeque<u8>>>,
    output_tx: broadcast::Sender<Vec<u8>>,
    input_tx: watch::Sender<Option<mpsc::Sender<Vec<u8>>>>,
    listener_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    connection_generation: Arc<AtomicU64>,
}

impl VirtualBoardManager {
    pub fn new(config: VirtualQemuConfig) -> Self {
        Self {
            events: crate::admin_events::AdminEvents::default(),
            config: Arc::new(config),
            devices: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }

    pub(crate) fn set_events(&mut self, events: crate::admin_events::AdminEvents) {
        self.events = events;
    }
    fn observe_serial(&self, hub: &SerialHub) {
        let mut changes = hub.input_tx.subscribe();
        let events = self.events.clone();
        tokio::spawn(async move {
            while changes.changed().await.is_ok() {
                events.invalidate(&["virtual"]);
            }
        });
    }
    pub fn enabled(&self) -> bool {
        self.config.enabled
    }

    pub async fn create_device(
        &self,
        requested_mac: Option<MacAddress>,
    ) -> anyhow::Result<VirtualDeviceSnapshot> {
        if !self.enabled() {
            bail!("virtual QEMU support is disabled");
        }

        let id = uuid::Uuid::new_v4().to_string();
        let mac_address = requested_mac.unwrap_or_else(|| generated_mac(&id));
        let device = self.get_or_create_device(&id, mac_address).await?;
        if let Err(error) = self.power_on_device(&device).await {
            self.devices.write().await.remove(&device.id);
            device.hub.shutdown().await;
            let run_dir = device.runtime.lock().await.run_dir.clone();
            if let Err(cleanup_error) = tokio::fs::remove_dir_all(&run_dir).await
                && cleanup_error.kind() != std::io::ErrorKind::NotFound
            {
                log::warn!(
                    "failed to clean up virtual device `{}` after start failure: {cleanup_error}",
                    device.id
                );
            }
            return Err(error);
        }
        self.snapshot_device(&device).await
    }

    pub(crate) async fn ensure_device(
        &self,
        id: &str,
        mac_address: MacAddress,
    ) -> anyhow::Result<()> {
        self.get_or_create_device(id, mac_address).await?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) async fn insert_test_device(
        &self,
        id: &str,
        mac_address: MacAddress,
    ) -> anyhow::Result<()> {
        if self.devices.read().await.contains_key(id) {
            bail!("virtual device `{id}` already exists");
        }
        let hub = SerialHub::new_at(Ipv4Addr::LOCALHOST).await?;
        let run_dir = self.config.runtime_dir.join(id);
        self.devices.write().await.insert(
            id.to_string(),
            Arc::new(VirtualDevice {
                id: id.to_string(),
                mac_address,
                tap: self
                    .config
                    .tap_pool
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "test-tap".into()),
                hub,
                runtime: Mutex::new(VirtualRuntime {
                    child: None,
                    deleting: false,
                    generation: 0,
                    qmp_path: run_dir.join("qmp.sock"),
                    run_dir,
                }),
            }),
        );
        Ok(())
    }

    async fn get_or_create_device(
        &self,
        id: &str,
        mac_address: MacAddress,
    ) -> anyhow::Result<Arc<VirtualDevice>> {
        if !self.enabled() {
            bail!("virtual QEMU support is disabled");
        }

        let mut devices = self.devices.write().await;
        if let Some(device) = devices.get(id) {
            if device.mac_address != mac_address {
                bail!("virtual device `{id}` is configured with conflicting MAC addresses");
            }
            return Ok(device.clone());
        }
        if devices
            .values()
            .any(|device| device.mac_address == mac_address)
        {
            bail!("virtual device MAC {mac_address} already exists");
        }
        let used_taps = devices
            .values()
            .map(|device| device.tap.as_str())
            .collect::<Vec<_>>();
        let tap = self
            .config
            .tap_pool
            .iter()
            .find(|tap| !used_taps.contains(&tap.as_str()))
            .cloned()
            .context("no free TAP device in virtual_qemu.tap_pool")?;
        let hub = SerialHub::new().await?;
        self.observe_serial(&hub);
        let run_dir = self.config.runtime_dir.join(id);
        let device = Arc::new(VirtualDevice {
            id: id.to_string(),
            mac_address,
            tap,
            hub,
            runtime: Mutex::new(VirtualRuntime {
                child: None,
                deleting: false,
                generation: 0,
                qmp_path: run_dir.join("qmp.sock"),
                run_dir,
            }),
        });
        devices.insert(id.to_string(), device.clone());
        Ok(device)
    }

    pub async fn delete_device(&self, id: &str) -> anyhow::Result<()> {
        let device = {
            let mut devices = self.devices.write().await;
            let device = devices
                .get(id)
                .cloned()
                .with_context(|| format!("virtual device `{id}` does not exist"))?;
            // Callers that already cloned this Arc must observe the terminal
            // state after acquiring the runtime lock instead of restarting QEMU.
            device.runtime.lock().await.deleting = true;
            devices.remove(id);
            device
        };
        if let Err(error) = self.power_off_device(&device).await {
            device.runtime.lock().await.deleting = false;
            self.devices.write().await.insert(id.to_string(), device);
            return Err(error);
        }
        device.hub.shutdown().await;
        let run_dir = device.runtime.lock().await.run_dir.clone();
        match tokio::fs::remove_dir_all(&run_dir).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("failed to remove virtual device runtime"),
        }
        Ok(())
    }

    pub async fn snapshots(&self) -> Vec<VirtualDeviceSnapshot> {
        let devices = self
            .devices
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut snapshots = Vec::with_capacity(devices.len());
        for device in devices {
            if let Ok(snapshot) = self.snapshot_device(&device).await {
                snapshots.push(snapshot);
            }
        }
        snapshots.sort_by(|left, right| left.id.cmp(&right.id));
        snapshots
    }

    pub async fn snapshot(&self, id: &str) -> Option<VirtualDeviceSnapshot> {
        let device = self.devices.read().await.get(id).cloned()?;
        self.snapshot_device(&device).await.ok()
    }

    pub async fn power_on(&self, id: &str, expected_mac: MacAddress) -> anyhow::Result<String> {
        let device = self.device(id).await?;
        if device.mac_address != expected_mac {
            bail!(
                "virtual device `{id}` has MAC {}, not {expected_mac}",
                device.mac_address
            );
        }
        self.power_on_device(&device).await?;
        Ok(format!("virtual QEMU device `{id}` is on"))
    }

    pub async fn power_off(&self, id: &str) -> anyhow::Result<String> {
        let device = self.device(id).await?;
        self.power_off_device(&device).await?;
        Ok(format!("virtual QEMU device `{id}` is off"))
    }

    pub async fn attach_serial(&self, id: &str) -> anyhow::Result<DuplexStream> {
        self.device(id).await?.hub.attach().await
    }

    pub async fn shutdown(&self) {
        let devices = self
            .devices
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for device in devices {
            if let Err(error) = self.power_off_device(&device).await {
                log::warn!("failed to stop virtual device `{}`: {error:#}", device.id);
            }
            device.hub.shutdown().await;
        }
    }

    async fn device(&self, id: &str) -> anyhow::Result<Arc<VirtualDevice>> {
        self.devices
            .read()
            .await
            .get(id)
            .cloned()
            .with_context(|| format!("virtual device `{id}` does not exist"))
    }

    async fn snapshot_device(
        &self,
        device: &VirtualDevice,
    ) -> anyhow::Result<VirtualDeviceSnapshot> {
        let mut runtime = device.runtime.lock().await;
        let powered = child_is_running(&mut runtime.child)?;
        Ok(VirtualDeviceSnapshot {
            id: device.id.clone(),
            mac_address: device.mac_address,
            tap: device.tap.clone(),
            powered,
            serial_connected: device.hub.is_connected().await,
            generation: runtime.generation,
        })
    }

    async fn power_on_device(&self, device: &VirtualDevice) -> anyhow::Result<()> {
        let mut runtime = device.runtime.lock().await;
        if runtime.deleting {
            bail!("virtual device `{}` is being deleted", device.id);
        }
        if child_is_running(&mut runtime.child)? {
            return Ok(());
        }

        prepare_runtime(&self.config, &runtime.run_dir).await?;
        if tokio::fs::try_exists(&runtime.qmp_path).await? {
            tokio::fs::remove_file(&runtime.qmp_path).await?;
        }
        let vars_path = runtime.run_dir.join("OVMF_VARS.fd");
        let esp_path = runtime.run_dir.join("esp");
        let serial_address = device.hub.address();
        let mut command = Command::new("ip");
        command
            .arg("netns")
            .arg("exec")
            .arg(&self.config.network_namespace)
            .arg(&self.config.qemu_binary)
            .arg("-machine")
            .arg("q35,accel=tcg")
            .arg("-cpu")
            .arg("max")
            .arg("-m")
            .arg(self.config.memory_mib.to_string())
            .arg("-smp")
            .arg(self.config.cpus.to_string())
            .arg("-display")
            .arg("none")
            .arg("-monitor")
            .arg("none")
            .arg("-drive")
            .arg(format!(
                "if=pflash,format=raw,readonly=on,file={}",
                self.config.ovmf_code.display()
            ))
            .arg("-drive")
            .arg(format!("if=pflash,format=raw,file={}", vars_path.display()))
            .arg("-drive")
            .arg(format!("format=raw,file=fat:rw:{}", esp_path.display()))
            .arg("-netdev")
            .arg(format!(
                "tap,id=net0,ifname={},script=no,downscript=no",
                device.tap
            ))
            .arg("-device")
            .arg(format!(
                "virtio-net-pci,netdev=net0,mac={}",
                device.mac_address
            ))
            .arg("-chardev")
            .arg(format!(
                "socket,id=serial0,host={},port={},server=off,reconnect-ms=100",
                serial_address.ip(),
                serial_address.port()
            ))
            .arg("-serial")
            .arg("chardev:serial0")
            .arg("-qmp")
            .arg(format!(
                "unix:{},server=on,wait=off",
                runtime.qmp_path.display()
            ))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let serial_generation = device.hub.connection_generation();
        let child = command.spawn().with_context(|| {
            format!(
                "failed to start virtual QEMU using {}",
                self.config.qemu_binary.display()
            )
        })?;
        runtime.child = Some(child);
        runtime.generation += 1;

        let deadline = Instant::now() + QEMU_START_TIMEOUT;
        while Instant::now() < deadline {
            if !child_is_running(&mut runtime.child)? {
                bail!("virtual QEMU exited before its serial channel connected");
            }
            if device.hub.connection_generation() > serial_generation
                && device.hub.is_connected().await
            {
                return Ok(());
            }
            sleep(Duration::from_millis(50)).await;
        }
        if let Some(child) = runtime.child.as_mut() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        runtime.child = None;
        bail!("timed out waiting for virtual QEMU serial connection")
    }

    async fn power_off_device(&self, device: &VirtualDevice) -> anyhow::Result<()> {
        let mut runtime = device.runtime.lock().await;
        if !child_is_running(&mut runtime.child)? {
            runtime.child = None;
            return Ok(());
        }

        let _ = qmp_quit(&runtime.qmp_path).await;
        if let Some(child) = runtime.child.as_mut()
            && timeout(QEMU_STOP_TIMEOUT, child.wait()).await.is_err()
        {
            child
                .kill()
                .await
                .context("failed to terminate virtual QEMU")?;
            child.wait().await.context("failed to reap virtual QEMU")?;
        }
        runtime.child = None;
        Ok(())
    }
}

impl SerialHub {
    async fn new() -> anyhow::Result<Self> {
        Self::new_at(VIRTUAL_SERVER_IP).await
    }

    async fn new_at(address: Ipv4Addr) -> anyhow::Result<Self> {
        let listener = TcpListener::bind((address, 0)).await?;
        let address = listener.local_addr()?;
        let (output_tx, _) = broadcast::channel(256);
        let hub = Self {
            address,
            history: Arc::new(Mutex::new(VecDeque::with_capacity(SERIAL_HISTORY_LIMIT))),
            output_tx,
            input_tx: watch::channel(None).0,
            listener_task: Arc::new(Mutex::new(None)),
            connection_generation: Arc::new(AtomicU64::new(0)),
        };
        let listener_hub = hub.clone();
        *hub.listener_task.lock().await = Some(tokio::spawn(async move {
            listener_hub.run_listener(listener).await;
        }));
        Ok(hub)
    }

    fn address(&self) -> SocketAddr {
        self.address
    }

    async fn is_connected(&self) -> bool {
        self.input_tx.borrow().is_some()
    }

    fn connection_generation(&self) -> u64 {
        self.connection_generation.load(Ordering::Acquire)
    }

    async fn attach(&self) -> anyhow::Result<DuplexStream> {
        let (client, hub_side) = tokio::io::duplex(SERIAL_HISTORY_LIMIT);
        let (mut hub_reader, mut hub_writer) = tokio::io::split(hub_side);
        // Subscribe and snapshot under the producer's lock, so bytes cross the
        // history/live boundary exactly once during discovery lease transfer.
        let history_guard = self.history.lock().await;
        let mut output_rx = self.output_tx.subscribe();
        let history = history_guard.iter().copied().collect::<Vec<_>>();
        drop(history_guard);
        let (attachment_closed_tx, mut attachment_closed_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            if !history.is_empty() && hub_writer.write_all(&history).await.is_err() {
                return;
            }
            loop {
                tokio::select! {
                    _ = &mut attachment_closed_rx => break,
                    output = output_rx.recv() => match output {
                        Ok(bytes) if hub_writer.write_all(&bytes).await.is_err() => break,
                        Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        });
        let mut input_rx = self.input_tx.subscribe();
        tokio::spawn(async move {
            let mut buffer = [0u8; 1024];
            'attachment: while let Ok(read) = hub_reader.read(&mut buffer).await {
                if read == 0 {
                    break;
                }
                let bytes = buffer[..read].to_vec();
                loop {
                    let current = input_rx.borrow().clone();
                    if let Some(sender) = current
                        && sender.send(bytes.clone()).await.is_ok()
                    {
                        break;
                    }
                    if input_rx.changed().await.is_err() {
                        break 'attachment;
                    }
                }
            }
            let _ = attachment_closed_tx.send(());
        });
        Ok(client)
    }

    async fn run_listener(&self, listener: TcpListener) {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let (mut reader, mut writer) = stream.into_split();
            let (input_tx, mut input_rx) = mpsc::channel::<Vec<u8>>(64);
            self.input_tx.send_replace(Some(input_tx));
            self.connection_generation.fetch_add(1, Ordering::AcqRel);
            let mut buffer = [0u8; 1024];
            loop {
                tokio::select! {
                    read = reader.read(&mut buffer) => {
                        match read {
                            Ok(0) | Err(_) => break,
                            Ok(read) => {
                                self.record_output(&buffer[..read]).await;
                            }
                        }
                    }
                    bytes = input_rx.recv() => {
                        match bytes {
                            Some(bytes) if writer.write_all(&bytes).await.is_err() => break,
                            Some(_) => {}
                            None => break,
                        }
                    }
                }
            }
            self.input_tx.send_replace(None);
        }
    }

    async fn record_output(&self, bytes: &[u8]) {
        let mut history = self.history.lock().await;
        let overflow = history.len().saturating_add(bytes.len())
            - history
                .len()
                .saturating_add(bytes.len())
                .min(SERIAL_HISTORY_LIMIT);
        let drain = overflow.min(history.len());
        history.drain(..drain);
        history.extend(bytes.iter().copied());
        let _ = self.output_tx.send(bytes.to_vec());
        drop(history);
    }

    async fn shutdown(&self) {
        if let Some(task) = self.listener_task.lock().await.take() {
            task.abort();
            let _ = task.await;
        }
        self.input_tx.send_replace(None);
    }
}

async fn prepare_runtime(config: &VirtualQemuConfig, run_dir: &Path) -> anyhow::Result<()> {
    let efi_dir = run_dir.join("esp/EFI/BOOT");
    tokio::fs::create_dir_all(&efi_dir)
        .await
        .context("failed to create virtual QEMU ESP")?;
    tokio::fs::copy(&config.ovmf_vars, run_dir.join("OVMF_VARS.fd"))
        .await
        .context("failed to copy OVMF VARS template")?;
    tokio::fs::copy(&config.axloader_efi, efi_dir.join("BOOTX64.EFI"))
        .await
        .context("failed to install axloader into virtual QEMU ESP")?;
    Ok(())
}

fn child_is_running(child: &mut Option<Child>) -> anyhow::Result<bool> {
    let Some(child) = child.as_mut() else {
        return Ok(false);
    };
    Ok(child.try_wait()?.is_none())
}

async fn qmp_quit(path: &Path) -> anyhow::Result<()> {
    let mut stream = UnixStream::connect(path).await?;
    let mut greeting = [0u8; 4096];
    let _ = timeout(Duration::from_secs(1), stream.read(&mut greeting)).await;
    stream
        .write_all(b"{\"execute\":\"qmp_capabilities\"}\r\n")
        .await?;
    stream.write_all(b"{\"execute\":\"quit\"}\r\n").await?;
    stream.flush().await?;
    Ok(())
}

fn generated_mac(seed: &str) -> MacAddress {
    let uuid = uuid::Uuid::parse_str(seed).expect("virtual device IDs are UUIDs");
    let bytes = uuid.as_bytes();
    MacAddress::new([
        (bytes[0] & 0xfe) | 0x02,
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
    ])
}

#[cfg(test)]
mod tests {
    use std::{net::Ipv4Addr, time::Duration};

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
        time::sleep,
    };

    use super::{SERIAL_HISTORY_LIMIT, SerialHub, generated_mac};

    #[test]
    fn generated_mac_is_locally_administered_unicast() {
        let mac = generated_mac("c69b3747-f23d-4075-8f58-54af9570dc04");
        assert_eq!(mac.octets()[0] & 0b11, 0b10);
    }

    #[tokio::test]
    async fn serial_hub_keeps_only_the_latest_64_kib() {
        let hub = SerialHub::new_at(Ipv4Addr::LOCALHOST).await.unwrap();
        hub.record_output(&vec![1; SERIAL_HISTORY_LIMIT]).await;
        hub.record_output(&[2, 3]).await;
        let history = hub.history.lock().await;
        assert_eq!(history.len(), SERIAL_HISTORY_LIMIT);
        assert_eq!(history.front(), Some(&1));
        assert_eq!(
            history.iter().rev().take(2).copied().collect::<Vec<_>>(),
            vec![3, 2]
        );
        drop(history);
        hub.shutdown().await;
    }

    #[tokio::test]
    async fn serial_attachment_survives_qemu_reconnect() {
        let hub = SerialHub::new_at(Ipv4Addr::LOCALHOST).await.unwrap();
        let mut attachment = hub.attach().await.unwrap();
        let mut first_qemu = TcpStream::connect(hub.address()).await.unwrap();
        wait_until_connected(&hub).await;

        first_qemu.write_all(b"first").await.unwrap();
        let mut first = [0u8; 5];
        attachment.read_exact(&mut first).await.unwrap();
        assert_eq!(&first, b"first");
        drop(first_qemu);

        let mut second_qemu = TcpStream::connect(hub.address()).await.unwrap();
        wait_until_connected(&hub).await;
        second_qemu.write_all(b"second").await.unwrap();
        let mut second = [0u8; 6];
        attachment.read_exact(&mut second).await.unwrap();
        assert_eq!(&second, b"second");

        attachment.write_all(b"input").await.unwrap();
        let mut input = [0u8; 5];
        second_qemu.read_exact(&mut input).await.unwrap();
        assert_eq!(&input, b"input");
        hub.shutdown().await;
    }

    #[tokio::test]
    async fn serial_input_waits_for_the_first_qemu_connection_without_getting_lost() {
        let hub = SerialHub::new_at(Ipv4Addr::LOCALHOST).await.unwrap();
        let mut attachment = hub.attach().await.unwrap();
        attachment.write_all(b"queued-input").await.unwrap();

        let mut qemu = TcpStream::connect(hub.address()).await.unwrap();
        let mut input = [0u8; 12];
        tokio::time::timeout(Duration::from_secs(1), qemu.read_exact(&mut input))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&input, b"queued-input");
        hub.shutdown().await;
    }

    async fn wait_until_connected(hub: &SerialHub) {
        for _ in 0..100 {
            if hub.is_connected().await {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
        panic!("serial hub did not accept the connection");
    }
}
