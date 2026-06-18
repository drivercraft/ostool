//! Runtime artifact preparation helpers.
//!
//! The build pipeline may produce an executable in Cargo's artifact directory
//! or receive a custom ELF path. This module normalizes that input into a runtime
//! ELF plus optional derived images consumed by QEMU, U-Boot, TFTP, and board runners.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, anyhow};
use colored::Colorize;
use object::{Architecture, Object};

use crate::{artifact::llvm_tools, process::ProcessContext, utils::PathResultExt};

/// Options controlling how an input ELF is prepared for runtime use.
pub(crate) struct RuntimeArtifactOptions {
    /// Input ELF path produced by Cargo or supplied by a custom build.
    pub(crate) elf_path: PathBuf,
    /// Whether to also produce a raw BIN image.
    pub(crate) to_bin: bool,
    /// Optional directory for the generated BIN image.
    pub(crate) bin_dir: Option<PathBuf>,
    /// Whether to preserve debug information when producing a BIN image.
    pub(crate) debug: bool,
    /// Cargo artifact directory reported by `--message-format=json`, when known.
    pub(crate) cargo_artifact_dir: Option<PathBuf>,
    /// Whether to copy the input ELF into a stripped runtime `.elf` file first.
    pub(crate) strip_elf: bool,
}

/// Runtime artifacts prepared from a single input ELF.
pub(crate) struct PreparedRuntimeArtifacts {
    elf: PathBuf,
    bin: Option<PathBuf>,
    source_artifact_dir: PathBuf,
    cargo_artifact_dir: Option<PathBuf>,
    runtime_artifact_dir: Option<PathBuf>,
    arch: Option<Architecture>,
}

impl PreparedRuntimeArtifacts {
    /// Returns the runtime ELF path.
    pub(crate) fn elf(&self) -> &Path {
        &self.elf
    }

    /// Returns the runtime BIN path, when one was generated.
    pub(crate) fn bin(&self) -> Option<&Path> {
        self.bin.as_deref()
    }

    /// Returns the Cargo artifact directory that produced the input ELF.
    pub(crate) fn cargo_artifact_dir(&self) -> Option<&Path> {
        self.cargo_artifact_dir
            .as_deref()
            .or(Some(self.source_artifact_dir.as_path()))
    }

    /// Returns the Cargo-reported artifact directory, when the input came from Cargo.
    pub(crate) fn cargo_source_artifact_dir(&self) -> Option<&Path> {
        self.cargo_artifact_dir.as_deref()
    }

    /// Returns the directory containing the artifact consumed by runners.
    pub(crate) fn runtime_artifact_dir(&self) -> Option<&Path> {
        self.runtime_artifact_dir.as_deref()
    }

    /// Returns the architecture detected from the input ELF.
    pub(crate) fn arch(&self) -> Option<Architecture> {
        self.arch
    }
}

/// Prepares runtime ELF/BIN artifacts from a Cargo or custom-build ELF.
pub(crate) fn prepare_runtime_artifacts(
    context: &ProcessContext,
    options: RuntimeArtifactOptions,
) -> anyhow::Result<PreparedRuntimeArtifacts> {
    let input_elf = options
        .elf_path
        .canonicalize()
        .with_path("failed to canonicalize file", &options.elf_path)?;
    let input_dir = input_elf
        .parent()
        .ok_or_else(|| anyhow!("invalid ELF file path: {}", input_elf.display()))?
        .to_path_buf();
    let arch = detect_architecture(&input_elf)?;

    let runtime_elf = if options.strip_elf {
        strip_runtime_elf(context, &input_elf, arch)?
    } else {
        input_elf
    };

    let mut prepared = PreparedRuntimeArtifacts {
        elf: runtime_elf.clone(),
        bin: None,
        source_artifact_dir: input_dir,
        cargo_artifact_dir: options.cargo_artifact_dir,
        runtime_artifact_dir: runtime_elf.parent().map(PathBuf::from),
        arch: Some(arch),
    };

    if options.to_bin {
        let bin_path = convert_runtime_bin(context, &runtime_elf, options.bin_dir, options.debug)?;
        prepared.runtime_artifact_dir = bin_path.parent().map(PathBuf::from);
        prepared.bin = Some(bin_path);
    }

    Ok(prepared)
}

