#[test]
fn public_api_matches_invocation_runtime_construction() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/pass_invocation_configs.rs");
    t.pass("tests/ui/pass_module_level_apis.rs");
    t.compile_fail("tests/ui/fail_cargo_pipeline.rs");
    t.compile_fail("tests/ui/fail_invocation_state.rs");
    t.compile_fail("tests/ui/fail_qemu_override.rs");
    t.compile_fail("tests/ui/fail_runner_path_field.rs");
}
