use std::path::Path;

use ostool::{
    board::{self, config::BoardRunConfig},
    build::{
        self, CargoBuildOutput, CargoQemuRunnerArgs, CargoRunnerKind, CargoUbootRunnerArgs,
        RuntimeArtifactInput,
        config::{BuildConfig, BuildSystem, Cargo, Custom},
    },
    invocation::{Invocation, InvocationOptions},
    ovmf::{Arch, FileType, Prebuilt, Source, default_cache_dir},
    run::{
        ShellCheckStep,
        qemu::{self, QemuConfig, RunQemuOptions},
        uboot::{self, UbootConfig},
    },
    variables::{VariableScope, expand_path_variables, expand_variables},
};

fn assert_prebuilt_api(prebuilt: &Prebuilt) {
    let _ = prebuilt.get_file(Arch::X64, FileType::Code);
}

fn main() {
    let scope = VariableScope::new("workspace".into(), "package".into(), "tmp".into());
    let _ = expand_variables("${workspace}", &scope).unwrap();
    let _ = expand_path_variables(Path::new("${tmpDir}/asset"), &scope).unwrap();
    let _ = default_cache_dir();
    let _ = Source::LATEST;
    let _ = assert_prebuilt_api;

    let mut invocation = Invocation::new(InvocationOptions::default()).unwrap();
    let cargo = Cargo {
        package: "kernel".into(),
        target: "aarch64-unknown-none".into(),
        disable_someboot_build_config: true,
        ..Cargo::default()
    };
    let cargo_build = BuildConfig {
        system: BuildSystem::Cargo(Box::new(cargo.clone())),
    };
    let custom_build = BuildConfig {
        system: BuildSystem::Custom(Custom {
            build_cmd: "true".into(),
            elf_path: "target/kernel.elf".into(),
            to_bin: false,
        }),
    };
    let qemu_config: QemuConfig = qemu::default_config_for_cargo(&invocation, &cargo);
    let _step = ShellCheckStep {
        shell_prefix: Some("root@starry:/root #".into()),
        shell_cmd: Some("echo pass".into()),
        success_regex: Some(vec!["PASS".into()]),
        fail_regex: Some(vec!["FAIL".into()]),
        timeout: Some(30),
    };
    let uboot_config: UbootConfig = uboot::default_config();
    let board_config = BoardRunConfig {
        board_type: "rk3568".into(),
        ..BoardRunConfig::default()
    };
    let qemu_runner = CargoRunnerKind::new_qemu(CargoQemuRunnerArgs {
        qemu: Some(qemu_config.clone()),
        debug: false,
        dtb_dump: false,
    });
    let uboot_runner = CargoRunnerKind::new_uboot(CargoUbootRunnerArgs {
        uboot: Some(uboot_config.clone()),
    });

    let _: BuildConfig = build::default_build_config();
    let _ = build::activate_build_config(&mut invocation, &cargo_build, None);
    let _ = qemu::default_config(&invocation);
    let _ = RunQemuOptions::default();
    let _ = board::RunBoardOptions::default();

    let _ = async {
        let _ = build::load_build_config_from_dir(&invocation, Path::new("."), false).await;
        let _ =
            build::load_build_config_from_path(&invocation, Path::new(".build.toml"), false)
                .await;
        let _ = build::build_with_config(&mut invocation, &custom_build, None).await;
        let _: anyhow::Result<CargoBuildOutput> =
            build::cargo_build(&mut invocation, &cargo, None).await;
        let _ = build::prepare_runtime_artifact(
            &mut invocation,
            RuntimeArtifactInput::new("target/kernel", true)
                .with_cargo_artifact_dir("target/aarch64/debug")
                .strip_elf(false),
        );
        let _ = build::cargo_run(&mut invocation, &cargo, None, &qemu_runner).await;
        let _ = build::cargo_run(&mut invocation, &cargo, None, &uboot_runner).await;

        let _ = qemu::read_config_from_path(&invocation, Path::new(".qemu.toml")).await;
        let _ = qemu::read_config_from_path_for_cargo(
            &invocation,
            &cargo,
            Path::new(".qemu.toml"),
        )
        .await;
        let _ = qemu::ensure_config_in_dir(&invocation, Path::new(".")).await;
        let _ = qemu::ensure_config_for_cargo(&invocation, &cargo).await;
        let _ = qemu::run_qemu(&mut invocation, &qemu_config, RunQemuOptions::default()).await;

        let _ = uboot::read_config_from_path(&invocation, Path::new(".uboot.toml")).await;
        let _ = uboot::read_config_from_path_for_cargo(
            &invocation,
            &cargo,
            Path::new(".uboot.toml"),
        )
        .await;
        let _ = uboot::ensure_config_in_dir(&invocation, Path::new(".")).await;
        let _ = uboot::ensure_config_for_cargo(&invocation, &cargo).await;
        let _ = uboot::run_uboot(&mut invocation, &uboot_config).await;

        let _ = board::read_run_config_from_path(&invocation, Path::new(".board.toml")).await;
        let _ = board::read_run_config_from_path_for_cargo(
            &invocation,
            &cargo,
            Path::new(".board.toml"),
        )
        .await;
        let _ = board::ensure_run_config_in_dir(&invocation, Path::new(".")).await;
        let _ =
            board::ensure_run_config_in_dir_for_cargo(&invocation, &cargo, Path::new("."))
                .await;
        let _ = board::run_board(
            &mut invocation,
            &cargo_build,
            None,
            &board_config,
            board::RunBoardOptions::default(),
        )
        .await;
        let _ = board::cargo_run_board(
            &mut invocation,
            &cargo,
            None,
            &board_config,
            board::RunBoardOptions::default(),
        )
        .await;
        let _ = board::run_prepared_board(
            &mut invocation,
            &board_config,
            board::RunBoardOptions::default(),
        )
        .await;
    };
}