fn detect_architecture(path: &Path) -> anyhow::Result<Architecture> {
    let binary_data = fs::read(path).with_path("failed to read ELF file", path)?;
    let file = object::File::parse(binary_data.as_slice())
        .with_context(|| format!("failed to parse ELF file: {}", path.display()))?;
    Ok(file.architecture())
}

fn strip_runtime_elf(
    context: &ProcessContext,
    elf_path: &Path,
    arch: Architecture,
) -> anyhow::Result<PathBuf> {
    let stripped_elf_path = elf_path.with_file_name(
        elf_path
            .file_stem()
            .ok_or_else(|| anyhow!("invalid ELF file path: {}", elf_path.display()))?
            .to_string_lossy()
            .to_string()
            + ".elf",
    );
    println!(
        "{}",
        format!(
            "Stripping ELF file...\r\n  original elf: {}\r\n  stripped elf: {}",
            elf_path.display(),
            stripped_elf_path.display()
        )
        .bold()
        .purple()
    );

    let objcopy_program = llvm_tools::llvm_objcopy()?;
    let mut objcopy = crate::process::command(objcopy_program, context);
    objcopy.arg(format!(
        "--binary-architecture={}",
        format!("{arch:?}").to_lowercase()
    ));
    objcopy.arg(elf_path);
    objcopy.arg(&stripped_elf_path);
    objcopy.run()?;

    Ok(stripped_elf_path)
}

fn convert_runtime_bin(
    context: &ProcessContext,
    elf_path: &Path,
    bin_dir: Option<PathBuf>,
    debug: bool,
) -> anyhow::Result<PathBuf> {
    let elf_path = elf_path
        .canonicalize()
        .with_path("failed to canonicalize file", elf_path)?;
    let bin_name = elf_path
        .file_stem()
        .ok_or_else(|| anyhow!("invalid ELF file path: {}", elf_path.display()))?
        .to_string_lossy()
        .to_string()
        + ".bin";

    let bin_path = if let Some(bin_dir) = bin_dir {
        bin_dir.join(bin_name)
    } else {
        elf_path.with_file_name(bin_name)
    };

    if let Some(parent) = bin_path.parent() {
        fs::create_dir_all(parent).with_path("failed to create directory", parent)?;
    }

    println!(
        "{}",
        format!(
            "Converting ELF to BIN format...\r\n  elf: {}\r\n  bin: {}",
            elf_path.display(),
            bin_path.display()
        )
        .bold()
        .purple()
    );

    let objcopy_program = llvm_tools::llvm_objcopy()?;
    let mut objcopy = crate::process::command(objcopy_program, context);

    if !debug {
        objcopy.arg("--strip-all");
    }

    objcopy
        .arg("-O")
        .arg("binary")
        .arg(&elf_path)
        .arg(&bin_path);
    objcopy.run()?;

    Ok(bin_path)
}

#[cfg(test)]
mod tests {
    use std::{
        env,
        ffi::OsString,
        fs,
        path::{Path, PathBuf},
        process::Command,
    };

    use crate::{
        artifact::runtime::{RuntimeArtifactOptions, prepare_runtime_artifacts},
        process::ProcessContext,
        project::{resolve_project_layout, variables::VariableScope},
    };

    fn process_context(root: &std::path::Path) -> ProcessContext {
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "").unwrap();

