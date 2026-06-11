use std::{
    fs,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context as _, anyhow, bail};
use httpboot_protocol::{
    BootArch, ImageFormat, SERIAL_PROTOCOL_VERSION, SerialBootOfferMessage, SerialReadyMessage,
    parse_serial_ready, render_serial_boot_offer,
};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};
use tokio_util::compat::{FuturesAsyncReadCompatExt, FuturesAsyncWriteCompatExt};

use crate::{
    board::{
        client::{
            BoardServerClient, BootConfig as RemoteBootConfig, HttpBootKernelUpload,
            SerialStatusResponse, SessionCreatedResponse, UefiBootArch,
        },
        config::BoardRunConfig,
        serial_stream::connect_serial_stream,
    },
    run::{
        execution::{RunnerExecutionSummary, RunnerExitStatus, timeout_duration},
        output_matcher::{
            ByteStreamMatcher, MATCH_DRAIN_DURATION, compile_regexes, print_match_event,
        },
        shell_init::{SHELL_INIT_DELAY, ShellAutoInitMatcher},
        uboot::UbootRunInput,
    },
    sterm::{AsyncTerminal, TerminalConfig},
};

const READY_WAIT_TIMEOUT: Duration = Duration::from_secs(180);
const READY_LINE_LIMIT: usize = 4096;
const BOOT_OFFER_SEND_DELAY: Duration = Duration::from_millis(200);
const BOOT_OFFER_BYTE_DELAY: Duration = Duration::from_millis(2);

pub(crate) async fn run_httpboot_remote(
    input: UbootRunInput,
    board_config: &BoardRunConfig,
    client: BoardServerClient,
    session: SessionCreatedResponse,
) -> anyhow::Result<()> {
    let runner = HttpBootBoardRunner {
        input,
        board_config: board_config.clone(),
        client,
        session,
    };
    runner.run().await
}

struct HttpBootBoardRunner {
    input: UbootRunInput,
    board_config: BoardRunConfig,
    client: BoardServerClient,
    session: SessionCreatedResponse,
}

impl HttpBootBoardRunner {
    async fn run(self) -> anyhow::Result<()> {
        let boot_profile = self
            .client
            .get_boot_profile(&self.session.session_id)
            .await
            .context("failed to get HTTP Boot profile")?;
        let RemoteBootConfig::UefiHttp(profile) = boot_profile.boot else {
            bail!(
                "unsupported board boot mode `{}`; expected `httpboot`",
                self.session.boot_mode
            );
        };
        let arch = profile
            .boot_arch
            .map(boot_arch_from_profile)
            .unwrap_or(BootArch::X86_64);
        let elf_path = self
            .input
            .artifacts()
            .elf()
            .ok_or_else(|| anyhow!("HTTP Boot requires a prepared runtime ELF"))?;
        let kernel_bytes =
            fs::read(elf_path).with_context(|| format!("failed to read {}", elf_path.display()))?;
        let kernel_sha256 = hex_sha256(&kernel_bytes);
        let upload = self
            .client
            .upload_http_boot_kernel(
                &self.session.session_id,
                HttpBootKernelUpload {
                    remote_name: "kernel.elf".into(),
                    arch: boot_arch_name(arch).into(),
                    image_format: "elf64".into(),
                    entry_symbol: Some("httpboot_entry".into()),
                    bytes: kernel_bytes,
                },
            )
            .await
            .context("failed to upload HTTP Boot kernel")?;

        println!("=== Axvisor HTTP Boot board run ===");
        println!("board_id: {}", self.session.board_id);
        println!("session_id: {}", self.session.session_id);
        println!("kernel: {}", elf_path.display());
        println!("kernel_url: {}", upload.kernel_url);
        println!("kernel_size: {:#x}", upload.kernel_size);
        println!("kernel_sha256: {kernel_sha256}");

        let serial_status = self
            .client
            .get_serial_status(&self.session.session_id)
            .await
            .context("failed to get HTTP Boot serial status")?;
        let ws_url = serial_ws_url(&self.client, &self.session.session_id, &serial_status)?;
        println!(
            "serial_console: {} @ {}",
            serial_status.port.as_deref().unwrap_or("unknown"),
            serial_status
                .baud_rate
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".into())
        );
        println!("Waiting for axloader on board serial...");

        let (serial_tx, serial_rx, tasks) = connect_serial_stream(ws_url).await?;
        let mut serial_rx = serial_rx.compat();
        let mut serial_tx = serial_tx.compat_write();
        wait_for_loader_ready(&mut serial_rx, arch).await?;
        tokio::time::sleep(BOOT_OFFER_SEND_DELAY).await;

        let offer = SerialBootOfferMessage {
            protocol_version: SERIAL_PROTOCOL_VERSION,
            boot_id: upload.boot_id,
            kernel_url: upload.kernel_url,
            kernel_size: upload.kernel_size,
            image_format: ImageFormat::Elf64,
            arch,
            entry_symbol: Some("httpboot_entry".into()),
        };
        let line = render_serial_boot_offer(&offer).context("failed to render boot offer")?;
        println!("{line}");
        send_boot_offer_line(&mut serial_tx, &line).await?;
        println!("HTTP Boot offer sent, entering serial terminal...");

        let result = run_terminal(
            serial_rx,
            serial_tx,
            line,
            arch,
            self.board_config.success_regex,
            self.board_config.fail_regex,
            self.board_config.shell_prefix,
            self.board_config.shell_init_cmd,
            self.board_config.timeout,
        )
        .await;
        let shutdown_result = tasks.shutdown_with_timeout(Duration::from_secs(2)).await;
        if result.is_ok() {
            shutdown_result?;
        } else if let Err(err) = shutdown_result {
            log::warn!("serial websocket cleanup failed: {err:#}");
        }
        result
    }
}

