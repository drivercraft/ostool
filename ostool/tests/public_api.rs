#[test]
fn public_api_matches_invocation_runtime_construction() {
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

    let t = trybuild::TestCases::new();
    t.pass("tests/ui/pass_invocation_configs.rs");
    t.pass("tests/ui/pass_module_level_apis.rs");
    t.compile_fail("tests/ui/fail_cargo_pipeline.rs");
    t.compile_fail("tests/ui/fail_invocation_state.rs");
    t.compile_fail("tests/ui/fail_qemu_override.rs");
    t.compile_fail("tests/ui/fail_runner_path_field.rs");
}
