use std::{
    fs,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context as _, anyhow, bail};
use httpboot_protocol::{BootArch, LoaderStatusPhase};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};
use tokio_util::compat::{FuturesAsyncReadCompatExt, FuturesAsyncWriteCompatExt};

use crate::{
    board::{
        client::{
            BoardServerClient, HttpBootKernelUpload, SerialStatusResponse, SessionCreatedResponse,
        },
        config::BoardRunConfig,
    },
    run::{
        execution::{RunnerExecutionSummary, RunnerExitStatus, timeout_duration},
        output_matcher::{
            FailStreamMatcher, MATCH_DRAIN_DURATION, compile_fail_regexes, print_fail_match,
        },
        shell_check::{
            ShellCheckDriver, ShellCheckMatcher, ShellCheckStep, normalize_shell_check_steps,
        },
        uboot::UbootRunInput,
    },
    sterm::{AsyncTerminal, TerminalConfig},
};

const LOADER_STATUS_POLL_INTERVAL: Duration = Duration::from_millis(500);

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
        anyhow::ensure!(
            matches!(self.session.boot_mode.as_str(), "httpboot" | "uefi_http"),
            "unsupported board boot mode `{}`; expected `httpboot`",
            self.session.boot_mode
        );
        let arch = boot_arch_from_elf(self.input.arch())?;
        let elf_path = self
            .input
            .artifacts()
            .elf()
            .ok_or_else(|| anyhow!("HTTP Boot requires a prepared runtime ELF"))?;
        let kernel_bytes =
            fs::read(elf_path).with_context(|| format!("failed to read {}", elf_path.display()))?;
        anyhow::ensure!(
            kernel_bytes.starts_with(b"\x7fELF\x02"),
            "HTTP Boot requires an ELF64 kernel"
        );
        let kernel_sha256 = hex_sha256(&kernel_bytes);
        let initramfs = match self.board_config.boot.initramfs_path() {
            Some(path) => {
                let bytes = fs::read(&path)
                    .with_context(|| format!("failed to read host initramfs {}", path.display()))?;
                anyhow::ensure!(
                    !bytes.is_empty(),
                    "host initramfs is empty: {}",
                    path.display()
                );
                anyhow::ensure!(
                    bytes.len() <= httpboot_protocol::MAX_HTTP_BOOT_INITRAMFS_BYTES,
                    "host initramfs exceeds HTTP Boot loader limit: {}",
                    path.display()
                );
                Some(bytes)
            }
            None => None,
        };
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
                    initramfs,
                    cmdline: self.board_config.boot.cmdline.clone(),
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
        println!("Waiting for axloader through the network control plane...");

        let (serial_tx, serial_rx, tasks) =
            crate::board::serial_stream::connect_serial_stream_with_commands(
                ws_url,
                self.client.websocket_authorization().await?,
                crate::board::serial_stream::registered_command_ids(
                    &self.board_config.shell_check_steps,
                ),
            )
            .await?;
        let serial_rx = serial_rx.compat();
        let serial_tx = serial_tx.compat_write();
        println!("Serial is connected for target-system interaction only.");
        let status_client = self.client.clone();
        let status_session_id = self.session.session_id.clone();
        let result = {
            let terminal = run_terminal(
                serial_rx,
                serial_tx,
                TerminalRunOptions {
                    fail_regex: self.board_config.fail_regex,
                    shell_check_steps: self.board_config.shell_check_steps,
                    timeout: self.board_config.timeout,
                },
            );
            let status = monitor_loader_status(status_client, status_session_id);
            tokio::pin!(terminal);
            tokio::pin!(status);
            tokio::select! {
                result = &mut terminal => result,
                result = &mut status => result,
            }
        };
        let shutdown_result = tasks.shutdown_with_timeout(Duration::from_secs(2)).await;
        match (result, shutdown_result) {
            (Err(error), Err(shutdown_error)) => Err(error.context(format!(
                "serial websocket cleanup failed: {shutdown_error:#}"
            ))),
            (Err(error), Ok(())) => Err(error),
            (Ok(()), shutdown_result) => shutdown_result,
        }
    }
}

