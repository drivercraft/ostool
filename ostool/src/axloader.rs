//! Direct axloader v7 control with the same serial discovery owner as the server.
use anyhow::{Context, anyhow, bail};
use clap::Subcommand;
use httpboot_protocol::{
    DEVICE_PROTOCOL_VERSION, DEVICE_PROTOCOL_VERSION_HEADER, DeviceBootImage, DeviceBootJob,
    DeviceBootStatus, DeviceOperationError, LoaderDeviceStatus, SerialBinding, SerialBindingMode,
    SerialParameters, decode_device_operation_error, device_protocol_version_header_value,
    validate_device_protocol_version,
};
use ostool_serial::{BindError, BindRequest, NativeBackend, SerialManager};
use reqwest::{Client, Response};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    time::{Instant, sleep},
};

use crate::http_io::{MAX_ERROR_BODY_BYTES, read_limited_body};

const CLI_BIND_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Subcommand)]
pub enum AxloaderCommand {
    /// Automatically discover/configure the UART and boot a kernel through axloader.
    Run {
        #[arg(long)]
        device: String,
        #[arg(long)]
        kernel: PathBuf,
        #[arg(long)]
        initramfs: Option<PathBuf>,
        #[arg(long)]
        cmdline: Option<String>,
    },
    /// Explicitly allow direct network boot without a serial tunnel.
    Continue {
        #[arg(long)]
        device: String,
    },
}
pub async fn execute(command: AxloaderCommand) -> anyhow::Result<()> {
    let device = match &command {
        AxloaderCommand::Run { device, .. } | AxloaderCommand::Continue { device } => {
            device.trim_end_matches('/').to_string()
        }
    };
    let client = Client::builder()
        .timeout(Duration::from_secs(300))
        .build()?;
    let mut observed = fetch_status(&client, &device).await?;
    ensure_v7(&observed)?;
    let serial = observed
        .serial
        .as_ref()
        .context("device did not report serial status")?;
    let mut binding = SerialBinding {
        serial_id: serial.serial_id.clone(),
        binding_id: format!("{:032x}", binding_nonce()),
        mode: SerialBindingMode::Direct,
    };
    let AxloaderCommand::Run {
        kernel,
        initramfs,
        cmdline,
        ..
    } = command
    else {
        if let Some(current) = &serial.binding
            && current.mode == SerialBindingMode::Direct
            && current.serial_id == serial.serial_id
        {
            binding = current.clone();
        }
        grant(&client, &device, &observed.boot_epoch, &binding).await?;
        println!("Direct network continue accepted (no serial tunnel)");
        println!("X-Boot-Epoch: {}", observed.boot_epoch);
        println!("X-Serial-Binding: {}", binding.binding_id);
        return Ok(());
    };
    let kernel = tokio::fs::read(kernel).await?;
    let initramfs = match initramfs {
        Some(path) => Some(tokio::fs::read(path).await?),
        None => None,
    };
    let manager = SerialManager::new(Arc::new(NativeBackend));
    let cli_deadline = Instant::now() + CLI_BIND_TIMEOUT;
    let mut generation = 0_u64;
    let (binding, parameters, mut lease, owner) = loop {
        let serial = observed
            .serial
            .as_ref()
            .context("device did not report serial status")?;
        if !serial.ready {
            if Instant::now() >= cli_deadline {
                anyhow::bail!(
                    "automatic serial unavailable before deadline: {:?}",
                    serial.error
                );
            }
            sleep(Duration::from_millis(250)).await;
            observed = fetch_status(&client, &device).await?;
            ensure_v7(&observed)?;
            continue;
        }
        let parameters = effective_parameters(serial.parameters);
        ostool_serial::validate_host_parameters(parameters)?;
        binding = SerialBinding {
            serial_id: serial.serial_id.clone(),
            binding_id: format!("{:032x}", binding_nonce()),
            mode: SerialBindingMode::Bound,
        };
        let owner = binding.binding_id.clone();
        generation = generation
            .checked_add(1)
            .context("serial binding generation exhausted")?;
        let remaining = cli_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            anyhow::bail!("serial identity did not become available before deadline");
        }
        match manager
            .bind(BindRequest {
                owner: owner.clone(),
                generation,
                mac_address: observed.mac_address,
                boot_epoch: observed.boot_epoch.clone(),
                serial_id: binding.serial_id.clone(),
                parameters,
                deadline: Instant::now() + remaining.min(Duration::from_secs(5)),
            })
            .await
        {
            Ok(lease) => break (binding, parameters, lease, owner),
            Err(error) if retryable_bind_error(&error) => {
                if Instant::now() >= cli_deadline {
                    anyhow::bail!("serial identity discovery failed before deadline: {error}");
                }
                sleep(Duration::from_millis(250)).await;
                observed = fetch_status(&client, &device).await?;
                ensure_v7(&observed)?;
            }
            Err(error) => return Err(error.into()),
        }
    };
    println!(
        "Serial identity verified on {} with {:?}",
        lease.locator().name,
        parameters
    );
    if let Err(error) = grant(&client, &device, &observed.boot_epoch, &binding).await {
        drop(lease);
        manager.wait_owner_released(&owner).await?;
        return Err(error);
    }
    lease.confirm();
    let job = DeviceBootJob {
        boot_id: format!("cli-{}", binding.binding_id),
        kernel: image(&kernel),
        initramfs: initramfs.as_deref().map(image),
        cmdline,
    };
    let base = format!("{device}/api/v1/boot/jobs");
    let boot = async {
        create_boot_job(&client, &base, &observed.boot_epoch, &job).await?;
        let mut files = vec![("kernel", kernel.as_slice())];
        if let Some(bytes) = initramfs.as_deref() {
            files.push(("initramfs", bytes));
        }
        for (kind, bytes) in files {
            let response = client
                .put(format!("{base}/{}/{kind}", job.boot_id))
                .header(
                    DEVICE_PROTOCOL_VERSION_HEADER,
                    device_protocol_version_header_value(),
                )
                .header("X-Boot-Epoch", &observed.boot_epoch)
                .header("X-Image-Sha256", image(bytes).sha256)
                .body(bytes.to_vec())
                .send()
                .await?;
            ensure_device_success(response)
                .await
                .with_context(|| format!("failed to upload {kind}"))?;
        }
        match start_job(
            &client,
            &device,
            &observed.boot_epoch,
            &job.boot_id,
            &binding.binding_id,
        )
        .await?
        {
            StartOutcome::Accepted => {}
            StartOutcome::Unconfirmed(reason) => {
                eprintln!("handoff result unconfirmed: {reason}");
                eprintln!("Continuing serial observation; press Ctrl-C to stop.");
            }
        }
        std::future::pending::<anyhow::Result<()>>().await
    };
    let console = async {
        let mut bytes = [0; 4096];
        let mut stdout = tokio::io::stdout();
        // Reuse the terminal's nonblocking owner: a blocking stdin worker would
        // keep the Tokio runtime alive after Ctrl-C while waiting for a newline.
        #[cfg(unix)]
        let mut stdin = {
            use std::io::IsTerminal;
            std::io::stdin()
                .is_terminal()
                .then(crate::sterm::Input::new)
                .transpose()?
        };
        #[cfg(not(unix))]
        let mut stdin = tokio::io::stdin();
        #[cfg(not(unix))]
        let mut input = [0; 4096];
        #[cfg(unix)]
        let mut input_open = stdin.is_some();
        #[cfg(not(unix))]
        let mut input_open = true;
        loop {
            let keyboard = async {
                #[cfg(unix)]
                return stdin
                    .as_mut()
                    .expect("input is enabled")
                    .next()
                    .await
                    .transpose();
                #[cfg(not(unix))]
                {
                    let n = stdin.read(&mut input).await?;
                    Ok::<_, std::io::Error>((n > 0).then(|| input[..n].to_vec()))
                }
            };
            let n = tokio::select! {
                result = lease.read(&mut bytes) => result?,
                result = keyboard, if input_open => {
                    if let Some(input) = result? {
                        tokio::time::timeout(Duration::from_secs(1), lease.write_all(&input)).await.context("serial write timed out")??;
                    } else {
                        input_open = false;
                    }
                    continue;
                }
            };
            if n == 0 {
                anyhow::bail!("serial closed");
            }
            stdout.write_all(&bytes[..n]).await?;
            stdout.flush().await?;
        }
    };
    let result = tokio::select! {result=boot=>result,result=console=>result,_=tokio::signal::ctrl_c()=>Ok(())};
    drop(lease);
    let release_result = manager.wait_owner_released(&owner).await;
    // Revocation is best effort: after handoff this endpoint no longer exists.
    let _ = client
        .delete(format!(
            "{device}/api/v1/serial/bindings/{}",
            binding.binding_id
        ))
        .header("X-Boot-Epoch", &observed.boot_epoch)
        .header(
            DEVICE_PROTOCOL_VERSION_HEADER,
            device_protocol_version_header_value(),
        )
        .timeout(Duration::from_secs(2))
        .send()
        .await;
    match result {
        Err(error) => Err(error),
        Ok(()) => release_result.map_err(Into::into),
    }
}

