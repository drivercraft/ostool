//! Offline, one-way board import. This tool is not part of the server runtime.
use anyhow::{Context, bail, ensure};
use clap::Parser;
use ostool_server::{
    BoardConfig, BootConfig, DatabaseConfig, environment,
    persistence::{Database, dtbs::normalize_dtb_name},
};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(
    version,
    about = "Import existing board TOML files and their referenced DTBs into PostgreSQL"
)]
struct Args {
    /// Target server configuration (optional; database settings can come from .env).
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    env_file: Option<PathBuf>,
    /// Directory containing the existing <board-id>.toml files.
    #[arg(long)]
    board_dir: PathBuf,
    /// Existing DTB directory, required when a source board references a DTB.
    #[arg(long)]
    dtb_dir: Option<PathBuf>,
    /// Validate source and database conflicts without writing business records.
    #[arg(long)]
    dry_run: bool,
}
#[derive(Default, Deserialize)]
struct TargetConfig {
    #[serde(default)]
    database: ostool_server::DatabaseConfig,
}
struct Input {
    boards: BTreeMap<String, BoardConfig>,
    dtbs: BTreeMap<String, Vec<u8>>,
}

async fn read_input(args: &Args) -> anyhow::Result<Input> {
    let mut directory = tokio::fs::read_dir(&args.board_dir)
        .await
        .with_context(|| format!("cannot read board directory {}", args.board_dir.display()))?;
    let mut paths = Vec::new();
    while let Some(entry) = directory.next_entry().await? {
        if entry.file_type().await?.is_file()
            && entry.path().extension().is_some_and(|ext| ext == "toml")
        {
            paths.push(entry.path());
        }
    }
    paths.sort();
    ensure!(
        !paths.is_empty(),
        "no board TOML files found in {}",
        args.board_dir.display()
    );
    let mut boards = BTreeMap::new();
    let mut macs = BTreeSet::new();
    let mut names = BTreeSet::new();
    for path in paths {
        let bytes = read_bounded(&path, 2 * 1024 * 1024).await?;
        let board: BoardConfig =
            toml::from_str(std::str::from_utf8(&bytes).context("board TOML must be UTF-8")?)
                .with_context(|| format!("invalid board file {}", path.display()))?;
        board
            .validate()
            .with_context(|| format!("invalid board {}", path.display()))?;
        ensure!(
            path.file_stem().and_then(|s| s.to_str()) == Some(board.id.as_str()),
            "board ID {} does not match filename {}",
            board.id,
            path.display()
        );
        if let Some(identity) = &board.network_identity {
            ensure!(
                macs.insert(identity.mac_address),
                "duplicate source MAC {}",
                identity.mac_address
            );
        }
        if let BootConfig::Uboot(profile) = &board.boot
            && let Some(name) = &profile.dtb_name
        {
            names.insert(normalize_dtb_name(name)?);
        }
        let id = board.id.clone();
        ensure!(
            boards.insert(id.clone(), board).is_none(),
            "duplicate board ID {id}"
        );
    }
    let mut dtbs = BTreeMap::new();
    for name in names {
        let root = args
            .dtb_dir
            .as_ref()
            .context("source boards reference DTBs; specify --dtb-dir")?;
        let content = read_bounded(&root.join(&name), 10 * 1024 * 1024).await?;
        ensure!(!content.is_empty(), "referenced DTB {name} is empty");
        dtbs.insert(name, content);
    }
    Ok(Input { boards, dtbs })
}
async fn read_bounded(path: &Path, limit: u64) -> anyhow::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("cannot read {}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).await?;
    ensure!(
        bytes.len() as u64 <= limit,
        "file exceeds size limit: {}",
        path.display()
    );
    Ok(bytes)
}
async fn run(args: Args) -> anyhow::Result<()> {
    // Complete source validation precedes database schema/business writes.
    let input = read_input(&args).await?;
    let mut database_config = DatabaseConfig::default();
    if let Some(path) = &args.config {
        let source = tokio::fs::read_to_string(path)
            .await
            .with_context(|| format!("cannot read target config {}", path.display()))?;
        database_config = toml::from_str::<TargetConfig>(&source)?.database;
    }
    environment::apply_database(&mut database_config, args.env_file.as_deref())?;
    if args.dry_run {
        database_config.migrate = false;
    }
    let database = Database::connect(&database_config).await?;
    if args.dry_run {
        let current_schema: String = sqlx::query_scalar("SELECT current_schema()")
            .fetch_one(database.pool())
            .await?;
        ensure!(
            database_config
                .schema
                .as_ref()
                .is_none_or(|schema| schema == &current_schema),
            "dry-run requires the target schema to be initialized by the new server"
        );
        let tables:i64=sqlx::query_scalar("SELECT count(*) FROM pg_tables WHERE schemaname=current_schema() AND tablename IN ('board_records','dtb_records')").fetch_one(database.pool()).await?;
        ensure!(
            tables == 2,
            "dry-run requires the target schema to be initialized by the new server"
        );
    }
    let mut tx = database.pool().begin().await?;
    // Serialize imports and inventory writes, including writes from a running service.
    sqlx::query("LOCK TABLE board_records,dtb_records IN SHARE ROW EXCLUSIVE MODE")
        .execute(&mut *tx)
        .await?;
    let mut existing = BTreeMap::new();
    let mut macs = BTreeMap::new();
    for row in sqlx::query("SELECT id,payload FROM board_records")
        .fetch_all(&mut *tx)
        .await?
    {
        let id: String = row.try_get("id")?;
        let board: BoardConfig = serde_json::from_value(row.try_get::<Value, _>("payload")?)?;
        board
            .validate()
            .with_context(|| format!("invalid database board {id}"))?;
        ensure!(board.id == id, "database board ID mismatch for {id}");
        if let Some(identity) = &board.network_identity
            && let Some(other) = macs.insert(identity.mac_address, id.clone())
        {
            bail!("database boards {other} and {id} share a MAC");
        }
        existing.insert(id, serde_json::to_value(board)?);
    }
    let mut insert_boards = Vec::new();
    let mut skipped_boards = 0;
    for (id, board) in &input.boards {
        let payload = serde_json::to_value(board)?;
        if let Some(value) = existing.get(id) {
            ensure!(
                value == &payload,
                "board {id} already exists with different contents; no records were imported"
            );
            skipped_boards += 1;
            continue;
        }
        if let Some(identity) = &board.network_identity
            && let Some(other) = macs.get(&identity.mac_address)
        {
            bail!(
                "board {id} MAC {} is already assigned to database board {other}",
                identity.mac_address
            );
        }
        insert_boards.push((id, payload));
    }
    let mut insert_dtbs = Vec::new();
    let mut skipped_dtbs = 0;
    for (name, content) in &input.dtbs {
        let stored: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT content FROM dtb_records WHERE name=$1")
                .bind(name)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some(stored) = stored {
            ensure!(
                &stored == content,
                "DTB {name} already exists with different contents; no records were imported"
            );
            skipped_dtbs += 1;
        } else {
            insert_dtbs.push((name, content));
        }
    }
    if args.dry_run {
        tx.rollback().await?;
        println!(
            "Validated: {} boards, {} referenced DTBs. Would import: {} boards, {} DTBs. Identical records skipped: {} boards, {} DTBs. No records written.",
            input.boards.len(),
            input.dtbs.len(),
            insert_boards.len(),
            insert_dtbs.len(),
            skipped_boards,
            skipped_dtbs
        );
        return Ok(());
    }
    for (name, content) in &insert_dtbs {
        sqlx::query("INSERT INTO dtb_records(name,size,sha256,content) VALUES ($1,$2,$3,$4)")
            .bind(name)
            .bind(content.len() as i64)
            .bind(format!("{:x}", Sha256::digest(content)))
            .bind(content)
            .execute(&mut *tx)
            .await?;
    }
    for (id, payload) in &insert_boards {
        sqlx::query("INSERT INTO board_records(id,payload) VALUES ($1,$2)")
            .bind(id)
            .bind(payload)
            .execute(&mut *tx)
            .await?;
    }
    if !insert_boards.is_empty() || !insert_dtbs.is_empty() {
        sqlx::query("INSERT INTO audit_events(id,request_id,finished_at,actor_name,actor_kind,source,action,summary,resource,result,details) VALUES($1,$1,now(),'迁移工具','system','cli','board.import','导入开发板及关联 DTB','boards','success',$2)")
            .bind(uuid::Uuid::new_v4())
            .bind(serde_json::json!({"boards":insert_boards.len(),"dtbs":insert_dtbs.len()}))
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    println!(
        "Imported: {} boards, {} referenced DTBs. Identical records skipped: {} boards, {} DTBs. Source files unchanged. Restart ostool-server to load the imported inventory.",
        insert_boards.len(),
        insert_dtbs.len(),
        skipped_boards,
        skipped_dtbs
    );
    Ok(())
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run(Args::parse()).await
}
