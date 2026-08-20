//! Async terminal core shared by local serial, remote websocket, and process I/O.
//!
//! Press `Ctrl+A` followed by `x` to exit when the exit sequence is enabled.

use std::{
    io::{self, IsTerminal, Write},
    process::Command,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::anyhow;
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEvent,
        KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    terminal::{disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use tokio::{
    sync::{mpsc, watch},
    time::{Instant, sleep_until},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalAction {
    SendBytes(Vec<u8>),
    ExitRequested,
    Noop,
}

#[derive(Debug, Clone)]
pub struct TerminalConfig {
    pub intercept_exit_sequence: bool,
    pub timeout: Option<Duration>,
    pub timeout_label: String,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            intercept_exit_sequence: true,
            timeout: None,
            timeout_label: "terminal".to_string(),
        }
    }
}

pub struct AsyncTerminal {
    config: TerminalConfig,
    key_processor: KeyProcessor,
}

#[derive(Clone)]
pub struct TerminalHandle {
    inner: Arc<TerminalState>,
}

#[derive(Clone)]
pub(crate) struct WeakTerminalHandle {
    inner: Weak<TerminalState>,
}

struct TerminalState {
    running: AtomicBool,
    timed_out: AtomicBool,
    stop_deadline: Mutex<Option<Instant>>,
    timeout_deadline: Mutex<Option<Instant>>,
    outbound_tx: TerminalOutboundSender,
    wake_version: AtomicU64,
    wake_tx: watch::Sender<u64>,
}

#[derive(Clone)]
enum TerminalOutboundSender {
    Bytes(mpsc::UnboundedSender<Vec<u8>>),
    Acknowledged(mpsc::UnboundedSender<TerminalInput>),
}

pub(crate) struct TerminalInput {
    bytes: Vec<u8>,
    on_flushed: Option<Box<dyn FnOnce(io::Result<()>) + Send>>,
}

impl TerminalInput {
    #[cfg(test)]
    pub(crate) fn for_test(
        bytes: Vec<u8>,
        on_flushed: impl FnOnce(io::Result<()>) + Send + 'static,
    ) -> Self {
        Self {
            bytes,
            on_flushed: Some(Box::new(on_flushed)),
        }
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn acknowledge_flushed(mut self) {
        if let Some(on_flushed) = self.on_flushed.take() {
            on_flushed(Ok(()));
        }
    }

    pub(crate) fn acknowledge_failed(mut self, error: impl Into<String>) {
        if let Some(on_flushed) = self.on_flushed.take() {
            on_flushed(Err(io::Error::other(error.into())));
        }
    }

    fn acknowledge_error(mut self, error: io::Error) {
        if let Some(on_flushed) = self.on_flushed.take() {
            on_flushed(Err(error));
        }
    }
}

impl Drop for TerminalInput {
    fn drop(&mut self) {
        if let Some(on_flushed) = self.on_flushed.take() {
            on_flushed(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "terminal input was dropped before it was flushed",
            )));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeySequenceState {
    Normal,
    CtrlAPressed,
}

#[derive(Debug, Clone)]
struct KeyProcessor {
    intercept_exit_sequence: bool,
    state: KeySequenceState,
}

impl AsyncTerminal {
    pub fn new(config: TerminalConfig) -> Self {
        let key_processor = KeyProcessor::new(config.intercept_exit_sequence);
        Self {
            config,
            key_processor,
        }
    }

    pub async fn run<F>(
        self,
        inbound_rx: mpsc::UnboundedReceiver<Vec<u8>>,
        outbound_tx: mpsc::UnboundedSender<Vec<u8>>,
        on_byte: F,
    ) -> anyhow::Result<()>
    where
        F: FnMut(&TerminalHandle, u8) + Send,
    {
        self.run_with_output(inbound_rx, outbound_tx, io::stdout(), on_byte)
            .await
    }

    pub(crate) async fn run_with_write_ack<F>(
        self,
        inbound_rx: mpsc::UnboundedReceiver<Vec<u8>>,
        outbound_tx: mpsc::UnboundedSender<TerminalInput>,
        on_byte: F,
    ) -> anyhow::Result<()>
    where
        F: FnMut(&TerminalHandle, &[u8]) + Send,
    {
        self.run_with_output_sender(
            inbound_rx,
            TerminalOutboundSender::Acknowledged(outbound_tx),
            io::stdout(),
            on_byte,
        )
        .await
    }

    async fn run_with_output<W, F>(
        self,
        inbound_rx: mpsc::UnboundedReceiver<Vec<u8>>,
        outbound_tx: mpsc::UnboundedSender<Vec<u8>>,
        output: W,
        on_byte: F,
    ) -> anyhow::Result<()>
    where
        W: Write,
        F: FnMut(&TerminalHandle, u8) + Send,
    {
        let mut on_byte = on_byte;
        self.run_with_output_sender(
            inbound_rx,
            TerminalOutboundSender::Bytes(outbound_tx),
            output,
            move |handle, chunk| {
                for byte in chunk {
                    on_byte(handle, *byte);
                }
            },
        )
        .await
    }

    async fn run_with_output_sender<W, F>(
        self,
        mut inbound_rx: mpsc::UnboundedReceiver<Vec<u8>>,
        outbound_tx: TerminalOutboundSender,
        mut output: W,
        mut on_byte: F,
    ) -> anyhow::Result<()>
    where
        W: Write,
        F: FnMut(&TerminalHandle, &[u8]) + Send,
    {
        let interactive_input_enabled = io::stdin().is_terminal() && io::stdout().is_terminal();
        self.run_with_output_mode(
            &mut inbound_rx,
            outbound_tx,
            &mut output,
            &mut on_byte,
            interactive_input_enabled,
        )
        .await
    }

    async fn run_with_output_mode<W, F>(
        mut self,
        inbound_rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
        outbound_tx: TerminalOutboundSender,
        output: &mut W,
        on_chunk: &mut F,
        interactive_input_enabled: bool,
    ) -> anyhow::Result<()>
    where
        W: Write,
        F: FnMut(&TerminalHandle, &[u8]) + Send,
    {
        if interactive_input_enabled {
            enable_raw_mode().ok();
            if let Err(e) = crossterm::execute!(io::stdout(), EnableMouseCapture) {
                debug!("EnableMouseCapture failed: {e}");
            }
        } else {
            debug!("keyboard input disabled because stdin/stdout are not TTY");
        }

        let handle = TerminalHandle::new(outbound_tx);
        if let Some(timeout) = self.config.timeout {
            handle.timeout_after(timeout);
        }

        let mut events = interactive_input_enabled.then(EventStream::new);
        let result = self
            .run_loop(&handle, inbound_rx, &mut events, output, on_chunk)
            .await;

        if interactive_input_enabled {
            if let Err(e) = crossterm::execute!(io::stdout(), DisableMouseCapture) {
                debug!("DisableMouseCapture failed: {e}");
            }
            restore_terminal_mode();
            println!();
            eprintln!("✓ 已退出串口终端模式");
        }

        if handle.timed_out() {
            return Err(anyhow!(
                "{} timed out after {}s",
                self.config.timeout_label,
                self.config.timeout.unwrap_or_default().as_secs()
            ));
        }

        result
    }

    async fn run_loop<W, F>(
        &mut self,
        handle: &TerminalHandle,
        inbound_rx: &mut mpsc::UnboundedReceiver<Vec<u8>>,
        events: &mut Option<EventStream>,
        output: &mut W,
        on_chunk: &mut F,
    ) -> anyhow::Result<()>
    where
        W: Write,
        F: FnMut(&TerminalHandle, &[u8]) + Send,
    {
        while handle.is_running() {
            let mut wake_rx = handle.subscribe();
            let mut stop_deadline = Box::pin(async {
                if let Some(deadline) = handle.stop_deadline() {
                    sleep_until(deadline).await;
                } else {
                    futures::future::pending::<()>().await;
                }
            });
            let mut timeout_deadline = Box::pin(async {
                if let Some(deadline) = handle.timeout_deadline() {
                    sleep_until(deadline).await;
                } else {
                    futures::future::pending::<()>().await;
                }
            });

            tokio::select! {
                maybe_chunk = inbound_rx.recv() => {
                    match maybe_chunk {
                        Some(chunk) => {
                            write_output(output, &chunk)?;
                            (on_chunk)(handle, &chunk);
                        }
                        None => break,
                    }
                }
                maybe_event = async {
                    if let Some(events) = events.as_mut() {
                        events.next().await
                    } else {
                        futures::future::pending().await
                    }
                } => {
                    match maybe_event {
                        Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                            match self.key_processor.process_key(key)? {
                                TerminalAction::SendBytes(bytes) => {
                                    handle.send(bytes)?;
                                }
                                TerminalAction::ExitRequested => {
                                    eprintln!("\r\nExit by: Ctrl+A+x");
                                    handle.stop();
                                }
                                TerminalAction::Noop => {}
                            }
                        }
                        Some(Ok(Event::Mouse(mouse))) => {
                            // Moved events fire on every pixel of cursor motion and
                            // cause a full TUI redraw on every move, saturating the
                            // UART output path and freezing the interface.
                            if !matches!(mouse.kind, MouseEventKind::Moved)
                                && let Some(bytes) = encode_mouse_event(mouse)
                            {
                                handle.send(bytes).ok();
                            }
                        }
                        Some(Ok(_)) => {}
                        Some(Err(err)) => return Err(err.into()),
                        None => break,
                    }
                }
                _ = &mut stop_deadline => {
                    handle.stop();
                }
                _ = &mut timeout_deadline => {
                    handle.mark_timed_out();
                    handle.stop();
                }
                changed = wake_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
            }
        }

        Ok(())
    }
}

impl TerminalHandle {
    #[cfg(test)]
    pub(crate) fn acknowledged_for_test() -> (Self, mpsc::UnboundedReceiver<TerminalInput>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self::new(TerminalOutboundSender::Acknowledged(tx)), rx)
    }

    fn new(outbound_tx: TerminalOutboundSender) -> Self {
        let (wake_tx, _wake_rx) = watch::channel(0u64);
        Self {
            inner: Arc::new(TerminalState {
                running: AtomicBool::new(true),
                timed_out: AtomicBool::new(false),
                stop_deadline: Mutex::new(None),
                timeout_deadline: Mutex::new(None),
                outbound_tx,
                wake_version: AtomicU64::new(0),
                wake_tx,
            }),
        }
    }

    pub fn stop(&self) {
        self.inner.running.store(false, Ordering::Release);
        self.wake();
    }

    pub fn stop_after(&self, duration: Duration) {
        let mut deadline = self.inner.stop_deadline.lock().unwrap();
        if deadline.is_none() {
            *deadline = Some(Instant::now() + duration);
            drop(deadline);
            self.wake();
        }
    }

    pub fn timeout_after(&self, duration: Duration) {
        let mut deadline = self.inner.timeout_deadline.lock().unwrap();
        if deadline.is_none() {
            *deadline = Some(Instant::now() + duration);
            drop(deadline);
            self.wake();
        }
    }

    pub fn send_after(&self, duration: Duration, bytes: Vec<u8>) {
        let handle = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(duration).await;
            if handle.is_running() {
                let _ = handle.send(bytes);
            }
        });
    }

    pub fn send_after_chunks(
        &self,
        duration: Duration,
        bytes: Vec<u8>,
        chunk_size: usize,
        chunk_delay: Duration,
    ) {
        self.send_after_chunks_then(duration, bytes, chunk_size, chunk_delay, |_, _| {});
    }

    pub(crate) fn send_after_chunks_then<F>(
        &self,
        duration: Duration,
        bytes: Vec<u8>,
        chunk_size: usize,
        chunk_delay: Duration,
        on_sent: F,
    ) where
        F: FnOnce(&TerminalHandle, io::Result<()>) + Send + 'static,
    {
        let handle = self.clone();
        let chunk_size = chunk_size.max(1);
        tokio::spawn(async move {
            tokio::time::sleep(duration).await;
            let chunk_count = bytes.len().div_ceil(chunk_size);
            let completion = Arc::new(Mutex::new(Some(on_sent)));
            if bytes.is_empty() {
                if let Some(on_sent) = completion.lock().unwrap().take() {
                    on_sent(&handle, Ok(()));
                }
                return;
            }
            for (index, chunk) in bytes.chunks(chunk_size).enumerate() {
                if !handle.is_running() {
                    if let Some(on_sent) = completion.lock().unwrap().take() {
                        on_sent(&handle, Err(terminal_send_cancelled_error()));
                    }
                    return;
                }
                let callback_handle = handle.clone();
                let completion = completion.clone();
                let is_last = index + 1 == chunk_count;
                let send_result = handle.send_with_completion(
                    chunk.to_vec(),
                    Box::new(move |result| {
                        if (result.is_err() || is_last)
                            && let Some(on_sent) = completion.lock().unwrap().take()
                        {
                            on_sent(&callback_handle, result);
                        }
                    }),
                );
                if send_result.is_err() {
                    return;
                }
                tokio::time::sleep(chunk_delay).await;
            }
        });
    }

    pub fn is_running(&self) -> bool {
        self.inner.running.load(Ordering::Acquire)
    }

    pub(crate) fn downgrade(&self) -> WeakTerminalHandle {
        WeakTerminalHandle {
            inner: Arc::downgrade(&self.inner),
        }
    }

    fn send(&self, bytes: Vec<u8>) -> io::Result<()> {
        self.send_input(bytes, None)
    }

    fn send_with_completion(
        &self,
        bytes: Vec<u8>,
        on_flushed: Box<dyn FnOnce(io::Result<()>) + Send>,
    ) -> io::Result<()> {
        self.send_input(bytes, Some(on_flushed))
    }

    fn send_input(
        &self,
        bytes: Vec<u8>,
        on_flushed: Option<Box<dyn FnOnce(io::Result<()>) + Send>>,
    ) -> io::Result<()> {
        match &self.inner.outbound_tx {
            TerminalOutboundSender::Bytes(tx) => tx
                .send(bytes)
                .map_err(|_| terminal_transport_closed_error()),
            TerminalOutboundSender::Acknowledged(tx) => {
                if let Err(error) = tx.send(TerminalInput { bytes, on_flushed }) {
                    error.0.acknowledge_error(terminal_transport_closed_error());
                    return Err(terminal_transport_closed_error());
                }
                Ok(())
            }
        }
    }

    fn timed_out(&self) -> bool {
        self.inner.timed_out.load(Ordering::Acquire)
    }

    fn mark_timed_out(&self) {
        self.inner.timed_out.store(true, Ordering::Release);
    }

    fn stop_deadline(&self) -> Option<Instant> {
        *self.inner.stop_deadline.lock().unwrap()
    }

    fn timeout_deadline(&self) -> Option<Instant> {
        *self.inner.timeout_deadline.lock().unwrap()
    }

    fn subscribe(&self) -> watch::Receiver<u64> {
        self.inner.wake_tx.subscribe()
    }

    fn wake(&self) {
        let version = self.inner.wake_version.fetch_add(1, Ordering::AcqRel) + 1;
        let _ = self.inner.wake_tx.send(version);
    }
}

