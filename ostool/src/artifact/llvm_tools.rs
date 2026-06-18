//! Rust toolchain LLVM tool lookup helpers.

use std::{
    env,
    ffi::{OsStr, OsString},
    path::PathBuf,
    process::Command,
};

use anyhow::{Context, anyhow, bail};

fn rustc_program() -> OsString {
    env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"))
}

fn rustc_output<I, S>(rustc: &OsStr, args: I) -> anyhow::Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new(rustc)
        .args(args)
        .output()
        .with_context(|| format!("failed to execute rustc: {}", rustc.to_string_lossy()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "rustc command failed with status {}: {}",
            output.status,
            stderr.trim()
        );
    }

    String::from_utf8(output.stdout).context("rustc output is not valid UTF-8")
}

fn rustc_sysroot(rustc: &OsStr) -> anyhow::Result<PathBuf> {
    Ok(PathBuf::from(
        rustc_output(rustc, ["--print", "sysroot"])?.trim(),
    ))
}

fn rustc_host(rustc: &OsStr) -> anyhow::Result<String> {
    rustc_output(rustc, ["-vV"])?
        .lines()
        .find_map(|line| line.strip_prefix("host: ").map(str::to_owned))
        .ok_or_else(|| anyhow!("failed to parse host triple from `rustc -vV`"))
}

fn rustlib_tool(rustc: &OsStr, tool: &str) -> anyhow::Result<PathBuf> {
    let mut path = rustc_sysroot(rustc)?;
    path.push("lib");
    path.push("rustlib");
    path.push(rustc_host(rustc)?);
    path.push("bin");
    path.push(format!("{tool}{}", env::consts::EXE_SUFFIX));
    Ok(path)
}

fn llvm_objcopy_with_rustc(rustc: &OsStr) -> anyhow::Result<PathBuf> {
    let path = rustlib_tool(rustc, "llvm-objcopy")?;
    if !path.exists() {
        bail!(
            "could not find toolchain llvm-objcopy at {}; install the Rust llvm-tools component",
            path.display()
        );
    }
    Ok(path)
}

pub(crate) fn llvm_objcopy() -> anyhow::Result<PathBuf> {
    let rustc = rustc_program();
    llvm_objcopy_with_rustc(&rustc)
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::llvm_objcopy_with_rustc;

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn missing_llvm_objcopy_error_mentions_llvm_tools_component() {
        let temp = tempfile::tempdir().unwrap();
        let sysroot = temp.path().join("sysroot");
        let fake_rustc = temp.path().join("fake-rustc");
        fs::write(
            &fake_rustc,
            format!(
                "#!/bin/sh\n\
                 if [ \"$1\" = \"--print\" ] && [ \"$2\" = \"sysroot\" ]; then\n\
                 \tprintf '%s\\n' '{}'\n\
                 elif [ \"$1\" = \"-vV\" ]; then\n\
                 \tprintf 'host: fake-host\\n'\n\
                 else\n\
                 \texit 1\n\
                 fi\n",
                sysroot.display()
            ),
        )
        .unwrap();
        make_executable(&fake_rustc);

        let err = llvm_objcopy_with_rustc(fake_rustc.as_os_str()).unwrap_err();
        let message = err.to_string();

        assert!(message.contains("could not find toolchain llvm-objcopy"));
        assert!(message.contains("install the Rust llvm-tools component"));
    }
}
