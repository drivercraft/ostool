use ostool::{
    board,
    build::{
        self, CargoQemuRunnerArgs, CargoRunnerKind, CargoUbootRunnerArgs,
        config::{BuildConfig, Cargo},
    },
    invocation::{Invocation, InvocationOptions},
    run::{
        qemu::{QemuConfig, RunQemuOptions},
        uboot::UbootConfig,
    },
};

fn main() {
    let invocation = Invocation::new(InvocationOptions::default()).unwrap();
    let _: BuildConfig = build::default_build_config();
    let cargo = Cargo {
        disable_someboot_build_config: true,
        ..Cargo::default()
    };
    let qemu: QemuConfig = ostool::run::qemu::default_config_for_cargo(&invocation, &cargo);
    let _ = ostool::run::qemu::default_config(&invocation);
    let uboot: UbootConfig = ostool::run::uboot::default_config();
    let _ = RunQemuOptions::default();
    let _ = board::RunBoardOptions::default();
    let _ = build::CargoRunnerKind::new_qemu(CargoQemuRunnerArgs {
        qemu: Some(qemu),
        debug: false,
        dtb_dump: false,
    });
    let _ = CargoRunnerKind::new_uboot(CargoUbootRunnerArgs {
        uboot: Some(uboot),
    });
}
