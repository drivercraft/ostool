use crate::{
    BindError, BindRequest, ManagerSnapshot, PortLocator, PortReservation, PortSelector,
    SerialBackend, SerialIo, SerialLease, SerialManager,
};
use httpboot_protocol::{MacAddress, SerialFrameDecoder, SerialParameters};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    sync::{mpsc, oneshot, watch},
    task::JoinSet,
    time::Instant,
};

pub(crate) enum Command {
    Bind(BindRequest, oneshot::Sender<Result<SerialLease, BindError>>),
    Exclude(Vec<PortSelector>, oneshot::Sender<Result<(), BindError>>),
    Reserve(
        PortLocator,
        oneshot::Sender<Result<PortReservation, BindError>>,
    ),
    Release(String, u64),
    Confirm(MacAddress, PortLocator, u64),
    Invalidate(MacAddress),
    WaitOwner(String, oneshot::Sender<()>),
}
struct Pending {
    request: BindRequest,
    reply: oneshot::Sender<Result<SerialLease, BindError>>,
    started: Instant,
    scanning: bool,
    scan_deadline: Instant,
}
struct Candidate {
    generation: u64,
    locator: PortLocator,
    stop: watch::Sender<bool>,
    closing: bool,
}
struct Owner {
    token: u64,
    name: String,
    locator: PortLocator,
}
enum Event {
    Found {
        locator: PortLocator,
        generation: u64,
        id: String,
        stream: Box<dyn SerialIo>,
        prefix: VecDeque<u8>,
    },
    Closed(String, u64),
    Inventory(std::io::Result<Vec<PortLocator>>),
    Fault(String, String),
}
struct Discovery {
    backend: Arc<dyn SerialBackend>,
    commands: mpsc::WeakUnboundedSender<Command>,
    events: mpsc::UnboundedSender<Event>,
    snapshot: watch::Sender<ManagerSnapshot>,
    expected: watch::Sender<Vec<(String, SerialParameters)>>,
    pending: Vec<Pending>,
    candidates: BTreeMap<String, Candidate>,
    leased: BTreeMap<String, Owner>,
    locations: BTreeMap<MacAddress, PortLocator>,
    exclusions: Vec<PortSelector>,
    excluded_replies: Vec<oneshot::Sender<Result<(), BindError>>>,
    reservations: Vec<(
        PortLocator,
        oneshot::Sender<Result<PortReservation, BindError>>,
    )>,
    release_waiters: Vec<(String, oneshot::Sender<()>)>,
    failed: BTreeSet<String>,
    errors: BTreeMap<String, String>,
    sequence: u64,
    ports: Vec<PortLocator>,
    enumerating: bool,
    next_enumeration: Instant,
    tasks: JoinSet<()>,
}

pub(crate) fn start(backend: Arc<dyn SerialBackend>) -> SerialManager {
    let (commands, mut requests) = mpsc::unbounded_channel();
    let (events, mut observations) = mpsc::unbounded_channel();
    let (snapshot, current) = watch::channel(ManagerSnapshot::default());
    let (expected, _) = watch::channel(Vec::new());
    let mut state = Discovery {
        backend,
        commands: commands.downgrade(),
        events,
        snapshot,
        expected,
        pending: Vec::new(),
        candidates: BTreeMap::new(),
        leased: BTreeMap::new(),
        locations: BTreeMap::new(),
        exclusions: Vec::new(),
        excluded_replies: Vec::new(),
        reservations: Vec::new(),
        release_waiters: Vec::new(),
        failed: BTreeSet::new(),
        errors: BTreeMap::new(),
        sequence: 0,
        ports: Vec::new(),
        enumerating: false,
        next_enumeration: Instant::now(),
        tasks: JoinSet::new(),
    };
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(25));
        loop {
            tokio::select! {
                command = requests.recv() => match command { Some(c) => state.command(c), None => break },
                event = observations.recv() => if let Some(event) = event { state.event(event); },
                _ = tick.tick() => {},
                result = state.tasks.join_next(), if !state.tasks.is_empty() => {
                    if let Some(Err(error)) = result { log::error!("serial discovery worker failed: {error}"); }
                }
            }
            state.reconcile();
        }
        for c in state.candidates.values() {
            let _ = c.stop.send(true);
        }
        while state.tasks.join_next().await.is_some() {}
    });
    SerialManager {
        commands,
        snapshot: current,
    }
}

