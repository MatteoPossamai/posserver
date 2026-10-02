use crate::{
    error::{Error, Result},
    models::{Transaction, User},
};
use rusqlite::{params, Connection, OptionalExtension};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::{path::Path, time::Duration};
pub fn open(path: &Path) -> Result<Connection> {
    let c = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    c.busy_timeout(Duration::from_millis(1500))?;
    c.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")?;
    if let Ok(value) = std::env::var("POSSERVER_TEST_DB_MAX_PAGES") {
        let pages = value.parse::<u32>().map_err(|_| crate::error::invalid())?;
        c.pragma_update(None, "max_page_count", pages)?;
    }
    Ok(c)
}
pub fn init(path: &Path) -> Result<()> {
    if let Some(p) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(p)?;
    }
    let c = Connection::open(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    c.busy_timeout(Duration::from_secs(2))?;
    let version: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version > 1 {
        return Err(Error::new(503, "future_schema"));
    }
    if version == 0 {
        c.execute_batch(include_str!("migrations/001.sql"))?;
    }
    check_schema(&c)?;
    if op(&c, "database_id")?.is_none() {
        set_op(&c, "database_id", uuid::Uuid::new_v4())?;
    }
    Ok(())
}
pub fn revision(c: &Connection) -> Result<i64> {
    Ok(c.query_row("SELECT value FROM revision", [], |r| r.get(0))?)
}
pub fn commit_revision(c: &Connection) -> Result<i64> {
    c.execute("UPDATE revision SET value=value+1", [])?;
    let r = revision(c)?;
    c.execute(
        "INSERT INTO backup_outbox(revision,created_at) VALUES(?1,?2)",
        params![r, chrono::Utc::now().timestamp()],
    )?;
    Ok(r)
}
pub fn user(c: &Connection, id: &str) -> Result<User> {
    c.query_row("SELECT id,name,language,timezone,reporting_currency,category_profile,version,rates,EXISTS(SELECT 1 FROM monzo_links WHERE user_id=users.id) FROM users WHERE id=?1",[id],|r| {
        let rates:String=r.get(7)?;
        Ok(User{id:r.get(0)?,name:r.get(1)?,language:r.get(2)?,timezone:r.get(3)?,reporting_currency:r.get(4)?,category_profile:r.get(5)?,version:r.get(6)?,rates:serde_json::from_str(&rates).map_err(|_|rusqlite::Error::InvalidQuery)?,monzo_linked:r.get(8)?})
    }).optional()?.ok_or(Error::new(404,"not_found"))
}
pub fn transaction_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Transaction> {
    let stamp: i64 = r.get(2)?;
    let date =
        chrono::DateTime::from_timestamp_micros(stamp).ok_or(rusqlite::Error::InvalidQuery)?;
    Ok(Transaction {
        id: r.get(0)?,
        user_id: r.get(1)?,
        occurred_at: date.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true),
        category_id: r.get(3)?,
        issuer: r.get(4)?,
        amount_minor: r.get(5)?,
        currency: r.get(6)?,
        source: r.get(7)?,
        version: r.get(8)?,
        monzo_transaction_id: r.get(9)?,
    })
}
pub const COLUMNS:&str="id,user_id,occurred_at,category_id,issuer,amount_minor,currency,source,version,monzo_transaction_id";
pub fn transaction(c: &Connection, u: &str, t: &str) -> Result<Transaction> {
    c.query_row(
        &format!(
            "SELECT {COLUMNS} FROM transactions WHERE user_id=?1 AND id=?2 AND deleted_at IS NULL"
        ),
        params![u, t],
        transaction_row,
    )
    .optional()?
    .ok_or(Error::new(404, "not_found"))
}
pub fn op(c: &Connection, key: &str) -> Result<Option<String>> {
    Ok(
        c.query_row("SELECT value FROM operational WHERE key=?1", [key], |r| {
            r.get(0)
        })
        .optional()?,
    )
}
pub fn set_op(c: &Connection, key: &str, value: impl ToString) -> Result<()> {
    c.execute(
        "INSERT INTO operational VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, value.to_string()],
    )?;
    Ok(())
}
pub fn op_number(c: &Connection, key: &str) -> Result<i64> {
    Ok(op(c, key)?.and_then(|s| s.parse().ok()).unwrap_or(0))
}

pub fn rows_range(
    c: &Connection,
    user: &str,
    start: i64,
    end: i64,
    category: Option<&str>,
) -> Result<Vec<Transaction>> {
    let mut stmt=c.prepare(&format!("SELECT {COLUMNS} FROM transactions WHERE user_id=?1 AND deleted_at IS NULL AND occurred_at>=?2 AND occurred_at<?3 AND (?4 IS NULL OR category_id=?4)"))?;
    let rows = stmt
        .query_map(params![user, start, end, category], transaction_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Canonical paths make process locks cover symlink and relative-path aliases.
pub fn resolve(path: &Path) -> Result<std::path::PathBuf> {
    if path.exists() {
        return Ok(path.canonicalize()?);
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(parent
        .canonicalize()?
        .join(path.file_name().ok_or_else(crate::error::invalid)?))
}

/// Validate the version and required columns without scanning finance data.
pub fn check_schema(c: &Connection) -> Result<()> {
    let version: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version != 1 {
        return Err(Error::new(503, "unsupported_schema"));
    }
    for query in [
        "SELECT id,name,language,timezone,reporting_currency,category_profile,version,rates FROM users LIMIT 0",
        "SELECT id,user_id,occurred_at,category_id,issuer,amount_minor,currency,source,version,deleted_at,monzo_account_id,monzo_transaction_id,legacy_batch_id,legacy_row_ordinal,original_category FROM transactions LIMIT 0",
        "SELECT user_id,account_id FROM monzo_links LIMIT 0",
        "SELECT user_id,account_id,last_transaction_id,last_created_at,seed_since FROM monzo_sync LIMIT 0",
        "SELECT id,user_id,source_sha256,row_count FROM legacy_batches LIMIT 0",
        "SELECT singleton,value FROM revision LIMIT 0",
        "SELECT revision,state,created_at,snapshot_path,sha256,attempts,error_code FROM backup_outbox LIMIT 0",
        "SELECT key,value FROM operational LIMIT 0",
    ] {c.prepare(query)?;}
    Ok(())
}
