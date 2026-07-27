use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

use anyhow::{Context as _, bail};
use url::Url;

use super::{config::BoardRunConfig, session::BoardSessionContext};
use crate::utils::replace_placeholders;

pub(crate) const SESSION_PROGRAM_FAILURE_MARKER: &str = "OSTOOL_SESSION_PROGRAM_FAILED";
const SESSION_PROGRAM_DOWNLOAD_ATTEMPTS: u32 = 60;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionFileUpload {
    source_path: PathBuf,
    relative_path: String,
}

impl SessionFileUpload {
    pub fn from_root(root: &Path, relative_path: &Path) -> anyhow::Result<Self> {
        let relative_path = normalize_relative_path(relative_path)?;
        let canonical_root = root
            .canonicalize()
            .with_context(|| format!("failed to resolve shared file root {}", root.display()))?;
        let source_path = canonical_root.join(&relative_path);
        let canonical_source = source_path.canonicalize().with_context(|| {
            format!(
                "failed to resolve shared session file {}",
                source_path.display()
            )
        })?;
        if !canonical_source.starts_with(&canonical_root) {
            bail!(
                "shared session file `{relative_path}` resolves outside {}",
                canonical_root.display()
            );
        }
        if !canonical_source.is_file() {
            bail!(
                "shared session file `{relative_path}` is not a regular file under {}",
                canonical_root.display()
            );
        }
        Ok(Self {
            source_path: canonical_source,
            relative_path,
        })
    }

    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }

    pub(crate) async fn read(&self) -> anyhow::Result<Vec<u8>> {
        tokio::fs::read(&self.source_path).await.with_context(|| {
            format!(
                "failed to read shared session file {}",
                self.source_path.display()
            )
        })
    }
}

pub(crate) fn collect_session_files(
    root: &Path,
    relative_paths: &[PathBuf],
) -> anyhow::Result<Vec<SessionFileUpload>> {
    let mut seen = BTreeSet::new();
    let mut uploads = Vec::with_capacity(relative_paths.len());
    for relative_path in relative_paths {
        let upload = SessionFileUpload::from_root(root, relative_path)?;
        if !seen.insert(upload.relative_path.clone()) {
            bail!(
                "duplicate shared session file path `{}`",
                upload.relative_path
            );
        }
        uploads.push(upload);
    }
    Ok(uploads)
}

pub(crate) fn expand_board_session_variables(
    board_config: &mut BoardRunConfig,
    context: &BoardSessionContext,
    uploaded_files: &BTreeMap<String, Url>,
) -> anyhow::Result<()> {
    if let Some(shell_init_cmd) = board_config.shell_init_cmd.as_deref() {
        board_config.shell_init_cmd = Some(expand_session_placeholders(
            shell_init_cmd,
            "shell_init_cmd",
            context,
            uploaded_files,
        )?);
    }
    if board_config.session_program.is_some() {
        prepare_session_program(board_config, context, uploaded_files)?;
    }
    Ok(())
}

fn expand_session_placeholders(
    value: &str,
    field_name: &str,
    context: &BoardSessionContext,
    uploaded_files: &BTreeMap<String, Url>,
) -> anyhow::Result<String> {
    replace_placeholders(value, |placeholder| match placeholder {
        "boardServerIp" => Ok(Some(context.server_ip.to_string())),
        "boardServerHttpBaseUrl" => Ok(Some(context.http_base_url.to_string())),
        "sessionFile" => bail!("session file placeholder must include a relative path"),
        value if value.starts_with("sessionFile:") => {
            let relative_path = value.trim_start_matches("sessionFile:");
            let relative_path = normalize_relative_path(Path::new(relative_path))?;
            let url = uploaded_files.get(&relative_path).ok_or_else(|| {
                anyhow!(
                    "{field_name} references shared session file `{relative_path}` that was not uploaded"
                )
            })?;
            Ok(Some(url.to_string()))
        }
        value if value.starts_with("boardServer") || value.starts_with("sessionFile") => {
            bail!("unknown board session placeholder `${{{value}}}`")
        }
        _ => Ok(None),
    })
}