impl Discovery {
    fn command(&mut self, command: Command) {
        match command {
            Command::Bind(request, reply) => {
                if reply.is_closed() {
                    return;
                }
                // Superseding boots cancel old requests without allowing stale completions.
                self.pending.retain(|p| {
                    !(p.request.owner == request.owner
                        && p.request.mac_address == request.mac_address
                        && p.request.generation < request.generation)
                });
                if self.pending.iter().any(|p| {
                    p.request.mac_address == request.mac_address
                        || p.request.serial_id == request.serial_id
                }) {
                    let _ = reply.send(Err(BindError::Busy));
                    return;
                }
                if self.pending.is_empty() {
                    self.errors.clear();
                }
                if !self
                    .pending
                    .iter()
                    .any(|p| p.request.parameters == request.parameters)
                {
                    self.failed.clear();
                }
                let scanning = !self.locations.contains_key(&request.mac_address);
                let started = Instant::now();
                self.pending.push(Pending {
                    scan_deadline: request.deadline.min(started + Duration::from_secs(5)),
                    request,
                    reply,
                    started,
                    scanning,
                });
            }
            Command::Exclude(selectors, reply) => {
                if selectors.iter().any(|selector| {
                    !self.exclusions.contains(selector)
                        && self
                            .leased
                            .values()
                            .any(|owner| owner.locator.matches(selector))
                }) {
                    let _ = reply.send(Err(BindError::Busy));
                    return;
                }
                self.exclusions = selectors;
                self.excluded_replies.push(reply);
            }
            Command::Reserve(port, reply) => {
                if self.leased.contains_key(&port.name)
                    || self.reservations.iter().any(|(p, _)| p.name == port.name)
                {
                    let _ = reply.send(Err(BindError::Busy));
                } else {
                    self.reservations.push((port, reply));
                }
            }
            Command::Release(port, token) => {
                if self
                    .leased
                    .get(&port)
                    .is_some_and(|owner| owner.token == token)
                {
                    self.leased.remove(&port);
                }
            }
            Command::Confirm(mac, locator, token) => {
                if self
                    .leased
                    .get(&locator.name)
                    .is_some_and(|lease| lease.token == token)
                {
                    self.locations.insert(mac, locator);
                }
            }
            Command::Invalidate(mac) => {
                self.locations.remove(&mac);
            }
            Command::WaitOwner(owner, reply) => {
                self.release_waiters.push((owner, reply));
            }
        }
    }
    fn next_token(&mut self) -> u64 {
        self.sequence = self
            .sequence
            .checked_add(1)
            .expect("serial lease sequence exhausted");
        self.sequence
    }
    fn event(&mut self, event: Event) {
        match event {
            Event::Fault(port, error) => {
                self.errors.insert(port, error);
            }
            Event::Inventory(result) => {
                self.enumerating = false;
                match result {
                    Ok(ports) => self.ports = ports,
                    Err(e) => log::debug!("serial inventory: {e}"),
                }
            }
            Event::Closed(name, generation) => {
                if self
                    .candidates
                    .get(&name)
                    .is_some_and(|c| c.generation == generation)
                {
                    let candidate = self.candidates.remove(&name).expect("matching candidate");
                    if !candidate.closing {
                        self.failed.insert(name);
                    }
                }
            }
            Event::Found {
                locator,
                generation,
                id,
                stream,
                prefix,
            } => {
                let current = self
                    .candidates
                    .get(&locator.name)
                    .is_some_and(|c| c.generation == generation && !c.closing);
                if self
                    .candidates
                    .get(&locator.name)
                    .is_some_and(|c| c.generation == generation)
                {
                    self.candidates.remove(&locator.name);
                }
                let position = self.pending.iter().position(|p| {
                    p.request.serial_id == id
                        && !p.reply.is_closed()
                        && p.request.deadline > Instant::now()
                });
                if current
                    && let Some(position) = position
                    && let Some(commands) = self.commands.upgrade()
                {
                    let pending = self.pending.remove(position);
                    let token = self.next_token();
                    self.leased.insert(
                        locator.name.clone(),
                        Owner {
                            token,
                            name: pending.request.owner.clone(),
                            locator: locator.clone(),
                        },
                    );
                    let lease = SerialLease {
                        stream: Some(stream),
                        prefix,
                        locator,

                        token,
                        mac: pending.request.mac_address,
                        commands,
                        decoder: SerialFrameDecoder::default(),
                        observed: None,
                    };
                    // A dropped receiver drops the returned lease and closes IO before release.
                    let _ = pending.reply.send(Ok(lease));
                } else {
                    // Keep the name reserved until this stale stream actually closes.
                    let stop = watch::channel(true).0;
                    self.candidates.insert(
                        locator.name.clone(),
                        Candidate {
                            generation,
                            locator: locator.clone(),
                            stop,
                            closing: true,
                        },
                    );
                    let events = self.events.clone();
                    self.tasks.spawn(async move {
                        let _ = tokio::task::spawn_blocking(move || drop(stream)).await;
                        let _ = events.send(Event::Closed(locator.name, generation));
                    });
                }
            }
        }
    }
    fn reconcile(&mut self) {
        let now = Instant::now();
        let count = self
            .pending
            .iter()
            .map(|p| p.request.parameters)
            .collect::<BTreeSet<_>>()
            .len();
        let scan_window = Duration::from_secs(5.max(count as u64 * 2));
        let mut remaining = Vec::new();
        for mut p in std::mem::take(&mut self.pending) {
            if p.reply.is_closed() {
                continue;
            }
            if !p.scanning && now.duration_since(p.started) >= Duration::from_secs(1) {
                self.locations.remove(&p.request.mac_address);
                p.scanning = true;
                p.scan_deadline = p.request.deadline.min(now + scan_window);
            }
            if p.scanning {
                p.scan_deadline = p
                    .scan_deadline
                    .max((p.started + scan_window).min(p.request.deadline));
            }
            if p.request.deadline <= now || (p.scanning && p.scan_deadline <= now) {
                self.locations.remove(&p.request.mac_address);
                let error = if self.errors.is_empty() {
                    BindError::Timeout
                } else {
                    BindError::DiscoveryFailed(
                        self.errors
                            .iter()
                            .map(|(port, error)| format!("{port}: {error}"))
                            .collect::<Vec<_>>()
                            .join("; "),
                    )
                };
                let _ = p.reply.send(Err(error));
            } else {
                remaining.push(p);
            }
        }
        self.pending = remaining;
        let expected = self
            .pending
            .iter()
            .map(|p| (p.request.serial_id.clone(), p.request.parameters))
            .collect::<Vec<_>>();
        self.expected.send_if_modified(|old| {
            if *old == expected {
                false
            } else {
                *old = expected;
                true
            }
        });
        let excluded = &self.exclusions;
        for c in self.candidates.values_mut() {
            if self.pending.is_empty()
                || excluded.iter().any(|p| c.locator.matches(p))
                || self
                    .reservations
                    .iter()
                    .any(|(p, _)| p.name == c.locator.name)
            {
                c.closing = true;
                let _ = c.stop.send(true);
            }
        }
        if !self
            .candidates
            .values()
            .any(|c| self.exclusions.iter().any(|p| c.locator.matches(p)))
        {
            for reply in self.excluded_replies.drain(..) {
                let _ = reply.send(Ok(()));
            }
        }
        let mut waiting = Vec::new();
        for (port, reply) in std::mem::take(&mut self.reservations) {
            if reply.is_closed() {
                continue;
            }
            if self.candidates.contains_key(&port.name) {
                waiting.push((port, reply));
                continue;
            }
            if let Some(commands) = self.commands.upgrade() {
                let token = self.next_token();
                self.leased.insert(
                    port.name.clone(),
                    Owner {
                        token,
                        name: "manual".into(),
                        locator: port.clone(),
                    },
                );
                let _ = reply.send(Ok(PortReservation {
                    port: port.name,
                    token,
                    commands,
                }));
            }
        }
        self.reservations = waiting;
        let mut waiters = Vec::new();
        for (owner, reply) in std::mem::take(&mut self.release_waiters) {
            if reply.is_closed() {
                continue;
            }
            if self.leased.values().any(|l| l.name == owner)
                || self.pending.iter().any(|p| p.request.owner == owner)
                || (self.pending.is_empty() && !self.candidates.is_empty())
            {
                waiters.push((owner, reply));
            } else {
                let _ = reply.send(());
            }
        }
        self.release_waiters = waiters;
        if !self.pending.is_empty() {
            if !self.enumerating && now >= self.next_enumeration {
                self.enumerating = true;
                self.next_enumeration = now + Duration::from_millis(250);
                let backend = self.backend.clone();
                let events = self.events.clone();
                self.tasks.spawn(async move {
                    let _ = events.send(Event::Inventory(backend.ports().await));
                });
            }
            let ports = self.ports.clone();
            {
                let full = self.pending.iter().any(|p| p.scanning);
                let mut serial_counts = BTreeMap::new();
                for port in &ports {
                    *serial_counts.entry(port.serial_number.clone()).or_insert(0) += 1;
                }
                for locator in ports {
                    if self.candidates.contains_key(&locator.name)
                        || self.leased.contains_key(&locator.name)
                        || self.failed.contains(&locator.name)
                        || self.exclusions.iter().any(|p| locator.matches(p))
                        || self
                            .reservations
                            .iter()
                            .any(|(p, _)| p.name == locator.name)
                    {
                        continue;
                    }
                    let cached = self.pending.iter().any(|p| {
                        self.locations
                            .get(&p.request.mac_address)
                            .is_some_and(|old| {
                                old.name == locator.name
                                    || old
                                        .aliases
                                        .iter()
                                        .any(|alias| locator.aliases.contains(alias))
                                    || old.serial_number.is_some()
                                        && old.serial_number == locator.serial_number
                                        && serial_counts.get(&locator.serial_number) == Some(&1)
                            })
                    });
                    if !full && !cached {
                        continue;
                    }
                    let generation = self.next_token();
                    let (stop, cancellation) = watch::channel(false);
                    self.candidates.insert(
                        locator.name.clone(),
                        Candidate {
                            generation,
                            locator: locator.clone(),
                            stop,
                            closing: false,
                        },
                    );
                    let backend = self.backend.clone();
                    let events = self.events.clone();
                    let expected = self.expected.subscribe();
                    self.tasks.spawn(scan(
                        backend,
                        locator,
                        generation,
                        expected,
                        cancellation,
                        events,
                    ));
                }
            }
        }
        self.snapshot.send_if_modified(|s| {
            let next = ManagerSnapshot {
                pending: self.pending.len(),
                candidates: self.candidates.len(),
                leased: self.leased.len(),
            };
            if *s == next {
                false
            } else {
                *s = next;
                true
            }
        });
    }
}