fn terminal_transport_closed_error() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "terminal transport closed")
}

fn terminal_send_cancelled_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::Interrupted,
        "terminal stopped before input was sent",
    )
}

impl WeakTerminalHandle {
    pub(crate) fn upgrade(&self) -> Option<TerminalHandle> {
        self.inner.upgrade().map(|inner| TerminalHandle { inner })
    }
}

impl KeyProcessor {
    fn new(intercept_exit_sequence: bool) -> Self {
        Self {
            intercept_exit_sequence,
            state: KeySequenceState::Normal,
        }
    }

    fn process_key(&mut self, key: KeyEvent) -> io::Result<TerminalAction> {
        if !self.intercept_exit_sequence {
            return encode_key_event(key);
        }

        match self.state {
            KeySequenceState::Normal => {
                if key.code == KeyCode::Char('a') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.state = KeySequenceState::CtrlAPressed;
                    Ok(TerminalAction::Noop)
                } else {
                    encode_key_event(key)
                }
            }
            KeySequenceState::CtrlAPressed => {
                if key.code == KeyCode::Char('x') {
                    self.state = KeySequenceState::Normal;
                    Ok(TerminalAction::ExitRequested)
                } else if key.code == KeyCode::Char('a') {
                    Ok(TerminalAction::Noop)
                } else {
                    self.state = KeySequenceState::Normal;
                    let mut bytes = vec![0x01];
                    match encode_key_event(key)? {
                        TerminalAction::SendBytes(mut key_bytes) => {
                            bytes.append(&mut key_bytes);
                            Ok(TerminalAction::SendBytes(bytes))
                        }
                        TerminalAction::ExitRequested | TerminalAction::Noop => {
                            Ok(TerminalAction::SendBytes(bytes))
                        }
                    }
                }
            }
        }
    }
}