fn prepare_session_program(
    board_config: &mut BoardRunConfig,
    context: &BoardSessionContext,
    uploaded_files: &BTreeMap<String, Url>,
) -> anyhow::Result<()> {
    let program = board_config
        .session_program
        .as_ref()
        .expect("caller checked session_program")
        .clone();
    let program_path = normalize_relative_path(&program.path)?;
    if !uploaded_files.contains_key(&program_path) {
        bail!("session program `{program_path}` was not uploaded");
    }

    let session_root = format!("/tmp/ostool-session/{}", context.session_id);
    let mut command = String::from(
        "ostool_marker=OSTOOL_SESSION_PROGRAM; \
         ostool_fetch() { ostool_url=$1; ostool_dest=$2; ostool_try=0; \
         while [ \"$ostool_try\" -lt ",
    );
    command.push_str(&SESSION_PROGRAM_DOWNLOAD_ATTEMPTS.to_string());
    command.push_str(
        " ]; do \
         if command -v curl >/dev/null 2>&1 && \
         curl --connect-timeout 2 --max-time 5 -fsSL \"$ostool_url\" -o \"$ostool_dest\"; \
         then return 0; fi; \
         if command -v wget >/dev/null 2>&1 && \
         wget -T 5 -O \"$ostool_dest\" \"$ostool_url\"; then return 0; fi; \
         ostool_try=$((ostool_try + 1)); sleep 1; done; return 1; }; ",
    );
    command.push_str("ostool_root=");
    command.push_str(&posix_shell_quote(&session_root));
    command.push_str("; rm -rf \"$ostool_root\" && mkdir -p \"$ostool_root\"");

    for (relative_path, url) in uploaded_files {
        let parent = relative_path.rsplit_once('/').map(|(parent, _)| parent);
        command.push_str(" && mkdir -p ");
        command.push_str(&session_root_path(parent));
        command.push_str(" && ostool_fetch ");
        command.push_str(&posix_shell_quote(url.as_str()));
        command.push(' ');
        command.push_str(&session_root_path(Some(relative_path)));
    }

    command.push_str(" && chmod +x ");
    command.push_str(&session_root_path(Some(&program_path)));
    command.push_str(" && cd \"$ostool_root\" && ");
    command.push_str(&posix_shell_quote(&format!("./{program_path}")));
    for (index, argument) in program.args.iter().enumerate() {
        let argument = expand_session_placeholders(
            argument,
            &format!("session_program.args[{index}]"),
            context,
            uploaded_files,
        )?;
        command.push(' ');
        command.push_str(&posix_shell_quote(&argument));
    }
    command.push_str(" || echo \"${ostool_marker}_FAILED\"");

    let failure_regex = format!(r"(?m)^{SESSION_PROGRAM_FAILURE_MARKER}\s*$");
    if !board_config.fail_regex.contains(&failure_regex) {
        board_config.fail_regex.push(failure_regex);
    }
    board_config.shell_init_cmd = Some(command);
    Ok(())
}

fn posix_shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn session_root_path(relative_path: Option<&str>) -> String {
    match relative_path {
        Some(relative_path) => {
            format!("\"$ostool_root\"/{}", posix_shell_quote(relative_path))
        }
        None => "\"$ostool_root\"".to_string(),
    }
}