async fn scan(
    backend: Arc<dyn SerialBackend>,
    locator: PortLocator,
    generation: u64,
    mut expected: watch::Receiver<Vec<(String, SerialParameters)>>,
    mut stop: watch::Receiver<bool>,
    events: mpsc::UnboundedSender<Event>,
) {
    let parameters = expected
        .borrow()
        .iter()
        .map(|p| p.1)
        .collect::<BTreeSet<_>>();
    let mut opened = None;
    for parameters in parameters {
        if *stop.borrow() {
            break;
        }
        // Do not abort an in-flight open: a late native descriptor must close.
        match backend.open(locator.clone(), parameters).await {
            Ok(stream) => {
                opened = Some((stream, parameters));
                break;
            }
            Err(error) => {
                let skip = matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::PermissionDenied
                );
                let _ = events.send(Event::Fault(
                    locator.name.clone(),
                    format!("open/configure {parameters:?}: {error}"),
                ));
                if skip {
                    break;
                }
            }
        }
    }
    let Some((mut stream, first)) = opened else {
        let _ = events.send(Event::Closed(locator.name, generation));
        return;
    };
    let mut decoder = SerialFrameDecoder::default();
    let mut history = VecDeque::new();
    let mut bytes = [0; 4096];
    let mut rotation = tokio::time::interval(Duration::from_secs(1));
    rotation.tick().await;
    let mut parameter_index = 0;
    let mut active_parameters = Some(first);
    loop {
        if *stop.borrow() {
            break;
        }
        tokio::select! {
            changed = stop.changed() => { if changed.is_err() || *stop.borrow() { break; } },
            changed = expected.changed() => { if changed.is_err() || expected.borrow().is_empty() { break; } },
            _ = rotation.tick() => {
                let parameters = expected.borrow().iter().map(|p|p.1).collect::<BTreeSet<_>>().into_iter().collect::<Vec<_>>();
                if parameters.is_empty() { break; }
                let next = parameters[parameter_index % parameters.len()];
                if Some(next) != active_parameters {
                    active_parameters = match stream.configure(next) {
                        Ok(()) => Some(next),
                        Err(error) => { let _ = events.send(Event::Fault(locator.name.clone(), format!("configure {next:?}: {error}"))); None }
                    };
                    decoder = SerialFrameDecoder::default();
                }
                parameter_index += 1;
            }
            result = stream.read(&mut bytes) => match result {
                Ok(0) => { let _ = events.send(Event::Fault(locator.name.clone(), "serial reader closed".into())); break; }
                Err(error) => { let _ = events.send(Event::Fault(locator.name.clone(), error.to_string())); break; }
                Ok(size) => {
                    history.extend(&bytes[..size]);
                    if history.len() > 64*1024 { history.drain(..history.len()-64*1024); }
                    for byte in &bytes[..size] {
                        if let Some(id) = decoder.push(*byte) {
                            let id = String::from_utf8(id.to_vec()).expect("ASCII decoder");
                            if expected.borrow().iter().any(|p|p.0 == id && Some(p.1) == active_parameters) {
                                let _ = events.send(Event::Found { locator, generation, id, stream, prefix: history }); return;
                            }
                        }
                    }
                }
            }
        }
    }
    let _ = tokio::task::spawn_blocking(move || drop(stream)).await;
    let _ = events.send(Event::Closed(locator.name, generation));
}
