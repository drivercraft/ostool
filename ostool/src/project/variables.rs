//! Placeholder expansion for workspace, package, temporary, and environment variables.

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use crate::utils::replace_placeholders;

use super::ProjectLayout;

/// Concrete variable inputs for one command or config-expansion context.
#[derive(Clone, Debug)]
pub struct VariableScope {
    workspace_dir: PathBuf,
    package_dir: PathBuf,
    tmp_dir: PathBuf,
}

impl VariableScope {
    /// Creates an explicit variable scope for command and config expansion.
    pub fn new(workspace_dir: PathBuf, package_dir: PathBuf, tmp_dir: PathBuf) -> Self {
        Self {
            workspace_dir,
            package_dir,
            tmp_dir,
        }
    }

    /// Builds a variable scope for a specific Cargo package directory.
    pub fn for_package(layout: &ProjectLayout, package_dir: PathBuf) -> Self {
        Self::new(
            layout.workspace_dir().to_path_buf(),
            package_dir,
            std::env::temp_dir(),
        )
    }

    /// Returns the workspace root replacement value.
    pub fn workspace_dir(&self) -> &Path {
        &self.workspace_dir
    }

    /// Returns the package root replacement value.
    pub fn package_dir(&self) -> &Path {
        &self.package_dir
    }

    /// Returns the temporary directory replacement value.
    pub fn tmp_dir(&self) -> &Path {
        &self.tmp_dir
    }
}

/// Expands ostool placeholders in a UTF-8 string.
pub fn expand_variables(input: &str, scope: &VariableScope) -> anyhow::Result<String> {
    let workspace_dir = scope.workspace_dir().display().to_string();
    let package_dir = scope.package_dir().display().to_string();
    let tmp_dir = scope.tmp_dir().display().to_string();

    replace_placeholders(input, |placeholder| {
        let value = match placeholder {
            "workspace" | "workspaceFolder" => Some(workspace_dir.clone()),
            "package" => Some(package_dir.clone()),
            "tmpDir" => Some(tmp_dir.clone()),
            p if p.starts_with("env:") => Some(std::env::var(&p[4..]).unwrap_or_default()),
            _ => None,
        };
        Ok(value)
    })
}

/// Expands placeholders in an OS string, falling back to the original on errors.
pub fn expand_os_value(value: &OsStr, scope: &VariableScope) -> String {
    expand_variables(&value.to_string_lossy(), scope)
        .unwrap_or_else(|_| value.to_string_lossy().into_owned())
}

/// Expands placeholders in a filesystem path.
pub fn expand_path_variables(path: &Path, scope: &VariableScope) -> anyhow::Result<PathBuf> {
    Ok(PathBuf::from(expand_variables(
        &path.to_string_lossy(),
        scope,
    )?))
}
