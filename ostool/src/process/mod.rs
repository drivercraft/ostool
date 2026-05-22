//! Process command construction and shell hook execution with ostool variables.

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use anyhow::Context as _;

use crate::{project::variables::VariableScope, utils::Command};

/// Concrete process inputs for command construction and shell hooks.
#[derive(Clone, Debug)]
pub struct ProcessContext {
    workdir: PathBuf,
    workspace_dir: PathBuf,
    variables: VariableScope,
    kernel_elf: Option<PathBuf>,
}

impl ProcessContext {
    /// Creates a process context from invocation layout, variables, and ELF state.
    pub fn new(
        workdir: PathBuf,
        workspace_dir: PathBuf,
        variables: VariableScope,
        kernel_elf: Option<PathBuf>,
    ) -> Self {
        Self {
            workdir,
            workspace_dir,
            variables,
            kernel_elf,
        }
    }

    /// Returns the directory commands should run from.
    pub fn workdir(&self) -> &Path {
        &self.workdir
    }

    /// Returns the Cargo workspace root exposed to child processes.
    pub fn workspace_dir(&self) -> &Path {
        &self.workspace_dir
    }

    /// Returns variable-expansion inputs for command arguments and hooks.
    pub fn variables(&self) -> &VariableScope {
        &self.variables
    }

    /// Returns the active kernel ELF path exported to shell hooks.
    pub fn kernel_elf(&self) -> Option<&Path> {
        self.kernel_elf.as_deref()
    }
}

/// Creates a command that expands ostool variables in its arguments.
pub fn command<S>(program: S, context: &ProcessContext) -> Command
where
    S: AsRef<OsStr>,
{
    let variables = context.variables().clone();
    let mut command = Command::new(program, context.workdir(), move |s| {
        crate::project::variables::expand_os_value(s, &variables)
    });
    command.env(
        "WORKSPACE_FOLDER",
        context.workspace_dir().display().to_string(),
    );
    command
}

/// Runs a shell command with invocation variables and `KERNEL_ELF` environment.
pub fn shell_run_cmd(context: &ProcessContext, cmd: &str) -> anyhow::Result<()> {
    let mut command = match std::env::consts::OS {
        "windows" => {
            let mut command = command("powershell", context);
            command.arg("-Command");
            command
        }
        _ => {
            let mut command = command("sh", context);
            command.arg("-c");
            command
        }
    };

    command.arg(cmd);

    if let Some(elf) = context.kernel_elf() {
        command.env("KERNEL_ELF", elf.display().to_string());
    }

    command
        .run()
        .with_context(|| format!("failed to run shell command: {cmd}"))?;
    Ok(())
}
