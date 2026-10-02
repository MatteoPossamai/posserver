use crate::{
    api::{fields, string},
    config::Config,
    db,
    error::{invalid, Error, Result},
    finance as f,
    models::*,
};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::Path,
    time::{Duration, Instant},
};

pub fn migrate(
    path: &Path,
    uid: &str,
    file: &Path,
    account: Option<&str>,
    dry: bool,
    legacy_decimals: bool,
) -> Result<Value> {
    let bytes = std::fs::read(file)?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let mut c = db::open(path)?;
    let u = db::user(&c, uid)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| invalid())?
        .trim_start_matches('\u{feff}');
    strict_quotes(text)?;
    let mut reader = csv::ReaderBuilder::new()
        .flexible(false)
        .from_reader(text.as_bytes());
    if reader.headers().map_err(|_| invalid())?
        != &csv::StringRecord::from(vec!["date", "category", "issuer", "amount", "currency"])
    {
        return Err(invalid());
    }
    let mut rows = Vec::new();
    for record in reader.records() {
        let record =
            record.map_err(|e| invalid().detail(json!({"line":e.position().map(|p|p.line())})))?;
        let line = record
            .position()
            .map(|p| p.line())
            .unwrap_or(rows.len() as u64 + 2);
        let parsed = (|| -> Result<Transaction> {
            let category = if u.category_profile == "personal" {
                f::normalize(&record[1]).ok_or_else(invalid)?
            } else {
                let category = f::categories("dad")
                    .iter()
                    .find(|cat| cat.it == record[1] || cat.id == record[1])
                    .ok_or_else(invalid)?;
                category.id.clone()
            };
            let time = f::instant(&record[0])?;
            let amount = if legacy_decimals {
                parse_legacy_money(&record[3])?
            } else {
                parse_money(&record[3])?
            };
            let t = Transaction {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: uid.into(),
                occurred_at: chrono::DateTime::from_timestamp_micros(time)
                    .unwrap()
                    .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true),
                category_id: category,
                issuer: record[2].trim().into(),
                amount_minor: amount,
                currency: record[4].into(),
                source: "legacy_csv".into(),
                version: 1,
                monzo_transaction_id: None,
            };
            f::validate(&t, &u.category_profile)?;
            Ok(t)
        })()
        .map_err(|e| e.detail(json!({"line":line})))?;
        rows.push((parsed, record[1].to_string()));
    }
    let tx = c.transaction_with_behavior(if dry {
        rusqlite::TransactionBehavior::Deferred
    } else {
        rusqlite::TransactionBehavior::Immediate
    })?;
    let prior: Option<(String, i64)> = tx
        .query_row(
            "SELECT source_sha256,row_count FROM legacy_batches WHERE user_id=?1",
            [uid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((sha, count)) = prior {
        if sha != hash {
            return Err(Error::new(409, "legacy_batch_conflict"));
        }
        return Ok(json!({"inserted":0,"duplicates":count,"dry_run":dry}));
    }
    if dry {
        return Ok(json!({"inserted":rows.len(),"duplicates":0,"dry_run":true}));
    }
    if let Some(a) = account {
        bind(&tx, uid, a)?;
    }
    let batch = uuid::Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO legacy_batches VALUES(?1,?2,?3,?4)",
        params![batch, uid, hash, rows.len() as i64],
    )?;
    for (i, (t, raw)) in rows.iter().enumerate() {
        tx.execute("INSERT INTO transactions(id,user_id,occurred_at,category_id,issuer,amount_minor,currency,source,legacy_batch_id,legacy_row_ordinal,original_category) VALUES(?1,?2,?3,?4,?5,?6,?7,'legacy_csv',?8,?9,?10)",params![t.id,uid,f::instant(&t.occurred_at)?,t.category_id,t.issuer,t.amount_minor,t.currency,batch,i as i64,raw])?;
    }
    if let Some(a) = account {
        if let Some(time) = rows
            .iter()
            .map(|(t, _)| f::instant(&t.occurred_at))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .max()
        {
            tx.execute("INSERT INTO monzo_sync(user_id,account_id,seed_since) VALUES(?1,?2,?3) ON CONFLICT(user_id) DO UPDATE SET seed_since=excluded.seed_since",params![uid,a,time])?;
        }
    }
    let rev = db::commit_revision(&tx)?;
    tx.commit()?;
    crate::fault("after_commit");
    Ok(json!({"inserted":rows.len(),"duplicates":0,"dry_run":false,"revision":rev}))
}
fn strict_quotes(text: &str) -> Result<()> {
    // csv's reader intentionally tolerates malformed quoting; reject it before parsing.
    let mut state = 0;
    let mut line = 1;
    for b in text.bytes() {
        state = match (state, b) {
            (0, b'"') => 2,
            (0, b',' | b'\r' | b'\n') => 0,
            (0, _) => 1,
            (1, b'"') => return Err(invalid().detail(json!({"line":line}))),
            (1, b',' | b'\r' | b'\n') => 0,
            (1, _) => 1,
            (2, b'"') => 3,
            (2, _) => 2,
            (3, b'"') => 2,
            (3, b',' | b'\r' | b'\n') => 0,
            (3, _) => return Err(invalid().detail(json!({"line":line}))),
            _ => unreachable!(),
        };
        if b == b'\n' {
            line += 1;
        }
    }
    if state == 2 {
        return Err(invalid().detail(json!({"line":line})));
    }
    Ok(())
}
pub fn parse_legacy_money(s: &str) -> Result<i64> {
    let sign = if s.starts_with('-') { -1_i128 } else { 1 };
    let s = s
        .strip_prefix('-')
        .or_else(|| s.strip_prefix('+'))
        .unwrap_or(s);
    let parts: Vec<_> = s.split('.').collect();
    if parts.len() != 2
        || parts[0].is_empty()
        || !(1..=2).contains(&parts[1].len())
        || !parts
            .iter()
            .all(|part| part.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(invalid());
    }
    let fraction = if parts[1].len() == 1 {
        format!("{}0", parts[1])
    } else {
        parts[1].to_owned()
    };
    let minor = format!("{}{}", parts[0], fraction)
        .parse::<i128>()
        .map_err(|_| invalid())?
        * sign;
    if minor.abs() > MAX_AMOUNT as i128 {
        return Err(invalid());
    }
    Ok(minor as i64)
}

