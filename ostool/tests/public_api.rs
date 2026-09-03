#[test]
fn public_api_matches_invocation_runtime_construction() {
    configure_zerocopy_rustflags();

    assert_cargo_qemu_override_args_stays_out_of_public_api();

    let t = trybuild::TestCases::new();
    t.pass("tests/ui/pass_invocation_configs.rs");
    t.pass("tests/ui/pass_module_level_apis.rs");
    t.compile_fail("tests/ui/fail_cargo_pipeline.rs");
    t.compile_fail("tests/ui/fail_invocation_state.rs");
    t.compile_fail("tests/ui/fail_runner_path_field.rs");
}

fn configure_zerocopy_rustflags() {
    let rustflags = std::env::var("RUSTFLAGS").unwrap_or_default();
    let zerocopy_cfg = "--cfg no_zerocopy_simd_x86_avx12_1_89_0 --check-cfg cfg(no_zerocopy_simd_x86_avx12_1_89_0)";
    if !rustflags.contains("no_zerocopy_simd_x86_avx12_1_89_0") {
        unsafe {
            std::env::set_var(
                "RUSTFLAGS",
                [rustflags.as_str(), zerocopy_cfg].join(" ").trim(),
            );
        }
    }
    if !std::env::var("CARGO_ENCODED_RUSTFLAGS")
        .unwrap_or_default()
        .contains("no_zerocopy_simd_x86_avx12_1_89_0")
    {
        unsafe {
            std::env::set_var(
                "CARGO_ENCODED_RUSTFLAGS",
                [
                    "--cfg",
                    "no_zerocopy_simd_x86_avx12_1_89_0",
                    "--check-cfg",
                    "cfg(no_zerocopy_simd_x86_avx12_1_89_0)",
                ]
                .join("\x1f"),
            );
        }
    }
}

fn assert_cargo_qemu_override_args_stays_out_of_public_api() {
    // Rustc changed the exact underline rendering for this unresolved import
    // across stable releases, so assert the API boundary instead of snapshotting
    // the diagnostic text.
    let temp = tempfile::tempdir().expect("failed to create public API probe dir");
    let src_dir = temp.path().join("src");
    std::fs::create_dir(&src_dir).expect("failed to create public API probe src dir");

    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = format!(
        r#"[package]
name = "ostool-public-api-probe"
version = "0.0.0"
edition = "2024"

[dependencies]
ostool = {{ path = "{}" }}
"#,
        manifest_dir.display()
    );
    std::fs::write(temp.path().join("Cargo.toml"), manifest)
        .expect("failed to write public API probe manifest");
    std::fs::write(
        src_dir.join("main.rs"),
        "fn main() {\n    let _ = core::mem::size_of::<ostool::build::CargoQemuOverrideArgs>();\n}\n",
    )
    .expect("failed to write public API probe source");

    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = std::process::Command::new(cargo)
        .arg("check")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--target-dir")
        .arg(temp.path().join("target"))
        .output()
        .expect("failed to run public API cargo probe");

    assert!(
        !output.status.success(),
        "CargoQemuOverrideArgs unexpectedly remains available in ostool::build"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("CargoQemuOverrideArgs") && stderr.contains("ostool::build"),
        "public API probe failed for an unexpected reason:\n{stderr}"
    );
}
