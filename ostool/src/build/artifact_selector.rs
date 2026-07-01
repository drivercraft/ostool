use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail};

/// Executable artifact resolved from Cargo JSON messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedCargoArtifact {
    elf_path: PathBuf,
    cargo_artifact_dir: PathBuf,
}

impl ResolvedCargoArtifact {
    pub(crate) fn new(elf_path: PathBuf, cargo_artifact_dir: PathBuf) -> Self {
        Self {
            elf_path,
            cargo_artifact_dir,
        }
    }

    pub(crate) fn elf_path(&self) -> &Path {
        &self.elf_path
    }

    pub(crate) fn cargo_artifact_dir(&self) -> &Path {
        &self.cargo_artifact_dir
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CargoExecutableArtifact {
    target_name: String,
    artifact: ResolvedCargoArtifact,
}

impl CargoExecutableArtifact {
    pub(crate) fn new(target_name: String, artifact: ResolvedCargoArtifact) -> Self {
        Self {
            target_name,
            artifact,
        }
    }

    fn target_name(&self) -> &str {
        &self.target_name
    }

    fn artifact(&self) -> &ResolvedCargoArtifact {
        &self.artifact
    }
}

pub(crate) fn select_executable_artifact(
    executable_artifacts: &[CargoExecutableArtifact],
    explicit_bin: Option<&str>,
    explicit_test: Option<&str>,
    default_run: Option<&str>,
    package: &str,
) -> anyhow::Result<ResolvedCargoArtifact> {
    if let Some(bin) = explicit_bin {
        return executable_artifacts
            .iter()
            .rev()
            .find(|candidate| candidate.target_name() == bin)
            .map(|candidate| candidate.artifact().clone())
            .ok_or_else(|| {
                anyhow!(
                    "binary target `{bin}` was not built for package `{package}`; check system.Cargo.bin or --bin"
                )
            });
    }

    if let Some(test) = explicit_test {
        return executable_artifacts
            .iter()
            .rev()
            .find(|candidate| candidate.target_name() == test)
            .map(|candidate| candidate.artifact().clone())
            .ok_or_else(|| {
                anyhow!(
                    "test target `{test}` was not built for package `{package}`; check system.Cargo.test or --test"
                )
            });
    }

    if executable_artifacts.is_empty() {
        bail!("no executable artifact found in cargo JSON output for package `{package}`");
    }

    if let Some(candidate) = executable_artifacts
        .iter()
        .rev()
        .find(|candidate| candidate.target_name() == package)
    {
        return Ok(candidate.artifact().clone());
    }

    if let Some(default_bin) = default_run
        && let Some(candidate) = executable_artifacts
            .iter()
            .rev()
            .find(|candidate| candidate.target_name() == default_bin)
    {
        return Ok(candidate.artifact().clone());
    }

    if executable_artifacts.len() == 1 {
        return Ok(executable_artifacts[0].artifact().clone());
    }

    let bins = executable_artifacts
        .iter()
        .map(|candidate| candidate.target_name())
        .collect::<Vec<_>>()
        .join(", ");
    bail!(
        "package `{package}` has multiple binary targets ({bins}); pass system.Cargo.bin or --bin"
    )
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{CargoExecutableArtifact, ResolvedCargoArtifact, select_executable_artifact};

    fn artifact(name: &str) -> ResolvedCargoArtifact {
        let cargo_artifact_dir = PathBuf::from("/tmp/ostool-target/debug");
        ResolvedCargoArtifact::new(cargo_artifact_dir.join(name), cargo_artifact_dir)
    }

    fn candidate(name: &str) -> CargoExecutableArtifact {
        CargoExecutableArtifact::new(name.to_string(), artifact(name))
    }

    fn select(
        artifacts: &[CargoExecutableArtifact],
        explicit_bin: Option<&str>,
        explicit_test: Option<&str>,
        default_run: Option<&str>,
        package: &str,
    ) -> anyhow::Result<ResolvedCargoArtifact> {
        select_executable_artifact(artifacts, explicit_bin, explicit_test, default_run, package)
    }

    #[test]
    fn select_executable_artifact_uses_explicit_bin_first() {
        let artifacts = vec![candidate("kernel"), candidate("kernel-qemu")];

        let selected = select(&artifacts, Some("kernel-qemu"), None, None, "kernel").unwrap();

        assert_eq!(
            selected.elf_path(),
            Path::new("/tmp/ostool-target/debug/kernel-qemu")
        );
    }

    #[test]
    fn select_executable_artifact_errors_when_explicit_bin_was_not_built() {
        let artifacts = vec![candidate("kernel")];

        let err = select(&artifacts, Some("missing-bin"), None, None, "kernel").unwrap_err();

        assert!(
            err.to_string()
                .contains("binary target `missing-bin` was not built")
        );
    }

    #[test]
    fn select_executable_artifact_uses_explicit_test_target() {
        let artifacts = vec![candidate("kernel"), candidate("axtest_kernel")];

        let selected = select(&artifacts, None, Some("axtest_kernel"), None, "kernel").unwrap();

        assert_eq!(
            selected.elf_path(),
            Path::new("/tmp/ostool-target/debug/axtest_kernel")
        );
    }

    #[test]
    fn select_executable_artifact_errors_when_explicit_test_was_not_built() {
        let artifacts = vec![candidate("kernel")];

        let err = select(&artifacts, None, Some("missing-test"), None, "kernel").unwrap_err();

        assert!(
            err.to_string()
                .contains("test target `missing-test` was not built")
        );
    }

    #[test]
    fn select_executable_artifact_prefers_package_name_before_default_run() {
        let artifacts = vec![candidate("helper"), candidate("kernel")];

        let selected = select(&artifacts, None, None, Some("helper"), "kernel").unwrap();

        assert_eq!(
            selected.elf_path(),
            Path::new("/tmp/ostool-target/debug/kernel")
        );
    }

    #[test]
    fn select_executable_artifact_uses_default_run_without_package_name_binary() {
        let artifacts = vec![candidate("helper"), candidate("boot-test")];

        let selected = select(&artifacts, None, None, Some("boot-test"), "kernel").unwrap();

        assert_eq!(
            selected.elf_path(),
            Path::new("/tmp/ostool-target/debug/boot-test")
        );
    }

    #[test]
    fn select_executable_artifact_uses_single_binary_as_fallback() {
        let artifacts = vec![candidate("helper")];

        let selected = select(&artifacts, None, None, None, "kernel").unwrap();

        assert_eq!(
            selected.elf_path(),
            Path::new("/tmp/ostool-target/debug/helper")
        );
    }

    #[test]
    fn select_executable_artifact_errors_on_empty_cargo_output() {
        let err = select(&[], None, None, None, "kernel").unwrap_err();

        assert!(err.to_string().contains("no executable artifact found"));
    }

    #[test]
    fn select_executable_artifact_errors_on_ambiguous_multiple_binaries() {
        let artifacts = vec![candidate("kernel-qemu"), candidate("kernel-uboot")];

        let err = select(&artifacts, None, None, None, "kernel").unwrap_err();

        let rendered = err.to_string();
        assert!(rendered.contains("multiple binary targets"));
        assert!(rendered.contains("kernel-qemu"));
        assert!(rendered.contains("kernel-uboot"));
    }
}