async fn fetch_status(client: &Client, device: &str) -> anyhow::Result<LoaderDeviceStatus> {
    let response = client
        .get(format!("{device}/api/v1/status"))
        .timeout(Duration::from_secs(5))
        .send()
        .await?;
    let response = ensure_device_success(response).await?;
    Ok(response.json().await?)
}

fn ensure_v7(status: &LoaderDeviceStatus) -> anyhow::Result<()> {
    validate_device_protocol_version(status.protocol_version).map_err(Into::into)
}

fn effective_parameters(reported: Option<SerialParameters>) -> SerialParameters {
    reported.unwrap_or_default()
}

fn retryable_bind_error(error: &BindError) -> bool {
    matches!(
        error,
        BindError::Timeout | BindError::DiscoveryFailed(_) | BindError::Busy | BindError::Io(_)
    )
}

async fn create_boot_job(
    client: &Client,
    base: &str,
    epoch: &str,
    job: &DeviceBootJob,
) -> anyhow::Result<()> {
    let response = client
        .post(base)
        .header(
            DEVICE_PROTOCOL_VERSION_HEADER,
            device_protocol_version_header_value(),
        )
        .header("X-Boot-Epoch", epoch)
        .json(job)
        .send()
        .await
        .map_err(|error| anyhow!("boot job create result unconfirmed: {error}"))?;
    let response = ensure_device_success(response).await?;
    let status: DeviceBootStatus = response
        .json()
        .await
        .context("invalid_device_response: failed to decode boot job status")?;
    validate_created_status(&status, &job.boot_id)
}

