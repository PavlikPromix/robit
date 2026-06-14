use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection};
use uuid::Uuid;

use super::models::{ItemKind, MoveStrategy, OperationSnapshot, OperationStatus};

pub fn app_data_dir() -> Result<PathBuf> {
    let base = dirs::data_local_dir().context("LOCALAPPDATA is not available")?;
    let path = base.join("RobitLinkMover");
    fs::create_dir_all(path.join("logs"))?;
    fs::create_dir_all(path.join("requests"))?;
    fs::create_dir_all(path.join("cancellations"))?;
    Ok(path)
}

pub fn db_path() -> Result<PathBuf> {
    Ok(app_data_dir()?.join("operations.sqlite"))
}

pub fn requests_dir() -> Result<PathBuf> {
    Ok(app_data_dir()?.join("requests"))
}

pub fn cancellations_dir() -> Result<PathBuf> {
    Ok(app_data_dir()?.join("cancellations"))
}

pub fn new_log_path(id: &str) -> Result<PathBuf> {
    Ok(app_data_dir()?.join("logs").join(format!("{id}.log")))
}

pub fn init_db() -> Result<()> {
    let conn = Connection::open(db_path()?)?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS operations (
            id TEXT PRIMARY KEY,
            source_path TEXT NOT NULL,
            destination_parent TEXT NOT NULL,
            destination_path TEXT NOT NULL,
            item_kind TEXT NOT NULL,
            strategy TEXT NOT NULL,
            status TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            log_path TEXT NOT NULL,
            error_message TEXT,
            progress_current INTEGER,
            progress_total INTEGER,
            progress_label TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_operations_created_at ON operations(created_at DESC);
        "#,
    )?;
    ensure_column(&conn, "destination_parent", "TEXT")?;
    ensure_column(&conn, "progress_current", "INTEGER")?;
    ensure_column(&conn, "progress_total", "INTEGER")?;
    ensure_column(&conn, "progress_label", "TEXT")?;
    Ok(())
}

pub fn create_operation(
    source_path: String,
    destination_parent: String,
    destination_path: String,
    item_kind: ItemKind,
    strategy: MoveStrategy,
) -> Result<OperationSnapshot> {
    init_db()?;
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let log_path = new_log_path(&id)?;
    fs::write(&log_path, "")?;

    let op = OperationSnapshot {
        id,
        source_path,
        destination_parent,
        destination_path,
        item_kind,
        strategy,
        status: OperationStatus::Planned,
        created_at: now.clone(),
        updated_at: now,
        log_path: log_path.to_string_lossy().to_string(),
        error_message: None,
        progress_current: Some(0),
        progress_total: Some(4),
        progress_label: Some("Ожидание запуска".to_string()),
    };

    let conn = Connection::open(db_path()?)?;
    conn.execute(
        r#"
        INSERT INTO operations
        (id, source_path, destination_parent, destination_path, item_kind, strategy, status, created_at, updated_at, log_path, error_message,
         progress_current, progress_total, progress_label)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
        "#,
        params![
            op.id,
            op.source_path,
            op.destination_parent,
            op.destination_path,
            serde_json::to_string(&op.item_kind)?,
            serde_json::to_string(&op.strategy)?,
            op.status.as_str(),
            op.created_at,
            op.updated_at,
            op.log_path,
            op.error_message,
            op.progress_current,
            op.progress_total,
            op.progress_label,
        ],
    )?;
    Ok(op)
}

pub fn list_operations() -> Result<Vec<OperationSnapshot>> {
    init_db()?;
    let conn = Connection::open(db_path()?)?;
    let mut stmt = conn.prepare(
        r#"
        SELECT id, source_path, destination_parent, destination_path, item_kind, strategy, status,
               created_at, updated_at, log_path, error_message,
               progress_current, progress_total, progress_label
        FROM operations
        ORDER BY created_at DESC
        LIMIT 100
        "#,
    )?;
    let rows = stmt.query_map([], row_to_operation)?;
    let mut items = Vec::new();
    for row in rows {
        items.push(row?);
    }
    Ok(items)
}

pub fn get_operation(id: &str) -> Result<OperationSnapshot> {
    init_db()?;
    let conn = Connection::open(db_path()?)?;
    conn.query_row(
        r#"
        SELECT id, source_path, destination_parent, destination_path, item_kind, strategy, status,
               created_at, updated_at, log_path, error_message,
               progress_current, progress_total, progress_label
        FROM operations
        WHERE id = ?1
        "#,
        [id],
        row_to_operation,
    )
    .with_context(|| format!("operation not found: {id}"))
}

pub fn update_status(id: &str, status: OperationStatus, error_message: Option<&str>) -> Result<()> {
    init_db()?;
    let now = Utc::now().to_rfc3339();
    let conn = Connection::open(db_path()?)?;
    conn.execute(
        r#"
        UPDATE operations
        SET status = ?2, updated_at = ?3, error_message = ?4
        WHERE id = ?1
        "#,
        params![id, status.as_str(), now, error_message],
    )?;
    Ok(())
}

pub fn update_progress(id: &str, current: u64, total: u64, label: impl AsRef<str>) -> Result<()> {
    init_db()?;
    let now = Utc::now().to_rfc3339();
    let conn = Connection::open(db_path()?)?;
    conn.execute(
        r#"
        UPDATE operations
        SET updated_at = ?2, progress_current = ?3, progress_total = ?4, progress_label = ?5
        WHERE id = ?1
        "#,
        params![id, now, current, total, label.as_ref()],
    )?;
    Ok(())
}

pub fn request_file_path(id: &str) -> Result<PathBuf> {
    Ok(requests_dir()?.join(format!("{id}.json")))
}

pub fn cancel_file_path(id: &str) -> Result<PathBuf> {
    Ok(cancellations_dir()?.join(format!("{id}.cancel")))
}

pub fn write_json_file<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    let data = serde_json::to_vec_pretty(value)?;
    fs::write(path, data)?;
    Ok(())
}

fn row_to_operation(row: &rusqlite::Row<'_>) -> rusqlite::Result<OperationSnapshot> {
    let destination_parent: Option<String> = row.get(2)?;
    let destination_path: String = row.get(3)?;
    let item_kind_json: String = row.get(4)?;
    let strategy_json: String = row.get(5)?;
    let status_text: String = row.get(6)?;
    let item_kind = serde_json::from_str(&item_kind_json).map_err(json_error)?;
    let strategy = serde_json::from_str(&strategy_json).map_err(json_error)?;
    let status = OperationStatus::try_from(status_text.as_str()).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            5,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
        )
    })?;
    Ok(OperationSnapshot {
        id: row.get(0)?,
        source_path: row.get(1)?,
        destination_parent: destination_parent
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| fallback_destination_parent(&destination_path)),
        destination_path,
        item_kind,
        strategy,
        status,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
        log_path: row.get(9)?,
        error_message: row.get(10)?,
        progress_current: row.get(11)?,
        progress_total: row.get(12)?,
        progress_label: row.get(13)?,
    })
}

fn fallback_destination_parent(destination_path: &str) -> String {
    Path::new(destination_path)
        .parent()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn ensure_column(conn: &Connection, name: &str, definition: &str) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(operations)")?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for column in columns {
        if column? == name {
            return Ok(());
        }
    }
    conn.execute(
        &format!("ALTER TABLE operations ADD COLUMN {name} {definition}"),
        [],
    )?;
    Ok(())
}

fn json_error(error: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}
