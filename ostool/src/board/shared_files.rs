use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

use anyhow::{Context as _, bail};
use url::Url;

use super::{config::BoardRunConfig, session::BoardSessionContext};
use crate::utils::replace_placeholders;

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
    for step in &mut board_config.shell_check_steps {
        step.shell_cmd = step
            .shell_cmd
            .as_deref()
            .map(|command| expand_shell_command(command, context, uploaded_files))
            .transpose()?;
    }
    Ok(())
}

fn expand_shell_command(
    command: &str,
    context: &BoardSessionContext,
    uploaded_files: &BTreeMap<String, Url>,
) -> anyhow::Result<String> {
    replace_placeholders(command, |placeholder| match placeholder {
        "boardServerIp" => Ok(Some(context.server_ip.to_string())),
        "boardServerHttpBaseUrl" => Ok(Some(context.http_base_url.to_string())),
        "sessionFile" => bail!("session file placeholder must include a relative path"),
        value if value.starts_with("sessionFile:") => {
            let relative_path = value.trim_start_matches("sessionFile:");
            let relative_path = normalize_relative_path(Path::new(relative_path))?;
            let url = uploaded_files.get(&relative_path).ok_or_else(|| {
                anyhow!(
                    "shell check command references shared session file `{relative_path}` that was not uploaded"
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
    use std::{collections::BTreeMap, fs, net::IpAddr, path::PathBuf};

    use tempfile::tempdir;
    use url::Url;

    use super::{collect_session_files, expand_board_session_variables};
    use crate::board::{config::BoardRunConfig, session::BoardSessionContext};

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
            shell_check_steps: vec![crate::run::ShellCheckStep {
                shell_prefix: Some("root#".into()),
                shell_cmd: Some(
                    "server=${boardServerIp}; file=${sessionFile:tools/probe.sh}; echo ${marker}"
                        .into(),
                ),
                ..Default::default()
            }],
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
            config.shell_check_steps[0].shell_cmd.as_deref(),
            Some(
                "server=192.168.1.2; file=http://192.168.1.2:2999/share/sessions/session-1/tools/probe.sh; echo ${marker}"
            )
        );
    }

    #[test]
    fn session_variables_reject_missing_uploaded_file() {
        let mut config = BoardRunConfig {
            shell_check_steps: vec![crate::run::ShellCheckStep {
                shell_prefix: Some("root#".into()),
                shell_cmd: Some("${sessionFile:missing.sh}".into()),
                ..Default::default()
            }],
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
    fn session_variables_expand_only_commands_across_all_shell_check_steps() {
        let mut config = BoardRunConfig {
            shell_check_steps: vec![
                crate::run::ShellCheckStep {
                    shell_prefix: Some("${sessionFile:not-a-reference-prefix}".into()),
                    shell_cmd: Some(
                        concat!(
                            "server=${boardServerIp}; base=${boardServerHttpBaseUrl}; ",
                            "wget ${sessionFile:tools/probe.sh} -O /tmp/probe.sh"
                        )
                        .into(),
                    ),
                    success_regex: Some(vec!["${sessionFile:not-a-reference-success}".into()]),
                    fail_regex: Some(vec!["${sessionFile:not-a-reference-fail}".into()]),
                    ..Default::default()
                },
                crate::run::ShellCheckStep {
                    shell_prefix: Some("root#".into()),
                    shell_cmd: Some("sh ${sessionFile:scripts/check.sh}".into()),
                    success_regex: Some(vec!["PASS".into()]),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let context = BoardSessionContext {
            session_id: "session-1".into(),
            server_ip: "192.168.1.2".parse().unwrap(),
            http_base_url: Url::parse("http://192.168.1.2:2999/").unwrap(),
        };
        let files = BTreeMap::from([
            (
                "tools/probe.sh".into(),
                Url::parse("http://192.168.1.2:2999/share/sessions/session-1/tools/probe.sh")
                    .unwrap(),
            ),
            (
                "scripts/check.sh".into(),
                Url::parse("http://192.168.1.2:2999/share/sessions/session-1/scripts/check.sh")
                    .unwrap(),
            ),
        ]);

        expand_board_session_variables(&mut config, &context, &files).unwrap();

        assert_eq!(
            config.shell_check_steps[0].shell_cmd.as_deref(),
            Some(concat!(
                "server=192.168.1.2; base=http://192.168.1.2:2999/; ",
                "wget http://192.168.1.2:2999/share/sessions/session-1/tools/probe.sh ",
                "-O /tmp/probe.sh"
            ))
        );
        assert_eq!(
            config.shell_check_steps[1].shell_cmd.as_deref(),
            Some("sh http://192.168.1.2:2999/share/sessions/session-1/scripts/check.sh")
        );
        assert_eq!(
            config.shell_check_steps[0].shell_prefix.as_deref(),
            Some("${sessionFile:not-a-reference-prefix}")
        );
        assert_eq!(
            config.shell_check_steps[0].success_regex.as_deref(),
            Some(&["${sessionFile:not-a-reference-success}".into()][..])
        );
        assert_eq!(
            config.shell_check_steps[0].fail_regex.as_deref(),
            Some(&["${sessionFile:not-a-reference-fail}".into()][..])
        );
    }
}
