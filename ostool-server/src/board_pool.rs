use std::collections::{BTreeMap, BTreeSet};

use crate::{config::BoardConfig, session::Session};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardAllocationStatus {
    BoardTypeNotFound,
    NoAvailableBoard,
}

pub fn allocate_board(
    boards: &BTreeMap<String, BoardConfig>,
    sessions: &BTreeMap<String, Session>,
    board_type: &str,
    required_tags: &[String],
) -> Result<BoardConfig, BoardAllocationStatus> {
    let leased_boards = sessions
        .values()
        .map(|session| session.board_id.as_str())
        .collect::<BTreeSet<_>>();

    let matching_boards = boards
        .values()
        .filter(|board| !board.disabled)
        .filter(|board| board.board_type == board_type)
        .filter(|board| required_tags.iter().all(|tag| board.tags.contains(tag)))
        .collect::<Vec<_>>();

    if matching_boards.is_empty() {
        let board_type_exists = boards
            .values()
            .any(|board| !board.disabled && board.board_type == board_type);
        return Err(if board_type_exists {
            BoardAllocationStatus::NoAvailableBoard
        } else {
            BoardAllocationStatus::BoardTypeNotFound
        });
    }

    matching_boards
        .into_iter()
        .find(|board| !leased_boards.contains(board.id.as_str()))
        .cloned()
        .ok_or(BoardAllocationStatus::NoAvailableBoard)
}
