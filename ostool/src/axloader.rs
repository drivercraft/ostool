//! Direct axloader v6 control with the same serial discovery owner as the server.
use anyhow::{Context, ensure};
use clap::Subcommand;
use httpboot_protocol::{
    DEVICE_PROTOCOL_VERSION, DeviceBootImage, DeviceBootJob, LoaderDeviceStatus, SerialBinding,
    SerialBindingMode, SerialParameters,
};
use ostool_serial::{BindError, BindRequest, NativeBackend, SerialManager};
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    time::{Instant, sleep},
};

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
    ensure!(
        observed.protocol_version == DEVICE_PROTOCOL_VERSION,
        "automatic serial requires axloader v6; upgrade this device"
    );
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
            ensure_v6(&observed)?;
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
                ensure_v6(&observed)?;
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
        arch: observed.arch,
        image_format: httpboot_protocol::ImageFormat::Elf64,
        kernel: image(&kernel),
        initramfs: initramfs.as_deref().map(image),
        cmdline,
        entry_symbol: Some("__x86_64_efi_pe_entry".into()),
    };
    let base = format!("{device}/api/v1/boot/jobs");
    let boot = async {
        client
            .post(&base)
            .header("X-Boot-Epoch", &observed.boot_epoch)
            .json(&job)
            .send()
            .await?
            .error_for_status()?;
        let mut files = vec![("kernel", kernel.as_slice())];
        if let Some(bytes) = initramfs.as_deref() {
            files.push(("initramfs", bytes));
        }
        for (kind, bytes) in files {
            client
                .put(format!("{base}/{}/{kind}", job.boot_id))
                .header("X-Boot-Epoch", &observed.boot_epoch)
                .header("X-Image-Sha256", image(bytes).sha256)
                .body(bytes.to_vec())
                .send()
                .await?
                .error_for_status()?;
        }
        client
            .post(format!("{base}/{}/start", job.boot_id))
            .header("X-Boot-Epoch", &observed.boot_epoch)
            .header("X-Serial-Binding", &binding.binding_id)
            .send()
            .await?
            .error_for_status()?;
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
    manager.wait_owner_released(&owner).await?;
    // Revocation is best effort: after handoff this endpoint no longer exists.
    let _ = client
        .delete(format!(
            "{device}/api/v1/serial/bindings/{}",
            binding.binding_id
        ))
        .header("X-Boot-Epoch", &observed.boot_epoch)
        .timeout(Duration::from_secs(2))
        .send()
        .await;
    result
}

async fn fetch_status(client: &Client, device: &str) -> anyhow::Result<LoaderDeviceStatus> {
    Ok(client
        .get(format!("{device}/api/v1/status"))
        .timeout(Duration::from_secs(5))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

fn ensure_v6(status: &LoaderDeviceStatus) -> anyhow::Result<()> {
    ensure!(
        status.protocol_version == DEVICE_PROTOCOL_VERSION,
        "automatic serial requires axloader v6; upgrade this device"
    );
    Ok(())
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
    use super::effective_parameters;
    use httpboot_protocol::{SerialFlowControl, SerialParity, SerialStopBits};

    #[test]
    fn missing_firmware_parameters_use_protocol_defaults() {
        let parameters = effective_parameters(None);
        assert_eq!(parameters.baud_rate, 115_200);
        assert_eq!(parameters.data_bits, 8);
        assert_eq!(parameters.parity, SerialParity::None);
        assert_eq!(parameters.stop_bits, SerialStopBits::One);
        assert_eq!(parameters.flow_control, SerialFlowControl::None);
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
    client
        .post(format!("{device}/api/v1/serial/continue"))
        .header("X-Boot-Epoch", epoch)
        .json(binding)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}