pub fn encode_key_event(key: KeyEvent) -> io::Result<TerminalAction> {
    let mut bytes = Vec::new();
    match key.code {
        KeyCode::Char(c) => handle_character_key(c, key.modifiers, &mut bytes),
        KeyCode::Enter => handle_enter_key(key.modifiers, &mut bytes),
        KeyCode::Backspace => handle_backspace_key(key.modifiers, &mut bytes),
        KeyCode::Tab => handle_tab_key(key.modifiers, &mut bytes),
        KeyCode::BackTab => bytes.extend_from_slice(&[0x1b, b'[', b'Z']),
        KeyCode::Esc => {
            if key.modifiers.contains(KeyModifiers::ALT) {
                bytes.extend_from_slice(&[0x1b, 0x1b]);
            } else {
                bytes.push(0x1b);
            }
        }
        KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right => {
            handle_arrow_key(key.code, key.modifiers, &mut bytes)
        }
        KeyCode::Home | KeyCode::End => handle_home_end_key(key.code, key.modifiers, &mut bytes),
        KeyCode::PageUp | KeyCode::PageDown => handle_page_key(key.code, key.modifiers, &mut bytes),
        KeyCode::Insert => handle_insert_key(key.modifiers, &mut bytes),
        KeyCode::Delete => handle_delete_key(key.modifiers, &mut bytes),
        KeyCode::F(n) => handle_function_key(n, key.modifiers, &mut bytes),
        KeyCode::Null
        | KeyCode::CapsLock
        | KeyCode::ScrollLock
        | KeyCode::NumLock
        | KeyCode::PrintScreen
        | KeyCode::Pause
        | KeyCode::Menu
        | KeyCode::KeypadBegin
        | KeyCode::Media(_)
        | KeyCode::Modifier(_) => {}
    }

    if bytes.is_empty() {
        Ok(TerminalAction::Noop)
    } else {
        Ok(TerminalAction::SendBytes(bytes))
    }
}

