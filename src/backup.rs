use crate::{
    config::{Config, Dropbox},
    db,
    error::{invalid, Error, Result},
    models::Manifest,
};
use fs2::FileExt;
use rusqlite::Connection;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const DROPBOX_RETENTION: usize = 5;

pub struct Lock(File);
impl Lock {
    pub fn acquire(path: &Path, shared: bool) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .mode(0o600)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        let result = if shared {
            FileExt::try_lock_shared(&file)
        } else {
            FileExt::try_lock_exclusive(&file)
        };
        result.map_err(|_| Error::new(409, "job_in_progress"))?;
        Ok(Self(file))
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}
pub fn status(c: &Connection, config: &Config) -> Result<Value> {
    let dropbox = db::op(c, "dropbox_revision")?.and_then(|s| s.parse::<i64>().ok());
    let err = db::op(c, "last_error")?.and_then(|s| serde_json::from_str::<Value>(&s).ok());
    Ok(
        json!({"current_revision":db::revision(c)?,"local_revision":db::op(c,"snapshot_revision")?.and_then(|s|s.parse::<i64>().ok()),"dropbox_revision":if config.backup.dropbox.is_some(){dropbox}else{None},"dropbox_configured":config.backup.dropbox.is_some(),"pending":c.query_row("SELECT COUNT(*) FROM backup_outbox WHERE state='pending'",[],|r|r.get::<_,i64>(0))?,"last_error":err}),
    )
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let stage = path.with_extension(format!("stage-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .mode(0o600)
            .write(true)
            .open(&stage)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&stage, path)?;
        sync_dir(path.parent().unwrap_or(Path::new(".")))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&stage);
    }
    result
}
fn sync_dir(dir: &Path) -> Result<()> {
    File::open(dir)?.sync_all()?;
    Ok(())
}
pub fn snapshot(path: &Path, config: &Config) -> Result<(PathBuf, PathBuf, Manifest)> {
    let dir = config.directory(path);
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)?;
    let stage = dir.join(format!("stage-{}.sqlite", uuid::Uuid::new_v4()));
    let src = db::open(path)?;
    let mut dest = Connection::open(&stage)?;
    fs::set_permissions(&stage, fs::Permissions::from_mode(0o600))?;
    if let Ok(value) = std::env::var("POSSERVER_TEST_SNAPSHOT_MAX_PAGES") {
        dest.pragma_update(
            None,
            "max_page_count",
            value.parse::<u32>().map_err(|_| invalid())?,
        )?;
    }
    let result = (|| -> Result<Manifest> {
        let backup = rusqlite::backup::Backup::new(&src, &mut dest)?;
        let start = Instant::now();
        loop {
            use rusqlite::backup::StepResult;
            match backup.step(256)? {
                StepResult::Done => break,
                StepResult::More | StepResult::Busy | StepResult::Locked => {
                    if start.elapsed() > Duration::from_secs(5) {
                        return Err(Error::new(503, "snapshot_busy"));
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                _ => return Err(Error::new(503, "snapshot_failed")),
            }
        }
        drop(backup);
        let integrity: String = dest.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if integrity != "ok" {
            return Err(Error::new(503, "snapshot_corrupt"));
        }
        db::check_schema(&dest)?;
        let revision = db::revision(&dest)?;
        let schema_version = dest.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        File::open(&stage)?.sync_all()?;
        Ok(Manifest {
            revision,
            schema_version,
            sha256: hash_file(&stage)?,
            created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            snapshot_path: Some(format!(
                "{}/revisions/{revision}.sqlite",
                config
                    .backup
                    .dropbox
                    .as_ref()
                    .map(|d| d.root.as_str())
                    .unwrap_or("/posserver")
            )),
        })
    })();
    drop(dest);
    let mut manifest = match result {
        Ok(m) => m,
        Err(e) => {
            let _ = fs::remove_file(&stage);
            return Err(e);
        }
    };
    let dbfile = dir.join(format!("{}.sqlite", manifest.revision));
    let mf = dir.join(format!("{}.json", manifest.revision));
    if dbfile.exists() && mf.exists() {
        let existing: Manifest = serde_json::from_slice(&fs::read(&mf)?)
            .map_err(|_| Error::new(503, "snapshot_corrupt"))?;
        if existing.revision != manifest.revision || existing.sha256 != hash_file(&dbfile)? {
            return Err(Error::new(503, "snapshot_corrupt"));
        }
        let prior =
            Connection::open_with_flags(&dbfile, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let candidate =
            Connection::open_with_flags(&stage, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        if db::op(&prior, "database_id")? != db::op(&candidate, "database_id")? {
            return Err(Error::new(503, "snapshot_lineage_conflict"));
        }
        drop((prior, candidate));
        fs::remove_file(&stage)?;
        manifest = existing;
    } else {
        fs::rename(&stage, &dbfile)?;
        sync_dir(&dir)?;
        atomic_write(&mf, &serde_json::to_vec(&manifest).unwrap())?;
    }
    let mut c = db::open(path)?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    db::set_op(&tx, "snapshot_revision", manifest.revision)?;
    db::set_op(&tx, "snapshot_bytes", fs::metadata(&dbfile)?.len())?;
    tx.execute("UPDATE backup_outbox SET snapshot_path=?1,sha256=?2 WHERE revision<=?3 AND state='pending'",rusqlite::params![dbfile.to_string_lossy(),manifest.sha256,manifest.revision])?;
    tx.commit()?;
    Ok((dbfile, mf, manifest))
}
fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| Error::new(503, "backup_unavailable"))
}
fn refresh(client: &reqwest::blocking::Client, d: &Dropbox) -> Result<String> {
    let refresh = d
        .refresh_token
        .as_deref()
        .ok_or(Error::new(503, "dropbox_token_rejected"))?;
    let key = d.app_key.as_deref().ok_or_else(invalid)?;
    let secret = d.app_secret.as_deref().ok_or_else(invalid)?;
    let res = client
        .post(format!(
            "{}/oauth2/token",
            d.api_base_url.trim_end_matches('/')
        ))
        .basic_auth(key, Some(secret))
        .form(&[("grant_type", "refresh_token"), ("refresh_token", refresh)])
        .send()
        .map_err(|_| Error::new(503, "dropbox_refresh_failed"))?;
    if !res.status().is_success() {
        return Err(Error::new(503, "dropbox_refresh_failed"));
    }
    let v: Value = res
        .json()
        .map_err(|_| Error::new(503, "dropbox_refresh_failed"))?;
    v["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or(Error::new(503, "dropbox_refresh_failed"))
}
fn retry_count(path: &Path, provider: &str) -> Result<()> {
    let mut c = db::open(path)?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let key = format!("{provider}_retries");
    let n = db::op_number(&tx, &key)?;
    db::set_op(&tx, &key, n + 1)?;
    tx.commit()?;
    Ok(())
}
fn upload(
    client: &reqwest::blocking::Client,
    path: &Path,
    d: &Dropbox,
    token: &mut String,
    refreshed: &mut bool,
    remote: &str,
    file: &Path,
) -> Result<()> {
    let size = fs::metadata(file)?.len();
    if size > 150 * 1024 * 1024 {
        return Err(Error::new(503, "dropbox_upload_too_large"));
    }
    let arg = json!({"path":remote,"mode":"overwrite","autorename":false,"mute":true}).to_string();
    let mut attempts = 0;
    loop {
        let response = client
            .post(format!(
                "{}/2/files/upload",
                d.content_base_url.trim_end_matches('/')
            ))
            .bearer_auth(token.as_str())
            .header("Content-Type", "application/octet-stream")
            .header("Dropbox-API-Arg", &arg)
            .body(reqwest::blocking::Body::sized(File::open(file)?, size))
            .send();
        match response {
            Ok(r) if r.status().is_success() => return Ok(()),
            Ok(r) if r.status().as_u16() == 401 && !*refreshed => {
                *refreshed = true;
                *token = refresh(client, d)?;
                retry_count(path, "dropbox")?;
                continue;
            }
            Ok(r) if r.status().as_u16() == 401 || r.status().as_u16() == 403 => {
                return Err(Error::new(503, "dropbox_token_rejected"))
            }
            Ok(r) if r.status().is_server_error() || r.status().as_u16() == 429 => {
                if attempts >= 2 {
                    return Err(Error::new(503, "backup_unavailable"));
                }
                let delay = r
                    .headers()
                    .get("Retry-After")
                    .and_then(|s| s.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0)
                    .min(1);
                std::thread::sleep(Duration::from_millis((100 << attempts).max(delay * 1000)));
            }
            Ok(_) => return Err(Error::new(503, "backup_upload_rejected")),
            Err(_) => {
                if attempts >= 2 {
                    return Err(Error::new(503, "backup_unavailable"));
                }
                std::thread::sleep(Duration::from_millis(100 << attempts));
            }
        }
        attempts += 1;
        retry_count(path, "dropbox")?;
    }
}
pub fn run(
    path: &Path,
    config: &Config,
    provider: &str,
    metrics: Option<&crate::metrics::Metrics>,
) -> Result<Value> {
    let resolved = db::resolve(path)?;
    let path = resolved.as_path();
    if provider != "dropbox" || config.backup.dropbox.is_none() {
        return Err(Error::new(409, "backup_not_configured"));
    }
    let _lock = Lock::acquire(&path.with_extension("backup.lock"), false)?;
    let c = db::open(path)?;
    let pending: i64 = c.query_row(
        "SELECT COUNT(*) FROM backup_outbox WHERE state='pending'",
        [],
        |r| r.get(0),
    )?;
    let retained: Vec<i64> = db::op(&c, "dropbox_retained")?
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default();
    if provider == "dropbox"
        && pending == 0
        && retained.len() <= DROPBOX_RETENTION
        && db::op(&c, "dropbox_revision")?.is_some()
    {
        return Ok(json!({"revision":db::op_number(&c,"dropbox_revision")?,"skipped":true}));
    }
    let interrupted = db::op_number(&c, &format!("{provider}_running"))? > 0;
    let prev_failed = interrupted
        || (db::op_number(&c, &format!("{provider}_attempt_at"))? > 0
            && db::op_number(&c, &format!("{provider}_attempt_success"))? == 0);

    drop(c);
    let start = Instant::now();
    let mut revision = None;
    let result = (|| -> Result<Value> {
        let started = Instant::now();
        let snap = snapshot(path, config);
        if let Some(metrics) = metrics {
            metrics
                .db_duration
                .with_label_values(&["snapshot"])
                .observe(started.elapsed().as_secs_f64());
            if let Err(error) = &snap {
                if let Some(reason) = error.db_reason {
                    metrics.counters.with_label_values(&[reason]).inc();
                }
            }
        }
        let (file, mf, m) = snap?;
        if prev_failed {
            retry_count(path, provider)?;
        }
        db::set_op(
            &db::open(path)?,
            &format!("{provider}_running"),
            chrono::Utc::now().timestamp(),
        )?;
        crate::fault("after_snapshot");
        revision = Some(m.revision);
        let confirmed = db::op_number(&db::open(path)?, &format!("{provider}_revision"))?;
        if m.revision < confirmed {
            return Err(Error::new(409, "stale_snapshot"));
        }
        match provider {
            "dropbox" => {
                let d = config.backup.dropbox.as_ref().unwrap();
                let client = client()?;
                let mut token = match &d.access_token {
                    Some(t) => t.clone(),
                    None => refresh(&client, d)?,
                };
                let mut refreshed = d.access_token.is_none();
                upload(
                    &client,
                    path,
                    d,
                    &mut token,
                    &mut refreshed,
                    &format!(
                        "{}/revisions/{}.sqlite",
                        d.root.trim_end_matches('/'),
                        m.revision
                    ),
                    &file,
                )?;
                upload(
                    &client,
                    path,
                    d,
                    &mut token,
                    &mut refreshed,
                    &format!("{}/manifest.json", d.root.trim_end_matches('/')),
                    &mf,
                )?;
            }
            _ => return Err(invalid()),
        }
        crate::fault("after_upload");
        Ok(json!({"revision":m.revision}))
    })();
    let mut c = db::open(path)?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let now = chrono::Utc::now().timestamp();
    db::set_op(&tx, &format!("{provider}_running"), 0)?;
    let success = result.is_ok();
    let counter = format!(
        "{provider}_{}_total",
        if success { "success" } else { "failure" }
    );
    let n = db::op_number(&tx, &counter)?;
    db::set_op(&tx, &counter, n + 1)?;
    db::set_op(&tx, &format!("{provider}_attempt_at"), now)?;
    db::set_op(
        &tx,
        &format!("{provider}_attempt_success"),
        if success { 1 } else { 0 },
    )?;
    db::set_op(
        &tx,
        &format!("{provider}_duration"),
        start.elapsed().as_secs_f64(),
    )?;
    if success {
        let r = revision.unwrap();
        db::set_op(&tx, &format!("{provider}_revision"), r)?;
        db::set_op(&tx, &format!("{provider}_success_at"), now)?;
        if provider == "dropbox" {
            let mut retained: Vec<i64> = db::op(&tx, "dropbox_retained")?
                .and_then(|v| serde_json::from_str(&v).ok())
                .unwrap_or_default();
            if !retained.contains(&r) {
                retained.push(r);
                retained.sort_unstable_by(|a, b| b.cmp(a));
            }
            db::set_op(
                &tx,
                "dropbox_retained",
                serde_json::to_string(&retained).unwrap(),
            )?;
            tx.execute(
                "UPDATE backup_outbox SET state='delivered',error_code=NULL WHERE revision<=?1",
                [r],
            )?;
        }
        if db::op(&tx, "last_error")?.is_some_and(|e| e.contains(provider)) {
            tx.execute("DELETE FROM operational WHERE key='last_error'", [])?;
        }
    } else {
        let code = result.as_ref().unwrap_err().code;
        db::set_op(&tx, "last_error", json!({"provider":provider,"code":code}))?;
        if provider == "dropbox" {
            tx.execute(
                "UPDATE backup_outbox SET attempts=attempts+1,error_code=?1 WHERE state='pending'",
                [code],
            )?;
        }
    }
    tx.commit()?;
    if success {
        if let Err(e) = prune_local(path, config) {
            db::set_op(&c, "last_error", json!({"provider":"local","code":e.code}))?;
        }
        if provider == "dropbox" && prune_remote(path, config).is_err() {
            db::set_op(
                &c,
                "last_error",
                json!({"provider":"dropbox","code":"retention_failed"}),
            )?;
        }
    }
    result
}
fn prune_local(path: &Path, config: &Config) -> Result<()> {
    let c = db::open(path)?;
    let mut revisions = Vec::new();
    for item in fs::read_dir(config.directory(path))? {
        let p = item?.path();
        if p.extension().is_some_and(|s| s == "sqlite") {
            if let Some(r) = p
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<i64>().ok())
            {
                revisions.push((r, p));
            }
        }
    }
    revisions.sort_by_key(|p| std::cmp::Reverse(p.0));
    for (r, p) in revisions.into_iter().skip(2) {
        let pending: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM backup_outbox WHERE state='pending' AND snapshot_path=?1)",
            [p.to_string_lossy().as_ref()],
            |r| r.get(0),
        )?;
        if !pending {
            fs::remove_file(&p)?;
            let _ = fs::remove_file(config.directory(path).join(format!("{r}.json")));
        }
    }
    Ok(())
}
pub fn restore(dest: &Path, source: &Path, mf: &Path) -> Result<Value> {
    let resolved = db::resolve(dest)?;
    let dest = resolved.as_path();
    let _lock = Lock::acquire(&dest.with_extension("server.lock"), false)
        .map_err(|_| Error::new(409, "server_running"))?;
    let manifest: Manifest = serde_json::from_slice(&fs::read(mf)?).map_err(|_| invalid())?;
    if manifest.schema_version != 1
        || manifest.sha256.len() != 64
        || manifest.sha256 != manifest.sha256.to_lowercase()
    {
        return Err(Error::new(422, "invalid_manifest"));
    }
    let bytes = fs::read(source)?;
    if format!("{:x}", Sha256::digest(&bytes)) != manifest.sha256 {
        return Err(Error::new(422, "snapshot_hash_mismatch"));
    }
    let c = Connection::open_with_flags(source, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    if c.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))? != "ok"
        || c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? != 1
        || db::revision(&c)? != manifest.revision
    {
        return Err(Error::new(422, "invalid_snapshot"));
    }
    db::check_schema(&c).map_err(|_| Error::new(422, "invalid_snapshot"))?;
    {
        use rusqlite::OptionalExtension;
        if c.query_row("PRAGMA foreign_key_check", [], |r| r.get::<_, String>(0))
            .optional()?
            .is_some()
        {
            return Err(Error::new(422, "invalid_snapshot"));
        }
    }
    drop(c);
    if source == dest {
        return Err(invalid());
    }
    if dest.exists() {
        let rollback = dest.with_extension(format!("rollback-{}.sqlite", uuid::Uuid::new_v4()));
        fs::copy(dest, &rollback)?;
        File::open(&rollback)?.sync_all()?;
    }
    atomic_write(dest, &bytes)?;
    for suffix in ["-wal", "-shm"] {
        let p = PathBuf::from(format!("{}{suffix}", dest.display()));
        if p.exists() {
            fs::remove_file(p)?;
        }
    }
    Ok(json!({"restored":true,"revision":manifest.revision}))
}

pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn prune_remote(path: &Path, config: &Config) -> Result<()> {
    let c = db::open(path)?;
    let mut retained: Vec<i64> = db::op(&c, "dropbox_retained")?
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default();
    if retained.len() <= DROPBOX_RETENTION {
        return Ok(());
    }
    let d = config.backup.dropbox.as_ref().unwrap();
    let client = client()?;
    let mut token = match &d.access_token {
        Some(t) => t.clone(),
        None => refresh(&client, d)?,
    };
    let mut refreshed = d.access_token.is_none();
    while retained.len() > DROPBOX_RETENTION {
        let oldest = *retained.last().unwrap();
        loop {
            let response=client.post(format!("{}/2/files/delete_v2",d.api_base_url.trim_end_matches('/'))).bearer_auth(&token).json(&json!({"path":format!("{}/revisions/{oldest}.sqlite",d.root.trim_end_matches('/'))})).send().map_err(|_|Error::new(503,"retention_failed"))?;
            if response.status().as_u16() == 401 && !refreshed {
                refreshed = true;
                token = refresh(&client, d)?;
                continue;
            }
            if !response.status().is_success() {
                // Dropbox's DeleteError path_lookup/not_found means a previous attempt
                // already removed the object. Treat that specific condition as idempotent.
                let not_found = if response.status().as_u16() == 409 {
                    let mut body = Vec::new();
                    response.take(65537).read_to_end(&mut body)?;
                    let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                    body.len() <= 65536
                        && v["error"][".tag"] == "path_lookup"
                        && (v["error"]["path_lookup"][".tag"] == "not_found"
                            || v["error"]["path_lookup"] == "not_found")
                } else {
                    false
                };
                if !not_found {
                    return Err(Error::new(503, "retention_failed"));
                }
            }
            break;
        }
        retained.pop();
        db::set_op(
            &c,
            "dropbox_retained",
            serde_json::to_string(&retained).unwrap(),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oversized_single_upload_fails_before_reading_or_contacting_provider() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("large.sqlite");
        File::create(&file)
            .unwrap()
            .set_len(150 * 1024 * 1024 + 1)
            .unwrap();
        let client = client().unwrap();
        let dropbox = Dropbox::default();
        let mut token = "synthetic".to_owned();
        let error = upload(
            &client,
            &dir.path().join("db.sqlite"),
            &dropbox,
            &mut token,
            &mut false,
            "/unused",
            &file,
        )
        .unwrap_err();
        assert_eq!(error.code, "dropbox_upload_too_large");
    }
}

/// Credential input stays off process arguments and stdout. This configures OAuth,
/// but does not claim that a remote snapshot has been verified.
pub fn setup_dropbox(path: &Path) -> Result<Value> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        app_key: String,
        app_secret: String,
        authorization_code: String,
        redirect_uri: Option<String>,
    }
    let mut bytes = Vec::new();
    std::io::stdin().take(65537).read_to_end(&mut bytes)?;
    if bytes.len() > 65536 {
        return Err(invalid());
    }
    let input: Input = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if [&input.app_key, &input.app_secret, &input.authorization_code]
        .iter()
        .any(|s| s.trim().is_empty())
    {
        return Err(invalid());
    }
    let mut config = if path.exists() {
        Config::load(Some(path))?
    } else {
        Config::default()
    };
    let mut dropbox = config.backup.dropbox.clone().unwrap_or_default();
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("code", input.authorization_code.as_str()),
    ];
    if let Some(uri) = &input.redirect_uri {
        form.push(("redirect_uri", uri));
    }
    let response = client()?
        .post(format!(
            "{}/oauth2/token",
            dropbox.api_base_url.trim_end_matches('/')
        ))
        .basic_auth(&input.app_key, Some(&input.app_secret))
        .form(&form)
        .send()
        .map_err(|_| Error::new(503, "dropbox_setup_failed"))?;
    if !response.status().is_success() {
        return Err(Error::new(422, "dropbox_setup_rejected"));
    }
    let response: Value = response
        .json()
        .map_err(|_| Error::new(502, "dropbox_invalid_response"))?;
    let refresh = response["refresh_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or(Error::new(422, "dropbox_offline_access_required"))?;
    dropbox.refresh_token = Some(refresh.into());
    dropbox.access_token = None;
    dropbox.app_key = Some(input.app_key);
    dropbox.app_secret = Some(input.app_secret);
    config.backup.dropbox = Some(dropbox);
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }
    atomic_write(path, &serde_json::to_vec_pretty(&config).unwrap())?;
    Ok(json!({"configured":true,"provider":"dropbox","verified":false}))
}
