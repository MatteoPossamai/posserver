use crate::{
    error::{invalid, Error, Result},
    models::*,
};
use chrono::{DateTime, Datelike, NaiveDate, TimeZone};
use serde_json::json;
use std::{
    collections::{BTreeMap, HashSet},
    sync::OnceLock,
};
pub fn currency(s: &str) -> Result<()> {
    if ["GBP", "EUR", "USD"].contains(&s) {
        Ok(())
    } else {
        Err(invalid())
    }
}
pub fn instant(s: &str) -> Result<i64> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.timestamp_micros())
        .map_err(|_| invalid())
}
#[derive(serde::Deserialize)]
struct Catalogue {
    personal: Vec<Category>,
    dad: Vec<Category>,
    aliases: BTreeMap<String, String>,
}
fn catalogue() -> &'static Catalogue {
    static CATALOGUE: OnceLock<Catalogue> = OnceLock::new();
    CATALOGUE.get_or_init(|| {
        let mut c: Catalogue = serde_json::from_str(include_str!("../docs/categories.json"))
            .expect("compiled catalogue");
        c.personal.sort_by(|a, b| a.id.cmp(&b.id));
        c.dad.sort_by(|a, b| a.id.cmp(&b.id));
        c
    })
}
pub fn categories(profile: &str) -> &'static [Category] {
    match profile {
        "personal" => &catalogue().personal,
        "dad" => &catalogue().dad,
        _ => &[],
    }
}
pub fn category(profile: &str, id: &str) -> Result<()> {
    if categories(profile)
        .binary_search_by(|c| c.id.as_str().cmp(id))
        .is_ok()
    {
        Ok(())
    } else {
        Err(invalid())
    }
}
pub fn normalize(s: &str) -> Option<String> {
    let normalized = s
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("_")
        .to_lowercase();
    let normalized = catalogue()
        .aliases
        .get(&normalized)
        .cloned()
        .unwrap_or(normalized);
    category("personal", &normalized).ok().map(|_| normalized)
}
pub fn decimal(s: &str) -> Result<(i128, i128)> {
    let parts: Vec<_> = s.split('.').collect();
    if parts.is_empty()
        || parts.len() > 2
        || parts[0].is_empty()
        || !parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        || parts.get(1).is_some_and(|p| p.len() > 8)
    {
        return Err(invalid());
    }
    let numerator = parts.concat().parse::<i128>().map_err(|_| invalid())?;
    if numerator <= 0 || numerator > i64::MAX as i128 {
        return Err(invalid());
    }
    Ok((
        numerator,
        10_i128.pow(parts.get(1).map_or(0, |p| p.len()) as u32),
    ))
}
pub fn rates(rates: &[FxRate]) -> Result<()> {
    let mut seen = HashSet::new();
    for r in rates {
        currency(&r.from)?;
        currency(&r.to)?;
        let (n, d) = decimal(&r.rate)?;
        if !seen.insert((&r.from, &r.to)) || (r.from == r.to && n != d) {
            return Err(invalid());
        }
    }
    Ok(())
}
pub fn user_settings(language: &str, tz: &str, curr: &str) -> Result<()> {
    if !["en", "it"].contains(&language) || tz.parse::<chrono_tz::Tz>().is_err() {
        return Err(invalid());
    }
    currency(curr)
}
pub fn validate(t: &Transaction, profile: &str) -> Result<()> {
    instant(&t.occurred_at)?;
    category(profile, &t.category_id)?;
    currency(&t.currency)?;
    if t.issuer.trim().is_empty()
        || t.issuer.chars().count() > 500
        || t.amount_minor.unsigned_abs() > MAX_AMOUNT as u64
    {
        return Err(invalid());
    }
    Ok(())
}
pub fn convert(t: &Transaction, u: &User, target: &str) -> Result<i64> {
    if t.currency == target {
        return Ok(t.amount_minor);
    }
    let r = u
        .rates
        .iter()
        .find(|r| r.from == t.currency && r.to == target)
        .ok_or_else(|| {
            Error::new(422, "missing_exchange_rate").detail(json!({"from":t.currency,"to":target}))
        })?;
    let (n, d) = decimal(&r.rate)?;
    let product = (t.amount_minor as i128)
        .checked_mul(n)
        .ok_or(Error::new(422, "amount_overflow"))?;
    let abs = product.abs();
    let amount = (abs / d + if (abs % d) * 2 >= d { 1 } else { 0 }) * product.signum();
    if amount.abs() > MAX_AMOUNT as i128 {
        return Err(Error::new(422, "amount_overflow"));
    }
    Ok(amount as i64)
}
pub fn month(s: &str) -> Result<NaiveDate> {
    if s.len() != 7 || s.as_bytes()[4] != b'-' {
        return Err(invalid());
    }
    NaiveDate::parse_from_str(&format!("{s}-01"), "%Y-%m-%d").map_err(|_| invalid())
}
pub fn shift(d: NaiveDate, n: i32) -> Result<NaiveDate> {
    let m = d.year() * 12 + d.month0() as i32 + n;
    NaiveDate::from_ymd_opt(m.div_euclid(12), m.rem_euclid(12) as u32 + 1, 1).ok_or_else(invalid)
}
pub fn bounds(u: &User, d: NaiveDate) -> Result<(i64, i64)> {
    let zone = u.timezone.parse::<chrono_tz::Tz>().map_err(|_| invalid())?;
    // At DST midnight gaps the month begins at the first real instant after the gap.
    // At repeated midnight choose the earliest occurrence.
    let boundary = |date: NaiveDate| -> Result<DateTime<chrono_tz::Tz>> {
        let midnight = date.and_hms_opt(0, 0, 0).unwrap();
        zone.from_local_datetime(&midnight)
            .earliest()
            .or_else(|| chrono_tz::GapInfo::new(&midnight, &zone).and_then(|gap| gap.end))
            .ok_or_else(invalid)
    };
    let start = boundary(d)?;
    let end = boundary(shift(d, 1)?)?;
    Ok((start.timestamp_micros(), end.timestamp_micros()))
}
pub fn sum(
    rows: &[Transaction],
    u: &User,
    target: &str,
    d: NaiveDate,
    cat: Option<&str>,
) -> Result<Summary> {
    let (start, end) = bounds(u, d)?;
    let mut s = Summary::default();
    for t in rows {
        let time = instant(&t.occurred_at)?;
        if time < start || time >= end || cat.is_some_and(|c| c != t.category_id) {
            continue;
        }
        let a = convert(t, u, target)?;
        add(&mut s, a)?;
    }
    Ok(s)
}
pub fn add(summary: &mut Summary, amount: i64) -> Result<()> {
    summary.count += 1;
    let field = if amount < 0 {
        &mut summary.expense_minor
    } else {
        &mut summary.income_minor
    };
    *field = field
        .checked_add(amount.abs())
        .filter(|v| *v <= MAX_AMOUNT)
        .ok_or(Error::new(422, "amount_overflow"))?;
    summary.net_minor = summary.income_minor - summary.expense_minor;
    Ok(())
}
pub fn month_categories(
    rows: &[Transaction],
    u: &User,
    target: &str,
    d: NaiveDate,
) -> Result<(Summary, BTreeMap<String, Summary>)> {
    let (start, end) = bounds(u, d)?;
    let mut total = Summary::default();
    let mut categories: BTreeMap<_, _> = self::categories(&u.category_profile)
        .iter()
        .map(|c| (c.id.clone(), Summary::default()))
        .collect();
    for t in rows {
        let time = instant(&t.occurred_at)?;
        if !(start..end).contains(&time) {
            continue;
        }
        let amount = convert(t, u, target)?;
        add(&mut total, amount)?;
        add(
            categories
                .get_mut(&t.category_id)
                .ok_or(Error::new(503, "storage_unavailable"))?,
            amount,
        )?;
    }
    Ok((total, categories))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_rates() {
        assert_eq!(decimal("0.5").unwrap(), (5, 10));
        assert!(decimal("0.123456789").is_err());
        assert!(decimal("NaN").is_err());
    }
}
