//! Menuconfig hooks for Cargo build configuration fields.
//!
//! The hooks keep the interactive `.build.toml` editor close to Cargo metadata:
//! package and feature choices come from the selected workspace, and target
//! candidates prefer docs.rs metadata before falling back to rustup.

use std::{path::Path, process::Command, sync::Arc};

use anyhow::{Context, anyhow, bail};
use jkconfig::data::{
    ElementHook, HookContext, HookFlow, HookOption, MessageLevel, MultiSelectBinding,
    MultiSelectSpec, SingleSelectBinding, SingleSelectSpec,
};

/// Builds the hook set used by the `.build.toml` menuconfig editor.
pub fn build_config_hooks(workspace_dir: &Path) -> Vec<ElementHook> {
    vec![
        feature_select_hook(workspace_dir),
        package_select_hook(workspace_dir),
        target_select_hook(workspace_dir),
    ]
}

fn feature_select_hook(workspace_dir: &Path) -> ElementHook {
    let path = "system.features";
    let cargo_toml = workspace_dir.join("Cargo.toml");
    ElementHook {
        path: path.into(),
        callback: Arc::new(move |ctx: &mut HookContext<'_>, path| {
            let package = ctx
                .get_string("system.package")?
                .unwrap_or_default()
                .trim()
                .to_string();
            if package.is_empty() {
                ctx.show_message(
                    jkconfig::data::MessageLevel::Warning,
                    "Select a package before editing features.",
                );
                return Ok(HookFlow::Consumed);
            }

            let feature_options = collect_feature_options(&cargo_toml, &package)?;
            let options = feature_options
                .into_iter()
                .map(|feature| HookOption::new(feature.clone(), feature))
                .collect();

            ctx.present_multi_select(MultiSelectSpec {
                title: format!("Features for {package}"),
                help: Some(
                    "Space toggle  Enter apply. Dependency features use dep_name/feature.".into(),
                ),
                options,
                selected: ctx.get_strings(path.clone())?,
                min_selected: None,
                max_selected: None,
                binding: MultiSelectBinding::SetStringArray { path: path.clone() },
            })?;

            Ok(HookFlow::Consumed)
        }),
    }
}

fn package_select_hook(workspace_dir: &Path) -> ElementHook {
    let path = "system.package";
    let cargo_toml = workspace_dir.join("Cargo.toml");

    ElementHook {
        path: path.into(),
        callback: Arc::new(move |ctx: &mut HookContext<'_>, path| {
            let mut items = Vec::new();
            if let Ok(metadata) = cargo_metadata::MetadataCommand::new()
                .manifest_path(&cargo_toml)
                .no_deps()
                .exec()
            {
                for pkg in &metadata.packages {
                    items.push(pkg.name.to_string());
                }
            }

            let options = items
                .into_iter()
                .map(|item| HookOption::new(item.clone(), item))
                .collect();
            ctx.present_single_select(SingleSelectSpec {
                title: "Select Package".into(),
                help: Some("Choose the Cargo package used by the build config.".into()),
                options,
                initial: ctx.get_string(path.clone())?,
                allow_clear: false,
                binding: SingleSelectBinding::SetString { path: path.clone() },
            })?;
            Ok(HookFlow::Consumed)
        }),
    }
}

