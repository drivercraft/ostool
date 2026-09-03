use std::collections::{BTreeMap, BTreeSet};

use crate::config::BoardConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardAllocationStatus {
    BoardTypeNotFound,
    BoardNotFound,
    BoardTypeMismatch {
        board_id: String,
        board_type: String,
        actual_board_type: String,
    },
    NoAvailableBoard,
}

pub fn allocate_board(
    boards: &BTreeMap<String, BoardConfig>,
    leased_boards: &BTreeSet<String>,
    board_type: &str,
    board_id: Option<&str>,
    required_tags: &[String],
) -> Result<BoardConfig, BoardAllocationStatus> {
    if let Some(board_id) = board_id {
        let board = boards
            .get(board_id)
            .ok_or(BoardAllocationStatus::BoardNotFound)?;
        if board.board_type != board_type {
            return Err(BoardAllocationStatus::BoardTypeMismatch {
                board_id: board_id.to_string(),
                board_type: board_type.to_string(),
                actual_board_type: board.board_type.clone(),
            });
        }
        if board.disabled
            || leased_boards.contains(&board.id)
            || !required_tags.iter().all(|tag| board.tags.contains(tag))
        {
            return Err(BoardAllocationStatus::NoAvailableBoard);
        }
        return Ok(board.clone());
    }

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
        .find(|board| !leased_boards.contains(&board.id))
        .cloned()
        .ok_or(BoardAllocationStatus::NoAvailableBoard)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use crate::{
        board_pool::{BoardAllocationStatus, allocate_board},
        config::{BoardConfig, BootConfig, CustomPowerManagement, PowerManagementConfig},
    };

    fn board(id: &str, board_type: &str) -> BoardConfig {
        BoardConfig {
            id: id.to_string(),
            board_type: board_type.to_string(),
            tags: Vec::new(),
            serial: None,
            power_management: PowerManagementConfig::Custom(CustomPowerManagement {
                power_on_cmd: "echo on".into(),
                power_off_cmd: "echo off".into(),
            }),
            boot: BootConfig::Uboot(Default::default()),
            notes: None,
            disabled: false,
        }
    }

    #[test]
    fn allocate_board_uses_requested_board_id() {
        let boards = BTreeMap::from([
            ("demo-01".to_string(), board("demo-01", "demo")),
            ("demo-02".to_string(), board("demo-02", "demo")),
        ]);

        let allocated =
            allocate_board(&boards, &BTreeSet::new(), "demo", Some("demo-02"), &[]).unwrap();

        assert_eq!(allocated.id, "demo-02");
    }

    #[test]
    fn allocate_board_rejects_busy_requested_board_id() {
        let boards = BTreeMap::from([("demo-01".to_string(), board("demo-01", "demo"))]);
        let leased_boards = BTreeSet::from(["demo-01".to_string()]);

        let err =
            allocate_board(&boards, &leased_boards, "demo", Some("demo-01"), &[]).unwrap_err();

        assert_eq!(err, BoardAllocationStatus::NoAvailableBoard);
    }

    #[test]
    fn allocate_board_rejects_mismatched_requested_board_type() {
        let boards = BTreeMap::from([("demo-01".to_string(), board("demo-01", "other"))]);

        let err =
            allocate_board(&boards, &BTreeSet::new(), "demo", Some("demo-01"), &[]).unwrap_err();

        assert_eq!(
            err,
            BoardAllocationStatus::BoardTypeMismatch {
                board_id: "demo-01".to_string(),
                board_type: "demo".to_string(),
                actual_board_type: "other".to_string(),
            }
        );
    }
}
