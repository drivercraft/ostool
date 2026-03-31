use std::collections::{BTreeMap, BTreeSet};

use crate::{config::BoardConfig, session::Session};

pub fn find_available_board(
    boards: &BTreeMap<String, BoardConfig>,
    sessions: &BTreeMap<String, Session>,
    board_type: &str,
    required_tags: &[String],
) -> Option<BoardConfig> {
    let leased_boards = sessions
        .values()
        .map(|session| session.board_id.as_str())
        .collect::<BTreeSet<_>>();

    boards
        .values()
        .filter(|board| !board.disabled)
        .filter(|board| board.board_type == board_type)
        .filter(|board| required_tags.iter().all(|tag| board.tags.contains(tag)))
        .find(|board| !leased_boards.contains(board.id.as_str()))
        .cloned()
}
