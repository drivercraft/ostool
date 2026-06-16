//! Optional analysis artifact generation for prepared ELF files.

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    artifact::{
        object_tools::{ObjectToolKind, ObjectTools},
        state::{DebugArtifactKind, OutputArtifacts},
    },
    process::{self, ProcessContext},
    utils::PathResultExt,
};

/// Optional analysis artifacts derived from the prepared runtime ELF.
#[derive(Default, Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct AnalysisArtifactConfig {
    /// Generate a disassembly text artifact.
    #[serde(default, skip_serializing_if = "is_false")]
    pub disassembly: bool,
    /// Generate an ELF information text artifact.
    #[serde(default, skip_serializing_if = "is_false")]
    pub elf_info: bool,
    /// Generate a symbol table text artifact.
    #[serde(default, skip_serializing_if = "is_false")]
    pub symbols: bool,
}

impl AnalysisArtifactConfig {
    pub(crate) fn is_empty(&self) -> bool {
        !self.disassembly && !self.elf_info && !self.symbols
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

pub(crate) fn generate_analysis_artifacts(
    context: &ProcessContext,
    elf_path: &Path,
    output_dir: &Path,
    config: &AnalysisArtifactConfig,
    tools: &ObjectTools,
    artifacts: &mut OutputArtifacts,
) -> anyhow::Result<()> {
    if config.is_empty() {
        return Ok(());
    }

    fs::create_dir_all(output_dir).with_path("failed to create directory", output_dir)?;

    if config.disassembly {
        run_analysis_tool(
            context,
            tools,
            elf_path,
            output_dir,
            artifacts,
            AnalysisToolSpec {
                tool: ObjectToolKind::Objdump,
                args: vec![OsString::from("-d"), elf_path.as_os_str().to_os_string()],
                kind: DebugArtifactKind::Disassembly,
            },
        )?;
    }

    if config.elf_info {
        run_analysis_tool(
            context,
            tools,
            elf_path,
            output_dir,
            artifacts,
            AnalysisToolSpec {
                tool: ObjectToolKind::Readobj,
                args: vec![
                    OsString::from("--file-headers"),
                    OsString::from("--program-headers"),
                    OsString::from("--sections"),
                    OsString::from("--symbols"),
                    elf_path.as_os_str().to_os_string(),
                ],
                kind: DebugArtifactKind::ElfInfo,
            },
        )?;
    }

    if config.symbols {
        run_analysis_tool(
            context,
            tools,
            elf_path,
            output_dir,
            artifacts,
            AnalysisToolSpec {
                tool: ObjectToolKind::Nm,
                args: vec![OsString::from("-n"), elf_path.as_os_str().to_os_string()],
                kind: DebugArtifactKind::Symbols,
            },
        )?;
    }

    Ok(())
}

struct AnalysisToolSpec {
    tool: ObjectToolKind,
    args: Vec<OsString>,
    kind: DebugArtifactKind,
}

fn run_analysis_tool(
    context: &ProcessContext,
    tools: &ObjectTools,
    elf_path: &Path,
    output_dir: &Path,
    artifacts: &mut OutputArtifacts,
    spec: AnalysisToolSpec,
) -> anyhow::Result<()> {
    let output_path = analysis_output_path(elf_path, output_dir, spec.kind)?;
    let mut command = process::command(tools.program(spec.tool), context);
    command.args(spec.args);
    command.print_cmd();
    let output = command
        .output()
        .with_context(|| format!("failed to run analysis tool for {}", elf_path.display()))?;
    if !output.status.success() {
        bail!(
            "analysis tool failed with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fs::write(&output_path, output.stdout).with_path("failed to write file", &output_path)?;
    artifacts.register_debug_artifact(spec.kind, output_path);
    Ok(())
}

fn analysis_output_path(
    elf_path: &Path,
    output_dir: &Path,
    kind: DebugArtifactKind,
) -> anyhow::Result<PathBuf> {
    let stem = elf_path
        .file_stem()
        .ok_or_else(|| anyhow!("invalid ELF file path: {}", elf_path.display()))?
        .to_string_lossy();
    let suffix = match kind {
        DebugArtifactKind::Disassembly => "disassembly.txt",
        DebugArtifactKind::ElfInfo => "elf-info.txt",
        DebugArtifactKind::Symbols => "symbols.txt",
    };
    Ok(output_dir.join(format!("{stem}.{suffix}")))
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use crate::{
        artifact::{
            analysis::{AnalysisArtifactConfig, generate_analysis_artifacts},
            object_tools::ObjectTools,
            state::{DebugArtifactKind, OutputArtifacts},
        },
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

    fn fake_tool(root: &std::path::Path, name: &str, body: &str) -> PathBuf {
        let script = root.join(name);
        fs::write(&script, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&script).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&script, permissions).unwrap();
        }
        script
    }

    #[test]
    fn generates_requested_analysis_outputs() {
        let temp = tempfile::tempdir().unwrap();
        let context = process_context(temp.path());
        let elf = temp.path().join("kernel.elf");
        fs::write(&elf, "elf").unwrap();
        let out = temp.path().join("analysis");
        let mut artifacts = OutputArtifacts::default();

        fake_tool(
            temp.path(),
            "rust-objdump",
            "#!/bin/sh\nprintf 'disassembly:%s\\n' \"$@\"\n",
        );
        fake_tool(
            temp.path(),
            "rust-readobj",
            "#!/bin/sh\nprintf 'elf-info:%s\\n' \"$@\"\n",
        );
        fake_tool(
            temp.path(),
            "rust-nm",
            "#!/bin/sh\nprintf 'symbols:%s\\n' \"$@\"\n",
        );

        let old_path = std::env::var_os("PATH").unwrap_or_default();
        let new_path = format!("{}:{}", temp.path().display(), old_path.to_string_lossy());
        unsafe {
            std::env::set_var("PATH", new_path);
        }

        generate_analysis_artifacts(
            &context,
            &elf,
            &out,
            &AnalysisArtifactConfig {
                disassembly: true,
                elf_info: true,
                symbols: true,
            },
            &ObjectTools,
            &mut artifacts,
        )
        .unwrap();

        unsafe {
            std::env::set_var("PATH", old_path);
        }

        let disassembly = artifacts
            .debug_artifacts()
            .get(DebugArtifactKind::Disassembly)
            .unwrap();
        let elf_info = artifacts
            .debug_artifacts()
            .get(DebugArtifactKind::ElfInfo)
            .unwrap();
        let symbols = artifacts
            .debug_artifacts()
            .get(DebugArtifactKind::Symbols)
            .unwrap();

        assert_eq!(disassembly, out.join("kernel.disassembly.txt").as_path());
        assert_eq!(elf_info, out.join("kernel.elf-info.txt").as_path());
        assert_eq!(symbols, out.join("kernel.symbols.txt").as_path());
        assert!(fs::read_to_string(disassembly).unwrap().contains("-d"));
        assert!(
            fs::read_to_string(elf_info)
                .unwrap()
                .contains("--file-headers")
        );
        assert!(fs::read_to_string(symbols).unwrap().contains("-n"));
    }
}
