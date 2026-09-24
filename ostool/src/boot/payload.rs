//! Host boot payload shared by QEMU, U-Boot, and board runners.

use std::path::PathBuf;

use anyhow::Result;
use httpboot_protocol::valid_host_cmdline;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::project::variables::{self, VariableScope};

/// Optional host initramfs and kernel command line.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct BootPayloadConfig {
    /// Path to a host initramfs archive; guest initrds are configured separately.
    pub initramfs: Option<String>,
    /// Boot arguments passed to the host kernel.
    pub cmdline: Option<String>,
}

impl BootPayloadConfig {
    pub(crate) fn replace_strings(&mut self, scope: &VariableScope) -> Result<()> {
        self.initramfs = self
            .initramfs
            .as_deref()
            .map(|path| variables::expand_variables(path, scope))
            .transpose()?;
        self.cmdline = self
            .cmdline
            .as_deref()
            .map(|value| variables::expand_variables(value, scope))
            .transpose()?;
        Ok(())
    }

    pub(crate) fn initramfs_path(&self) -> Option<PathBuf> {
        self.initramfs.as_ref().map(PathBuf::from)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.initramfs
                .as_ref()
                .is_none_or(|path| !path.trim().is_empty()),
            "initramfs path cannot be empty"
        );
        anyhow::ensure!(
            self.cmdline
                .as_ref()
                .is_none_or(|cmdline| valid_host_cmdline(cmdline)),
            "kernel command line must be at most 4095 printable ASCII bytes"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::BootPayloadConfig;

    #[test]
    fn rejects_control_bytes_and_oversize() {
        for value in [
            "console=ttyS0\nreset".to_string(),
            "console=ttyS0\x1b".to_string(),
            "x".repeat(4096),
        ] {
            assert!(
                BootPayloadConfig {
                    cmdline: Some(value),
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            BootPayloadConfig {
                cmdline: Some("rdinit=\"/sbin/my init\" -- arg".into()),
                ..Default::default()
            }
            .validate()
            .is_ok()
        );
        assert!(
            BootPayloadConfig {
                cmdline: Some("value='literal'".into()),
                ..Default::default()
            }
            .validate()
            .is_ok()
        );
    }
}
