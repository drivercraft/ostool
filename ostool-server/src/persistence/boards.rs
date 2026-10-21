//! Development-board repository. PostgreSQL is authoritative in production.
pub use super::legacy::boards::QuarantinedBoard;
use super::{legacy::boards::BoardFiles, postgres::Database};
use crate::config::BoardConfig;
use anyhow::bail;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
#[derive(Debug)]
pub struct BoardStore {
    files: BoardFiles,
    database: Option<Database>,
}
impl BoardStore {
    /// Open PostgreSQL storage, importing legacy TOML once when necessary.
    pub async fn open_postgres(database: Database, legacy_dir: PathBuf) -> anyhow::Result<Self> {
        let store = Self::file_fixture(legacy_dir);
        store.ensure_dir().await?;
        store.with_database(database).await
    }
    /// Explicit file adapter for isolated fixtures; never selected by edition.
    #[doc(hidden)]
    pub fn file_fixture(board_dir: PathBuf) -> Self {
        Self {
            files: BoardFiles::new(board_dir),
            database: None,
        }
    }
    /// Compatibility constructor for earlier library callers. Creates file fixtures.
    #[doc(hidden)]
    pub fn new(board_dir: PathBuf) -> Self {
        Self::file_fixture(board_dir)
    }
    /// Import legacy files once; thereafter PostgreSQL is the source of truth.
    pub async fn with_database(mut self, database: Database) -> anyhow::Result<Self> {
        use sqlx::Row;
        let mut tx = database.pool().begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(29992001)")
            .execute(&mut *tx)
            .await?;
        let done = sqlx::query("SELECT name FROM storage_imports WHERE name='boards'")
            .fetch_optional(&mut *tx)
            .await?;
        if done.is_none() {
            for board in self.files.load_all().await?.values() {
                sqlx::query("INSERT INTO board_records(id,payload) VALUES($1,$2) ON CONFLICT(id) DO NOTHING")
                    .bind(&board.id).bind(serde_json::to_value(board)?).execute(&mut *tx).await?;
            }
            sqlx::query("INSERT INTO storage_imports(name) VALUES('boards')")
                .execute(&mut *tx)
                .await?;
        }
        // Decode before commit so invalid database records fail startup clearly.
        for row in sqlx::query("SELECT payload FROM board_records")
            .fetch_all(&mut *tx)
            .await?
        {
            let board: BoardConfig = serde_json::from_value(row.try_get("payload")?)?;
            board.validate()?;
        }
        tx.commit().await?;
        self.database = Some(database);
        Ok(self)
    }

    /// Legacy source/diagnostic path, not the authoritative PostgreSQL location.
    pub fn path_for_id(&self, id: &str) -> PathBuf {
        self.files.path_for_id(id)
    }
    pub async fn ensure_dir(&self) -> anyhow::Result<()> {
        self.files.ensure_dir().await
    }
    pub async fn load_all(&self) -> anyhow::Result<BTreeMap<String, BoardConfig>> {
        if let Some(database) = &self.database {
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
            return Ok(boards);
        }
        self.files.load_all().await
    }
    pub async fn quarantined(&self) -> anyhow::Result<Vec<QuarantinedBoard>> {
        self.files.quarantined().await
    }
    pub async fn write_board(&self, board: &BoardConfig) -> anyhow::Result<()> {
        if let Some(database) = &self.database {
            return database
                .sync_board(&board.id, &serde_json::to_value(board)?)
                .await;
        }
        self.files.write_board(board).await
    }
    pub async fn delete_board(&self, id: &str) -> anyhow::Result<()> {
        if let Some(database) = &self.database {
            return database.delete_board(id).await;
        }
        self.files.delete_board(id).await
    }
}
