use crate::{
    config::Config,
    db,
    error::{invalid, Error, Result},
    finance as f,
    models::*,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rusqlite::params;
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path};

pub fn fields(v: &Value, allowed: &[&str], required: &[&str]) -> Result<()> {
    let obj = v.as_object().ok_or_else(invalid)?;
    if obj.keys().any(|k| !allowed.contains(&k.as_str()))
        || required.iter().any(|k| !obj.contains_key(*k))
    {
        return Err(invalid());
    }
    Ok(())
}
pub fn string<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    v[k].as_str().ok_or_else(invalid)
}
pub fn number(v: &Value, k: &str) -> Result<i64> {
    v[k].as_i64().ok_or_else(invalid)
}
fn version(v: &Value, n: i64) -> Result<()> {
    if number(v, "expected_version")? != n {
        Err(Error::new(409, "version_conflict"))
    } else {
        Ok(())
    }
}
fn output(status: u16, v: Value) -> Result<(u16, Value)> {
    Ok((status, v))
}
pub fn dispatch(
    path: &Path,
    config: &Config,
    method: &str,
    parts: &[&str],
    q: &BTreeMap<String, String>,
    b: &Value,
) -> Result<(u16, Value)> {
    let mut c = db::open(path)?;
    if method == "GET" {
        c.execute_batch("BEGIN DEFERRED")?;
    }
    if parts == ["backups", "status"] && method == "GET" {
        return output(200, crate::backup::status(&c, config)?);
    }
    if parts == ["backups", "retry"] && method == "POST" {
        return output(202, json!({"queued":true}));
    }
    if parts == ["users"] {
        return match method {
            "GET" => {
                let mut s = c.prepare("SELECT id FROM users ORDER BY name,id")?;
                let ids = s
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let users = ids
                    .iter()
                    .map(|id| db::user(&c, id))
                    .collect::<Result<Vec<_>>>()?;
                output(200, json!({"users":users}))
            }
            "POST" => {
                fields(
                    b,
                    &[
                        "name",
                        "language",
                        "timezone",
                        "reporting_currency",
                        "category_profile",
                    ],
                    &[
                        "name",
                        "language",
                        "timezone",
                        "reporting_currency",
                        "category_profile",
                    ],
                )?;
                let name = string(b, "name")?.trim();
                let lang = string(b, "language")?;
                let tz = string(b, "timezone")?;
                let curr = string(b, "reporting_currency")?;
                let profile = string(b, "category_profile")?;
                f::user_settings(lang, tz, curr)?;
                if name.is_empty()
                    || name.chars().count() > 100
                    || !["personal", "dad"].contains(&profile)
                {
                    return Err(invalid());
                }
                let id = uuid::Uuid::new_v4().to_string();
                let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                tx.execute("INSERT INTO users(id,name,language,timezone,reporting_currency,category_profile,version) VALUES(?1,?2,?3,?4,?5,?6,1)",params![id,name,lang,tz,curr,profile])?;
                if let Some(account) = config
                    .monzo
                    .links_by_user_name
                    .get(name)
                    .filter(|_| profile == "personal")
                {
                    let already_bound = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM monzo_links WHERE account_id=?1)",
                        [account],
                        |r| r.get::<_, bool>(0),
                    )?;
                    if !already_bound {
                        tx.execute(
                            "INSERT INTO monzo_links VALUES(?1,?2)",
                            params![id, account],
                        )?;
                    }
                }
                let r = db::commit_revision(&tx)?;
                tx.commit()?;
                crate::fault("after_commit");
                output(201, json!({"user":db::user(&c,&id)?,"revision":r}))
            }
            _ => Err(Error::new(405, "method_not_allowed")),
        };
    }
    if parts.len() < 2 || parts[0] != "users" {
        return Err(Error::new(404, "not_found"));
    }
    let uid = parts[1];
    let u = db::user(&c, uid)?;
    if parts.len() == 2 && method == "GET" {
        return output(200, json!({"user":u}));
    }
    match parts.get(2).copied() {
        Some("categories") if parts.len() == 3 && method == "GET" => output(
            200,
            json!({"categories":f::categories(&u.category_profile)}),
        ),
        Some("settings") if parts.len() == 3 && method == "PATCH" => {
            fields(
                b,
                &[
                    "expected_version",
                    "language",
                    "timezone",
                    "reporting_currency",
                    "rates",
                ],
                &["expected_version"],
            )?;
            let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let mut current = db::user(&tx, uid)?;
            version(b, current.version)?;
            let original = serde_json::to_value(&current).unwrap();
            if b.get("language").is_some() {
                current.language = string(b, "language")?.into();
            }
            if b.get("timezone").is_some() {
                current.timezone = string(b, "timezone")?.into();
            }
            if b.get("reporting_currency").is_some() {
                current.reporting_currency = string(b, "reporting_currency")?.into();
            }
            if let Some(r) = b.get("rates") {
                current.rates = serde_json::from_value(r.clone()).map_err(|_| invalid())?;
                f::rates(&current.rates)?;
            }
            f::user_settings(
                &current.language,
                &current.timezone,
                &current.reporting_currency,
            )?;
            let r = if serde_json::to_value(&current).unwrap() != original {
                current.version += 1;
                tx.execute("UPDATE users SET language=?1,timezone=?2,reporting_currency=?3,rates=?4,version=?5 WHERE id=?6",params![current.language,current.timezone,current.reporting_currency,serde_json::to_string(&current.rates).unwrap(),current.version,uid])?;
                db::commit_revision(&tx)?
            } else {
                db::revision(&tx)?
            };
            tx.commit()?;
            output(200, json!({"user":current,"revision":r}))
        }
        Some("transactions") => transactions(&mut c, &u, method, &parts[3..], q, b),
        Some("reports") if method == "GET" && parts.len() == 4 => report(&c, &u, parts[3], q),
        _ => Err(Error::new(404, "not_found")),
    }
}
fn transactions(
    c: &mut rusqlite::Connection,
    u: &User,
    method: &str,
    parts: &[&str],
    q: &BTreeMap<String, String>,
    b: &Value,
) -> Result<(u16, Value)> {
    if parts.is_empty() && method == "GET" {
        let allowed = ["from", "to", "category_id", "kind", "limit", "cursor"];
        if q.keys().any(|k| !allowed.contains(&k.as_str())) {
            return Err(invalid());
        }
        let limit = q
            .get("limit")
            .map(|s| s.parse::<usize>().map_err(|_| invalid()))
            .transpose()?
            .unwrap_or(50);
        if !(1..=100).contains(&limit) {
            return Err(invalid());
        }
        let from = q.get("from").map(|s| f::instant(s)).transpose()?;
        let to = q.get("to").map(|s| f::instant(s)).transpose()?;
        if from.zip(to).is_some_and(|(a, b)| a >= b) {
            return Err(invalid());
        }
        if let Some(cat) = q.get("category_id") {
            f::category(&u.category_profile, cat)?;
        }
        let kind = q.get("kind").map(String::as_str).unwrap_or("all");
        if !["all", "income", "expense"].contains(&kind) {
            return Err(invalid());
        }
        let filter =
            json!({"user":u.id,"from":from,"to":to,"category_id":q.get("category_id"),"kind":kind});
        let boundary = if let Some(cursor) = q.get("cursor") {
            let data = URL_SAFE_NO_PAD.decode(cursor).map_err(|_| invalid())?;
            let v: Value = serde_json::from_slice(&data).map_err(|_| invalid())?;
            if v["filter"] != filter {
                return Err(invalid());
            }
            Some((number(&v, "time")?, string(&v, "id")?.to_string()))
        } else {
            None
        };
        // Keyset bounds are applied in SQL; limit memory to one page plus a continuation row.
        let sql=format!("SELECT {} FROM transactions WHERE user_id=?1 AND deleted_at IS NULL AND (?2 IS NULL OR occurred_at>=?2) AND (?3 IS NULL OR occurred_at<?3) AND (?4 IS NULL OR category_id=?4) AND (?5='all' OR (?5='income' AND amount_minor>0) OR (?5='expense' AND amount_minor<0)) AND (?6 IS NULL OR occurred_at<?6 OR (occurred_at=?6 AND id<?7)) ORDER BY occurred_at DESC,id DESC LIMIT ?8",db::COLUMNS);
        let mut stmt = c.prepare(&sql)?;
        let mut rows = stmt
            .query_map(
                params![
                    u.id,
                    from,
                    to,
                    q.get("category_id"),
                    kind,
                    boundary.as_ref().map(|b| b.0),
                    boundary.as_ref().map(|b| &b.1),
                    (limit + 1) as i64
                ],
                db::transaction_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let more = rows.len() > limit;
        rows.truncate(limit);
        let cursor = if more {
            let t = rows.last().unwrap();
            Some(
                URL_SAFE_NO_PAD.encode(
                    serde_json::to_vec(
                        &json!({"filter":filter,"time":f::instant(&t.occurred_at)?,"id":t.id}),
                    )
                    .unwrap(),
                ),
            )
        } else {
            None
        };
        return output(200, json!({"transactions":rows,"next_cursor":cursor}));
    }
    if parts == ["latest"] && method == "GET" {
        use rusqlite::OptionalExtension;
        let t=c.query_row(&format!("SELECT {} FROM transactions WHERE user_id=?1 AND deleted_at IS NULL ORDER BY occurred_at DESC,id DESC LIMIT 1",db::COLUMNS),[&u.id],db::transaction_row).optional()?;
        return output(200, json!({"transaction":t}));
    }
    if parts.len() == 1 && method == "GET" {
        return output(
            200,
            json!({"transaction":db::transaction(c,&u.id,parts[0])?}),
        );
    }
    if (parts.is_empty() && method == "POST")
        || (parts.len() == 1 && ["PATCH", "DELETE"].contains(&method))
    {
        let allowed = if method == "DELETE" {
            vec!["expected_version"]
        } else {
            vec![
                "occurred_at",
                "category_id",
                "issuer",
                "amount_minor",
                "currency",
                "expected_version",
            ]
        };
        let required = if method == "POST" {
            vec![
                "occurred_at",
                "category_id",
                "issuer",
                "amount_minor",
                "currency",
            ]
        } else {
            vec!["expected_version"]
        };
        fields(b, &allowed, &required)?;
        if method == "POST" && b.get("expected_version").is_some() {
            return Err(invalid());
        }
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut t = if method == "POST" {
            Transaction {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: u.id.clone(),
                occurred_at: String::new(),
                category_id: String::new(),
                issuer: String::new(),
                amount_minor: 0,
                currency: String::new(),
                source: "manual".into(),
                version: 1,
                monzo_transaction_id: None,
            }
        } else {
            let t = db::transaction(&tx, &u.id, parts[0])?;
            version(b, t.version)?;
            t
        };
        if method == "DELETE" {
            tx.execute(
                "UPDATE transactions SET deleted_at=?1,version=version+1 WHERE id=?2",
                params![chrono::Utc::now().timestamp_micros(), t.id],
            )?;
            let r = db::commit_revision(&tx)?;
            tx.commit()?;
            return output(200, json!({"deleted":true,"revision":r}));
        }
        if b.get("occurred_at").is_some() {
            let time = f::instant(string(b, "occurred_at")?)?;
            t.occurred_at = chrono::DateTime::from_timestamp_micros(time)
                .unwrap()
                .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true);
        }
        if b.get("category_id").is_some() {
            t.category_id = string(b, "category_id")?.into();
        }
        if b.get("issuer").is_some() {
            t.issuer = string(b, "issuer")?.trim().into();
        }
        if b.get("amount_minor").is_some() {
            t.amount_minor = number(b, "amount_minor")?;
        }
        if b.get("currency").is_some() {
            t.currency = string(b, "currency")?.into();
        }
        f::validate(&t, &u.category_profile)?;
        if method == "POST" {
            tx.execute("INSERT INTO transactions(id,user_id,occurred_at,category_id,issuer,amount_minor,currency,source) VALUES(?1,?2,?3,?4,?5,?6,?7,'manual')",params![t.id,t.user_id,f::instant(&t.occurred_at)?,t.category_id,t.issuer,t.amount_minor,t.currency])?;
        } else {
            t.version += 1;
            tx.execute("UPDATE transactions SET occurred_at=?1,category_id=?2,issuer=?3,amount_minor=?4,currency=?5,version=?6 WHERE id=?7",params![f::instant(&t.occurred_at)?,t.category_id,t.issuer,t.amount_minor,t.currency,t.version,t.id])?;
        }
        let r = db::commit_revision(&tx)?;
        tx.commit()?;
        crate::fault("after_commit");
        return output(
            if method == "POST" { 201 } else { 200 },
            json!({"transaction":t,"revision":r}),
        );
    }
    Err(Error::new(404, "not_found"))
}
fn report(
    c: &rusqlite::Connection,
    u: &User,
    kind: &str,
    q: &BTreeMap<String, String>,
) -> Result<(u16, Value)> {
    let target = q
        .get("currency")
        .map(String::as_str)
        .unwrap_or(&u.reporting_currency);
    f::currency(target)?;
    // dispatch holds a read transaction for coherent settings and finance rows.
    match kind {
        "month" => {
            if q.keys()
                .any(|k| !["month", "currency"].contains(&k.as_str()))
            {
                return Err(invalid());
            }
            let month = q.get("month").ok_or_else(invalid)?;
            let d = f::month(month)?;
            let previous = f::shift(d, -1)?;
            let rows = db::rows_range(
                c,
                &u.id,
                f::bounds(u, previous)?.0,
                f::bounds(u, d)?.1,
                None,
            )?;
            let (current, by_category) = f::month_categories(&rows, u, target, d)?;
            let mut prev = serde_json::to_value(f::sum(&rows, u, target, previous, None)?).unwrap();
            prev["month"] = json!(previous.format("%Y-%m").to_string());
            let categories = by_category
                .into_iter()
                .map(|(id, summary)| {
                    let mut v = serde_json::to_value(summary).unwrap();
                    v["category_id"] = json!(id);
                    v
                })
                .collect::<Vec<_>>();
            output(
                200,
                json!({"month":month,"timezone":u.timezone,"currency":target,"settings_version":u.version,"rates":u.rates,"current":current,"previous":prev,"categories":categories}),
            )
        }
        "trend" => {
            if q.keys().any(|k| {
                !["from_month", "to_month", "category_id", "currency"].contains(&k.as_str())
            }) {
                return Err(invalid());
            }
            let from = f::month(q.get("from_month").ok_or_else(invalid)?)?;
            let to = f::month(q.get("to_month").ok_or_else(invalid)?)?;
            if from > to {
                return Err(invalid());
            }
            if let Some(cat) = q.get("category_id") {
                f::category(&u.category_profile, cat)?;
            }
            let mut summaries = BTreeMap::new();
            let mut d = from;
            while d <= to {
                if summaries.len() >= 120 {
                    return Err(invalid());
                }
                summaries.insert(d.format("%Y-%m").to_string(), Summary::default());
                d = f::shift(d, 1)?;
            }
            let rows = db::rows_range(
                c,
                &u.id,
                f::bounds(u, from)?.0,
                f::bounds(u, to)?.1,
                q.get("category_id").map(String::as_str),
            )?;
            let zone = u.timezone.parse::<chrono_tz::Tz>().map_err(|_| invalid())?;
            for t in &rows {
                let key = chrono::DateTime::from_timestamp_micros(f::instant(&t.occurred_at)?)
                    .unwrap()
                    .with_timezone(&zone)
                    .format("%Y-%m")
                    .to_string();
                f::add(
                    summaries.get_mut(&key).ok_or_else(invalid)?,
                    f::convert(t, u, target)?,
                )?;
            }
            let points = summaries
                .into_iter()
                .map(|(month, summary)| {
                    let mut v = serde_json::to_value(summary).unwrap();
                    v["month"] = json!(month);
                    v
                })
                .collect::<Vec<_>>();
            output(
                200,
                json!({"currency":target,"timezone":u.timezone,"settings_version":u.version,"rates":u.rates,"points":points}),
            )
        }
        _ => Err(Error::new(404, "not_found")),
    }
}
