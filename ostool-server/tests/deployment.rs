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
        assert_eq!(
            output.join("ostool-server.caddy").exists(),
            edition == "public"
        );
        assert!(!output.join("ostool-server-caddy.service").exists());
        assert!(!root.path().join("unexpected").exists());
        if edition == "public" {
            let proxy = fs::read_to_string(output.join("ostool-server.caddy")).unwrap();
            assert!(!proxy.contains("sensitive-password"));
            assert!(!proxy.contains("SMTP"));
            assert!(proxy.contains("reverse_proxy 127.0.0.1:2999"));
            assert!(proxy.contains("https://boards.example.com"));
            assert!(!proxy.contains("admin off"));
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

#[cfg(unix)]
#[test]
fn dev_uses_only_docker_and_keeps_release_authentication_for_both_editions() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let docker = bin.join("docker");
    fs::write(&docker,r#"#!/usr/bin/env bash
set -euo pipefail
printf 'args:' >> "$SCRIPT_TEST_TRACE"
printf ' <%s>' "$@" >> "$SCRIPT_TEST_TRACE"
printf ' sites=<%s> upstream=<%s> profiles=<%s>\n' "${OSTOOL_GATEWAY_SITES:-}" "${OSTOOL_GATEWAY_UPSTREAM:-}" "${COMPOSE_PROFILES:-}" >> "$SCRIPT_TEST_TRACE"
if [[ " $* " == *' config --environment '* ]]; then cat "$OSTOOL_ENV_FILE"; fi
"#).unwrap();
    fs::set_permissions(&docker, fs::Permissions::from_mode(0o755)).unwrap();
    for name in ["systemctl", "journalctl", "sudo", "node", "cargo"] {
        let file = bin.join(name);
        fs::write(
            &file,
            "#!/bin/sh\nprintf 'unexpected native/deploy tool\\n' >&2\nexit 99\n",
        )
        .unwrap();
        fs::set_permissions(file, fs::Permissions::from_mode(0o755)).unwrap();
    }
    for edition in ["local", "public"] {
        let env_file = root.path().join(format!("{edition} config.env"));
        let trace = root.path().join(format!("{edition}.trace"));
        let listener = if edition == "public" {
            "127.0.0.1:2999"
        } else {
            "0.0.0.0:2999"
        };
        fs::write(&env_file,format!("OSTOOL_EDITION={edition}\nOSTOOL_LISTEN_ADDR={listener}\nOSTOOL_PUBLIC_BASE_URL=https://127.0.0.1:3443\nOSTOOL_ALLOWED_ORIGINS=https://localhost:3443,https://192.0.2.10:3443\nOSTOOL_ADMIN_PASSWORD=private-test-value\n")).unwrap();
        for action in ["start", "restart", "check", "logs", "seed", "stop"] {
            let mut command = Command::new(concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/dev.sh"));
            command.arg(action).arg("--env-file").arg(&env_file);
            if action == "logs" {
                command.arg("server");
            }
            let output = command
                .env(
                    "PATH",
                    format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
                )
                .env("SCRIPT_TEST_TRACE", &trace)
                .env("OSTOOL_EDITION", "stale-value")
                .env("COMPOSE_PROFILES", "unexpected-profile")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{edition} {action}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(!String::from_utf8_lossy(&output.stdout).contains("private-test-value"));
        }
        let trace = fs::read_to_string(trace).unwrap();
        assert!(trace.contains("<build>") || trace.contains("<--build>"));
        assert!(trace.contains("<--force-recreate>"));
        assert!(trace.contains("<--entrypoint> <ostool-server-seed-demo>"));
        assert!(trace.contains("<logs> <--tail> <100> <server>"));
        assert!(!trace.contains("unexpected-profile"));
        assert!(!trace.contains("--dev-demo"));
        assert!(!trace.contains("OSTOOL_AUTH_ENABLED"));
        if edition == "public" {
            assert!(trace.contains("<--profile> <gateway> <up>"));
            assert!(trace.contains(
                "sites=<https://127.0.0.1:3443 https://localhost:3443 https://192.0.2.10:3443>"
            ));
        } else {
            assert!(trace.contains("<--profile> <gateway> <stop> <caddy>"));
        }
    }
}

#[test]
fn script_help_is_plain_in_pipes_and_invalid_arguments_fail_before_environment_checks() {
    for name in ["dev.sh", "deploy.sh"] {
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("scripts")
            .join(name);
        let help = Command::new(&script)
            .arg("--help")
            .env("OSTOOL_ENV_FILE", "/nonexistent/.env")
            .output()
            .unwrap();
        assert!(help.status.success());
        let text = String::from_utf8(help.stdout).unwrap();
        assert!(text.contains("用法："));
        assert!(text.contains("命令："));
        assert!(text.contains("选项："));
        assert!(text.contains("示例："));
        assert!(!text.contains('\u{1b}'));
        let args: Vec<&str> = if name == "dev.sh" {
            vec!["start", "--env-file"]
        } else {
            vec!["docker", "up", "--env-file"]
        };
        let error = Command::new(&script).args(args).output().unwrap();
        assert_eq!(error.status.code(), Some(2));
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/deploy.sh");
    let build = Command::new(&script)
        .args([
            "systemd",
            "build",
            "--dry-run",
            "--env-file",
            "/nonexistent/.env",
        ])
        .output()
        .unwrap();
    assert!(build.status.success());
    assert!(String::from_utf8_lossy(&build.stdout).contains("计划：cargo build"));
    for action in ["cert", "seed", "unsupported"] {
        assert_eq!(
            Command::new(&script)
                .args(["docker", action])
                .output()
                .unwrap()
                .status
                .code(),
            Some(2)
        );
    }
}

#[test]
fn public_executable_rejects_plaintext_origins_and_exposed_backends_before_startup() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join(".env");
    for (address, origin, valid) in [
        ("127.0.0.1:2999", "http://192.168.7.10:2999", false),
        ("0.0.0.0:2999", "https://boards.example.com", false),
        ("127.0.0.1:2999", "https://boards.example.com", true),
    ] {
        fs::write(&file,format!("OSTOOL_EDITION=public\nOSTOOL_LISTEN_ADDR={address}\nOSTOOL_PUBLIC_BASE_URL={origin}\nOSTOOL_NETWORK_INTERFACE=lo\nOSTOOL_DATABASE_URL=postgres://unreachable.invalid/unused\n")).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_ostool-server"))
            .current_dir(root.path())
            .args(["--env-file"])
            .arg(&file)
            .arg("--check-config")
            .output()
            .unwrap();
        assert_eq!(
            result.status.success(),
            valid,
            "{address} {origin}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}