fn image(bytes: &[u8]) -> DeviceBootImage {
    DeviceBootImage {
        size: bytes.len() as u64,
        sha256: Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        StartObservation, StartOutcome, classify_start_status, create_boot_job,
        effective_parameters, ensure_v7, start_job,
    };
    use httpboot_protocol::{
        DEVICE_PROTOCOL_VERSION, DeviceBootImage, DeviceBootJob, DeviceBootStatus,
        DeviceOperationError, LoaderDeviceStatus, MacAddress, SerialFlowControl, SerialParity,
        SerialStopBits,
    };
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        sync::oneshot,
    };

    #[test]
    fn missing_firmware_parameters_use_protocol_defaults() {
        let parameters = effective_parameters(None);
        assert_eq!(parameters.baud_rate, 115_200);
        assert_eq!(parameters.data_bits, 8);
        assert_eq!(parameters.parity, SerialParity::None);
        assert_eq!(parameters.stop_bits, SerialStopBits::One);
        assert_eq!(parameters.flow_control, SerialFlowControl::None);
    }

    fn status(
        protocol_version: u16,
        epoch: &str,
        boot: Option<DeviceBootStatus>,
    ) -> LoaderDeviceStatus {
        LoaderDeviceStatus {
            protocol_version,
            boot_epoch: epoch.into(),
            mac_address: MacAddress::ZERO,
            current_mac_address: MacAddress::ZERO,
            arch: "future-isa".into(),
            loader_version: "axloader".into(),
            hardware: Default::default(),
            boot,
            ota: None,
            serial: None,
        }
    }

    #[test]
    fn start_recovery_only_accepts_the_same_ready_epoch_and_job() {
        let ready = DeviceBootStatus {
            boot_id: "boot-1".into(),
            phase: "ready".into(),
            kernel_received: true,
            initramfs_received: false,
            last_error: None,
        };
        assert_eq!(
            classify_start_status(
                &status(DEVICE_PROTOCOL_VERSION, "epoch-1", Some(ready.clone())),
                "epoch-1",
                "boot-1"
            ),
            StartObservation::Ready
        );
        assert!(matches!(
            classify_start_status(
                &status(DEVICE_PROTOCOL_VERSION, "epoch-2", Some(ready.clone())),
                "epoch-1",
                "boot-1"
            ),
            StartObservation::Unconfirmed(_)
        ));
        assert!(matches!(
            classify_start_status(
                &status(DEVICE_PROTOCOL_VERSION, "epoch-1", Some(ready)),
                "epoch-1",
                "other-boot"
            ),
            StartObservation::Unconfirmed(_)
        ));

        let failure = DeviceOperationError::new("invalid_elf", "bad machine");
        let failed = DeviceBootStatus {
            boot_id: "boot-1".into(),
            phase: "failed".into(),
            kernel_received: false,
            initramfs_received: false,
            last_error: Some(failure.clone()),
        };
        assert_eq!(
            classify_start_status(
                &status(DEVICE_PROTOCOL_VERSION, "epoch-1", Some(failed)),
                "epoch-1",
                "boot-1"
            ),
            StartObservation::Failed(failure)
        );
    }

    #[test]
    fn status_gate_rejects_non_v7_devices() {
        assert!(ensure_v7(&status(DEVICE_PROTOCOL_VERSION, "epoch", None)).is_ok());
        let error = ensure_v7(&status(6, "epoch", None)).unwrap_err();
        assert!(error.to_string().contains("unsupported_protocol"));
    }

    async fn read_headers(socket: &mut tokio::net::TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let read = socket.read(&mut buffer).await.unwrap();
            assert_ne!(read, 0);
            bytes.extend_from_slice(&buffer[..read]);
            if bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                return String::from_utf8(bytes).unwrap().to_ascii_lowercase();
            }
        }
    }

    async fn respond(socket: &mut tokio::net::TcpStream, status: &str, body: &str) {
        let response = format!(
            "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    }

    async fn respond_truncated(socket: &mut tokio::net::TcpStream, status: &str) {
        let response =
            format!("HTTP/1.1 {status}\r\ncontent-length: 2\r\nconnection: close\r\n\r\nx");
        socket.write_all(response.as_bytes()).await.unwrap();
        socket.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn uncertain_start_queries_same_job_and_retries_ready_job_once() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (abort_tx, abort_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut first_start, _) = listener.accept().await.unwrap();
            let request = read_headers(&mut first_start).await;
            assert!(request.starts_with("post /api/v1/boot/jobs/boot-1/start "));
            assert!(request.contains("x-axloader-protocol-version: 7\r\n"));
            respond_truncated(&mut first_start, "202 Accepted").await;

            let (mut status_socket, _) = tokio::select! {
                accepted = listener.accept() => accepted.unwrap(),
                _ = abort_rx => panic!("truncated start body must enter recovery"),
            };
            let request = read_headers(&mut status_socket).await;
            assert!(request.starts_with("get /api/v1/status "));
            respond(
                &mut status_socket,
                "200 OK",
                r#"{"protocol_version":7,"boot_epoch":"epoch-1","mac_address":"02:00:00:00:00:01","current_mac_address":"02:00:00:00:00:01","arch":"future-isa","loader_version":"axloader","hardware":{},"boot":{"boot_id":"boot-1","phase":"ready","kernel_received":true,"initramfs_received":false,"last_error":null},"ota":null}"#,
            )
            .await;

            let (mut retry, _) = listener.accept().await.unwrap();
            let request = read_headers(&mut retry).await;
            assert!(request.starts_with("post /api/v1/boot/jobs/boot-1/start "));
            assert!(request.contains("x-axloader-protocol-version: 7\r\n"));
            respond(&mut retry, "202 Accepted", "").await;
        });

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let outcome = start_job(
            &client,
            &format!("http://{address}"),
            "epoch-1",
            "boot-1",
            &"a".repeat(32),
        )
        .await
        .unwrap();
        let _ = abort_tx.send(());
        assert_eq!(outcome, StartOutcome::Accepted);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn changed_epoch_during_start_recovery_returns_unconfirmed_without_retry() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (done_tx, done_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut start, _) = listener.accept().await.unwrap();
            read_headers(&mut start).await;
            respond(
                &mut start,
                "500 Internal Server Error",
                r#"{"code":"temporary","message":"response lost"}"#,
            )
            .await;

            let (mut status_socket, _) = listener.accept().await.unwrap();
            read_headers(&mut status_socket).await;
            respond(
                &mut status_socket,
                "200 OK",
                r#"{"protocol_version":7,"boot_epoch":"epoch-2","mac_address":"02:00:00:00:00:01","current_mac_address":"02:00:00:00:00:01","arch":"future-isa","loader_version":"axloader","hardware":{},"boot":null,"ota":null}"#,
            )
            .await;
            tokio::select! {
                _ = listener.accept() => panic!("changed epoch must not retry start"),
                _ = done_rx => {}
            }
        });

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let outcome = start_job(
            &client,
            &format!("http://{address}"),
            "epoch-1",
            "boot-1",
            &"a".repeat(32),
        )
        .await
        .unwrap();
        assert!(matches!(outcome, StartOutcome::Unconfirmed(_)));
        done_tx.send(()).unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn second_uncertain_start_returns_unconfirmed_without_more_retries() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (done_tx, done_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            for attempt in 0..2 {
                let (mut start, _) = listener.accept().await.unwrap();
                read_headers(&mut start).await;
                respond_truncated(&mut start, "202 Accepted").await;
                if attempt == 0 {
                    let (mut status_socket, _) = listener.accept().await.unwrap();
                    read_headers(&mut status_socket).await;
                    respond(
                        &mut status_socket,
                        "200 OK",
                        r#"{"protocol_version":7,"boot_epoch":"epoch-1","mac_address":"02:00:00:00:00:01","current_mac_address":"02:00:00:00:00:01","arch":"future-isa","loader_version":"axloader","hardware":{},"boot":{"boot_id":"boot-1","phase":"ready","kernel_received":true,"initramfs_received":false,"last_error":null},"ota":null}"#,
                    )
                    .await;
                }
            }
            tokio::select! {
                _ = listener.accept() => panic!("a second uncertain retry must not trigger another request"),
                _ = done_rx => {}
            }
        });

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let outcome = start_job(
            &client,
            &format!("http://{address}"),
            "epoch-1",
            "boot-1",
            &"a".repeat(32),
        )
        .await
        .unwrap();
        assert!(matches!(outcome, StartOutcome::Unconfirmed(_)));
        done_tx.send(()).unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn explicit_start_failure_is_terminal_and_keeps_code_and_message() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_headers(&mut socket).await;
            assert!(request.starts_with("post /api/v1/boot/jobs/boot-1/start "));
            assert!(request.contains("x-axloader-protocol-version: 7\r\n"));
            respond(
                &mut socket,
                "422 Unprocessable Entity",
                r#"{"code":"elf_machine_mismatch","message":"actual 51966 expected 258"}"#,
            )
            .await;
        });

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let error = start_job(
            &client,
            &format!("http://{address}"),
            "epoch-1",
            "boot-1",
            &"a".repeat(32),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("elf_machine_mismatch"), "{error}");
        assert!(error.contains("actual 51966 expected 258"), "{error}");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn create_status_real_http_covers_failed_receiving_and_id_mismatch() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (done_tx, done_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            for body in [
                r#"{"boot_id":"boot-1","phase":"failed","kernel_received":false,"initramfs_received":false,"last_error":{"code":"invalid_elf","message":"bad machine"}}"#,
                r#"{"boot_id":"boot-1","phase":"receiving","kernel_received":false,"initramfs_received":false,"last_error":null}"#,
                r#"{"boot_id":"other-boot","phase":"receiving","kernel_received":false,"initramfs_received":false,"last_error":null}"#,
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = read_headers(&mut socket).await;
                assert!(request.starts_with("post /api/v1/boot/jobs "));
                assert!(request.contains("x-axloader-protocol-version: 7\r\n"));
                respond(&mut socket, "200 OK", body).await;
            }
            tokio::select! {
                _ = listener.accept() => panic!("create status checks must not issue another request"),
                _ = done_rx => {}
            }
        });

        let job = DeviceBootJob {
            boot_id: "boot-1".into(),
            kernel: DeviceBootImage {
                size: 1,
                sha256: "00".repeat(32),
            },
            initramfs: None,
            cmdline: None,
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let error = create_boot_job(
            &client,
            &format!("http://{address}/api/v1/boot/jobs"),
            "epoch-1",
            &job,
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("invalid_elf"), "{error}");
        assert!(error.contains("bad machine"), "{error}");
        create_boot_job(
            &client,
            &format!("http://{address}/api/v1/boot/jobs"),
            "epoch-1",
            &job,
        )
        .await
        .unwrap();
        let error = create_boot_job(
            &client,
            &format!("http://{address}/api/v1/boot/jobs"),
            "epoch-1",
            &job,
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("invalid_device_response"), "{error}");
        done_tx.send(()).unwrap();
        server.await.unwrap();
    }
}
fn binding_nonce() -> u128 {
    // A process-local token is a correlation identifier, not an authentication secret.
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(1);
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    time ^ ((std::process::id() as u128) << 64) ^ SEQUENCE.fetch_add(1, Ordering::Relaxed) as u128
}
async fn grant(
    client: &Client,
    device: &str,
    epoch: &str,
    binding: &SerialBinding,
) -> anyhow::Result<()> {
    let response = client
        .post(format!("{device}/api/v1/serial/continue"))
        .header(
            DEVICE_PROTOCOL_VERSION_HEADER,
            device_protocol_version_header_value(),
        )
        .header("X-Boot-Epoch", epoch)
        .json(binding)
        .send()
        .await?;
    ensure_device_success(response).await?;
    Ok(())
}

