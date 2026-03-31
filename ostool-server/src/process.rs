use anyhow::{Context, bail};
use tokio::process::Command;

pub async fn run_shell_command(command: &str) -> anyhow::Result<()> {
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
        .await
        .with_context(|| format!("failed to start command `{command}`"))?;

    if status.success() {
        Ok(())
    } else {
        bail!("command `{command}` exited with status {status}");
    }
}

pub async fn run_program_command(program: &str, args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new(program)
        .args(args)
        .status()
        .await
        .with_context(|| {
            if args.is_empty() {
                format!("failed to start command `{program}`")
            } else {
                format!("failed to start command `{} {}`", program, args.join(" "))
            }
        })?;

    if status.success() {
        Ok(())
    } else if args.is_empty() {
        bail!("command `{program}` exited with status {status}");
    } else {
        bail!(
            "command `{} {}` exited with status {status}",
            program,
            args.join(" ")
        );
    }
}
