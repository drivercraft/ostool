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
            BoardServerClient, BootConfig as RemoteBootConfig, HttpBootKernelUpload,
            SerialStatusResponse, SessionCreatedResponse, UefiBootArch,
        },
        config::BoardRunConfig,
        serial_stream::connect_serial_stream,
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
            connect_serial_stream(ws_url, self.client.websocket_authorization().await?).await?;
        let serial_rx = serial_rx.compat();
        let serial_tx = serial_tx.compat_write();
        println!("Serial is connected for target-system interaction only.");
        let status_client = self.client.clone();
        let status_session_id = self.session.session_id.clone();
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
        let result = tokio::select! {
            result = &mut terminal => result,
            result = &mut status => result,
        };
        let shutdown_result = tasks.shutdown_with_timeout(Duration::from_secs(2)).await;
        if result.is_ok() {
            shutdown_result?;
        } else if let Err(err) = shutdown_result {
            log::warn!("serial websocket cleanup failed: {err:#}");
        }
        result
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

struct TerminalRunOptions {
    fail_regex: Vec<String>,
    shell_check_steps: Vec<ShellCheckStep>,
    timeout: Option<u64>,
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

async fn run_terminal<R, W>(
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

    use super::write_board_input;

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
