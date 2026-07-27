use std::path::{Path, PathBuf};

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
        let mut declared_paths = relative_paths.to_vec();
        if let Some(program) = self.board_config.session_program.as_ref() {
            declared_paths.push(program.path.clone());
        }
        self.session_files = collect_session_files(root, &declared_paths)?;
        Ok(self)
    }

    /// Resolves every file declared by the board configuration under `root`.
    ///
    /// This includes both `session_files` and `session_program.path`.
    pub fn with_session_root(mut self, root: &Path) -> anyhow::Result<Self> {
        let declared_files = self.board_config.session_files.clone();
        self = self.with_session_files(root, &declared_files)?;
        Ok(self)
    }

    pub(crate) fn into_parts(self) -> (BoardRunConfig, RunBoardOptions, Vec<SessionFileUpload>) {
        (self.board_config, self.options, self.session_files)
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use tempfile::tempdir;

    use super::*;
    use crate::board::config::BoardSessionProgram;

    #[test]
    fn session_root_collects_declared_files_and_program() {
        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join("bin")).unwrap();
        fs::write(root.path().join("config.toml"), b"config").unwrap();
        fs::write(root.path().join("bin/probe"), b"probe").unwrap();
        let config = BoardRunConfig {
            board_type: "test-board".into(),
            session_files: vec![PathBuf::from("config.toml")],
            session_program: Some(BoardSessionProgram {
                path: PathBuf::from("bin/probe"),
                args: Vec::new(),
            }),
            ..Default::default()
        };

        let request = BoardRunRequest::new(config, RunBoardOptions::default())
            .with_session_root(root.path())
            .unwrap();
        let (_, _, uploads) = request.into_parts();
        let paths = uploads
            .iter()
            .map(SessionFileUpload::relative_path)
            .collect::<Vec<_>>();

        assert_eq!(paths, ["config.toml", "bin/probe"]);
    }

    #[test]
    fn session_root_rejects_duplicate_program_path() {
        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join("bin")).unwrap();
        fs::write(root.path().join("bin/probe"), b"probe").unwrap();
        let config = BoardRunConfig {
            board_type: "test-board".into(),
            session_files: vec![PathBuf::from("bin/probe")],
            session_program: Some(BoardSessionProgram {
                path: PathBuf::from("bin/probe"),
                args: Vec::new(),
            }),
            ..Default::default()
        };

        let error = BoardRunRequest::new(config, RunBoardOptions::default())
            .with_session_root(root.path())
            .unwrap_err();

        assert!(error.to_string().contains("duplicate"));
    }
}
