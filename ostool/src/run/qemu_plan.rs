//! Crate-private QEMU command planning.
//!
//! This module keeps command argument ordering testable without owning runtime
//! side effects such as OVMF preparation, DTB cleanup, stdio, or process spawn.

use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
};

use crate::{process::ProcessContext, utils::Command};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum QemuBootSource {
    DirectKernelLoader {
        path: PathBuf,
    },
    UefiPflash {
        code: PathBuf,
        vars: PathBuf,
        esp_dir: PathBuf,
    },
}

impl QemuBootSource {
    pub(crate) fn direct_kernel_loader(path: impl Into<PathBuf>) -> Self {
        Self::DirectKernelLoader { path: path.into() }
    }

    pub(crate) fn uefi_pflash(
        code: impl Into<PathBuf>,
        vars: impl Into<PathBuf>,
        esp_dir: impl Into<PathBuf>,
    ) -> Self {
        Self::UefiPflash {
            code: code.into(),
            vars: vars.into(),
            esp_dir: esp_dir.into(),
        }
    }

    fn append_args(&self, args: &mut Vec<OsString>) {
        match self {
            Self::DirectKernelLoader { path } => {
                args.push("-kernel".into());
                args.push(path.as_os_str().to_os_string());
            }
            Self::UefiPflash {
                code,
                vars,
                esp_dir,
            } => {
                args.push("-drive".into());
                args.push(
                    format!(
                        "if=pflash,format=raw,unit=0,readonly=on,file={}",
                        code.display()
                    )
                    .into(),
                );
                args.push("-drive".into());
                args.push(format!("if=pflash,format=raw,unit=1,file={}", vars.display()).into());
                args.push("-drive".into());
                args.push(format!("format=raw,file=fat:rw:{}", esp_dir.display()).into());
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct QemuCommandPlanInput {
    pub(crate) executable: String,
    pub(crate) config_args: Vec<OsString>,
    pub(crate) default_machine: String,
    pub(crate) dtb_dump_path: Option<PathBuf>,
    pub(crate) debug: bool,
    pub(crate) boot_source: Option<QemuBootSource>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct QemuCommandPlan {
    executable: String,
    args: Vec<OsString>,
}

impl QemuCommandPlan {
    pub(crate) fn render(&self, context: &ProcessContext) -> Command {
        let mut cmd = crate::process::command(&self.executable, context);
        for arg in &self.args {
            cmd.arg(arg);
        }
        cmd
    }

    #[cfg(test)]
    fn args(&self) -> &[OsString] {
        &self.args
    }
}

pub(crate) fn build_qemu_command_plan(input: QemuCommandPlanInput) -> QemuCommandPlan {
    let mut args = Vec::new();
    let mut need_machine = true;

    for arg in input.config_args {
        if arg == OsStr::new("-machine") || arg == OsStr::new("-M") {
            need_machine = false;
        }
        args.push(arg);
    }

    if let Some(dtb_dump_path) = input.dtb_dump_path {
        args.push("-machine".into());
        args.push(format!("dumpdtb={}", dtb_dump_path.display()).into());
    }

    if need_machine {
        args.push("-machine".into());
        args.push(input.default_machine.into());
    }

    if input.debug {
        args.push("-s".into());
        args.push("-S".into());
    }

    if let Some(boot_source) = input.boot_source {
        boot_source.append_args(&mut args);
    }

    QemuCommandPlan {
        executable: input.executable,
        args,
    }
}

#[cfg(test)]
mod tests {
    use super::{QemuBootSource, QemuCommandPlanInput, build_qemu_command_plan};
    use std::{ffi::OsString, path::PathBuf};

    fn plan_args(input: QemuCommandPlanInput) -> Vec<String> {
        build_qemu_command_plan(input)
            .args()
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    fn base_input() -> QemuCommandPlanInput {
        QemuCommandPlanInput {
            executable: "qemu-system-aarch64".into(),
            config_args: vec![OsString::from("-nographic")],
            default_machine: "virt".into(),
            dtb_dump_path: None,
            debug: false,
            boot_source: None,
        }
    }

    #[test]
    fn plan_renders_direct_kernel_loader_after_default_machine() {
        let args = plan_args(QemuCommandPlanInput {
            boot_source: Some(QemuBootSource::direct_kernel_loader("target/kernel.elf")),
            ..base_input()
        });

        assert_eq!(
            args,
            vec![
                "-nographic",
                "-machine",
                "virt",
                "-kernel",
                "target/kernel.elf",
            ]
        );
    }

    #[test]
    fn plan_keeps_user_machine_override_and_dtb_dump_order() {
        let args = plan_args(QemuCommandPlanInput {
            config_args: vec![
                OsString::from("-nographic"),
                OsString::from("-machine"),
                OsString::from("virt,gic-version=3"),
            ],
            dtb_dump_path: Some(PathBuf::from("target/qemu.dtb")),
            ..base_input()
        });

        assert_eq!(
            args,
            vec![
                "-nographic",
                "-machine",
                "virt,gic-version=3",
                "-machine",
                "dumpdtb=target/qemu.dtb",
            ]
        );
    }

    #[test]
    fn plan_keeps_short_machine_override() {
        let args = plan_args(QemuCommandPlanInput {
            config_args: vec![
                OsString::from("-nographic"),
                OsString::from("-M"),
                OsString::from("virt"),
            ],
            ..base_input()
        });

        assert_eq!(args, vec!["-nographic", "-M", "virt"]);
    }

    #[test]
    fn plan_renders_debug_before_boot_source() {
        let args = plan_args(QemuCommandPlanInput {
            debug: true,
            boot_source: Some(QemuBootSource::direct_kernel_loader("target/kernel.bin")),
            ..base_input()
        });

        assert_eq!(
            args,
            vec![
                "-nographic",
                "-machine",
                "virt",
                "-s",
                "-S",
                "-kernel",
                "target/kernel.bin",
            ]
        );
    }

    #[test]
    fn plan_renders_uefi_pflash_without_kernel_loader() {
        let args = plan_args(QemuCommandPlanInput {
            boot_source: Some(QemuBootSource::uefi_pflash(
                "OVMF_CODE.fd",
                "kernel.vars.fd",
                "kernel.esp",
            )),
            ..base_input()
        });

        assert_eq!(
            args,
            vec![
                "-nographic",
                "-machine",
                "virt",
                "-drive",
                "if=pflash,format=raw,unit=0,readonly=on,file=OVMF_CODE.fd",
                "-drive",
                "if=pflash,format=raw,unit=1,file=kernel.vars.fd",
                "-drive",
                "format=raw,file=fat:rw:kernel.esp",
            ]
        );
        assert!(!args.iter().any(|arg| arg == "-kernel"));
    }

    #[test]
    fn plan_allows_no_boot_source() {
        let args = plan_args(base_input());

        assert_eq!(args, vec!["-nographic", "-machine", "virt"]);
    }
}