fn target_select_hook(workspace_dir: &Path) -> ElementHook {
    let path = "system.target";
    let cargo_toml = workspace_dir.join("Cargo.toml");

    ElementHook {
        path: path.into(),
        callback: Arc::new(move |ctx: &mut HookContext<'_>, path| {
            let package = ctx
                .get_string("system.package")?
                .unwrap_or_default()
                .trim()
                .to_string();
            let current_target = ctx.get_string(path.clone())?;

            let mut warnings = Vec::new();
            let (options, help) = if package.is_empty() {
                fallback_rustup_targets()?
            } else {
                match collect_package_doc_targets(&cargo_toml, &package) {
                    Ok(Some(doc_targets)) => (
                        build_target_options(TargetCandidateSet::DocsRs(&doc_targets)),
                        "Select a target declared by the selected package docs.rs metadata."
                            .to_string(),
                    ),
                    Ok(None) => fallback_rustup_targets()?,
                    Err(err) => {
                        warnings.push(format!(
                            "Failed to inspect docs.rs targets for package '{package}': {err}"
                        ));
                        fallback_rustup_targets()?
                    }
                }
            };

            if options.is_empty() {
                bail!("No target candidates available for selection");
            }

            for warning in warnings {
                ctx.show_message(MessageLevel::Warning, warning);
            }

            ctx.present_single_select(SingleSelectSpec {
                title: "Select Target".into(),
                help: Some(help),
                options,
                initial: current_target,
                allow_clear: false,
                binding: SingleSelectBinding::SetString { path: path.clone() },
            })?;

            Ok(HookFlow::Consumed)
        }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RustupTargetOption {
    triple: String,
    installed: bool,
}

enum TargetCandidateSet<'targets> {
    DocsRs(&'targets [String]),
    Rustup(&'targets [RustupTargetOption]),
}

fn fallback_rustup_targets() -> anyhow::Result<(Vec<HookOption>, String)> {
    let rustup_targets = collect_rustup_targets()?;
    if rustup_targets.is_empty() {
        bail!("No Rust targets available from `rustup target list`");
    }
    Ok((
        build_target_options(TargetCandidateSet::Rustup(&rustup_targets)),
        "Package has no docs.rs targets; showing rustup targets.".to_string(),
    ))
}

/// Collect feature names for the selected package and its workspace dependencies.
///
/// Metadata is loaded with `cargo_metadata::MetadataCommand::no_deps()`, so
/// dependency feature options are collected only when the dependency is another
/// workspace member. Features from external crates are not included.
fn collect_feature_options(
    manifest_path: &Path,
    package_name: &str,
) -> anyhow::Result<Vec<String>> {
    let metadata = cargo_metadata::MetadataCommand::new()
        .manifest_path(manifest_path)
        .no_deps()
        .exec()?;
    let Some(pkg) = metadata
        .packages
        .iter()
        .find(|pkg| pkg.name == package_name)
    else {
        bail!(
            "package '{package_name}' not found in {}",
            manifest_path.display()
        );
    };

    let mut features = pkg.features.keys().cloned().collect::<Vec<_>>();
    features.sort();

    for dependency in &pkg.dependencies {
        let Some(dep_pkg) = metadata
            .packages
            .iter()
            .find(|candidate| candidate.name == dependency.name)
        else {
            continue;
        };
        let mut dep_features = dep_pkg.features.keys().cloned().collect::<Vec<_>>();
        dep_features.sort();
        features.extend(
            dep_features
                .into_iter()
                .map(|feature| format!("{}/{}", dependency.name, feature)),
        );
    }

    Ok(features)
}

fn collect_package_doc_targets(
    manifest_path: &Path,
    package_name: &str,
) -> anyhow::Result<Option<Vec<String>>> {
    let metadata = cargo_metadata::MetadataCommand::new()
        .manifest_path(manifest_path)
        .no_deps()
        .exec()
        .with_context(|| {
            format!(
                "failed to load cargo metadata from {}",
                manifest_path.display()
            )
        })?;
    let Some(pkg) = metadata
        .packages
        .iter()
        .find(|pkg| pkg.name == package_name)
    else {
        bail!(
            "package '{package_name}' not found in {}",
            manifest_path.display()
        );
    };

    parse_docs_rs_targets(&pkg.metadata)
}

fn parse_docs_rs_targets(metadata: &serde_json::Value) -> anyhow::Result<Option<Vec<String>>> {
    let Some(docs) = metadata.get("docs") else {
        return Ok(None);
    };
    let Some(docs_rs) = docs.get("rs") else {
        return Ok(None);
    };

    let targets = match docs_rs.get("targets") {
        Some(serde_json::Value::Array(values)) => {
            let mut targets = Vec::with_capacity(values.len());
            for value in values {
                let target = value.as_str().ok_or_else(|| {
                    anyhow!("package.metadata.docs.rs.targets must be an array of strings")
                })?;
                let target = target.trim();
                if target.is_empty() {
                    bail!("package.metadata.docs.rs.targets must not contain empty strings");
                }
                if !targets.iter().any(|existing| existing == target) {
                    targets.push(target.to_string());
                }
            }
            Some(targets)
        }
        Some(_) => bail!("package.metadata.docs.rs.targets must be an array of strings"),
        None => None,
    };

    let default_target = match docs_rs.get("default-target") {
        Some(serde_json::Value::String(value)) => {
            let value = value.trim();
            if value.is_empty() {
                bail!("package.metadata.docs.rs.default-target must not be empty");
            }
            Some(value.to_string())
        }
        Some(_) => bail!("package.metadata.docs.rs.default-target must be a string"),
        None => None,
    };

    let mut normalized = match targets {
        Some(targets) if !targets.is_empty() => targets,
        _ => Vec::new(),
    };

    if let Some(default_target) = default_target {
        if let Some(index) = normalized
            .iter()
            .position(|target| target == &default_target)
        {
            let value = normalized.remove(index);
            normalized.insert(0, value);
        } else {
            normalized.insert(0, default_target);
        }
    }

    if normalized.is_empty() {
        Ok(None)
    } else {
        Ok(Some(normalized))
    }
}

fn collect_rustup_targets() -> anyhow::Result<Vec<RustupTargetOption>> {
    let output = Command::new("rustup")
        .args(["target", "list"])
        .output()
        .context("failed to run `rustup target list`")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "`rustup target list` failed with {}:\n{}",
            output.status,
            stderr.trim()
        );
    }

    let stdout = String::from_utf8(output.stdout)
        .context("`rustup target list` output is not valid UTF-8")?;
    Ok(parse_rustup_targets(&stdout))
}

fn parse_rustup_targets(output: &str) -> Vec<RustupTargetOption> {
    let mut installed = Vec::new();
    let mut available = Vec::new();

    for line in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let installed_flag = line.ends_with(" (installed)");
        let triple = line
            .strip_suffix(" (installed)")
            .unwrap_or(line)
            .trim()
            .to_string();
        if triple.is_empty() {
            continue;
        }

        let option = RustupTargetOption {
            triple,
            installed: installed_flag,
        };
        if installed_flag {
            installed.push(option);
        } else {
            available.push(option);
        }
    }

    installed.extend(available);
    installed
}

