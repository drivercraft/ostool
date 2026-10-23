//! Deployment planning/rendering never touches host services or /etc.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
fn run(file: &Path, output: &Path, edition: &str) -> Output {
    Command::new(concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/deploy.sh"))
        .args(["systemd", "up", "--env-file"])
        .arg(file)
        .arg("--output-dir")
        .arg(output)
        // A stale caller variable must not change the selected file's edition.
        .env(
            "OSTOOL_EDITION",
            if edition == "local" {
                "public"
            } else {
                "local"
            },
        )
        .output()
        .unwrap()
}
#[test]
fn native_render_selects_proxy_by_edition_and_does_not_expose_backend_secrets() {
    for edition in ["local", "public"] {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join(".env");
        let output = root.path().join("out");
        fs::write(&file,format!("OSTOOL_EDITION={edition}\nOSTOOL_LISTEN_ADDR=127.0.0.1:2999\nOSTOOL_PUBLIC_BASE_URL=https://boards.example.com\nOSTOOL_SITE_ADDRESSES=boards.example.com\nOSTOOL_DATABASE_URL=postgres://user:sensitive-password@localhost/db\nOSTOOL_SMTP_PASSWORD='$(touch {})'\n",root.path().join("unexpected").display())).unwrap();
        let result = run(&file, &output, edition);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            fs::read(output.join(".env")).unwrap(),
            fs::read(file).unwrap()
        );
        assert!(output.join("ostool-server.service").exists());
        assert_eq!(output.join("Caddyfile").exists(), edition == "public");
        assert_eq!(
            output.join("ostool-server-caddy.service").exists(),
            edition == "public"
        );
        assert!(!root.path().join("unexpected").exists());
        if edition == "public" {
            let proxy = fs::read_to_string(output.join("ostool-server-caddy.service")).unwrap();
            assert!(!proxy.contains("sensitive-password"));
            assert!(!proxy.contains("SMTP"));
            assert!(proxy.contains("OSTOOL_UPSTREAM=127.0.0.1:2999"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(output.join(".env"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
}
#[test]
fn public_plan_rejects_exposed_native_api_before_any_installation() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join(".env");
    let output = root.path().join("out");
    fs::write(&file,"OSTOOL_EDITION=public\nOSTOOL_LISTEN_ADDR=0.0.0.0:2999\nOSTOOL_PUBLIC_BASE_URL=https://boards.example.com\n").unwrap();
    assert!(!run(&file, &output, "public").status.success());
    assert!(!output.exists());
}
#[test]
fn executable_checks_one_env_without_creating_a_toml_or_starting_hardware() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join(".env");
    fs::write(&file,"OSTOOL_EDITION=local\nOSTOOL_LISTEN_ADDR=127.0.0.1:2999\nOSTOOL_NETWORK_INTERFACE=lo\nOSTOOL_DATABASE_URL=postgres://unreachable.invalid/unused\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_ostool-server"))
        .current_dir(root.path())
        .args(["--env-file"])
        .arg(&file)
        .arg("--check-config")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    fs::write(&file, "OSTOOL_EDITION=local\nOSTOOL_NETWORK_INTERFACE=lo\n").unwrap();
    let missing = Command::new(env!("CARGO_BIN_EXE_ostool-server"))
        .current_dir(root.path())
        .args(["--env-file"])
        .arg(&file)
        .arg("--check-config")
        .env_remove("OSTOOL_DATABASE_URL")
        .output()
        .unwrap();
    assert!(!missing.status.success());
}