async fn monitor_loader_status(
    client: BoardServerClient,
    session_id: String,
) -> anyhow::Result<()> {
    let mut last_phase = None;
    loop {
        let status = client
            .get_loader_status(&session_id)
            .await
            .context("failed to query axloader network status")?;
        if let Some(phase) = status.status {
            let phase_name = loader_phase_name(&phase);
            if last_phase.as_deref() != Some(phase_name) {
                println!("axloader: {phase_name}");
                last_phase = Some(phase_name.to_string());
            }
            if let LoaderStatusPhase::Failed { code, message } = phase {
                bail!("axloader failed ({code}): {message}");
            }
        }
        tokio::time::sleep(LOADER_STATUS_POLL_INTERVAL).await;
    }
}

fn loader_phase_name(phase: &LoaderStatusPhase) -> &'static str {
    match phase {
        LoaderStatusPhase::Accepted => "accepted",
        LoaderStatusPhase::Downloading { .. } => "downloading",
        LoaderStatusPhase::Verified => "verified",
        LoaderStatusPhase::ReadyToHandoff => "ready_to_handoff",
        LoaderStatusPhase::Failed { .. } => "failed",
    }
}

pub(super) struct TerminalRunOptions {
    pub(super) fail_regex: Vec<String>,
    pub(super) shell_check_steps: Vec<ShellCheckStep>,
    pub(super) timeout: Option<u64>,
}

async fn write_board_input<W>(
    writer: &mut W,
    input: crate::sterm::TerminalInput,
) -> anyhow::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    if let Err(error) = writer.write_all(input.bytes()).await {
        input.acknowledge_failed(format!(
            "failed to write HTTP Boot board serial input: {error}"
        ));
        return Err(error).context("failed to write HTTP Boot board serial input");
    }
    if let Err(error) = writer.flush().await {
        input.acknowledge_failed(format!(
            "failed to flush HTTP Boot board serial input: {error}"
        ));
        return Err(error).context("failed to flush HTTP Boot board serial input");
    }
    input.acknowledge_flushed();
    Ok(())
}