#[derive(Debug)]
enum StartAttempt {
    Accepted,
    Rejected(anyhow::Error),
    Uncertain(String),
}

#[derive(Debug, PartialEq, Eq)]
enum StartObservation {
    Ready,
    Failed(DeviceOperationError),
    Unconfirmed(String),
}

#[derive(Debug, PartialEq, Eq)]
enum StartOutcome {
    Accepted,
    Unconfirmed(String),
}

async fn start_job(
    client: &Client,
    device: &str,
    epoch: &str,
    boot_id: &str,
    binding_id: &str,
) -> anyhow::Result<StartOutcome> {
    match perform_start(client, device, epoch, boot_id, binding_id).await {
        StartAttempt::Accepted => Ok(StartOutcome::Accepted),
        StartAttempt::Rejected(error) => Err(error),
        StartAttempt::Uncertain(reason) => {
            let observation = observe_start(client, device, epoch, boot_id).await;
            match observation {
                StartObservation::Ready => {
                    match perform_start(client, device, epoch, boot_id, binding_id).await {
                        StartAttempt::Accepted => Ok(StartOutcome::Accepted),
                        StartAttempt::Rejected(error) => Err(error),
                        StartAttempt::Uncertain(retry) => Ok(StartOutcome::Unconfirmed(format!(
                            "{reason}; retry: {retry}"
                        ))),
                    }
                }
                StartObservation::Failed(error) => Err(device_operation_failure(&error)),
                StartObservation::Unconfirmed(status) => Ok(StartOutcome::Unconfirmed(format!(
                    "{reason}; status query: {status}"
                ))),
            }
        }
    }
}