fn build_target_options(candidates: TargetCandidateSet<'_>) -> Vec<HookOption> {
    match candidates {
        TargetCandidateSet::DocsRs(targets) => targets
            .iter()
            .cloned()
            .map(|target| HookOption {
                value: target.clone(),
                label: target,
                detail: Some("docs.rs target".into()),
                disabled: false,
            })
            .collect(),
        TargetCandidateSet::Rustup(targets) => targets
            .iter()
            .map(|target| HookOption {
                value: target.triple.clone(),
                label: target.triple.clone(),
                detail: Some(if target.installed {
                    "installed".into()
                } else {
                    "available".into()
                }),
                disabled: false,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    use jkconfig::data::ElementHook;

    use super::{
        RustupTargetOption, TargetCandidateSet, build_config_hooks, build_target_options,
        collect_package_doc_targets, parse_rustup_targets,
    };

    #[test]
    fn collect_package_doc_targets_uses_targets_list() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = write_workspace_with_package(
            temp.path(),
            "kernel",
            Some(
                r#"[package.metadata.docs.rs]
targets = ["riscv64gc-unknown-none-elf", "aarch64-unknown-none"]
"#,
            ),
        );

        let targets = collect_package_doc_targets(&manifest, "kernel")
            .unwrap()
            .unwrap();
        assert_eq!(
            targets,
            vec![
                "riscv64gc-unknown-none-elf".to_string(),
                "aarch64-unknown-none".to_string()
            ]
        );
    }

    #[test]
    fn collect_package_doc_targets_uses_default_target_when_targets_missing() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = write_workspace_with_package(
            temp.path(),
            "kernel",
            Some(
                r#"[package.metadata.docs.rs]
default-target = "aarch64-unknown-none"
"#,
            ),
        );

        let targets = collect_package_doc_targets(&manifest, "kernel")
            .unwrap()
            .unwrap();
        assert_eq!(targets, vec!["aarch64-unknown-none".to_string()]);
    }

    #[test]
    fn collect_package_doc_targets_moves_default_target_to_front() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = write_workspace_with_package(
            temp.path(),
            "kernel",
            Some(
                r#"[package.metadata.docs.rs]
targets = ["x86_64-unknown-none", "aarch64-unknown-none", "x86_64-unknown-none"]
default-target = "aarch64-unknown-none"
"#,
            ),
        );

        let targets = collect_package_doc_targets(&manifest, "kernel")
            .unwrap()
            .unwrap();
        assert_eq!(
            targets,
            vec![
                "aarch64-unknown-none".to_string(),
                "x86_64-unknown-none".to_string()
            ]
        );
    }

    #[test]
    fn collect_package_doc_targets_rejects_invalid_docs_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = write_workspace_with_package(
            temp.path(),
            "kernel",
            Some(
                r#"[package.metadata.docs.rs]
targets = "aarch64-unknown-none"
"#,
            ),
        );

        let err = collect_package_doc_targets(&manifest, "kernel")
            .unwrap_err()
            .to_string();
        assert!(err.contains("targets"));
        assert!(err.contains("array of strings"));
    }

    #[test]
    fn collect_package_doc_targets_errors_for_missing_package() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = write_workspace_with_package(temp.path(), "kernel", None);

        let err = collect_package_doc_targets(&manifest, "missing")
            .unwrap_err()
            .to_string();
        assert!(err.contains("package 'missing' not found"));
    }

    #[test]
    fn parse_rustup_targets_prioritizes_installed_entries() {
        let parsed = parse_rustup_targets(
            "aarch64-unknown-none\nx86_64-unknown-none (installed)\nriscv64gc-unknown-none-elf\nthumbv7em-none-eabihf (installed)\n",
        );

        let triples: Vec<_> = parsed.iter().map(|target| target.triple.as_str()).collect();
        let installed: Vec<_> = parsed.iter().map(|target| target.installed).collect();
        assert_eq!(
            triples,
            vec![
                "x86_64-unknown-none",
                "thumbv7em-none-eabihf",
                "aarch64-unknown-none",
                "riscv64gc-unknown-none-elf"
            ]
        );
        assert_eq!(installed, vec![true, true, false, false]);
    }

    #[test]
    fn parse_rustup_targets_handles_empty_output() {
        let parsed = parse_rustup_targets("");
        assert!(parsed.is_empty());
    }

    #[test]
    fn build_target_options_marks_rustup_install_state() {
        let options = build_target_options(TargetCandidateSet::Rustup(&[
            RustupTargetOption {
                triple: "x86_64-unknown-none".into(),
                installed: true,
            },
            RustupTargetOption {
                triple: "aarch64-unknown-none".into(),
                installed: false,
            },
        ]));
        assert_eq!(options[0].detail.as_deref(), Some("installed"));
        assert_eq!(options[1].detail.as_deref(), Some("available"));
    }

    #[test]
    fn build_config_hooks_include_system_target_hook() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::write(temp.path().join("src/lib.rs"), "").unwrap();

        let hooks: Vec<ElementHook> = build_config_hooks(temp.path());
        assert!(
            hooks
                .iter()
                .any(|hook| hook.path.as_key() == "system.target")
        );
    }

    fn write_workspace_with_package(root: &Path, package: &str, metadata: Option<&str>) -> PathBuf {
        fs::write(
            root.join("Cargo.toml"),
            format!("[workspace]\nmembers = [\"{package}\"]\nresolver = \"3\"\n"),
        )
        .unwrap();

        let package_dir = root.join(package);
        fs::create_dir_all(package_dir.join("src")).unwrap();
        let mut cargo_toml =
            format!("[package]\nname = \"{package}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n");
        if let Some(metadata) = metadata {
            cargo_toml.push('\n');
            cargo_toml.push_str(metadata);
        }
        fs::write(package_dir.join("Cargo.toml"), cargo_toml).unwrap();
        fs::write(package_dir.join("src/lib.rs"), "").unwrap();
        root.join("Cargo.toml")
    }
}
