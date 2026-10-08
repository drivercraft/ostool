//! Discovery, lease transfer and cancellation through the shared public API.
use ostool_serial::{
    BackendFuture, BindRequest, ManagerSnapshot, PortLocator, PortSelector, SerialBackend,
    SerialIo, SerialManager, SerialParameters,
};
use std::{
    collections::BTreeMap,
    io,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf},
    sync::{mpsc, watch},
    time::Instant,
};

struct TestIo {
    io: DuplexStream,
    closed: Arc<AtomicUsize>,
    parameters: Arc<Mutex<Vec<SerialParameters>>>,
}
impl Drop for TestIo {
    fn drop(&mut self) {
        self.closed.fetch_add(1, Ordering::SeqCst);
    }
}
impl AsyncRead for TestIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        c: &mut Context<'_>,
        b: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_read(c, b)
    }
}
impl AsyncWrite for TestIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        c: &mut Context<'_>,
        b: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.io).poll_write(c, b)
    }
    fn poll_flush(mut self: Pin<&mut Self>, c: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(c)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, c: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(c)
    }
}
impl SerialIo for TestIo {
    fn configure(&mut self, p: SerialParameters) -> io::Result<()> {
        self.parameters.lock().unwrap().push(p);
        Ok(())
    }
}
struct TestBackend {
    ports: Vec<PortLocator>,
    streams: Mutex<BTreeMap<String, DuplexStream>>,
    opens: mpsc::UnboundedSender<String>,
    gate: watch::Receiver<bool>,
    closed: Arc<AtomicUsize>,
    parameters: Arc<Mutex<Vec<SerialParameters>>>,
}
impl SerialBackend for TestBackend {
    fn ports(&self) -> BackendFuture<'_, Vec<PortLocator>> {
        Box::pin(async { Ok(self.ports.clone()) })
    }
    fn open(
        &self,
        p: PortLocator,
        config: SerialParameters,
    ) -> BackendFuture<'_, Box<dyn SerialIo>> {
        Box::pin(async move {
            self.opens.send(p.name.clone()).unwrap();
            let mut gate = self.gate.clone();
            gate.wait_for(|v| *v).await.map_err(io::Error::other)?;
            let io = self
                .streams
                .lock()
                .unwrap()
                .remove(&p.name)
                .ok_or_else(|| io::Error::other("port already opened"))?;
            self.parameters.lock().unwrap().push(config);
            Ok(Box::new(TestIo {
                io,
                closed: self.closed.clone(),
                parameters: self.parameters.clone(),
            }) as Box<dyn SerialIo>)
        })
    }
}
fn parameters() -> SerialParameters {
    SerialParameters {
        baud_rate: 57600,
        data_bits: 8,
        parity: httpboot_protocol::SerialParity::None,
        stop_bits: httpboot_protocol::SerialStopBits::One,
        flow_control: httpboot_protocol::SerialFlowControl::None,
    }
}
fn request(owner: &str, n: u8) -> BindRequest {
    BindRequest {
        owner: owner.into(),
        generation: 1,
        mac_address: format!("02:00:00:00:00:{n:02x}").parse().unwrap(),
        boot_epoch: format!("{n:032x}"),
        serial_id: format!("{n:032x}"),
        parameters: parameters(),
        deadline: Instant::now() + Duration::from_secs(60),
    }
}
fn backend(
    names: &[&str],
    ready: bool,
) -> (
    Arc<TestBackend>,
    Vec<DuplexStream>,
    mpsc::UnboundedReceiver<String>,
    watch::Sender<bool>,
) {
    let mut streams = BTreeMap::new();
    let mut peers = Vec::new();
    for name in names {
        let (a, b) = tokio::io::duplex(65536);
        streams.insert(name.to_string(), a);
        peers.push(b);
    }
    let (opens, rx) = mpsc::unbounded_channel();
    let (gate, wait) = watch::channel(ready);
    (
        Arc::new(TestBackend {
            ports: names
                .iter()
                .map(|n| PortLocator {
                    name: n.to_string(),
                    aliases: vec![format!("alias/{n}")],
                    serial_number: None,
                })
                .collect(),
            streams: Mutex::new(streams),
            opens,
            gate: wait,
            closed: Arc::default(),
            parameters: Arc::default(),
        }),
        peers,
        rx,
        gate,
    )
}
async fn snapshot(manager: &SerialManager, predicate: impl Fn(&ManagerSnapshot) -> bool) {
    let mut rx = manager.subscribe();
    tokio::time::timeout(Duration::from_secs(2), rx.wait_for(predicate))
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn parallel_discovery_transfers_tail_and_waits_for_actual_release() {
    let (backend, mut peers, mut opens, _gate) = backend(&["a", "b"], true);
    let manager = SerialManager::new(backend.clone());
    let m = manager.clone();
    let first = tokio::spawn(async move { m.bind(request("first", 1)).await.unwrap() });
    let m = manager.clone();
    let second = tokio::spawn(async move { m.bind(request("second", 2)).await.unwrap() });
    let a = opens.recv().await.unwrap();
    let b = opens.recv().await.unwrap();
    assert_ne!(a, b);
    let one = format!("\r\nAXLOADER-SERIAL/1 {:032x}\r\nKERNEL_ONE\n", 1);
    let two = format!("\r\nAXLOADER-SERIAL/1 {:032x}\r\nKERNEL_TWO\n", 2);
    peers[0].write_all(one.as_bytes()).await.unwrap();
    peers[1].write_all(two.as_bytes()).await.unwrap();
    let mut first = first.await.unwrap();
    let mut second = second.await.unwrap();
    let mut bytes = vec![0; one.len()];
    first.read_exact(&mut bytes).await.unwrap();
    assert_eq!(bytes, one.as_bytes());
    let mut bytes = vec![0; two.len()];
    second.read_exact(&mut bytes).await.unwrap();
    assert_eq!(bytes, two.as_bytes());
    let m = manager.clone();
    let waiter = tokio::spawn(async move { m.wait_owner_released("first").await });
    snapshot(&manager, |s| s.leased == 2).await;
    assert!(!waiter.is_finished());
    drop(first);
    assert!(waiter.await.unwrap().is_ok());
    assert_eq!(backend.closed.load(Ordering::SeqCst), 1);
    drop(second);
    snapshot(&manager, |s| s.leased == 0).await;
    assert_eq!(backend.closed.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn cancelled_open_and_relay_reservation_reclaim_late_descriptors() {
    let (backend, _peers, mut opens, gate) = backend(&["a"], false);
    let manager = SerialManager::new(backend.clone());
    let m = manager.clone();
    let pending = tokio::spawn(async move { m.bind(request("cancelled", 1)).await });
    assert_eq!(opens.recv().await.unwrap(), "a");
    pending.abort();
    let _ = pending.await;
    let m = manager.clone();
    let locator = backend.ports[0].clone();
    let reserve = tokio::spawn(async move { m.reserve(locator).await.unwrap() });
    snapshot(&manager, |s| s.pending == 0).await;
    assert!(!reserve.is_finished());
    gate.send_replace(true);
    let guard = reserve.await.unwrap();
    assert_eq!(backend.closed.load(Ordering::SeqCst), 1);
    drop(guard);
    snapshot(&manager, |s| s.leased == 0 && s.candidates == 0).await;
}
#[tokio::test]
async fn configured_relay_alias_never_opens() {
    let (backend, _peers, mut opens, _gate) = backend(&["relay"], true);
    let manager = SerialManager::new(backend);
    manager
        .exclude(vec![PortSelector::Path("alias/relay".into())])
        .await
        .unwrap();
    let m = manager.clone();
    let pending = tokio::spawn(async move { m.bind(request("board", 1)).await });
    snapshot(&manager, |s| s.pending == 1).await;
    assert!(opens.try_recv().is_err());
    pending.abort();
    let _ = pending.await;
    snapshot(&manager, |s| s.pending == 0 && s.candidates == 0).await;
}

#[tokio::test]
async fn confirmed_location_reopens_only_known_port_with_current_parameters_and_id() {
    let (backend, mut peers, mut opens, _gate) = backend(&["a", "unused"], true);
    let manager = SerialManager::new(backend.clone());
    let m = manager.clone();
    let first = tokio::spawn(async move { m.bind(request("first", 1)).await.unwrap() });
    let a = opens.recv().await.unwrap();
    let b = opens.recv().await.unwrap();
    assert_ne!(a, b);
    peers[0]
        .write_all(format!("\r\nAXLOADER-SERIAL/1 {:032x}\r\n", 1).as_bytes())
        .await
        .unwrap();
    let lease = first.await.unwrap();
    lease.confirm();
    drop(lease);
    manager.wait_owner_released("first").await.unwrap();
    snapshot(&manager, |s| s.leased == 0 && s.candidates == 0).await;
    let (io, mut peer) = tokio::io::duplex(65536);
    backend.streams.lock().unwrap().insert("a".into(), io);
    let mut reboot = request("second", 1);
    reboot.serial_id = format!("{:032x}", 3);
    reboot.boot_epoch = reboot.serial_id.clone();
    reboot.parameters.baud_rate = 921600;
    let m = manager.clone();
    let second = tokio::spawn(async move { m.bind(reboot).await.unwrap() });
    assert_eq!(opens.recv().await.unwrap(), "a");
    peer.write_all(format!("\r\nAXLOADER-SERIAL/1 {:032x}\r\n", 1).as_bytes())
        .await
        .unwrap();
    assert!(!second.is_finished());
    peer.write_all(format!("\r\nAXLOADER-SERIAL/1 {:032x}\r\n", 3).as_bytes())
        .await
        .unwrap();
    let lease = second.await.unwrap();
    assert_eq!(lease.locator().name, "a");
    assert!(opens.try_recv().is_err());
    assert_eq!(
        backend.parameters.lock().unwrap().last().unwrap().baud_rate,
        921600
    );
    drop(lease);
    manager.wait_owner_released("second").await.unwrap();
    // A fresh manager has no location and must discover again.
    drop(manager);
    let (io, mut peer) = tokio::io::duplex(65536);
    backend.streams.lock().unwrap().insert("a".into(), io);
    let manager = SerialManager::new(backend.clone());
    let m = manager.clone();
    let third = tokio::spawn(async move { m.bind(request("third", 1)).await.unwrap() });
    let a = opens.recv().await.unwrap();
    let b = opens.recv().await.unwrap();
    assert_ne!(a, b);
    peer.write_all(format!("\r\nAXLOADER-SERIAL/1 {:032x}\r\n", 1).as_bytes())
        .await
        .unwrap();
    drop(third.await.unwrap());
    manager.wait_owner_released("third").await.unwrap();
}

struct SelectiveBackend {
    backend: Arc<TestBackend>,
    inventory: watch::Receiver<bool>,
}
impl SerialBackend for SelectiveBackend {
    fn ports(&self) -> BackendFuture<'_, Vec<PortLocator>> {
        Box::pin(async {
            let mut inventory = self.inventory.clone();
            inventory
                .wait_for(|ready| *ready)
                .await
                .map_err(io::Error::other)?;
            self.backend.ports().await
        })
    }
    fn open(
        &self,
        port: PortLocator,
        parameters: SerialParameters,
    ) -> BackendFuture<'_, Box<dyn SerialIo>> {
        Box::pin(async move {
            if parameters.data_bits == 7 {
                Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "driver rejects seven data bits",
                ))
            } else {
                self.backend.open(port, parameters).await
            }
        })
    }
}
#[tokio::test]
async fn unsupported_combination_does_not_starve_other_bindings() {
    let (backend, mut peers, mut opens, _gate) = backend(&["uart"], true);
    let (inventory, waiting) = watch::channel(false);
    let manager = SerialManager::new(Arc::new(SelectiveBackend {
        backend,
        inventory: waiting,
    }));
    let mut unsupported = request("unsupported", 1);
    unsupported.parameters.data_bits = 7;
    unsupported.deadline = Instant::now() + Duration::from_secs(2);
    let m = manager.clone();
    let bad = tokio::spawn(async move { m.bind(unsupported).await });
    let m = manager.clone();
    let good = tokio::spawn(async move { m.bind(request("supported", 2)).await });
    snapshot(&manager, |s| s.pending == 2).await;
    inventory.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(1), opens.recv())
        .await
        .expect("supported settings were never tried")
        .unwrap();
    peers[0]
        .write_all(format!("\r\nAXLOADER-SERIAL/1 {:032x}\r\n", 2).as_bytes())
        .await
        .unwrap();
    let lease = tokio::time::timeout(Duration::from_secs(1), good)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let error = bad
        .await
        .unwrap()
        .err()
        .expect("unsupported configuration was accepted");
    assert!(
        error.to_string().contains("driver rejects seven data bits"),
        "{error}"
    );
    drop(lease);
    manager.wait_owner_released("supported").await.unwrap();
    snapshot(&manager, |s| s.candidates == 0 && s.leased == 0).await;
}
