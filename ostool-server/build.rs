use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const EMBED_WEB_DIR_ENV: &str = "OSTOOL_SERVER_WEB_DIST_DIR";

fn main() {
    for path in [
        "webui/index.html",
        "webui/package.json",
        "webui/pnpm-lock.yaml",
        "webui/tsconfig.json",
        "webui/vite.config.ts",
        "webui/src",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR"));
    let webui_dir = manifest_dir.join("webui");
    if !webui_dir.exists() {
        panic!("missing frontend directory: {}", webui_dir.display());
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("missing OUT_DIR"));
    let web_work_dir = out_dir.join("web-work");
    let web_dist_dir = out_dir.join("web-assets");

    recreate_dir(&web_work_dir);
    recreate_dir(&web_dist_dir);

    copy_webui_source(&webui_dir, &web_work_dir);

    let node_modules_marker = web_work_dir.join("node_modules/.modules.yaml");
    if !node_modules_marker.exists() {
        run_pnpm(&web_work_dir, &["install", "--frozen-lockfile"], None);
    }

    println!(
        "cargo:rustc-env={EMBED_WEB_DIR_ENV}={}",
        web_dist_dir.display()
    );
    run_pnpm(
        &web_work_dir,
        &["run", "build"],
        Some((EMBED_WEB_DIR_ENV, web_dist_dir.as_os_str())),
    );

    let index_html = web_dist_dir.join("index.html");
    if !index_html.exists() {
        panic!(
            "frontend build succeeded but {} was not produced",
            index_html.display()
        );
    }
}

fn recreate_dir(dir: &Path) {
    if dir.exists() {
        fs::remove_dir_all(dir)
            .unwrap_or_else(|err| panic!("failed to clean directory {}: {err}", dir.display()));
    }
    fs::create_dir_all(dir)
        .unwrap_or_else(|err| panic!("failed to create directory {}: {err}", dir.display()));
}

fn copy_webui_source(source: &Path, target: &Path) {
    copy_dir_all(source, target);
}

fn copy_dir_all(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap_or_else(|err| {
        panic!(
            "failed to create target directory {}: {err}",
            target.display()
        )
    });

    let entries = fs::read_dir(source)
        .unwrap_or_else(|err| panic!("failed to read directory {}: {err}", source.display()));

    for entry in entries {
        let entry = entry
            .unwrap_or_else(|err| panic!("failed to read entry in {}: {err}", source.display()));
        let file_name = entry.file_name();
        let path = entry.path();
        let destination = target.join(&file_name);
        let file_type = entry.file_type().unwrap_or_else(|err| {
            panic!(
                "failed to determine file type for {}: {err}",
                path.display()
            )
        });

        if should_skip(&file_name) {
            continue;
        }

        if file_type.is_dir() {
            copy_dir_all(&path, &destination);
        } else if file_type.is_file() {
            fs::copy(&path, &destination).unwrap_or_else(|err| {
                panic!(
                    "failed to copy {} to {}: {err}",
                    path.display(),
                    destination.display()
                )
            });
        }
    }
}

fn should_skip(file_name: &std::ffi::OsStr) -> bool {
    matches!(
        file_name.to_str(),
        Some("node_modules") | Some("dist") | Some(".vite") | Some(".cache")
    )
}

fn run_pnpm(work_dir: &Path, args: &[&str], extra_env: Option<(&str, &std::ffi::OsStr)>) {
    let mut command = Command::new("pnpm");
    command
        .arg("--dir")
        .arg(work_dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    if let Some((key, value)) = extra_env {
        command.env(key, value);
    }

    let status = command.status().unwrap_or_else(|err| {
        panic!(
            "failed to run `pnpm --dir {} {}`: {err}",
            work_dir.display(),
            args.join(" ")
        )
    });

    if !status.success() {
        panic!(
            "`pnpm --dir {} {}` exited with status {status}",
            work_dir.display(),
            args.join(" ")
        );
    }
}