async fn send_boot_offer_line<W>(serial_tx: &mut W, line: &str) -> anyhow::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    write_serial_line_slowly(serial_tx, line).await?;
    serial_tx
        .write_all(b"\n")
        .await
        .context("failed to terminate HTTP Boot offer line")?;
    serial_tx
        .flush()
        .await
        .context("failed to flush HTTP Boot offer")?;
    Ok(())
}

async fn write_serial_line_slowly<W>(serial_tx: &mut W, line: &str) -> anyhow::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    for byte in line.as_bytes() {
        serial_tx
            .write_all(core::slice::from_ref(byte))
            .await
            .context("failed to write HTTP Boot offer byte")?;
        serial_tx
            .flush()
            .await
            .context("failed to flush HTTP Boot offer byte")?;
        tokio::time::sleep(BOOT_OFFER_BYTE_DELAY).await;
    }
    Ok(())
}

async fn wait_for_loader_ready<R>(serial_rx: &mut R, arch: BootArch) -> anyhow::Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut line = Vec::new();
    let started = Instant::now();
    let mut buffer = [0u8; 256];
    while started.elapsed() < READY_WAIT_TIMEOUT {
        let read = tokio::time::timeout(Duration::from_millis(250), serial_rx.read(&mut buffer))
            .await
            .ok()
            .transpose()
            .context("failed to read serial while waiting for axloader")?
            .unwrap_or(0);
        if read == 0 {
            continue;
        }
        for byte in &buffer[..read] {
            match *byte {
                b'\r' => {}
                b'\n' => {
                    let text = String::from_utf8_lossy(&line).to_string();
                    println!("{text}");
                    if let Ok(ready) = parse_serial_ready(&text) {
                        validate_ready(&ready, arch)?;
                        return Ok(());
                    }
                    line.clear();
                }
                byte => {
                    if line.len() >= READY_LINE_LIMIT {
                        line.clear();
                    }
                    line.push(byte);
                }
            }
        }
    }

    if !line.is_empty() {
        println!("{}", String::from_utf8_lossy(&line));
    }
    bail!("timed out waiting for axloader ready on board serial")
}

fn validate_ready(ready: &SerialReadyMessage, expected_arch: BootArch) -> anyhow::Result<()> {
    if ready.protocol_version != SERIAL_PROTOCOL_VERSION {
        bail!(
            "unsupported axloader serial protocol version `{}`",
            ready.protocol_version
        );
    }
    if ready.arch != expected_arch {
        bail!(
            "axloader arch {:?} does not match published kernel arch {:?}",
            ready.arch,
            expected_arch
        );
    }
    Ok(())
}

