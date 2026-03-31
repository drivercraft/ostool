use std::process::Command;

use anyhow::{Context, bail};

pub fn run_shell_command(command: &str) -> anyhow::Result<()> {
    if command.trim().is_empty() {
        return Ok(());
    }

    let mut process = if cfg!(target_os = "windows") {
        let mut process = Command::new("powershell");
        process.arg("-Command").arg(command);
        process
    } else {
        let mut process = Command::new("sh");
        process.arg("-c").arg(command);
        process
    };

    let status = process
        .status()
        .with_context(|| format!("failed to start command `{command}`"))?;

    if status.success() {
        Ok(())
    } else {
        bail!("command `{command}` exited with status {status}");
    }
}