async fn perform_start(
    client: &Client,
    device: &str,
    epoch: &str,
    boot_id: &str,
    binding_id: &str,
) -> StartAttempt {
    let response = client
        .post(format!("{device}/api/v1/boot/jobs/{boot_id}/start"))
        .header(
            DEVICE_PROTOCOL_VERSION_HEADER,
            device_protocol_version_header_value(),
        )
        .header("X-Boot-Epoch", epoch)
        .header("X-Serial-Binding", binding_id)
        .send()
        .await;
    let response = match response {
        Ok(response) => response,
        Err(error) => return StartAttempt::Uncertain(error.to_string()),
    };
    if response.status().is_success() {
        return match response.bytes().await {
            Ok(_) => StartAttempt::Accepted,
            Err(error) => StartAttempt::Uncertain(error.to_string()),
        };
    }
    let status = response.status();
    let error = device_response_error(response).await;
    if status.is_server_error() {
        StartAttempt::Uncertain(format!("{error:#}"))
    } else {
        StartAttempt::Rejected(error)
    }
}

async fn observe_start(
    client: &Client,
    device: &str,
    epoch: &str,
    boot_id: &str,
) -> StartObservation {
    let status = match fetch_status(client, device).await {
        Ok(status) => status,
        Err(error) => return StartObservation::Unconfirmed(format!("{error:#}")),
    };
    classify_start_status(&status, epoch, boot_id)
}

