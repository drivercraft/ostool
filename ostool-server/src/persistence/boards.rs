//! PostgreSQL development-board repository.
use super::postgres::Database;
use crate::config::BoardConfig;
use anyhow::bail;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug)]
pub struct BoardStore {
    database: Database,
}
impl BoardStore {
    pub fn new(database: Database) -> Self {
        Self { database }
    }
    pub async fn load_all(&self) -> anyhow::Result<BTreeMap<String, BoardConfig>> {
        let database = &self.database;
        use sqlx::Row;
        let mut boards = BTreeMap::new();
        let mut macs = BTreeSet::new();
        for row in sqlx::query("SELECT id,payload FROM board_records ORDER BY id")
            .fetch_all(database.pool())
            .await?
        {
            let id: String = row.try_get("id")?;
            let board: BoardConfig = serde_json::from_value(row.try_get("payload")?)?;
            board.validate()?;
            if board.id != id {
                bail!("board database id mismatch: {id}");
            }
            if let Some(identity) = &board.network_identity
                && !macs.insert(identity.mac_address)
            {
                bail!("duplicate board MAC in PostgreSQL");
            }
            boards.insert(id, board);
        }
        Ok(boards)
    }

    pub async fn write_board(&self, board: &BoardConfig) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO board_records (id,payload) VALUES ($1,$2) ON CONFLICT (id) DO UPDATE SET payload=excluded.payload,updated_at=now()")
            .bind(&board.id)
            .bind(serde_json::to_value(board)?)
            .execute(self.database.pool())
            .await?;
        Ok(())
    }
    /// Preserve the public document when an administrator renames a board.
    pub async fn replace_board(
        &self,
        previous_id: &str,
        board: &BoardConfig,
    ) -> anyhow::Result<()> {
        let mut tx = self.database.begin().await?;
        sqlx::query("INSERT INTO board_records(id,payload) VALUES($1,$2) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload,updated_at=now()")
            .bind(&board.id).bind(serde_json::to_value(board)?).execute(&mut *tx).await?;
        if previous_id != board.id {
            sqlx::query("UPDATE device_documents SET board_id=$2,revision=revision+1,updated_at=now() WHERE board_id=$1").bind(previous_id).bind(&board.id).execute(&mut *tx).await?;
            sqlx::query("DELETE FROM board_records WHERE id=$1")
                .bind(previous_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn delete_board(&self, id: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM board_records WHERE id=$1")
            .bind(id)
            .execute(self.database.pool())
            .await?;
        Ok(())
    }
}