async fn run_terminal<R, W>(
    serial_rx: R,
    serial_tx: W,
    boot_offer_line: String,
    arch: BootArch,
    success_regex: Vec<String>,
    fail_regex: Vec<String>,
    shell_prefix: Option<String>,
    shell_init_cmd: Option<String>,
    timeout: Option<u64>,
) -> anyhow::Result<()>
where
    R: tokio::io::AsyncRead + Send + Unpin + 'static,
    W: tokio::io::AsyncWrite + Send + Unpin + 'static,
{
    let (success_regex, fail_regex) = compile_regexes(&success_regex, &fail_regex)?;
    let matcher = Arc::new(Mutex::new(ByteStreamMatcher::new(
        success_regex,
        fail_regex,
    )));
    let res = Arc::new(Mutex::new(None));
    let res_clone = res.clone();
    let matcher_clone = matcher.clone();
    let shell_init = Arc::new(Mutex::new(ShellAutoInitMatcher::new(
        shell_prefix,
        shell_init_cmd,
    )));
    let shell_init_clone = shell_init.clone();
    let ready_monitor = Arc::new(Mutex::new(LoaderReadyMonitor::new(arch)));
    let ready_monitor_clone = ready_monitor.clone();
    let boot_offer_bytes = boot_offer_line_bytes(&boot_offer_line);

    let (inbound_tx, inbound_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<Vec<u8>>();

    let read_task = tokio::spawn(async move {
        let mut serial_rx = serial_rx;
        let mut buffer = [0u8; 1024];
        loop {
            let read = serial_rx
                .read(&mut buffer)
                .await
                .context("failed to read serial output")?;
            if read == 0 {
                break;
            }
            if inbound_tx.send(buffer[..read].to_vec()).is_err() {
                break;
            }
        }
        Ok::<(), anyhow::Error>(())
    });

    let write_task = tokio::spawn(async move {
        let mut serial_tx = serial_tx;
        while let Some(bytes) = outbound_rx.recv().await {
            serial_tx
                .write_all(&bytes)
                .await
                .context("failed to write serial input")?;
            serial_tx
                .flush()
                .await
                .context("failed to flush serial input")?;
        }
        Ok::<(), anyhow::Error>(())
    });

    let terminal = AsyncTerminal::new(TerminalConfig {
        intercept_exit_sequence: true,
        timeout: timeout_duration(timeout),
        timeout_label: "HTTP Boot kernel boot".to_string(),
    });
    let started_at = Instant::now();
    let terminal_result = terminal
        .run(inbound_rx, outbound_tx, move |handle, byte| {
            let mut matcher = matcher_clone.lock().unwrap();
            if let Some(matched) = matcher.observe_byte(byte) {
                print_match_event(&matched);
                let mut res_lock = res_clone.lock().unwrap();
                *res_lock = Some(matched);
                handle.stop_after(MATCH_DRAIN_DURATION);
            }

            let mut shell_init = shell_init_clone.lock().unwrap();
            if let Some(shell_init) = shell_init.as_mut()
                && let Some(command) = shell_init.observe_byte(byte)
            {
                handle.send_after(SHELL_INIT_DELAY, command);
            }

            if matcher.should_stop() {
                handle.stop();
            }

            let mut ready_monitor = ready_monitor_clone.lock().unwrap();
            if ready_monitor.observe_byte(byte) {
                handle.send_after(BOOT_OFFER_SEND_DELAY, boot_offer_bytes.clone());
            }
        })
        .await;

    shutdown_serial_task(write_task, Duration::from_secs(1)).await?;
    shutdown_serial_task(read_task, Duration::from_millis(300)).await?;

    let mut res_lock = res.lock().unwrap();
    RunnerExecutionSummary::new(
        "HTTP Boot kernel boot",
        RunnerExitStatus::not_available(),
        started_at.elapsed(),
    )
    .with_terminal_error(terminal_result.err())
    .with_stream_match(res_lock.take())
    .into_result()
}

fn boot_offer_line_bytes(line: &str) -> Vec<u8> {
    let mut bytes = line.as_bytes().to_vec();
    bytes.push(b'\n');
    bytes
}

struct LoaderReadyMonitor {
    arch: BootArch,
    line: Vec<u8>,
}

impl LoaderReadyMonitor {
    fn new(arch: BootArch) -> Self {
        Self {
            arch,
            line: Vec::new(),
        }
    }

    fn observe_byte(&mut self, byte: u8) -> bool {
        match byte {
            b'\r' => false,
            b'\n' => {
                let ready = self.observe_line();
                self.line.clear();
                ready
            }
            byte => {
                if self.line.len() >= READY_LINE_LIMIT {
                    self.line.clear();
                }
                self.line.push(byte);
                false
            }
        }
    }

    fn observe_line(&self) -> bool {
        let text = String::from_utf8_lossy(&self.line);
        parse_serial_ready(&text)
            .map(|ready| validate_ready(&ready, self.arch).is_ok())
            .unwrap_or(false)
    }
}

async fn shutdown_serial_task(
    mut task: tokio::task::JoinHandle<anyhow::Result<()>>,
    timeout: Duration,
) -> anyhow::Result<()> {
    match tokio::time::timeout(timeout, &mut task).await {
        Ok(Ok(Ok(()))) => Ok(()),
        Ok(Ok(Err(err))) => Err(err),
        Ok(Err(err)) if !err.is_cancelled() => Err(anyhow!("serial task join error: {err}")),
        Ok(Err(_)) => Ok(()),
        Err(_) => {
            task.abort();
            let _ = task.await;
            Ok(())
        }
    }
}

fn serial_ws_url(
    client: &BoardServerClient,
    session_id: &str,
    serial_status: &SerialStatusResponse,
) -> anyhow::Result<reqwest::Url> {
    if !serial_status.available {
        bail!("session `{session_id}` has no serial console available");
    }
    let ws_path = serial_status
        .ws_url
        .as_deref()
        .ok_or_else(|| anyhow!("server did not return a serial websocket URL"))?;
    client.resolve_ws_url(ws_path)
}

fn boot_arch_from_profile(arch: UefiBootArch) -> BootArch {
    match arch {
        UefiBootArch::X86_64 => BootArch::X86_64,
        UefiBootArch::Aarch64 => BootArch::Aarch64,
        UefiBootArch::Loongarch64 => BootArch::Loongarch64,
        UefiBootArch::Riscv64 => BootArch::Riscv64,
        UefiBootArch::Other => BootArch::Other,
    }
}

fn boot_arch_name(arch: BootArch) -> &'static str {
    match arch {
        BootArch::X86_64 => "x86_64",
        BootArch::Aarch64 => "aarch64",
        BootArch::Loongarch64 => "loongarch64",
        BootArch::Riscv64 => "riscv64",
        BootArch::Other => "other",
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}
