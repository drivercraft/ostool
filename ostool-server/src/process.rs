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

/// Bound explicit admin commands without killing an unrelated process group.
/// The group is created by this invocation and reaped before releasing ownership.
#[cfg(target_os = "linux")]
pub async fn run_admin_shell_command(command: &str) -> anyhow::Result<()> {
    use nix::{
        sys::signal::{Signal, killpg},
        unistd::Pid,
    };
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
        .context("start power command")?;
    let pid = child.id().context("power command has no process id")?;
    struct Group(Pid);
    impl Drop for Group {
        fn drop(&mut self) {
            let _ = killpg(self.0, Signal::SIGKILL);
        }
    }
    let _group = Group(Pid::from_raw(pid as i32));
    match tokio::time::timeout(std::time::Duration::from_secs(60), child.wait()).await {
        Ok(status) => {
            if status?.success() {
                Ok(())
            } else {
                anyhow::bail!("power command failed")
            }
        }
        Err(_) => {
            // SIGKILL the group even if the shell has descendants holding IO.
            let _ = killpg(Pid::from_raw(pid as i32), Signal::SIGKILL);
            child.wait().await.context("reap timed out power command")?;
            anyhow::bail!("power command timed out after 60 seconds")
        }
    }
}
#[cfg(not(target_os = "linux"))]
pub async fn run_admin_shell_command(command: &str) -> anyhow::Result<()> {
    let mut process = if cfg!(windows) {
        let mut p = Command::new("powershell");
        p.arg("-Command");
        p
    } else {
        let mut p = Command::new("sh");
        p.arg("-c");
        p
    };
    let mut child = process
        .arg(command)
        .kill_on_drop(true)
        .spawn()
        .context("start device command")?;
    match tokio::time::timeout(std::time::Duration::from_secs(60), child.wait()).await {
        Ok(status) => {
            if status?.success() {
                Ok(())
            } else {
                anyhow::bail!("device command failed")
            }
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            anyhow::bail!("device command timed out after 60 seconds")
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    #[tokio::test]
    async fn cancelling_a_device_command_kills_its_own_descendants() {
        let root = tempfile::tempdir().unwrap();
        let ready = root.path().join("ready");
        let leaked = root.path().join("leaked");
        let quote =
            |p: &std::path::Path| format!("'{}'", p.display().to_string().replace('\'', "'\\''"));
        let command = format!(
            "(sleep 1; printf leaked > {}) & printf ready > {}; wait",
            quote(&leaked),
            quote(&ready)
        );
        let task = tokio::spawn(async move { super::run_admin_shell_command(&command).await });
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !ready.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        assert!(!leaked.exists());
    }
}