fn write_output(output: &mut impl Write, chunk: &[u8]) -> io::Result<()> {
    for segment in chunk.split_inclusive(|byte| *byte == b'\n') {
        output.write_all(segment)?;
        if segment.ends_with(b"\n") {
            output.flush()?;
        }
    }
    output.flush()
}

fn handle_character_key(c: char, modifiers: KeyModifiers, bytes: &mut Vec<u8>) {
    if modifiers.contains(KeyModifiers::CONTROL) {
        let ctrl_char = match c {
            'a'..='z' => ((c as u8 - b'a') + 1) as char,
            'A'..='Z' => ((c as u8 - b'A') + 1) as char,
            '2' => '\x00',
            '3' => '\x1b',
            '4' => '\x1c',
            '5' => '\x1d',
            '6' => '\x1e',
            '7' => '\x1f',
            '8' => '\x7f',
            '?' => '\x7f',
            '[' => '\x1b',
            ']' => '\x1d',
            '^' => '\x1e',
            '_' => '\x1f',
            _ => c,
        };
        bytes.push(ctrl_char as u8);
    } else if modifiers.contains(KeyModifiers::ALT) {
        bytes.push(0x1b);
        bytes.push(c as u8);
    } else {
        bytes.push(c as u8);
    }
}

fn handle_enter_key(modifiers: KeyModifiers, bytes: &mut Vec<u8>) {
    if modifiers.contains(KeyModifiers::ALT) {
        bytes.extend_from_slice(&[0x1b, b'\r']);
    } else if modifiers.contains(KeyModifiers::SHIFT) {
        bytes.extend_from_slice(&[0x1b, b'[', b'Z']);
    } else {
        bytes.push(b'\r');
    }
}