fn normalize_relative_path(path: &Path) -> anyhow::Result<String> {
    if path.as_os_str().is_empty() {
        bail!("shared session file path must not be empty");
    }
    let mut segments = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => {
                let segment = segment
                    .to_str()
                    .ok_or_else(|| anyhow!("shared session file path must contain valid UTF-8"))?;
                if segment.trim().is_empty() {
                    bail!("shared session file path contains an empty segment");
                }
                segments.push(segment.to_string());
            }
            Component::CurDir => bail!("shared session file path must not contain `.` segments"),
            Component::ParentDir => {
                bail!("shared session file path must not contain `..` segments")
            }
            Component::RootDir | Component::Prefix(_) => {
                bail!("shared session file path must be relative")
            }
        }
    }
    if segments.is_empty() {
        bail!("shared session file path must not be empty");
    }
    Ok(segments.join("/"))
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs, net::IpAddr, path::PathBuf, process::Command};

    use tempfile::tempdir;
    use url::Url;

    use super::{
        SESSION_PROGRAM_FAILURE_MARKER, collect_session_files, expand_board_session_variables,
    };
    use crate::board::{
        config::{BoardRunConfig, BoardSessionProgram},
        session::BoardSessionContext,
    };

    #[test]
    fn shared_files_keep_their_relative_paths() {
        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join("tools/network")).unwrap();
        fs::write(root.path().join("tools/network/probe.sh"), b"probe").unwrap();

        let uploads =
            collect_session_files(root.path(), &[PathBuf::from("tools/network/probe.sh")]).unwrap();

        assert_eq!(uploads[0].relative_path(), "tools/network/probe.sh");
    }

    #[test]
    fn shared_files_reject_parent_and_symlink_escape() {
        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("probe.sh"), b"probe").unwrap();
        assert!(collect_session_files(root.path(), &[PathBuf::from("../probe.sh")]).is_err());

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                outside.path().join("probe.sh"),
                root.path().join("probe.sh"),
            )
            .unwrap();
            assert!(collect_session_files(root.path(), &[PathBuf::from("probe.sh")]).is_err());
        }
    }

    #[test]
    fn shared_files_reject_duplicate_paths() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("probe.sh"), b"probe").unwrap();

        let error = collect_session_files(
            root.path(),
            &[PathBuf::from("probe.sh"), PathBuf::from("probe.sh")],
        )
        .unwrap_err();

        assert!(error.to_string().contains("duplicate"));
    }

    #[test]
    fn session_variables_expand_known_values_and_preserve_shell_variables() {
        let mut config = BoardRunConfig {
            shell_init_cmd: Some(
                "server=${boardServerIp}; file=${sessionFile:tools/probe.sh}; echo ${marker}"
                    .into(),
            ),
            ..Default::default()
        };
        let context = BoardSessionContext {
            session_id: "session-1".into(),
            server_ip: "192.168.1.2".parse::<IpAddr>().unwrap(),
            http_base_url: Url::parse("http://192.168.1.2:2999/").unwrap(),
        };
        let files = BTreeMap::from([(
            "tools/probe.sh".into(),
            Url::parse("http://192.168.1.2:2999/share/sessions/session-1/tools/probe.sh").unwrap(),
        )]);

        expand_board_session_variables(&mut config, &context, &files).unwrap();

        assert_eq!(
            config.shell_init_cmd.as_deref(),
            Some(
                "server=192.168.1.2; file=http://192.168.1.2:2999/share/sessions/session-1/tools/probe.sh; echo ${marker}"
            )
        );
    }

    #[test]
    fn session_variables_reject_missing_uploaded_file() {
        let mut config = BoardRunConfig {
            shell_init_cmd: Some("${sessionFile:missing.sh}".into()),
            ..Default::default()
        };
        let context = BoardSessionContext {
            session_id: "session-1".into(),
            server_ip: "192.168.1.2".parse().unwrap(),
            http_base_url: Url::parse("http://192.168.1.2:2999/").unwrap(),
        };

        assert!(expand_board_session_variables(&mut config, &context, &BTreeMap::new()).is_err());
    }

    #[test]
    fn session_program_builds_download_and_exec_command_with_quoted_args() {
        let mut config = BoardRunConfig {
            shell_prefix: Some("root@starry:".into()),
            session_program: Some(BoardSessionProgram {
                path: PathBuf::from("bin/probe"),
                args: vec![
                    "--server=${boardServerIp}".into(),
                    "argument with spaces".into(),
                    "single'quote".into(),
                ],
            }),
            ..Default::default()
        };
        let context = BoardSessionContext {
            session_id: "session-1".into(),
            server_ip: "192.168.1.2".parse().unwrap(),
            http_base_url: Url::parse("http://192.168.1.2:2999/").unwrap(),
        };
        let files = BTreeMap::from([(
            "bin/probe".into(),
            Url::parse("http://192.168.1.2:2999/share/sessions/session-1/bin/probe").unwrap(),
        )]);

        expand_board_session_variables(&mut config, &context, &files).unwrap();

        let command = config.shell_init_cmd.as_deref().unwrap();
        assert!(command.contains("/tmp/ostool-session/session-1"));
        assert!(command.contains("share/sessions/session-1/bin/probe"));
        assert!(command.contains("'--server=192.168.1.2'"));
        assert!(command.contains("'argument with spaces'"));
        assert!(command.contains("'single'\"'\"'quote'"));
        assert!(command.contains("curl"));
        assert!(command.contains("wget"));
        assert!(command.contains("${ostool_marker}_FAILED"));
        assert!(!command.contains(SESSION_PROGRAM_FAILURE_MARKER));
        assert!(
            config
                .fail_regex
                .iter()
                .any(|regex| regex.contains(SESSION_PROGRAM_FAILURE_MARKER))
        );
    }

    #[test]
    fn session_program_rejects_missing_uploaded_program() {
        let mut config = BoardRunConfig {
            shell_prefix: Some("root@starry:".into()),
            session_program: Some(BoardSessionProgram {
                path: PathBuf::from("bin/missing"),
                args: Vec::new(),
            }),
            ..Default::default()
        };
        let context = BoardSessionContext {
            session_id: "session-1".into(),
            server_ip: "192.168.1.2".parse().unwrap(),
            http_base_url: Url::parse("http://192.168.1.2:2999/").unwrap(),
        };

        let error =
            expand_board_session_variables(&mut config, &context, &BTreeMap::new()).unwrap_err();

        assert!(error.to_string().contains("bin/missing"));
    }

    #[cfg(unix)]
    #[test]
    fn session_program_command_downloads_executes_and_reports_failure() {
        use std::os::unix::fs::PermissionsExt;

        let tools = tempdir().unwrap();
        let curl = tools.path().join("curl");
        fs::write(
            &curl,
            r#"#!/bin/sh
destination=
while [ "$#" -gt 0 ]; do
    if [ "$1" = "-o" ]; then
        shift
        destination=$1
    fi
    shift
done
cat >"$destination" <<'PROGRAM'
#!/bin/sh
printf 'PROBE_ARG=%s\n' "$1"
exit "${OSTOOL_FAKE_PROGRAM_EXIT:-0}"
PROGRAM
"#,
        )
        .unwrap();
        let mut permissions = fs::metadata(&curl).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&curl, permissions).unwrap();

        let mut config = BoardRunConfig {
            shell_prefix: Some("root@starry:".into()),
            session_program: Some(BoardSessionProgram {
                path: PathBuf::from("bin/probe"),
                args: vec!["argument with spaces".into()],
            }),
            ..Default::default()
        };
        let context = BoardSessionContext {
            session_id: "session-shell-test".into(),
            server_ip: "192.168.1.2".parse().unwrap(),
            http_base_url: Url::parse("http://192.168.1.2:2999/").unwrap(),
        };
        let files = BTreeMap::from([(
            "bin/probe".into(),
            Url::parse("http://192.168.1.2:2999/share/sessions/session-shell-test/bin/probe")
                .unwrap(),
        )]);
        expand_board_session_variables(&mut config, &context, &files).unwrap();
        let command = config.shell_init_cmd.as_deref().unwrap();
        let path = format!(
            "{}:{}",
            tools.path().display(),
            std::env::var("PATH").unwrap_or_default()
        );

        let success = Command::new("sh")
            .arg("-c")
            .arg(command)
            .env("PATH", &path)
            .output()
            .unwrap();
        assert!(success.status.success());
        assert_eq!(
            String::from_utf8_lossy(&success.stdout).trim(),
            "PROBE_ARG=argument with spaces"
        );

        let failure = Command::new("sh")
            .arg("-c")
            .arg(command)
            .env("PATH", &path)
            .env("OSTOOL_FAKE_PROGRAM_EXIT", "7")
            .output()
            .unwrap();
        assert!(failure.status.success());
        assert!(
            String::from_utf8_lossy(&failure.stdout)
                .lines()
                .any(|line| line == SESSION_PROGRAM_FAILURE_MARKER)
        );

        let session_root = std::path::Path::new("/tmp/ostool-session/session-shell-test");
        if session_root.exists() {
            fs::remove_dir_all(session_root).unwrap();
        }
    }
}