pub(super) async fn run_terminal<R, W>(
    serial_rx: R,
    serial_tx: W,
    options: TerminalRunOptions,
) -> anyhow::Result<()>
where
    R: tokio::io::AsyncRead + Send + Unpin + 'static,
    W: tokio::io::AsyncWrite + Send + Unpin + 'static,
{
    let TerminalRunOptions {
        fail_regex,
        shell_check_steps,
        timeout,
    } = options;
    let fail_regex = compile_fail_regexes(&fail_regex)?;
    let matcher = Arc::new(Mutex::new(FailStreamMatcher::new(fail_regex)));
    let res = Arc::new(Mutex::new(None));
    let res_clone = res.clone();
    let matcher_clone = matcher.clone();
    let shell_check_matcher = if shell_check_steps.is_empty() {
        None
    } else {
        let mut steps = shell_check_steps;
        let resolved = normalize_shell_check_steps(&mut steps, "HTTP Boot runtime config")?;
        Some(ShellCheckMatcher::from_steps(resolved)?)
    };
    let shell_check_driver = shell_check_matcher.map(ShellCheckDriver::new);
    let shell_check_driver_clone = shell_check_driver.clone();

    let (inbound_tx, inbound_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<crate::sterm::TerminalInput>();

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
        while let Some(input) = outbound_rx.recv().await {
            write_board_input(&mut serial_tx, input).await?;
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
        .run_with_write_ack(inbound_rx, outbound_tx, move |handle, chunk| {
            let mut matcher = matcher_clone.lock().unwrap();
            for byte in chunk {
                if let Some(matched) = matcher.observe_byte(*byte) {
                    print_fail_match(&matched);
                    let mut res_lock = res_clone.lock().unwrap();
                    *res_lock = Some(matched);
                    handle.stop_after(MATCH_DRAIN_DURATION);
                }
            }

            if let Some(shell_check_driver) = shell_check_driver_clone.as_ref() {
                shell_check_driver.observe_chunk(handle, chunk);
            }

            if matcher.should_stop() {
                handle.stop();
            }
        })
        .await;

    shutdown_serial_task(write_task, Duration::from_secs(1)).await?;
    shutdown_serial_task(read_task, Duration::from_millis(300)).await?;

    let shell_check_completed = shell_check_driver
        .as_ref()
        .is_some_and(ShellCheckDriver::completed);
    let shell_check_failure = shell_check_driver
        .as_ref()
        .and_then(ShellCheckDriver::completion_error);
    let mut res_lock = res.lock().unwrap();
    RunnerExecutionSummary::new(
        "HTTP Boot kernel boot",
        RunnerExitStatus::not_available(),
        started_at.elapsed(),
    )
    .with_terminal_error(terminal_result.err())
    .with_shell_check_error(shell_check_failure)
    .with_shell_check_completed(shell_check_completed)
    .with_fail_match(res_lock.take())
    .into_result()
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

fn boot_arch_from_elf(arch: Option<object::Architecture>) -> anyhow::Result<BootArch> {
    match arch {
        Some(object::Architecture::X86_64) => Ok(BootArch::X86_64),
        Some(object::Architecture::Riscv64) => Ok(BootArch::Riscv64),
        Some(object::Architecture::LoongArch64) => Ok(BootArch::Loongarch64),
        Some(arch) => bail!("unsupported HTTP Boot kernel architecture: {arch:?}"),
        None => bail!("HTTP Boot requires a detected kernel ELF architecture"),
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

#[cfg(test)]
mod tests {
    use std::{
        pin::Pin,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
        task::{Context, Poll},
        time::Duration,
    };

    use tokio::io::AsyncWrite;

    use super::{HttpBootBoardRunner, write_board_input};
    use crate::{
        artifact::{
            runtime::{RuntimeArtifactOptions, prepare_runtime_artifacts},
            state::OutputArtifacts,
        },
        board::{
            client::{
                BoardServerClient, SessionCreatedResponse,
                tests::{read_http_request_bytes, write_http_response},
            },
            config::BoardRunConfig,
        },
        process::ProcessContext,
        project::variables::VariableScope,
        run::uboot::UbootRunInput,
    };

    fn minimal_elf(machine: u16, class: u8) -> Vec<u8> {
        let mut bytes = vec![0_u8; 0x104];
        bytes[..4].copy_from_slice(b"\x7fELF");
        bytes[4] = class;
        bytes[5] = object::elf::ELFDATA2LSB;
        bytes[6] = object::elf::EV_CURRENT;
        bytes[16..18].copy_from_slice(&object::elf::ET_EXEC.to_le_bytes());
        bytes[18..20].copy_from_slice(&machine.to_le_bytes());
        bytes[20..24].copy_from_slice(&u32::from(object::elf::EV_CURRENT).to_le_bytes());
        if class == object::elf::ELFCLASS64 {
            bytes[24..32].copy_from_slice(&0x8000_0000_u64.to_le_bytes());
            bytes[32..40].copy_from_slice(&64_u64.to_le_bytes());
            bytes[52..54].copy_from_slice(&64_u16.to_le_bytes());
            bytes[54..56].copy_from_slice(&56_u16.to_le_bytes());
            bytes[56..58].copy_from_slice(&1_u16.to_le_bytes());
            bytes[64..68].copy_from_slice(&object::elf::PT_LOAD.to_le_bytes());
            bytes[68..72].copy_from_slice(&(object::elf::PF_R | object::elf::PF_X).to_le_bytes());
            bytes[72..80].copy_from_slice(&0x100_u64.to_le_bytes());
            bytes[80..88].copy_from_slice(&0x8000_0000_u64.to_le_bytes());
            bytes[88..96].copy_from_slice(&0x8000_0000_u64.to_le_bytes());
            bytes[96..104].copy_from_slice(&4_u64.to_le_bytes());
            bytes[104..112].copy_from_slice(&4_u64.to_le_bytes());
            bytes[112..120].copy_from_slice(&4_u64.to_le_bytes());
        } else {
            bytes[40..42].copy_from_slice(&52_u16.to_le_bytes());
        }
        bytes
    }

    fn test_input(root: &std::path::Path, bytes: &[u8]) -> UbootRunInput {
        let elf = root.join("kernel.elf");
        std::fs::write(&elf, bytes).unwrap();
        let context = ProcessContext::new(
            root.to_path_buf(),
            root.to_path_buf(),
            VariableScope::new(root.to_path_buf(), root.to_path_buf(), root.to_path_buf()),
            Some(elf.clone()),
        );
        let prepared = prepare_runtime_artifacts(
            &context,
            RuntimeArtifactOptions {
                elf_path: elf,
                to_bin: false,
                bin_dir: None,
                debug: false,
                cargo_artifact_dir: None,
                strip_elf: false,
            },
        )
        .unwrap();
        let mut artifacts = OutputArtifacts::default();
        artifacts.apply_prepared_runtime_artifacts(&prepared);
        UbootRunInput::new(context, artifacts, prepared.arch())
    }

    fn test_runner(input: UbootRunInput, port: u16, boot_mode: &str) -> HttpBootBoardRunner {
        HttpBootBoardRunner {
            input,
            board_config: BoardRunConfig::default(),
            client: BoardServerClient::new("127.0.0.1", port).unwrap(),
            session: SessionCreatedResponse {
                session_id: "demo".into(),
                board_id: "board-1".into(),
                lease_expires_at: chrono::Utc::now(),
                serial_available: true,
                boot_mode: boot_mode.into(),
                ws_url: None,
            },
        }
    }

    #[derive(Clone, Copy)]
    enum PreflightFailureDelivery {
        Websocket,
        DuringShutdown,
        LoaderStatus,
    }

    async fn run_preflight_failure(
        delivery: PreflightFailureDelivery,
        reason: &'static str,
    ) -> String {
        use futures::{SinkExt, StreamExt};
        use tokio::sync::oneshot;
        use tokio_tungstenite::tungstenite::Message;

        let temp = tempfile::tempdir().unwrap();
        let input = test_input(
            temp.path(),
            &minimal_elf(object::elf::EM_RISCV, object::elf::ELFCLASS64),
        );
        let http = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http_address = http.local_addr().unwrap();
        let websocket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let websocket_address = websocket.local_addr().unwrap();
        let (poll_started, poll_received) = oneshot::channel();
        let (failure_sent, failure_received) = oneshot::channel();
        let websocket_server = tokio::spawn(async move {
            let (socket, _) = websocket.accept().await.unwrap();
            let mut websocket = tokio_tungstenite::accept_async(socket).await.unwrap();
            if matches!(delivery, PreflightFailureDelivery::Websocket) {
                poll_received.await.unwrap();
            } else {
                // A status response wins first. Deliver the server error during
                // cleanup, after the runner releases its terminal writer.
                assert_eq!(
                    websocket.next().await.unwrap().unwrap(),
                    Message::Text(r#"{"type":"close"}"#.into())
                );
            }
            if !matches!(delivery, PreflightFailureDelivery::LoaderStatus) {
                websocket
                    .send(Message::Text(
                        serde_json::json!({"type": "error", "message": reason})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
            }
            websocket.close(None).await.unwrap();
            let _ = failure_sent.send(());
        });
        let http_server = tokio::spawn(async move {
            let (mut upload, _) = http.accept().await.unwrap();
            let request = read_http_request_bytes(&mut upload).await;
            assert!(request.starts_with(b"PUT /api/v1/sessions/demo/http-boot/kernel "));
            let response = serde_json::json!({
                "boot_id": "boot-1",
                "kernel_url": format!("http://{http_address}/boot/sessions/demo/kernel.elf"),
                "kernel_size": 260,
                "kernel_sha256": null
            });
            write_http_response(&mut upload, "201 Created", &response.to_string()).await;
            let (mut serial, _) = http.accept().await.unwrap();
            let request = read_http_request_bytes(&mut serial).await;
            assert!(request.starts_with(b"GET /api/v1/sessions/demo/serial "));
            let response = serde_json::json!({
                "available": true,
                "connected": false,
                "ws_url": format!("ws://{websocket_address}/serial")
            });
            write_http_response(&mut serial, "200 OK", &response.to_string()).await;
            let (mut status, _) = http.accept().await.unwrap();
            let request = read_http_request_bytes(&mut status).await;
            assert!(request.starts_with(b"GET /api/v1/sessions/demo/loader-status "));
            if matches!(delivery, PreflightFailureDelivery::LoaderStatus) {
                let response = serde_json::json!({
                    "session_id": "demo",
                    "boot_id": "boot-1",
                    "registration_id": "epoch-1",
                    "status": {"phase": "failed", "code": "boot_preflight_failed", "message": reason}
                });
                write_http_response(&mut status, "200 OK", &response.to_string()).await;
            } else {
                if matches!(delivery, PreflightFailureDelivery::Websocket) {
                    poll_started.send(()).unwrap();
                    failure_received.await.unwrap();
                }
                write_http_response(
                    &mut status,
                    "404 Not Found",
                    r#"{"code":"not_found","message":"session not found"}"#,
                )
                .await;
            }
        });
        let mut runner = test_runner(input, http_address.port(), "httpboot");
        runner.board_config.shell_check_steps = vec![super::ShellCheckStep {
            success_regex: Some(vec!["KERNEL_READY".into()]),
            ..Default::default()
        }];
        let result = tokio::time::timeout(Duration::from_secs(6), runner.run()).await;
        http_server.abort();
        websocket_server.abort();
        let _ = http_server.await;
        let _ = websocket_server.await;
        let error = result
            .expect("HTTP Boot runner did not finish after a preflight failure")
            .expect_err("preflight failure must fail the HTTP Boot run");
        format!("{error:#}")
    }

    #[tokio::test]
    async fn httpboot_preserves_websocket_preflight_error_after_session_release() {
        let reason = "kernel architecture Riscv64 does not match device X86_64";
        let error = run_preflight_failure(PreflightFailureDelivery::Websocket, reason).await;
        assert!(error.contains(reason), "{error}");
    }

    #[tokio::test]
    async fn httpboot_preserves_preflight_error_received_during_shutdown() {
        let reason = "kernel ELF architecture Riscv64 does not match declared X86_64";
        let error = run_preflight_failure(PreflightFailureDelivery::DuringShutdown, reason).await;
        assert!(error.contains(reason), "{error}");
    }

    #[tokio::test]
    async fn httpboot_preserves_polled_preflight_failure() {
        let reason = "kernel architecture Riscv64 does not match device Loongarch64";
        let error = run_preflight_failure(PreflightFailureDelivery::LoaderStatus, reason).await;
        assert!(error.contains("boot_preflight_failed"), "{error}");
        assert!(error.contains(reason), "{error}");
    }

    #[tokio::test]
    async fn httpboot_upload_uses_elf_architecture_without_boot_profile() {
        for (machine, arch, mode) in [
            (object::elf::EM_X86_64, "x86_64", "httpboot"),
            (object::elf::EM_RISCV, "riscv64", "httpboot"),
            (object::elf::EM_LOONGARCH, "loongarch64", "uefi_http"),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let bytes = minimal_elf(machine, object::elf::ELFCLASS64);
            let input = test_input(temp.path(), &bytes);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = read_http_request_bytes(&mut socket).await;
                write_http_response(&mut socket, "409 Conflict", "upload intercepted").await;
                request
            });

            let error = test_runner(input, port, mode).run().await.unwrap_err();
            let request = server.await.unwrap();
            let header_end = request
                .windows(4)
                .position(|part| part == b"\r\n\r\n")
                .unwrap();
            let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();

            assert!(
                headers.starts_with("put /api/v1/sessions/demo/http-boot/kernel "),
                "{headers}"
            );
            assert!(
                headers.contains(&format!("x-httpboot-arch: {arch}\r\n")),
                "{headers}"
            );
            assert!(
                headers.contains("x-httpboot-image-format: elf64\r\n"),
                "{headers}"
            );
            assert_eq!(&request[header_end + 4..], bytes);
            assert_eq!(error.to_string(), "failed to upload HTTP Boot kernel");
        }
    }

    #[tokio::test]
    async fn httpboot_rejects_unsupported_kernel_architectures_before_upload() {
        for (machine, class) in [
            (object::elf::EM_AARCH64, object::elf::ELFCLASS64),
            (object::elf::EM_NONE, object::elf::ELFCLASS64),
            (object::elf::EM_X86_64, object::elf::ELFCLASS32),
            (object::elf::EM_RISCV, object::elf::ELFCLASS32),
            (object::elf::EM_LOONGARCH, object::elf::ELFCLASS32),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let input = test_input(temp.path(), &minimal_elf(machine, class));
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            drop(listener);

            let error = test_runner(input, port, "httpboot")
                .run()
                .await
                .unwrap_err();

            assert!(
                error
                    .to_string()
                    .contains("unsupported HTTP Boot kernel architecture"),
                "{error:#}"
            );
        }
    }

    #[tokio::test]
    async fn httpboot_rejects_non_elf_kernel_before_upload() {
        let temp = tempfile::tempdir().unwrap();
        let mut bytes = vec![0_u8; 32];
        bytes[..4].copy_from_slice(&object::macho::MH_MAGIC_64.to_le_bytes());
        bytes[4..8].copy_from_slice(&object::macho::CPU_TYPE_X86_64.to_le_bytes());
        bytes[12..16].copy_from_slice(&object::macho::MH_EXECUTE.to_le_bytes());
        let input = test_input(temp.path(), &bytes);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let error = test_runner(input, port, "httpboot")
            .run()
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "HTTP Boot requires an ELF64 kernel");
    }

    #[tokio::test]
    async fn httpboot_rejects_missing_kernel_architecture_before_upload() {
        let temp = tempfile::tempdir().unwrap();
        let context = ProcessContext::new(
            temp.path().into(),
            temp.path().into(),
            VariableScope::new(temp.path().into(), temp.path().into(), temp.path().into()),
            None,
        );
        let input = UbootRunInput::new(context, OutputArtifacts::default(), None);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let error = test_runner(input, port, "httpboot")
            .run()
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "HTTP Boot requires a detected kernel ELF architecture"
        );
    }

    #[derive(Clone, Copy)]
    enum WriterFailure {
        None,
        Write,
        Flush,
    }

    struct FlushCheckingWriter {
        callback_ran: Arc<AtomicBool>,
        failure: WriterFailure,
        flushes: usize,
    }

    impl AsyncWrite for FlushCheckingWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            if matches!(self.failure, WriterFailure::Write) {
                Poll::Ready(Err(std::io::Error::other("injected write failure")))
            } else {
                Poll::Ready(Ok(bytes.len()))
            }
        }

        fn poll_flush(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            assert!(!self.callback_ran.load(Ordering::Acquire));
            if matches!(self.failure, WriterFailure::Flush) {
                return Poll::Ready(Err(std::io::Error::other("injected flush failure")));
            }
            self.flushes += 1;
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn board_writer_acknowledges_only_after_flush() {
        let callback_ran = Arc::new(AtomicBool::new(false));
        let callback_ran_clone = callback_ran.clone();
        let input = crate::sterm::TerminalInput::for_test(b"command\n".to_vec(), move |result| {
            result.unwrap();
            callback_ran_clone.store(true, Ordering::Release);
        });
        let mut writer = FlushCheckingWriter {
            callback_ran: callback_ran.clone(),
            failure: WriterFailure::None,
            flushes: 0,
        };

        write_board_input(&mut writer, input).await.unwrap();

        assert_eq!(writer.flushes, 1);
        assert!(callback_ran.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn board_writer_reports_write_failure() {
        let error_seen = Arc::new(Mutex::new(None));
        let error_seen_clone = error_seen.clone();
        let input = crate::sterm::TerminalInput::for_test(b"command\n".to_vec(), move |result| {
            *error_seen_clone.lock().unwrap() = result.err().map(|error| error.to_string());
        });
        let mut writer = FlushCheckingWriter {
            callback_ran: Arc::new(AtomicBool::new(false)),
            failure: WriterFailure::Write,
            flushes: 0,
        };

        let error = write_board_input(&mut writer, input).await.unwrap_err();

        assert!(
            error
                .to_string()
                .contains("failed to write HTTP Boot board serial input")
        );
        assert_eq!(writer.flushes, 0);
        assert_eq!(
            error_seen.lock().unwrap().as_deref(),
            Some("failed to write HTTP Boot board serial input: injected write failure")
        );
    }

    #[tokio::test]
    async fn board_writer_reports_flush_failure() {
        let error_seen = Arc::new(Mutex::new(None));
        let error_seen_clone = error_seen.clone();
        let input = crate::sterm::TerminalInput::for_test(b"command\n".to_vec(), move |result| {
            *error_seen_clone.lock().unwrap() = result.err().map(|error| error.to_string());
        });
        let mut writer = FlushCheckingWriter {
            callback_ran: Arc::new(AtomicBool::new(false)),
            failure: WriterFailure::Flush,
            flushes: 0,
        };

        let error = write_board_input(&mut writer, input).await.unwrap_err();

        assert!(
            error
                .to_string()
                .contains("failed to flush HTTP Boot board serial input")
        );
        assert_eq!(writer.flushes, 0);
        assert_eq!(
            error_seen.lock().unwrap().as_deref(),
            Some("failed to flush HTTP Boot board serial input: injected flush failure")
        );
    }

    #[tokio::test]
    async fn board_writer_reports_first_chunk_failure_once_for_chunked_operation() {
        let (handle, mut rx) = crate::sterm::TerminalHandle::acknowledged_for_test();
        let completions = Arc::new(Mutex::new(Vec::new()));
        let completions_clone = completions.clone();
        handle.send_after_chunks_then(
            Duration::ZERO,
            vec![b'x'; 192],
            64,
            Duration::ZERO,
            move |_, result| {
                completions_clone
                    .lock()
                    .unwrap()
                    .push(result.err().map(|error| error.to_string()));
            },
        );

        let first = rx.recv().await.unwrap();
        assert_eq!(first.bytes().len(), 64);
        let mut failing_writer = FlushCheckingWriter {
            callback_ran: Arc::new(AtomicBool::new(false)),
            failure: WriterFailure::Write,
            flushes: 0,
        };
        let error = write_board_input(&mut failing_writer, first)
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("failed to write HTTP Boot board serial input")
        );

        for _ in 0..2 {
            let input = rx.recv().await.unwrap();
            assert_eq!(input.bytes().len(), 64);
            let mut writer = FlushCheckingWriter {
                callback_ran: Arc::new(AtomicBool::new(false)),
                failure: WriterFailure::None,
                flushes: 0,
            };
            write_board_input(&mut writer, input).await.unwrap();
        }
        tokio::task::yield_now().await;

        assert_eq!(
            completions.lock().unwrap().as_slice(),
            &[Some(
                "failed to write HTTP Boot board serial input: injected write failure".to_string()
            )]
        );
    }
}