fn handle_backspace_key(modifiers: KeyModifiers, bytes: &mut Vec<u8>) {
    if modifiers.contains(KeyModifiers::ALT) {
        bytes.extend_from_slice(&[0x1b, 0x7f]);
    } else if modifiers.contains(KeyModifiers::CONTROL) {
        bytes.push(b'\x08');
    } else {
        bytes.push(0x7f);
    }
}

fn handle_tab_key(modifiers: KeyModifiers, bytes: &mut Vec<u8>) {
    if modifiers.contains(KeyModifiers::SHIFT) {
        bytes.extend_from_slice(&[0x1b, b'[', b'Z']);
    } else if modifiers.contains(KeyModifiers::ALT) {
        bytes.extend_from_slice(&[0x1b, b'\t']);
    } else if modifiers.contains(KeyModifiers::CONTROL) {
        bytes.extend_from_slice(&[0x1b, b'[', b'I']);
    } else {
        bytes.push(b'\t');
    }
}

fn handle_arrow_key(key: KeyCode, modifiers: KeyModifiers, bytes: &mut Vec<u8>) {
    let base_sequence = match key {
        KeyCode::Up => b'A',
        KeyCode::Down => b'B',
        KeyCode::Right => b'C',
        KeyCode::Left => b'D',
        _ => return,
    };

    if modifiers.contains(KeyModifiers::ALT) {
        bytes.extend_from_slice(&[0x1b, b'[', b'1', b';', b'3', base_sequence]);
    } else if modifiers.contains(KeyModifiers::SHIFT) {
        bytes.extend_from_slice(&[0x1b, b'[', b'1', b';', b'2', base_sequence]);
    } else if modifiers.contains(KeyModifiers::CONTROL) {
        bytes.extend_from_slice(&[0x1b, b'[', b'1', b';', b'5', base_sequence]);
    } else {
        bytes.extend_from_slice(&[0x1b, b'[', base_sequence]);
    }
}

fn handle_home_end_key(key: KeyCode, modifiers: KeyModifiers, bytes: &mut Vec<u8>) {
    let base_sequence = match key {
        KeyCode::Home => b'H',
        KeyCode::End => b'F',
        _ => return,
    };

    if modifiers.contains(KeyModifiers::SHIFT) {
        bytes.extend_from_slice(&[0x1b, b'[', b'1', b';', b'2', base_sequence]);
    } else if modifiers.contains(KeyModifiers::CONTROL) {
        bytes.extend_from_slice(&[0x1b, b'[', b'1', b';', b'5', base_sequence]);
    } else {
        bytes.extend_from_slice(&[0x1b, b'[', base_sequence]);
    }
}

fn handle_page_key(key: KeyCode, modifiers: KeyModifiers, bytes: &mut Vec<u8>) {
    let base_sequence = match key {
        KeyCode::PageUp => b'5',
        KeyCode::PageDown => b'6',
        _ => return,
    };

    if modifiers.contains(KeyModifiers::SHIFT) {
        bytes.extend_from_slice(&[0x1b, b'[', base_sequence, b';', b'2', b'~']);
    } else if modifiers.contains(KeyModifiers::CONTROL) {
        bytes.extend_from_slice(&[0x1b, b'[', base_sequence, b';', b'5', b'~']);
    } else if modifiers.contains(KeyModifiers::ALT) {
        bytes.extend_from_slice(&[0x1b, b'[', base_sequence, b';', b'3', b'~']);
    } else {
        bytes.extend_from_slice(&[0x1b, b'[', base_sequence, b'~']);
    }
}

fn handle_insert_key(modifiers: KeyModifiers, bytes: &mut Vec<u8>) {
    if modifiers.contains(KeyModifiers::SHIFT) {
        bytes.extend_from_slice(&[0x1b, b'[', b'2', b';', b'2', b'~']);
    } else if modifiers.contains(KeyModifiers::CONTROL) {
        bytes.extend_from_slice(&[0x1b, b'[', b'2', b';', b'5', b'~']);
    } else {
        bytes.extend_from_slice(&[0x1b, b'[', b'2', b'~']);
    }
}

fn handle_delete_key(modifiers: KeyModifiers, bytes: &mut Vec<u8>) {
    if modifiers.contains(KeyModifiers::SHIFT) {
        bytes.extend_from_slice(&[0x1b, b'[', b'3', b';', b'2', b'~']);
    } else if modifiers.contains(KeyModifiers::CONTROL) {
        bytes.extend_from_slice(&[0x1b, b'[', b'3', b';', b'5', b'~']);
    } else if modifiers.contains(KeyModifiers::ALT) {
        bytes.extend_from_slice(&[0x1b, b'[', b'3', b';', b'3', b'~']);
    } else {
        bytes.extend_from_slice(&[0x1b, b'[', b'3', b'~']);
    }
}