fn classify_start_status(
    status: &LoaderDeviceStatus,
    epoch: &str,
    boot_id: &str,
) -> StartObservation {
    if status.protocol_version != DEVICE_PROTOCOL_VERSION {
        return StartObservation::Unconfirmed(format!(
            "device protocol changed to {}",
            status.protocol_version
        ));
    }
    if status.boot_epoch != epoch {
        return StartObservation::Unconfirmed(format!(
            "boot epoch changed from {epoch} to {}",
            status.boot_epoch
        ));
    }
    let Some(boot) = status.boot.as_ref() else {
        return StartObservation::Unconfirmed(format!("boot job {boot_id} is unavailable"));
    };
    if boot.boot_id != boot_id {
        return StartObservation::Unconfirmed(format!(
            "device reports boot job {} instead of {boot_id}",
            boot.boot_id
        ));
    }
    match boot.phase.as_str() {
        "ready" => StartObservation::Ready,
        "failed" => StartObservation::Failed(boot.last_error.clone().unwrap_or_else(|| {
            DeviceOperationError::new(
                "invalid_device_response",
                "failed boot job did not include last_error",
            )
        })),
        phase => StartObservation::Unconfirmed(format!("boot job {boot_id} is {phase}")),
    }
}

async fn ensure_device_success(response: Response) -> anyhow::Result<Response> {
    if response.status().is_success() {
        Ok(response)
    } else {
        Err(device_response_error(response).await)
    }
}

