use std::path::{Path, PathBuf};

use crate::{invocation::Invocation, project::variables};

use super::{
    RunBoardOptions,
    config::BoardRunConfig,
    shared_files::{SessionFileUpload, collect_session_files},
};

#[derive(Clone, Debug)]
pub struct BoardRunRequest {
    board_config: BoardRunConfig,
    options: RunBoardOptions,
    session_files: Vec<SessionFileUpload>,
}

impl BoardRunRequest {
    pub fn new(board_config: BoardRunConfig, options: RunBoardOptions) -> Self {
        Self {
            board_config,
            options,
            session_files: Vec::new(),
        }
    }

    pub fn with_session_files(
        mut self,
        root: &Path,
        relative_paths: &[PathBuf],
    ) -> anyhow::Result<Self> {
        self.session_files = collect_session_files(root, relative_paths)?;
        Ok(self)
    }

    /// Resolve declared files relative to the board configuration source.
    /// An omitted source is the workspace's `.board.toml`, matching the CLI loader.
    pub fn with_configured_session_files(
        self,
        invocation: &Invocation,
        config_path: Option<&Path>,
    ) -> anyhow::Result<Self> {
        let path = match config_path {
            Some(path) => variables::expand_path_variables(path, &invocation.variable_scope()?)?,
            None => invocation.workspace_dir().join(".board.toml"),
        };
        let root = path
            .parent()
            .filter(|root| !root.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let relative_paths = self.board_config.session_files.clone();
        self.with_session_files(root, &relative_paths)
    }

    pub(crate) fn into_parts(self) -> (BoardRunConfig, RunBoardOptions, Vec<SessionFileUpload>) {
        (self.board_config, self.options, self.session_files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invocation::InvocationOptions;

    #[tokio::test]
    async fn configured_uploads_use_the_board_file_directory_and_workspace_default() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("src")).unwrap();
        std::fs::write(temp.path().join("Cargo.toml"), "[package]\nname=\"request-fixture\"\nversion=\"0.0.0\"\nedition=\"2024\"\n[workspace]\n").unwrap();
        std::fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::create_dir(temp.path().join("boards")).unwrap();
        std::fs::write(temp.path().join("shared.txt"), "workspace file").unwrap();
        std::fs::write(temp.path().join("boards/shared.txt"), "board file").unwrap();
        let invocation = Invocation::new(InvocationOptions::new(
            Some(temp.path().into()),
            None,
            None,
            false,
        ))
        .unwrap();
        for (source, expected) in [
            (None, "workspace file"),
            (
                Some(Path::new("${workspace}/boards/board.toml")),
                "board file",
            ),
        ] {
            let config = BoardRunConfig {
                session_files: vec!["shared.txt".into()],
                ..Default::default()
            };
            let request = BoardRunRequest::new(config, RunBoardOptions::default())
                .with_configured_session_files(&invocation, source)
                .unwrap();
            let (_, _, files) = request.into_parts();
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].relative_path(), "shared.txt");
            assert_eq!(files[0].read().await.unwrap(), expected.as_bytes());
        }
    }
}