fn handle_function_key(n: u8, modifiers: KeyModifiers, bytes: &mut Vec<u8>) {
    match n {
        1..=4 => {
            let f_char = match n {
                1 => b'P',
                2 => b'Q',
                3 => b'R',
                4 => b'S',
                _ => return,
            };

            if modifiers.contains(KeyModifiers::SHIFT) {
                bytes.extend_from_slice(&[0x1b, b'[', b'1', b';', b'2', f_char]);
            } else if modifiers.contains(KeyModifiers::ALT) {
                bytes.extend_from_slice(&[0x1b, b'[', b'1', b';', b'3', f_char]);
            } else if modifiers.contains(KeyModifiers::CONTROL) {
                bytes.extend_from_slice(&[0x1b, b'[', b'1', b';', b'5', f_char]);
            } else {
                bytes.extend_from_slice(&[0x1b, b'O', f_char]);
            }
        }
        5..=12 => {
            let f_sequence = match n {
                5 => &b"15"[..],
                6 => &b"17"[..],
                7 => &b"18"[..],
                8 => &b"19"[..],
                9 => &b"20"[..],
                10 => &b"21"[..],
                11 => &b"23"[..],
                12 => &b"24"[..],
                _ => return,
            };

            if modifiers.contains(KeyModifiers::SHIFT) {
                bytes.extend_from_slice(&[0x1b, b'[']);
                bytes.extend_from_slice(f_sequence);
                bytes.extend_from_slice(b";2~");
            } else if modifiers.contains(KeyModifiers::ALT) {
                bytes.extend_from_slice(&[0x1b, b'[']);
                bytes.extend_from_slice(f_sequence);
                bytes.extend_from_slice(b";3~");
            } else if modifiers.contains(KeyModifiers::CONTROL) {
                bytes.extend_from_slice(&[0x1b, b'[']);
                bytes.extend_from_slice(f_sequence);
                bytes.extend_from_slice(b";5~");
            } else {
                bytes.extend_from_slice(&[0x1b, b'[']);
                bytes.extend_from_slice(f_sequence);
                bytes.push(b'~');
            }
        }
        13..=24 => {
            let f_num = n + 12;
            let f_str = f_num.to_string();

            if modifiers.contains(KeyModifiers::SHIFT) {
                bytes.extend_from_slice(&[0x1b, b'[']);
                bytes.extend_from_slice(f_str.as_bytes());
                bytes.extend_from_slice(b";2~");
            } else if modifiers.contains(KeyModifiers::ALT) {
                bytes.extend_from_slice(&[0x1b, b'[']);
                bytes.extend_from_slice(f_str.as_bytes());
                bytes.extend_from_slice(b";3~");
            } else if modifiers.contains(KeyModifiers::CONTROL) {
                bytes.extend_from_slice(&[0x1b, b'[']);
                bytes.extend_from_slice(f_str.as_bytes());
                bytes.extend_from_slice(b";5~");
            } else {
                bytes.extend_from_slice(&[0x1b, b'[']);
                bytes.extend_from_slice(f_str.as_bytes());
                bytes.push(b'~');
            }
        }
        _ => {}
    }
}

pub fn restore_terminal_mode() {
    let _ = disable_raw_mode();
    let _ = Command::new("stty").arg("echo").arg("icanon").status();
    let _ = io::stdout().flush();
}

/// Encode a crossterm MouseEvent as SGR mouse escape bytes (`\x1b[<Cb;Cx;CyM/m`).
///
/// SGR (1006) mode is the de-facto standard for terminal emulators; crossterm enables
/// it automatically when EventStream is active.  We re-encode the already-parsed
/// MouseEvent back into the wire format so the QEMU guest receives correct sequences.
pub fn encode_mouse_event(mouse: MouseEvent) -> Option<Vec<u8>> {
    let button_bits: u8 = match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left) => 0,
        MouseEventKind::Down(MouseButton::Middle) | MouseEventKind::Up(MouseButton::Middle) => 1,
        MouseEventKind::Down(MouseButton::Right) | MouseEventKind::Up(MouseButton::Right) => 2,
        MouseEventKind::Drag(MouseButton::Left) => 32,
        MouseEventKind::Drag(MouseButton::Middle) => 33,
        MouseEventKind::Drag(MouseButton::Right) => 34,
        MouseEventKind::ScrollUp => 64,
        MouseEventKind::ScrollDown => 65,
        MouseEventKind::ScrollLeft => 66,
        MouseEventKind::ScrollRight => 67,
        // Moved events are filtered by the caller; this arm keeps the
        // match exhaustive if the filter is ever removed.
        MouseEventKind::Moved => return None,
    };

    let mut cb = button_bits;
    if mouse.modifiers.contains(KeyModifiers::SHIFT) {
        cb |= 4;
    }
    if mouse.modifiers.contains(KeyModifiers::ALT) {
        cb |= 8;
    }
    if mouse.modifiers.contains(KeyModifiers::CONTROL) {
        cb |= 16;
    }

    let cx = mouse.column + 1;
    let cy = mouse.row + 1;

    let final_byte = match mouse.kind {
        MouseEventKind::Up(_) => b'm',
        _ => b'M',
    };

    Some(format!("\x1b[<{cb};{cx};{cy}{}", final_byte as char).into_bytes())
}

#[cfg(test)]
mod tests {
    use std::{
        io::{self, Cursor, Write},
        sync::{Arc, Mutex},
        time::Duration,
    };

    use crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use tokio::sync::mpsc;

    use super::{KeyProcessor, TerminalAction, TerminalHandle, encode_key_event, write_output};