        let layout = resolve_project_layout(Some(root.to_path_buf())).unwrap();
        let scope = VariableScope::for_package(&layout, root.to_path_buf());
        ProcessContext::new(root.to_path_buf(), root.to_path_buf(), scope, None)
    }

    fn copy_current_exe(root: &Path) -> PathBuf {
        let source = std::env::current_exe().unwrap();
        let input = root.join("sample");
        fs::copy(&source, &input).unwrap();
        input
    }

    fn rust_objcopy_program() -> OsString {
        env::var_os("OSTOOL_TEST_RUST_OBJCOPY").unwrap_or_else(|| OsString::from("rust-objcopy"))
    }

    fn run_rust_objcopy(args: &[OsString]) {
        let program = rust_objcopy_program();
        let output = Command::new(&program)
            .args(args)
            .output()
            .unwrap_or_else(|err| {
                panic!(
                    "failed to execute {}; install cargo-binutils for this comparison test: {err}",
                    program.to_string_lossy()
                )
            });

        assert!(
            output.status.success(),
            "{} failed with status {}\nstdout:\n{}\nstderr:\n{}",
            program.to_string_lossy(),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn prepares_stripped_elf_without_mutating_tool_state() {
        let temp = tempfile::tempdir().unwrap();
        let context = process_context(temp.path());
        let input = copy_current_exe(temp.path());

        let prepared = prepare_runtime_artifacts(
            &context,
            RuntimeArtifactOptions {
                elf_path: input.clone(),
                to_bin: false,
                bin_dir: None,
                debug: false,
                cargo_artifact_dir: Some(temp.path().join("target/debug")),
                strip_elf: true,
            },
        )
        .unwrap();

        let expected_elf = input.with_file_name("sample.elf");
        assert_eq!(prepared.elf(), expected_elf);
        assert!(prepared.bin().is_none());
        assert_eq!(
            prepared.cargo_artifact_dir(),
            Some(temp.path().join("target/debug").as_path())
        );
        assert_eq!(
            prepared.cargo_source_artifact_dir(),
            Some(temp.path().join("target/debug").as_path())
        );
        assert_eq!(prepared.runtime_artifact_dir(), Some(temp.path()));
        assert!(prepared.arch().is_some());
        assert!(expected_elf.exists());
    }

    #[test]
    fn prepares_optional_bin_in_custom_output_dir() {
        let temp = tempfile::tempdir().unwrap();
        let context = process_context(temp.path());
        let input = copy_current_exe(temp.path());
        let bin_dir = temp.path().join("bin-out");

        let prepared = prepare_runtime_artifacts(
            &context,
            RuntimeArtifactOptions {
                elf_path: input,
                to_bin: true,
                bin_dir: Some(bin_dir.clone()),
                debug: false,
                cargo_artifact_dir: None,
                strip_elf: true,
            },
        )
        .unwrap();

        let expected_bin = bin_dir.join("sample.bin");
        assert_eq!(prepared.bin(), Some(expected_bin.as_path()));
        assert_eq!(prepared.cargo_artifact_dir(), Some(temp.path()));
        assert!(prepared.cargo_source_artifact_dir().is_none());
        assert_eq!(prepared.runtime_artifact_dir(), Some(bin_dir.as_path()));
        assert!(expected_bin.exists());
    }

    fn compare_to_rust_objcopy(debug: bool) {
        let temp = tempfile::tempdir().unwrap();
        let context = process_context(temp.path());
        let input = copy_current_exe(temp.path());
        let actual_dir = temp.path().join("actual");
        let expected_dir = temp.path().join("expected");

        let prepared = prepare_runtime_artifacts(
            &context,
            RuntimeArtifactOptions {
                elf_path: input.clone(),
                to_bin: true,
                bin_dir: Some(actual_dir),
                debug,
                cargo_artifact_dir: None,
                strip_elf: false,
            },
        )
        .unwrap();

        fs::create_dir_all(&expected_dir).unwrap();
        let expected_bin = expected_dir.join("sample.bin");
        let mut args = Vec::new();
        if !debug {
            args.push(OsString::from("--strip-all"));
        }
        args.push(OsString::from("-O"));
        args.push(OsString::from("binary"));
        args.push(input.as_os_str().to_os_string());
        args.push(expected_bin.as_os_str().to_os_string());
        run_rust_objcopy(&args);

        let actual_bin = prepared.bin().unwrap();
        assert_eq!(
            fs::read(actual_bin).unwrap(),
            fs::read(expected_bin).unwrap()
        );
    }

    #[test]
    fn to_bin_matches_rust_objcopy_without_debug() {
        compare_to_rust_objcopy(false);
    }

    #[test]
    fn to_bin_matches_rust_objcopy_with_debug() {
        compare_to_rust_objcopy(true);
    }
}
