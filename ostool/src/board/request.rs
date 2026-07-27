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
        self.session_files = collect_session_files(root, relative_paths)?;
        Ok(self)
    }

    pub(crate) fn into_parts(self) -> (BoardRunConfig, RunBoardOptions, Vec<SessionFileUpload>) {
        (self.board_config, self.options, self.session_files)
    }
}
