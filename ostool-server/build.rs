use std::{
    path::Path,
    process::{Command, Stdio},
};

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

    let webui_dir = Path::new("webui");
    if !webui_dir.exists() {
        panic!("missing frontend directory: {}", webui_dir.display());
    }

    let node_modules_marker = webui_dir.join("node_modules/.modules.yaml");
    let lockfile = webui_dir.join("pnpm-lock.yaml");
    if !node_modules_marker.exists() {
        if lockfile.exists() {
            run_pnpm(webui_dir, &["install", "--frozen-lockfile"]);
        } else {
            run_pnpm(webui_dir, &["install"]);
        }
    }

    run_pnpm(webui_dir, &["build"]);

    let index_html = Path::new("web/dist/index.html");
    if !index_html.exists() {
        panic!(
            "frontend build succeeded but {} was not produced",
            index_html.display()
        );
    }
}

fn run_pnpm(webui_dir: &Path, args: &[&str]) {
    let status = Command::new("pnpm")
        .arg("--dir")
        .arg(webui_dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .unwrap_or_else(|err| {
            panic!(
                "failed to run `pnpm --dir {} {}`: {err}",
                webui_dir.display(),
                args.join(" ")
            )
        });

    if !status.success() {
        panic!(
            "`pnpm --dir {} {}` exited with status {status}",
            webui_dir.display(),
            args.join(" ")
        );
    }
}
