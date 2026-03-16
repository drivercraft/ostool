use std::{collections::HashMap, path::PathBuf};

use anyhow::Context;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
struct TargetBuildInfo {
    #[serde(default)]
    rustflags: Vec<String>,
    #[serde(default)]
    cargoargs: Vec<String>,
}

pub fn detect_build_config(manifest_path: &PathBuf, target: &str) -> anyhow::Result<Vec<String>> {
    let mut cargo_args = Vec::new();

    let meta = read_metadata(manifest_path, true)?;
    let mut someboot_roots = collect_someboot_roots(&meta);
    if someboot_roots.is_empty() {
        // `--no-deps` metadata does not include transitive crates.io packages.
        // Fall back to full metadata when no local/direct hit is found.
        let meta_with_deps = read_metadata(manifest_path, false)?;
        someboot_roots = collect_someboot_roots(&meta_with_deps);
    }

    if someboot_roots.is_empty() {
        return Ok(cargo_args);
    }

    let build_info_path = someboot_roots
        .into_iter()
        .map(|root| root.join("build-info.toml"))
        .find(|p| p.exists());

    let Some(build_info_path) = build_info_path else {
        return Ok(cargo_args);
    };

    let build_info_raw = std::fs::read_to_string(&build_info_path).with_context(|| {
        format!(
            "failed to read build-info.toml: {}",
            build_info_path.display()
        )
    })?;

    let build_info: HashMap<String, TargetBuildInfo> = toml::from_str(&build_info_raw)
        .with_context(|| {
            format!(
                "failed to parse build-info.toml at {}",
                build_info_path.display()
            )
        })?;

    let Some(matched) = pick_target_build_info(&build_info, target) else {
        return Ok(cargo_args);
    };

    cargo_args.extend(matched.cargoargs.iter().cloned());

    if !matched.rustflags.is_empty() {
        cargo_args.push("--config".to_string());
        cargo_args.push(rustflags_to_cargo_override(target, &matched.rustflags));
    }

    Ok(cargo_args)
}

fn read_metadata(
    manifest_path: &PathBuf,
    no_deps: bool,
) -> anyhow::Result<cargo_metadata::Metadata> {
    let mut cmd = cargo_metadata::MetadataCommand::new();
    cmd.manifest_path(manifest_path);
    if no_deps {
        cmd.no_deps();
    }

    cmd.exec().with_context(|| {
        let mode = if no_deps { "--no-deps" } else { "with deps" };
        format!(
            "failed to read Cargo metadata ({mode}) from manifest path: {}",
            manifest_path.display()
        )
    })
}

fn collect_someboot_roots(meta: &cargo_metadata::Metadata) -> Vec<PathBuf> {
    let mut someboot_roots: Vec<PathBuf> = meta
        .packages
        .iter()
        .flat_map(|pkg| pkg.dependencies.iter())
        .filter(|dep| dep.name == "someboot")
        .filter_map(|dep| dep.path.clone())
        .map(|p| p.into_std_path_buf())
        .collect();

    someboot_roots.extend(
        meta.packages
            .iter()
            .filter(|pkg| pkg.name == "someboot")
            .filter_map(|pkg| {
                pkg.manifest_path
                    .parent()
                    .map(|p| p.as_std_path().to_path_buf())
            }),
    );

    someboot_roots.sort();
    someboot_roots.dedup();
    someboot_roots
}

fn pick_target_build_info<'a>(
    build_info: &'a HashMap<String, TargetBuildInfo>,
    target: &str,
) -> Option<&'a TargetBuildInfo> {
    if let Some(exact) = build_info.get(target) {
        return Some(exact);
    }

    let mut contains_target: Vec<_> = build_info
        .iter()
        .filter(|(cfg_target, _)| target.contains(cfg_target.as_str()))
        .collect();

    contains_target.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(b.0)));
    if let Some((_, info)) = contains_target.first() {
        return Some(*info);
    }

    let mut target_contains: Vec<_> = build_info
        .iter()
        .filter(|(cfg_target, _)| cfg_target.contains(target))
        .collect();

    target_contains.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(b.0)));
    target_contains.first().map(|(_, info)| *info)
}

fn rustflags_to_cargo_override(target: &str, rustflags: &[String]) -> String {
    let rustflags_toml =
        toml::Value::Array(rustflags.iter().cloned().map(toml::Value::String).collect())
            .to_string();

    format!("target.{target}.rustflags={rustflags_toml}")
}

#[cfg(test)]
mod tests {
    use super::detect_build_config;
    use std::path::PathBuf;

    #[test]
    fn test_local() {
        detect_build_config_works_for_sparreal_manifest(
            "/home/ubuntu/workspace/sparreal-os/Cargo.toml",
        );
    }

    #[test]
    fn test_crateio() {
        detect_build_config_works_for_sparreal_manifest(
            "/home/ubuntu/workspace/tgoskits/Cargo.toml",
        );
    }

    fn detect_build_config_works_for_sparreal_manifest(p: &str) {
        let manifest_path = PathBuf::from(p);
        if !manifest_path.exists() {
            return;
        }

        let args = detect_build_config(&manifest_path, "aarch64-unknown-none")
            .expect("detect_build_config should succeed");

        println!("Detected cargo args: ");
        for arg in &args {
            println!("  {arg}");
        }

        assert!(
            args.len() >= 4,
            "expected at least cargoargs and rustflags config, got: {args:?}"
        );
        assert_eq!(&args[0..2], ["-Z", "build-std=core,alloc"]);
        assert_eq!(args[2], "--config");

        let rustflags_config = args
            .iter()
            .find(|arg| arg.starts_with("target.aarch64-unknown-none.rustflags="))
            .expect("target rustflags command-line override should exist");

        assert!(rustflags_config.contains("\"-C\""));
        assert!(rustflags_config.contains("\"relocation-model=pic\""));
        assert!(rustflags_config.contains("\"-Clink-args=-pie\""));

        let contains_match_args =
            detect_build_config(&manifest_path, "aarch64-unknown-none-softfloat")
                .expect("contains match should succeed");
        assert_eq!(&contains_match_args[0..2], ["-Z", "build-std=core,alloc"]);
        assert!(
            contains_match_args
                .iter()
                .any(|arg| arg.starts_with("target.aarch64-unknown-none-softfloat.rustflags="))
        );
    }
}