pub fn parse_money(s: &str) -> Result<i64> {
    let sign = if s.starts_with('-') { -1_i128 } else { 1 };
    let s = s
        .strip_prefix('-')
        .or_else(|| s.strip_prefix('+'))
        .unwrap_or(s);
    let p: Vec<_> = s.split('.').collect();
    if p.len() != 2
        || p[0].is_empty()
        || p[1].len() != 2
        || !p.iter().all(|s| s.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(invalid());
    }
    let a = p.concat().parse::<i128>().map_err(|_| invalid())? * sign;
    if a.abs() > MAX_AMOUNT as i128 {
        return Err(invalid());
    }
    Ok(a as i64)
}
pub fn bind(c: &rusqlite::Connection, u: &str, a: &str) -> Result<()> {
    if a.trim().is_empty() {
        return Err(invalid());
    }
    let existing: Option<String> = c
        .query_row(
            "SELECT account_id FROM monzo_links WHERE user_id=?1",
            [u],
            |r| r.get(0),
        )
        .optional()?;
    if existing.as_deref().is_some_and(|old| old != a) {
        return Err(Error::new(409, "monzo_account_conflict"));
    }
    let other: Option<String> = c
        .query_row(
            "SELECT user_id FROM monzo_links WHERE account_id=?1",
            [a],
            |r| r.get(0),
        )
        .optional()?;
    if other.as_deref().is_some_and(|old| old != u) {
        return Err(Error::new(409, "monzo_account_conflict"));
    }
    c.execute(
        "INSERT OR IGNORE INTO monzo_links VALUES(?1,?2)",
        params![u, a],
    )?;
    Ok(())
}
pub fn reconcile(path: &Path, u: &str, t: &str, id: &str, account: &str) -> Result<Value> {
    if id.trim().is_empty() {
        return Err(invalid());
    }
    let mut c = db::open(path)?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    db::user(&tx, u)?;
    let (source, identity): (String, Option<String>) = tx
        .query_row(
            "SELECT source,monzo_transaction_id FROM transactions WHERE user_id=?1 AND id=?2",
            params![u, t],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or(Error::new(404, "not_found"))?;
    if source != "legacy_csv" || identity.as_deref().is_some_and(|existing| existing != id) {
        return Err(Error::new(409, "legacy_identity_conflict"));
    }
    bind(&tx, u, account)?;
    if tx.query_row("SELECT EXISTS(SELECT 1 FROM transactions WHERE user_id=?1 AND monzo_account_id=?2 AND monzo_transaction_id=?3 AND id<>?4)",params![u,account,id,t],|r|r.get::<_,bool>(0))? {return Err(Error::new(409,"legacy_identity_conflict"));}
    tx.execute(
        "UPDATE transactions SET monzo_account_id=?1,monzo_transaction_id=?2 WHERE id=?3",
        params![account, id, t],
    )?;
    let rev = if identity.is_some() {
        db::revision(&tx)?
    } else {
        db::commit_revision(&tx)?
    };
    tx.commit()?;
    Ok(json!({"reconciled":true,"revision":rev}))
}
#[derive(PartialEq, Eq)]
struct Raw {
    id: String,
    time: i64,
    category: String,
    issuer: String,
    amount: i64,
    currency: String,
    excluded: bool,
    unknown: bool,
}
fn raw(v: &Value) -> Result<Raw> {
    let fail = || Error::new(502, "monzo_invalid_response");
    let id = v["id"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 500)
        .ok_or_else(fail)?
        .to_string();
    let time = f::instant(v["created"].as_str().ok_or_else(fail)?).map_err(|_| fail())?;
    let cat = v["category"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(fail)?;
    let amount = v["local_amount"]
        .as_i64()
        .filter(|n| n.unsigned_abs() <= MAX_AMOUNT as u64)
        .ok_or_else(fail)?;
    let currency = v["local_currency"].as_str().ok_or_else(fail)?.to_string();
    f::currency(&currency).map_err(|_| fail())?;
    let name = match v.get("counterparty") {
        None | Some(Value::Null) => None,
        Some(Value::Object(o)) => match o.get("name") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.as_str()),
            _ => return Err(fail()),
        },
        _ => return Err(fail()),
    };
    let issuer = if let Some(name) = name.filter(|s| !s.is_empty()) {
        name
    } else {
        v["description"]
            .as_str()
            .ok_or_else(fail)?
            .split("  ")
            .next()
            .unwrap_or("")
    }
    .to_uppercase()
    .replace(' ', "_");
    if issuer.is_empty() || issuer.chars().count() > 500 {
        return Err(fail());
    }
    let excluded = issuer.contains("212") || issuer.starts_with("POT_");
    let category = f::normalize(cat);
    let unknown = category.is_none();
    Ok(Raw {
        id,
        time,
        category: category.unwrap_or("general".into()),
        issuer,
        amount,
        currency,
        excluded,
        unknown,
    })
}
pub fn monzo(
    path: &Path,
    config: &Config,
    uid: &str,
    b: &Value,
    metrics: Option<&crate::metrics::Metrics>,
) -> Result<Value> {
    let resolved = db::resolve(path)?;
    let path = resolved.as_path();
    fields(b, &["access_token", "since"], &["access_token"])?;
    let token = string(b, "access_token")?;
    if token.trim().is_empty() || token.len() > 8192 {
        return Err(invalid());
    }
    let c = db::open(path)?;
    db::user(&c, uid)?;
    let account: String = c
        .query_row(
            "SELECT account_id FROM monzo_links WHERE user_id=?1",
            [uid],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(Error::new(409, "monzo_not_linked"))?;
    let _lock =
        crate::backup::Lock::acquire(&path.with_extension(format!("import-{uid}.lock")), false)
            .map_err(|_| Error::new(409, "import_in_progress"))?;
    let saved:Option<(Option<String>,Option<i64>,Option<i64>)>=c.query_row("SELECT last_transaction_id,last_created_at,seed_since FROM monzo_sync WHERE user_id=?1 AND account_id=?2",params![uid,account],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let old_id = saved.as_ref().and_then(|s| s.0.clone());
    let old_time = saved.as_ref().and_then(|s| s.1);
    let since = if b.get("since").is_some() {
        f::instant(string(b, "since")?)?
    } else {
        saved
            .as_ref()
            .and_then(|s| s.1.or(s.2))
            .ok_or(Error::new(422, "import_start_required"))?
            .checked_sub(1_000_000)
            .ok_or_else(invalid)?
    };
    let mut cursor = chrono::DateTime::from_timestamp_micros(since)
        .ok_or_else(invalid)?
        .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true);
    drop(c);
    let _guard = metrics
        .map(|m| crate::metrics::GaugeGuard::new(m.gauge("posserver_monzo_imports_in_flight")));
    let duration = Instant::now();
    let result = (|| -> Result<Value> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(4))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| Error::new(502, "monzo_unavailable"))?;
        let mut staged: Vec<Raw> = Vec::new();
        let mut seen = HashMap::new();
        let mut previous_time = None;
        let started = Instant::now();
        loop {
            if metrics.is_some_and(|m| m.stopping.load(std::sync::atomic::Ordering::Relaxed))
                || started.elapsed() > Duration::from_secs(120)
            {
                return Err(Error::new(502, "monzo_unavailable"));
            }
            let res = client
                .get(format!(
                    "{}/transactions",
                    config.monzo.base_url.trim_end_matches('/')
                ))
                .bearer_auth(token)
                .query(&[
                    ("account_id", account.as_str()),
                    ("since", cursor.as_str()),
                    ("limit", "100"),
                ])
                .send()
                .map_err(|_| Error::new(502, "monzo_unavailable"))?;
            match res.status().as_u16() {
                200 => {}
                401 | 403 => return Err(Error::new(422, "monzo_token_rejected")),
                429 => return Err(Error::new(503, "monzo_rate_limited")),
                _ => return Err(Error::new(502, "monzo_unavailable")),
            }
            let mut bounded = res.take(16 * 1024 * 1024 + 1);
            let mut bytes = Vec::new();
            use std::io::Read;
            bounded
                .read_to_end(&mut bytes)
                .map_err(|_| Error::new(502, "monzo_unavailable"))?;
            if bytes.len() > 16 * 1024 * 1024 {
                return Err(Error::new(502, "monzo_invalid_response"));
            }
            let page: Value = serde_json::from_slice(&bytes)
                .map_err(|_| Error::new(502, "monzo_invalid_response"))?;
            let rows = page["transactions"]
                .as_array()
                .ok_or(Error::new(502, "monzo_invalid_response"))?;
            if rows.is_empty() {
                break;
            }
            if rows.len() > 100 {
                return Err(Error::new(502, "monzo_invalid_response"));
            }
            let mut added = 0;
            let mut last = None;
            for v in rows {
                let r = raw(v)?;
                if let Some(&index) = seen.get(&r.id) {
                    if staged[index] != r {
                        return Err(Error::new(502, "monzo_invalid_response"));
                    }
                    last = Some(r.id);
                    continue;
                }
                if previous_time.is_some_and(|p| r.time < p) {
                    return Err(Error::new(502, "monzo_invalid_response"));
                }
                previous_time = Some(r.time);
                if staged.len() >= config.monzo.max_objects.min(100_000) {
                    return Err(Error::new(502, "monzo_invalid_response"));
                }
                last = Some(r.id.clone());
                seen.insert(r.id.clone(), staged.len());
                staged.push(r);
                added += 1;
            }
            let next = last.unwrap();
            if added == 0 || next == cursor {
                return Err(Error::new(502, "monzo_pagination_stalled"));
            }
            cursor = next;
        }
        let mut c = db::open(path)?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (mut inserted, mut duplicates, mut excluded, mut reconciled, mut unknown) =
            (0, 0, 0, 0, 0);
        for r in &staged {
            if r.excluded {
                excluded += 1;
                continue;
            }
            if r.unknown {
                unknown += 1;
            }
            let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM transactions WHERE user_id=?1 AND monzo_account_id=?2 AND monzo_transaction_id=?3)",params![uid,account,r.id],|r|r.get(0))?;
            if exists {
                duplicates += 1;
                continue;
            }
            let second = r.time.div_euclid(1_000_000) * 1_000_000;
            let mut stmt=tx.prepare("SELECT id,monzo_transaction_id FROM transactions WHERE user_id=?1 AND source='legacy_csv' AND occurred_at>=?2 AND occurred_at<?3 AND issuer=?4 AND amount_minor=?5 AND currency=?6 AND monzo_transaction_id IS NULL")?;
            let candidates = stmt
                .query_map(
                    params![
                        uid,
                        second,
                        second + 1_000_000,
                        r.issuer,
                        r.amount,
                        r.currency
                    ],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);
            if candidates.len() > 1 {
                return Err(Error::new(409, "legacy_match_ambiguous").detail(
                    json!({"transaction_ids":candidates.iter().map(|p|&p.0).collect::<Vec<_>>()}),
                ));
            }
            if let Some((id, _)) = candidates.first() {
                tx.execute(
                "UPDATE transactions SET monzo_account_id=?1,monzo_transaction_id=?2 WHERE id=?3",
                params![account, r.id, id],
            )?;
                reconciled += 1;
            } else {
                // A sole matching legacy row already bound elsewhere cannot silently become a new row.
                let bound: Vec<String> = {
                    let mut s=tx.prepare("SELECT id FROM transactions WHERE user_id=?1 AND source='legacy_csv' AND occurred_at>=?2 AND occurred_at<?3 AND issuer=?4 AND amount_minor=?5 AND currency=?6")?;
                    let out = s
                        .query_map(
                            params![
                                uid,
                                second,
                                second + 1_000_000,
                                r.issuer,
                                r.amount,
                                r.currency
                            ],
                            |r| r.get(0),
                        )?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    out
                };
                if !bound.is_empty() {
                    return Err(Error::new(409, "legacy_match_ambiguous")
                        .detail(json!({"transaction_ids":bound})));
                }
                tx.execute("INSERT INTO transactions(id,user_id,occurred_at,category_id,issuer,amount_minor,currency,source,monzo_account_id,monzo_transaction_id) VALUES(?1,?2,?3,?4,?5,?6,?7,'monzo',?8,?9)",params![uuid::Uuid::new_v4().to_string(),uid,r.time,r.category,r.issuer,r.amount,r.currency,account,r.id])?;
                inserted += 1;
            }
        }
        let newest = staged.last().filter(|r| {
            old_time.is_none_or(|t| {
                r.time > t
                    || (r.time == t
                        && old_id
                            .as_ref()
                            .is_some_and(|id| staged.iter().any(|v| &v.id == id)))
            })
        });
        let last_id = newest.map(|r| r.id.clone()).or(old_id);
        let last_time = newest.map(|r| r.time).or(old_time);
        let cursor_changed = saved.as_ref().and_then(|s| s.0.as_ref()) != last_id.as_ref();
        if cursor_changed {
            tx.execute("INSERT INTO monzo_sync(user_id,account_id,last_transaction_id,last_created_at) VALUES(?1,?2,?3,?4) ON CONFLICT(user_id) DO UPDATE SET last_transaction_id=excluded.last_transaction_id,last_created_at=excluded.last_created_at",params![uid,account,last_id,last_time])?;
        }
        let rev = if inserted + reconciled > 0 || cursor_changed {
            db::commit_revision(&tx)?
        } else {
            db::revision(&tx)?
        };
        db::set_op(&tx, "monzo_last_success", chrono::Utc::now().timestamp())?;
        tx.commit()?;
        crate::fault("after_commit");
        Ok(
            json!({"inserted":inserted,"duplicates":duplicates,"excluded":excluded,"reconciled":reconciled,"unknown_categories":unknown,"last_raw_id":last_id,"revision":rev}),
        )
    })();
    if let Some(m) = metrics {
        m.import_result(&result);
        m.import_duration
            .with_label_values::<&str>(&[])
            .observe(duration.elapsed().as_secs_f64());
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quotes_and_money() {
        assert!(strict_quotes("a,\"unfinished").is_err());
        assert!(strict_quotes("a,b\"bad").is_err());
        assert!(strict_quotes("a,\"b\"extra").is_err());
        assert!(strict_quotes("a,\"b\"\"c\"\r\n").is_ok());
        assert_eq!(parse_money("-1.01").unwrap(), -101);
        assert!(parse_money("-1.001").is_err());
    }
}

pub fn link(path: &Path, user: &str, account: &str) -> Result<Value> {
    let mut c = db::open(path)?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let previous = db::user(&tx, user)?;
    bind(&tx, user, account)?;
    let revision = if previous.monzo_linked {
        db::revision(&tx)?
    } else {
        db::commit_revision(&tx)?
    };
    tx.commit()?;
    Ok(json!({"linked":true,"revision":revision}))
}