async fn device_response_error(response: Response) -> anyhow::Error {
    let status = response.status();
    let body = match read_limited_body(response, MAX_ERROR_BODY_BYTES).await {
        Ok(body) => body,
        Err(error) => {
            return anyhow!("invalid_device_response (HTTP {status}): {error}");
        }
    };
    match decode_device_operation_error(&body) {
        Ok(error) => anyhow!(
            "device request failed (HTTP {status}, {}): {}",
            error.code,
            error.message
        ),
        Err(_) => anyhow!(
            "invalid_device_response (HTTP {status}): {}",
            String::from_utf8_lossy(&body).trim()
        ),
    }
}

fn device_operation_failure(error: &DeviceOperationError) -> anyhow::Error {
    anyhow!("axloader boot failed ({}): {}", error.code, error.message)
}

fn validate_created_status(status: &DeviceBootStatus, boot_id: &str) -> anyhow::Result<()> {
    if status.boot_id != boot_id {
        bail!(
            "invalid_device_response: boot job create returned {} instead of {boot_id}",
            status.boot_id
        );
    }
    if status.phase == "failed" {
        let error = status.last_error.clone().unwrap_or_else(|| {
            DeviceOperationError::new(
                "invalid_device_response",
                "failed boot job did not include last_error",
            )
        });
        return Err(device_operation_failure(&error));
    }
    Ok(())
}