    struct FlushCountingWriter {
        buf: Vec<u8>,
        flushes: usize,
        writes: Vec<Vec<u8>>,
    }

    impl FlushCountingWriter {
        fn new() -> Self {
            Self {
                buf: Vec::new(),
                flushes: 0,
                writes: Vec::new(),
            }
        }
    }

    impl Write for FlushCountingWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.buf.extend_from_slice(buf);
            self.writes.push(buf.to_vec());
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushes += 1;
            Ok(())
        }
    }

    #[test]
    fn ctrl_a_x_requests_exit() {
        let mut processor = KeyProcessor::new(true);
        assert_eq!(
            processor
                .process_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL))
                .unwrap(),
            TerminalAction::Noop
        );
        assert_eq!(
            processor
                .process_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
                .unwrap(),
            TerminalAction::ExitRequested
        );
    }

    #[test]
    fn ctrl_a_then_other_key_replays_ctrl_a_and_key() {
        let mut processor = KeyProcessor::new(true);
        let _ = processor.process_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
        assert_eq!(
            processor
                .process_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE))
                .unwrap(),
            TerminalAction::SendBytes(vec![0x01, b'b'])
        );
    }

    #[test]
    fn plain_key_encoding_is_preserved() {
        assert_eq!(
            encode_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).unwrap(),
            TerminalAction::SendBytes(vec![b'\r'])
        );
        assert_eq!(
            encode_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)).unwrap(),
            TerminalAction::SendBytes(vec![0x1b, b'[', b'A'])
        );
    }

    #[test]
    fn write_output_flushes_on_newline_boundaries() {
        let mut writer = FlushCountingWriter::new();

        write_output(&mut writer, b"line1\nline2").unwrap();

        assert_eq!(writer.buf, b"line1\nline2");
        assert_eq!(writer.flushes, 2);
    }

    #[test]
    fn write_output_preserves_existing_carriage_returns() {
        let mut writer = FlushCountingWriter::new();

        write_output(&mut writer, b"line1\r\nline2").unwrap();

        assert_eq!(writer.buf, b"line1\r\nline2");
        assert_eq!(writer.flushes, 2);
    }

    #[test]
    fn write_output_submits_complete_segments_instead_of_individual_bytes() {
        let mut writer = FlushCountingWriter::new();

        write_output(&mut writer, b"log line\nprompt").unwrap();

        assert_eq!(
            writer.writes,
            [b"log line\n".as_slice(), b"prompt".as_slice()]
        );
    }

    #[test]
    fn stop_after_does_not_mark_timeout() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let handle = TerminalHandle::new(super::TerminalOutboundSender::Bytes(tx));
        handle.stop_after(Duration::from_millis(10));
        assert!(!handle.timed_out());
        assert!(handle.stop_deadline().is_some());
        assert!(handle.timeout_deadline().is_none());
    }

    #[test]
    fn timeout_after_sets_timeout_deadline_only() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let handle = TerminalHandle::new(super::TerminalOutboundSender::Bytes(tx));
        handle.timeout_after(Duration::from_millis(10));
        assert!(!handle.timed_out());
        assert!(handle.stop_deadline().is_none());
        assert!(handle.timeout_deadline().is_some());
    }

    #[tokio::test]
    async fn send_after_chunks_splits_long_terminal_input() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let handle = TerminalHandle::new(super::TerminalOutboundSender::Bytes(tx));

        handle.send_after_chunks(
            Duration::ZERO,
            b"abcdef".to_vec(),
            2,
            Duration::from_millis(1),
        );

        assert_eq!(rx.recv().await.unwrap(), b"ab");
        assert_eq!(rx.recv().await.unwrap(), b"cd");
        assert_eq!(rx.recv().await.unwrap(), b"ef");
    }

    #[tokio::test]
    async fn acknowledged_input_completes_only_after_writer_acknowledges_flush() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let handle = TerminalHandle::new(super::TerminalOutboundSender::Acknowledged(tx));
        let completed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let completed_clone = completed.clone();

        handle.send_after_chunks_then(
            Duration::ZERO,
            b"abcdef".to_vec(),
            3,
            Duration::ZERO,
            move |_, result| {
                result.unwrap();
                completed_clone.store(true, std::sync::atomic::Ordering::Release);
            },
        );

        let first = rx.recv().await.unwrap();
        assert_eq!(first.bytes(), b"abc");
        assert!(!completed.load(std::sync::atomic::Ordering::Acquire));
        let last = rx.recv().await.unwrap();
        assert_eq!(last.bytes(), b"def");
        assert!(!completed.load(std::sync::atomic::Ordering::Acquire));
        last.acknowledge_flushed();
        tokio::task::yield_now().await;
        assert!(completed.load(std::sync::atomic::Ordering::Acquire));
    }

    #[tokio::test]
    async fn acknowledged_input_reports_writer_failure() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let handle = TerminalHandle::new(super::TerminalOutboundSender::Acknowledged(tx));
        let failure = Arc::new(Mutex::new(None));
        let failure_clone = failure.clone();

        handle.send_after_chunks_then(
            Duration::ZERO,
            vec![b'x'; 192],
            64,
            Duration::ZERO,
            move |_, result| {
                *failure_clone.lock().unwrap() = result.err().map(|err| err.to_string())
            },
        );
        let input = rx.recv().await.unwrap();
        assert_eq!(input.bytes().len(), 64);
        input.acknowledge_failed("serial flush failed");
        tokio::task::yield_now().await;

        assert_eq!(
            failure.lock().unwrap().as_deref(),
            Some("serial flush failed")
        );
    }

    #[test]
    fn acknowledged_send_reports_closed_transport_to_completion_once() {
        let (tx, rx) = mpsc::unbounded_channel();
        let handle = TerminalHandle::new(super::TerminalOutboundSender::Acknowledged(tx));
        drop(rx);
        let completions = Arc::new(Mutex::new(Vec::new()));
        let completions_clone = completions.clone();

        let send_error = handle
            .send_with_completion(
                b"command\n".to_vec(),
                Box::new(move |result| {
                    let error = result.unwrap_err();
                    completions_clone
                        .lock()
                        .unwrap()
                        .push((error.kind(), error.to_string()));
                }),
            )
            .unwrap_err();

        assert_eq!(send_error.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(send_error.to_string(), "terminal transport closed");
        assert_eq!(
            completions.lock().unwrap().as_slice(),
            &[(
                io::ErrorKind::BrokenPipe,
                "terminal transport closed".to_string()
            )]
        );
    }

    #[tokio::test]
    async fn acknowledged_send_reports_terminal_stop_before_enqueue() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let handle = TerminalHandle::new(super::TerminalOutboundSender::Acknowledged(tx));
        handle.stop();
        let (completion_tx, completion_rx) = tokio::sync::oneshot::channel();

        handle.send_after_chunks_then(
            Duration::ZERO,
            b"command\n".to_vec(),
            64,
            Duration::ZERO,
            move |_, result| {
                let error = result.unwrap_err();
                let _ = completion_tx.send((error.kind(), error.to_string()));
            },
        );

        let completion = tokio::time::timeout(Duration::from_millis(100), completion_rx)
            .await
            .expect("send completion was not called after the terminal stopped")
            .unwrap();
        assert_eq!(
            completion,
            (
                io::ErrorKind::Interrupted,
                "terminal stopped before input was sent".to_string()
            )
        );
    }

    #[tokio::test]
    async fn non_tty_mode_consumes_output_without_event_stream() {
        let terminal = super::AsyncTerminal::new(super::TerminalConfig {
            intercept_exit_sequence: true,
            timeout: None,
            timeout_label: "test terminal".to_string(),
        });
        let (inbound_tx, mut inbound_rx) = mpsc::unbounded_channel();
        let (outbound_tx, outbound_rx) = mpsc::unbounded_channel();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_clone = seen.clone();
        let mut written = Vec::new();

        inbound_tx.send(b"hello".to_vec()).unwrap();
        drop(inbound_tx);

        terminal
            .run_with_output_mode(
                &mut inbound_rx,
                super::TerminalOutboundSender::Bytes(outbound_tx),
                &mut Cursor::new(&mut written),
                &mut move |_handle, chunk| seen_clone.lock().unwrap().extend_from_slice(chunk),
                false,
            )
            .await
            .unwrap();

        drop(outbound_rx);
        assert_eq!(written, b"hello");
        assert_eq!(*seen.lock().unwrap(), b"hello");
    }

    #[tokio::test]
    async fn non_tty_mode_still_honors_timeout() {
        let terminal = super::AsyncTerminal::new(super::TerminalConfig {
            intercept_exit_sequence: true,
            timeout: Some(Duration::from_millis(10)),
            timeout_label: "test terminal".to_string(),
        });
        let (_inbound_tx, mut inbound_rx) = mpsc::unbounded_channel();
        let (outbound_tx, _outbound_rx) = mpsc::unbounded_channel();
        let mut written = Vec::new();

        let err = terminal
            .run_with_output_mode(
                &mut inbound_rx,
                super::TerminalOutboundSender::Bytes(outbound_tx),
                &mut Cursor::new(&mut written),
                &mut |_handle, _byte| {},
                false,
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("timed out"));
    }

    #[test]
    fn encode_scroll_up_mouse_event() {
        let mouse = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        // SGR: \x1b[<Cb;Cx;CyM — ScrollUp=64, column+1=11, row+1=6
        assert_eq!(
            super::encode_mouse_event(mouse),
            Some(b"\x1b[<64;11;6M".to_vec())
        );
    }

    #[test]
    fn encode_click_with_shift_modifier() {
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::SHIFT,
        };
        // button=0, SHIFT|=4, so Cb=4
        assert_eq!(
            super::encode_mouse_event(mouse),
            Some(b"\x1b[<4;1;1M".to_vec())
        );
    }

    #[test]
    fn encode_mouse_up_uses_m_terminator() {
        let mouse = MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 2,
            row: 3,
            modifiers: KeyModifiers::NONE,
        };
        // Up events use 'm' terminator
        assert_eq!(
            super::encode_mouse_event(mouse),
            Some(b"\x1b[<0;3;4m".to_vec())
        );
    }

    #[test]
    fn encode_moved_returns_none() {
        let mouse = MouseEvent {
            kind: MouseEventKind::Moved,
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(super::encode_mouse_event(mouse), None);
    }
}
