mod support;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, process::Command};
use support::*;

fn catalogue() -> Value {
    serde_json::from_str(include_str!("../docs/categories.json")).unwrap()
}

#[test]
fn spec_every_observed_csv_category_has_exact_alias_mapping() {
    let c = catalogue();
    let ids: Vec<_> = c["personal"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| id(v))
        .collect();
    for raw in c["observed_csv_labels"].as_array().unwrap() {
        let k = raw
            .as_str()
            .unwrap()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("_")
            .to_lowercase();
        let normalized = c["aliases"][&k].as_str().unwrap_or(&k);
        assert!(ids.contains(&normalized), "missing {raw}");
    }
    assert_eq!(c["aliases"]["lesuire"], "leisure");
    assert_eq!(c["aliases"]["trasport"], "transport");
    assert_ne!(
        ids.iter().position(|x| *x == "food"),
        ids.iter().position(|x| *x == "groceries")
    );
}
#[test]
fn spec_dad_labels_preserved_and_all_categories_bilingual_unique() {
    let c = catalogue();
    let labels: Vec<_> = c["dad"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["it"].as_str().unwrap())
        .collect();
    assert_eq!(
        labels,
        vec![
            "spesa",
            "gasolio",
            "telefono",
            "luce",
            "acqua",
            "gas",
            "abbigliamento",
            "man. Auto bollo",
            "spese mediche",
            "tari (rifiuti)",
            "tares (imu)",
            "assicurazione",
            "ferie - festa",
            "scuola",
            "varie",
            "legna",
            "tasse INPS"
        ]
    );
    let mut ids = std::collections::HashSet::new();
    for profile in ["dad", "personal"] {
        for v in c[profile].as_array().unwrap() {
            assert!(ids.insert(id(v)));
            assert!(!v["en"].as_str().unwrap().is_empty());
            assert!(!v["it"].as_str().unwrap().is_empty());
        }
    }
}
#[test]
fn users_start_empty_and_creation_has_per_user_settings() {
    let a = App::new();
    assert_eq!(a.api("GET", "/users", None, 200)["users"], json!([]));
    let u = a.user("Matteo", "personal", "Europe/London");
    let d = a.user("Papà", "dad", "Europe/Rome");
    assert_ne!(id(&u), id(&d));
    assert_eq!(d["language"], "it");
    assert_eq!(u["version"], 1);
    assert_eq!(d["reporting_currency"], "EUR");
    assert_eq!(u["rates"], json!([]));
}
#[test]
fn user_settings_validation_and_conflict_are_atomic() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "Europe/London");
    let path = format!("/users/{}/settings", id(&u));
    for patch in [
        json!({"expected_version":1,"timezone":"Mars/Base"}),
        json!({"expected_version":1,"language":"fr"}),
        json!({"expected_version":1,"rates":[{"from":"EUR","to":"GBP","rate":"0"}]}),
        json!({"expected_version":1,"rates":[{"from":"EUR","to":"GBP","rate":"NaN"}]}),
    ] {
        a.api("PATCH", &path, Some(patch), 422);
    }
    let changed = a.api(
        "PATCH",
        &path,
        Some(json!({"expected_version":1,"language":"it"})),
        200,
    );
    assert_eq!(changed["user"]["version"], 2);
    error(
        &a.api(
            "PATCH",
            &path,
            Some(json!({"expected_version":1,"timezone":"Europe/Rome"})),
            409,
        ),
        "version_conflict",
    );
    assert_eq!(
        a.api("GET", &format!("/users/{}", id(&u)), None, 200)["user"]["timezone"],
        "Europe/London"
    );
}
#[test]
fn categories_are_hardcoded_and_profile_scoped() {
    let a = App::new();
    for (name, profile) in [("Matteo", "personal"), ("Papà", "dad")] {
        let u = a.user(name, profile, "UTC");
        let mut expected = catalogue()[profile].as_array().unwrap().clone();
        expected.sort_by_key(|v| id(v).to_string());
        assert_eq!(
            a.api("GET", &format!("/users/{}/categories", id(&u)), None, 200)["categories"],
            json!(expected)
        );
    }
}
#[test]
fn manual_entries_latest_and_reads_are_user_scoped() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let d = a.user("Papà", "dad", "Europe/Rome");
    assert!(a.api(
        "GET",
        &format!("/users/{}/transactions/latest", id(&d)),
        None,
        200
    )["transaction"]
        .is_null());
    let t = a.create(id(&u), "2026-10-01T10:00:00Z", -123, "groceries", "GBP");
    a.create(id(&d), "2030-01-01T00:00:00Z", -50, "dad_groceries", "EUR");
    assert_eq!(a.rows(id(&u)).len(), 1);
    assert_eq!(
        a.api(
            "GET",
            &format!("/users/{}/transactions/latest", id(&u)),
            None,
            200
        )["transaction"]["id"],
        t["id"]
    );
    a.api(
        "GET",
        &format!("/users/{}/transactions/{}", id(&d), id(&t)),
        None,
        404,
    );
    a.api(
        "PATCH",
        &format!("/users/{}/transactions/{}", id(&d), id(&t)),
        Some(json!({"expected_version":1,"amount_minor":-999})),
        404,
    );
    a.api(
        "DELETE",
        &format!("/users/{}/transactions/{}", id(&d), id(&t)),
        Some(json!({"expected_version":1})),
        404,
    );
    assert_eq!(a.rows(id(&u))[0]["amount_minor"], -123);
}
#[test]
fn invalid_manual_inputs_never_write() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let rev = a.status()["current_revision"].clone();
    for (key, value) in [
        ("occurred_at", json!("2026-02-30T00:00:00Z")),
        ("occurred_at", json!("2026-10-01")),
        ("issuer", json!("  ")),
        ("currency", json!("JPY")),
        ("category_id", json!("dad_groceries")),
        ("amount_minor", json!(1.5)),
        ("amount_minor", json!(9007199254740992_i64)),
        ("source", json!("monzo")),
    ] {
        let mut b = manual("2026-10-01T00:00:00Z", -10, "groceries", "GBP");
        b[key] = value;
        a.api(
            "POST",
            &format!("/users/{}/transactions", id(&u)),
            Some(b),
            422,
        );
    }
    assert!(a.rows(id(&u)).is_empty());
    assert_eq!(a.status()["current_revision"], rev);
}
#[test]
fn edit_delete_versions_and_tombstones() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let t = a.create(id(&u), "2026-10-01T00:00:00Z", -10, "groceries", "GBP");
    let p = format!("/users/{}/transactions/{}", id(&u), id(&t));
    let changed = a.api(
        "PATCH",
        &p,
        Some(json!({"expected_version":1,"amount_minor":-20})),
        200,
    );
    assert_eq!(changed["transaction"]["version"], 2);
    error(
        &a.api(
            "PATCH",
            &p,
            Some(json!({"expected_version":1,"amount_minor":-99})),
            409,
        ),
        "version_conflict",
    );
    error(
        &a.api("DELETE", &p, Some(json!({"expected_version":1})), 409),
        "version_conflict",
    );
    a.api("DELETE", &p, Some(json!({"expected_version":2})), 200);
    assert!(a.rows(id(&u)).is_empty());
    a.api("GET", &p, None, 404);
    assert_eq!(
        a.report(id(&u), "2026-10", "GBP", 200)["current"]["count"],
        0
    );
}
#[test]
fn stable_pagination_handles_equal_instants_and_newer_inserts() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let mut ids = Vec::new();
    for _ in 0..3 {
        ids.push(
            a.create(id(&u), "2026-10-01T00:00:00Z", -1, "groceries", "GBP")["id"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }
    ids.sort();
    ids.reverse();
    let first = a.api(
        "GET",
        &format!("/users/{}/transactions?limit=2", id(&u)),
        None,
        200,
    );
    assert_eq!(first["transactions"][0]["id"], ids[0]);
    assert_eq!(first["transactions"][1]["id"], ids[1]);
    a.create(id(&u), "2026-10-02T00:00:00Z", -2, "groceries", "GBP");
    let cursor = first["next_cursor"].as_str().unwrap();
    let second = a.api(
        "GET",
        &format!("/users/{}/transactions?limit=2&cursor={cursor}", id(&u)),
        None,
        200,
    );
    assert_eq!(second["transactions"].as_array().unwrap().len(), 1);
    assert_eq!(second["transactions"][0]["id"], ids[2]);
    assert!(second["next_cursor"].is_null());
}
#[test]
fn transaction_filters_use_exclusive_end_and_sign() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    for (date, amount, cat) in [
        ("2026-10-01T00:00:00Z", -10, "groceries"),
        ("2026-10-01T12:00:00Z", 10, "salary"),
        ("2026-10-01T13:00:00Z", 0, "groceries"),
        ("2026-10-02T00:00:00Z", -20, "groceries"),
    ] {
        a.create(id(&u), date, amount, cat, "GBP");
    }
    let p=format!("/users/{}/transactions?from=2026-10-01T00:00:00Z&to=2026-10-02T00:00:00Z&category_id=groceries&kind=expense",id(&u));
    assert_eq!(
        a.api("GET", &p, None, 200)["transactions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn reports_signs_refunds_zero_rows_and_previous_year() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    for (date, amount, cat) in [
        ("2026-01-01T00:00:00Z", -1000, "groceries"),
        ("2026-01-02T00:00:00Z", 200, "refund"),
        ("2026-01-03T00:00:00Z", 0, "general"),
        ("2025-12-31T23:59:59Z", -50, "groceries"),
    ] {
        a.create(id(&u), date, amount, cat, "GBP");
    }
    let r = a.report(id(&u), "2026-01", "GBP", 200);
    assert_eq!(
        r["current"],
        json!({"expense_minor":1000,"income_minor":200,"net_minor":-800,"count":3})
    );
    assert_eq!(r["previous"]["month"], "2025-12");
    assert_eq!(r["previous"]["expense_minor"], 50);
    assert_eq!(
        r["categories"].as_array().unwrap().len(),
        catalogue()["personal"].as_array().unwrap().len()
    );
    let cat = r["categories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["category_id"] == "groceries")
        .unwrap();
    assert_eq!(cat["expense_minor"], 1000);
}
#[test]
fn month_boundaries_follow_user_timezone_and_dst() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "Europe/London");
    for (date, amount) in [
        ("2026-03-31T22:59:59Z", -10),
        ("2026-03-31T23:00:00Z", -20),
        ("2026-04-30T22:59:59Z", -30),
        ("2026-04-30T23:00:00Z", -40),
    ] {
        a.create(id(&u), date, amount, "groceries", "GBP");
    }
    let r = a.report(id(&u), "2026-04", "GBP", 200);
    assert_eq!(r["current"]["expense_minor"], 50);
    assert_eq!(r["previous"]["expense_minor"], 10);
}
#[test]
fn trend_zero_fills_months_and_filters_categories() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    a.create(id(&u), "2026-02-02T00:00:00Z", -100, "groceries", "GBP");
    a.create(id(&u), "2026-02-02T00:00:00Z", -500, "rent", "GBP");
    let r=a.api("GET",&format!("/users/{}/reports/trend?from_month=2026-01&to_month=2026-03&category_id=groceries&currency=GBP",id(&u)),None,200);
    assert_eq!(r["points"].as_array().unwrap().len(), 3);
    for (i, amount) in [0, 100, 0].iter().enumerate() {
        assert_eq!(r["points"][i]["expense_minor"], *amount);
    }
    assert_eq!(r["points"][1]["month"], "2026-02");
}
#[test]
fn fx_exact_rounding_directed_rates_and_changes_preserve_sources() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    a.create(id(&u), "2026-10-01T00:00:00Z", -101, "groceries", "EUR");
    a.create(id(&u), "2026-10-01T00:00:00Z", 101, "refund", "EUR");
    error(
        &a.report(id(&u), "2026-10", "GBP", 422),
        "missing_exchange_rate",
    );
    let p = format!("/users/{}/settings", id(&u));
    a.api(
        "PATCH",
        &p,
        Some(json!({"expected_version":1,"rates":[{"from":"EUR","to":"GBP","rate":"0.5"}]})),
        200,
    );
    let r = a.report(id(&u), "2026-10", "GBP", 200);
    assert_eq!(r["current"]["expense_minor"], 51);
    assert_eq!(r["current"]["income_minor"], 51);
    assert_eq!(r["settings_version"], 2);
    a.api(
        "PATCH",
        &p,
        Some(json!({"expected_version":2,"rates":[{"from":"EUR","to":"GBP","rate":"0.87"}]})),
        200,
    );
    assert_eq!(
        a.report(id(&u), "2026-10", "GBP", 200)["current"]["expense_minor"],
        88
    );
    assert_eq!(a.rows(id(&u))[0]["currency"], "EUR");
    a.create(id(&u), "2026-10-01T00:00:00Z", -100, "groceries", "GBP");
    error(
        &a.report(id(&u), "2026-10", "EUR", 422),
        "missing_exchange_rate",
    );
}
#[test]
fn rates_are_per_user_and_round_each_transaction_before_sum() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let d = a.user("Papà", "dad", "UTC");
    a.api(
        "PATCH",
        &format!("/users/{}/settings", id(&u)),
        Some(json!({"expected_version":1,"rates":[{"from":"EUR","to":"GBP","rate":"0.5"}]})),
        200,
    );
    for _ in 0..2 {
        a.create(id(&u), "2026-10-01T00:00:00Z", -1, "groceries", "EUR");
    }
    a.create(id(&d), "2026-10-01T00:00:00Z", -1, "dad_groceries", "EUR");
    assert_eq!(
        a.report(id(&u), "2026-10", "GBP", 200)["current"]["expense_minor"],
        2
    );
    error(
        &a.report(id(&d), "2026-10", "GBP", 422),
        "missing_exchange_rate",
    );
}
#[test]
fn invalid_report_queries_are_rejected() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    for q in [
        "month=2026-13",
        "month=2026-1",
        "month=oops",
        "month=2026-10&currency=JPY",
    ] {
        a.api(
            "GET",
            &format!("/users/{}/reports/month?{q}", id(&u)),
            None,
            422,
        );
    }
    for q in [
        "from_month=2026-03&to_month=2026-01",
        "from_month=2000-01&to_month=2026-10",
        "from_month=2026-01&to_month=2026-02&category_id=dad_groceries",
    ] {
        a.api(
            "GET",
            &format!("/users/{}/reports/trend?{q}", id(&u)),
            None,
            422,
        );
    }
}
#[test]
fn monzo_preserves_python_amount_currency_issuer_and_exclusions() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let mut named = tx("tx_a", "2026-10-01T00:00:00.123456Z", -123, "ignored");
    named["counterparty"] = json!({"name":"some person"});
    named["local_currency"] = json!("EUR");
    a.monzo.page(vec![
        named,
        tx("tx_b", "2026-10-01T00:00:01Z", -99, "test shop"),
        tx("tx_c", "2026-10-01T00:00:02Z", -99, "Trading212"),
        tx("tx_d", "2026-10-01T00:00:03Z", -99, "POT savings"),
        tx("tx_e", "2026-10-01T00:00:04Z", -99, "SHOP212OTHER"),
    ]);
    a.monzo.page(vec![]);
    let r = a.import(id(&u), Some("2026-09-30T00:00:00Z"), 200);
    assert_eq!(r["inserted"], 2);
    assert_eq!(r["excluded"], 3);
    assert_eq!(r["last_raw_id"], "tx_e");
    let rows = a.rows(id(&u));
    let v = rows
        .iter()
        .find(|v| v["monzo_transaction_id"] == "tx_a")
        .unwrap();
    assert_eq!(v["issuer"], "SOME_PERSON");
    assert_eq!(v["amount_minor"], -123);
    assert_eq!(v["currency"], "EUR");
    assert!(v["occurred_at"].as_str().unwrap().contains("123456"));
    assert_eq!(rows[0]["issuer"], "TEST_SHOP");
    let requests = a.monzo.requests();
    assert_eq!(requests.len(), 2);
    for req in requests {
        assert_eq!(req.method, "GET");
        assert_eq!(
            req.header("Authorization"),
            Some("Bearer synthetic-monzo-token")
        );
        assert!(req.path.starts_with("/transactions?"));
        assert!(req.path.contains("account_id=acc_test"));
        assert!(req.path.contains("limit=100"));
        assert!(!req.path.contains("synthetic-monzo-token"));
    }
}
#[test]
fn monzo_same_second_more_than_page_and_replay_are_lossless() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let date = "2026-10-01T00:00:00Z";
    a.monzo.page(
        (0..100)
            .map(|n| tx(&format!("tx_{n:03}"), date, -1, "shop"))
            .collect(),
    );
    a.monzo.page(vec![tx("tx_100", date, -2, "shop")]);
    a.monzo.page(vec![]);
    assert_eq!(
        a.import(id(&u), Some("2026-09-30T00:00:00Z"), 200)["inserted"],
        101
    );
    assert!(a.monzo.requests()[1].path.contains("since=tx_099"));
    a.monzo.page(vec![
        tx("tx_099", date, -1, "shop"),
        tx("tx_100", date, -2, "shop"),
        tx("tx_101", date, -3, "shop"),
    ]);
    a.monzo.page(vec![]);
    let r = a.import(id(&u), None, 200);
    assert_eq!(r["inserted"], 1);
    assert_eq!(r["duplicates"], 2);
    assert_eq!(a.monzo.remaining(), 0);
    assert_eq!(
        a.report(id(&u), "2026-10", "GBP", 200)["current"]["count"],
        102
    );
}
#[test]
fn monzo_excluded_only_page_advances_and_terminates() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    a.monzo.page(vec![tx(
        "tx_pot",
        "2026-10-01T00:00:00Z",
        -1,
        "POT savings",
    )]);
    a.monzo.page(vec![]);
    let r = a.import(id(&u), Some("2026-09-30T00:00:00Z"), 200);
    assert_eq!(r["inserted"], 0);
    assert_eq!(r["excluded"], 1);
    assert_eq!(r["last_raw_id"], "tx_pot");
    assert!(a.rows(id(&u)).is_empty());
    assert!(a.monzo.requests()[1].path.contains("since=tx_pot"));
}
#[test]
fn monzo_start_ignores_all_manual_and_other_user_history() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let d = a.user("Papà", "dad", "UTC");
    a.create(id(&u), "2035-01-01T00:00:00Z", -1, "groceries", "GBP");
    a.create(id(&d), "2040-01-01T00:00:00Z", -2, "dad_groceries", "EUR");
    error(&a.import(id(&u), None, 422), "import_start_required");
    error(
        &a.import(id(&d), Some("2026-01-01T00:00:00Z"), 409),
        "monzo_not_linked",
    );
    assert!(a.monzo.requests().is_empty());
    a.monzo
        .page(vec![tx("tx_a", "2026-01-01T00:00:00Z", -4, "shop")]);
    a.monzo.page(vec![]);
    a.import(id(&u), Some("2025-12-31T00:00:00Z"), 200);
    assert_eq!(a.rows(id(&d)).len(), 1);
    assert_eq!(a.rows(id(&u)).len(), 2);
    a.monzo.page(vec![]);
    a.import(id(&u), None, 200);
    let last = a.monzo.requests().last().unwrap().path.clone();
    assert!(last.contains("2025-12-31"));
    assert!(!last.contains("2035"));
    assert!(!last.contains("2040"));
}
#[test]
fn monzo_provider_failures_never_refresh_or_advance_cursor() {
    for (status, code, http) in [
        (401, "monzo_token_rejected", 422),
        (403, "monzo_token_rejected", 422),
        (429, "monzo_rate_limited", 503),
        (500, "monzo_unavailable", 502),
    ] {
        let a = App::new();
        let u = a.user("Matteo", "personal", "UTC");
        let rev = a.status()["current_revision"].clone();
        a.monzo.reply(
            status,
            json!({"message":"synthetic-monzo-token upstream private body"}),
        );
        error(&a.import(id(&u), Some("2026-01-01T00:00:00Z"), http), code);
        assert!(a.rows(id(&u)).is_empty());
        assert_eq!(a.status()["current_revision"], rev);
        assert!(a.monzo.requests().iter().all(|r| !r.path.contains("oauth")));
        a.monzo.page(vec![]);
        a.import(id(&u), Some("2026-01-01T00:00:00Z"), 200);
    }
}
#[test]
fn monzo_later_page_failure_rolls_back_entire_import() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    a.monzo
        .page(vec![tx("tx_a", "2026-10-01T00:00:00Z", -10, "shop")]);
    a.monzo.raw(200, b"{not-json".to_vec());
    error(
        &a.import(id(&u), Some("2026-09-30T00:00:00Z"), 502),
        "monzo_invalid_response",
    );
    assert!(a.rows(id(&u)).is_empty());
    error(&a.import(id(&u), None, 422), "import_start_required");
}
#[test]
fn monzo_invalid_objects_abort_without_fallback_amount() {
    for (key, value) in [
        ("local_amount", json!(1.5)),
        ("local_currency", json!("JPY")),
        ("created", json!("bad")),
        ("id", json!("")),
    ] {
        let a = App::new();
        let u = a.user("Matteo", "personal", "UTC");
        let mut bad = tx("tx_bad", "2026-10-01T00:00:00Z", -1, "shop");
        bad[key] = value;
        a.monzo
            .page(vec![tx("tx_ok", "2026-10-01T00:00:00Z", -1, "shop"), bad]);
        error(
            &a.import(id(&u), Some("2026-09-30T00:00:00Z"), 502),
            "monzo_invalid_response",
        );
        assert!(a.rows(id(&u)).is_empty());
    }
}
#[test]
fn monzo_repeated_page_fails_instead_of_looping() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let t = tx("tx_a", "2026-10-01T00:00:00Z", -1, "shop");
    a.monzo.page(vec![t.clone()]);
    a.monzo.page(vec![t]);
    error(
        &a.import(id(&u), Some("2026-09-30T00:00:00Z"), 502),
        "monzo_pagination_stalled",
    );
    assert!(a.rows(id(&u)).is_empty());
    assert_eq!(a.monzo.requests().len(), 2);
}
#[test]
fn monzo_unknown_categories_zero_and_declined_preserve_python_rules() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let mut t = tx("tx_a", "2026-10-01T00:00:00Z", 0, "shop");
    t["category"] = json!("new_provider_category");
    t["decline_reason"] = json!("INSUFFICIENT_FUNDS");
    a.monzo.page(vec![t]);
    a.monzo.page(vec![]);
    let r = a.import(id(&u), Some("2026-09-30T00:00:00Z"), 200);
    assert_eq!(r["unknown_categories"], 1);
    assert_eq!(a.rows(id(&u))[0]["category_id"], "general");
    assert_eq!(a.rows(id(&u))[0]["amount_minor"], 0);
}
#[test]
fn monzo_reimport_preserves_user_edits_and_deletion() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let rows = vec![
        tx("tx_a", "2026-10-01T00:00:00Z", -1, "shop"),
        tx("tx_b", "2026-10-01T00:00:01Z", -2, "shop"),
    ];
    a.monzo.page(rows.clone());
    a.monzo.page(vec![]);
    a.import(id(&u), Some("2026-09-30T00:00:00Z"), 200);
    let saved = a.rows(id(&u));
    a.api(
        "PATCH",
        &format!("/users/{}/transactions/{}", id(&u), id(&saved[0])),
        Some(json!({"expected_version":1,"category_id":"rent"})),
        200,
    );
    a.api(
        "DELETE",
        &format!("/users/{}/transactions/{}", id(&u), id(&saved[1])),
        Some(json!({"expected_version":1})),
        200,
    );
    a.monzo.page(rows);
    a.monzo.page(vec![]);
    assert_eq!(a.import(id(&u), None, 200)["duplicates"], 2);
    let after = a.rows(id(&u));
    assert_eq!(after.len(), 1);
    assert_eq!(after[0]["category_id"], "rent");
}

const HEADER: &str = "date,category,issuer,amount,currency\n";
#[test]
fn csv_migration_normalizes_exact_amounts_and_is_idempotent() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let p=a.csv(&format!("{HEADER}2026-10-01T00:00:00Z,lesuire,SHOP,-1.01,EUR\n2026-10-01T00:00:00Z,groceries ,SHOP,-0.10,GBP\n"));
    let before = fs::read(&p).unwrap();
    let first = output_json(&a.migrate(id(&u), &p, &[]));
    assert_eq!(first["inserted"], 2);
    let second = output_json(&a.migrate(id(&u), &p, &[]));
    assert_eq!(second["inserted"], 0);
    assert_eq!(second["duplicates"], 2);
    assert_eq!(a.rows(id(&u)).len(), 2);
    assert_eq!(fs::read(&p).unwrap(), before);
    let rows = a.rows(id(&u));
    assert!(rows
        .iter()
        .any(|v| v["category_id"] == "leisure" && v["amount_minor"] == -101));
}
#[test]
fn csv_preserves_identical_legitimate_rows_and_dry_run_is_read_only() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let row = "2026-10-01T00:00:00Z,groceries,SHOP,-1.00,GBP\n";
    let p = a.csv(&format!("{HEADER}{row}{row}"));
    let rev = a.status()["current_revision"].clone();
    assert_eq!(
        output_json(&a.migrate(id(&u), &p, &["--dry-run"]))["inserted"],
        2
    );
    assert!(a.rows(id(&u)).is_empty());
    assert_eq!(a.status()["current_revision"], rev);
    assert_eq!(output_json(&a.migrate(id(&u), &p, &[]))["inserted"], 2);
    assert_eq!(a.rows(id(&u)).len(), 2);
}
#[test]
fn csv_invalid_late_rows_rollback_and_unknown_categories_fail() {
    for bad in [
        "2026-10-02T00:00:00Z,groceries,SHOP,-1.001,GBP",
        "2026-10-02T00:00:00Z,unknown,SHOP,-1.00,GBP",
        "2026-02-30T00:00:00Z,groceries,SHOP,-1.00,GBP",
        "2026-10-02T00:00:00Z,groceries,SHOP,NaN,GBP",
        "2026-10-02T00:00:00Z,groceries,SHOP,-1.00,JPY",
        "2026-10-02T00:00:00Z,groceries,SHOP,-1.00,GBP,extra",
    ] {
        let a = App::new();
        let u = a.user("Matteo", "personal", "UTC");
        let p = a.csv(&format!(
            "{HEADER}2026-10-01T00:00:00Z,groceries,SHOP,-1.00,GBP\n{bad}\n"
        ));
        let o = a.migrate(id(&u), &p, &[]);
        assert_eq!(o.status.code(), Some(2));
        assert!(a.rows(id(&u)).is_empty());
    }
}
#[test]
fn csv_bom_crlf_and_quoted_issuer_are_accepted() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let p=a.csv("\u{feff}date,category,issuer,amount,currency\r\n2026-10-01T00:00:00Z,groceries,\"SHOP, LTD\",-1.00,GBP\r\n");
    assert_eq!(output_json(&a.migrate(id(&u), &p, &[]))["inserted"], 1);
    assert_eq!(a.rows(id(&u))[0]["issuer"], "SHOP, LTD");
}
#[test]
fn csv_boundary_monzo_reconciles_without_duplicate_even_if_category_changed() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let p = a.csv(&format!(
        "{HEADER}2026-10-01T00:00:00Z,rent,SHOP,-1.00,GBP\n"
    ));
    output_json(&a.migrate(id(&u), &p, &["--monzo-account-id", "acc_test"]));
    a.monzo.page(vec![
        tx("tx_existing", "2026-10-01T00:00:00.123Z", -100, "SHOP"),
        tx("tx_new", "2026-10-01T00:00:00.456Z", -200, "SHOP"),
    ]);
    a.monzo.page(vec![]);
    let r = a.import(id(&u), None, 200);
    assert_eq!(r["reconciled"], 1);
    assert_eq!(r["inserted"], 1);
    let rows = a.rows(id(&u));
    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .any(|v| v["monzo_transaction_id"] == "tx_existing" && v["category_id"] == "rent"));
    assert!(a.monzo.requests()[0].path.contains("2026-09-30"));
}
#[test]
fn csv_ambiguous_legacy_matches_abort_without_loss() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let row = "2026-10-01T00:00:00Z,groceries,SHOP,-1.00,GBP\n";
    let p = a.csv(&format!("{HEADER}{row}{row}"));
    output_json(&a.migrate(id(&u), &p, &["--monzo-account-id", "acc_test"]));
    a.monzo.page(vec![tx(
        "tx_existing",
        "2026-10-01T00:00:00.123Z",
        -100,
        "SHOP",
    )]);
    a.monzo.page(vec![]);
    error(&a.import(id(&u), None, 409), "legacy_match_ambiguous");
    assert_eq!(a.rows(id(&u)).len(), 2);
    assert!(a
        .rows(id(&u))
        .iter()
        .all(|v| v["monzo_transaction_id"].is_null()));
}
#[test]
fn restart_preserves_users_transactions_and_pending_backups() {
    let mut a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    a.create(id(&u), "2026-10-01T00:00:00Z", -123, "groceries", "GBP");
    let status = a.status();
    a.stop();
    a.start();
    assert_eq!(a.rows(id(&u))[0]["amount_minor"], -123);
    assert_eq!(a.status()["current_revision"], status["current_revision"]);
    assert_eq!(a.status()["pending"], status["pending"]);
}
#[test]
fn every_domain_commit_queues_backup_without_remote_requests() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let initial = a.status();
    let t = a.create(id(&u), "2026-10-01T00:00:00Z", -1, "groceries", "GBP");
    assert_eq!(
        a.status()["current_revision"].as_i64().unwrap(),
        initial["current_revision"].as_i64().unwrap() + 1
    );
    a.api(
        "PATCH",
        &format!("/users/{}/transactions/{}", id(&u), id(&t)),
        Some(json!({"expected_version":1,"amount_minor":-2})),
        200,
    );
    a.api(
        "DELETE",
        &format!("/users/{}/transactions/{}", id(&u), id(&t)),
        Some(json!({"expected_version":2})),
        200,
    );
    let after = a.status();
    assert!(after["pending"].as_i64().unwrap() >= 4);
    assert!(after["dropbox_revision"].is_null());
    assert!(a.dropbox.requests().is_empty());
}
#[test]
fn dropbox_upload_is_consistent_sqlite_with_current_wal_data() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    a.create(id(&u), "2026-10-01T00:00:00Z", -123, "groceries", "GBP");
    let revision = a.status()["current_revision"].as_i64().unwrap();
    for _ in 0..6 {
        a.dropbox
            .reply(200, json!({"name":"database.sqlite","rev":"mock-rev"}));
    }
    output_json(&a.backup("dropbox"));
    let reqs = a.dropbox.requests();
    assert!(!reqs.is_empty());
    let mut db_found = false;
    let mut manifest_found = false;
    for r in reqs {
        assert_eq!(r.method, "POST");
        assert_eq!(r.path, "/2/files/upload");
        assert_eq!(
            r.header("Authorization"),
            Some("Bearer synthetic-dropbox-token")
        );
        let arg: Value = serde_json::from_str(r.header("Dropbox-API-Arg").unwrap()).unwrap();
        assert_eq!(arg["mode"], "overwrite");
        assert_eq!(arg["autorename"], false);
        assert!(arg["path"].as_str().unwrap().starts_with("/posserver/"));
        if r.body.starts_with(b"SQLite format 3\0") {
            let p = a.dir.path().join("uploaded.sqlite");
            fs::write(&p, &r.body).unwrap();
            let db = rusqlite::Connection::open(&p).unwrap();
            assert_eq!(
                db.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                "ok"
            );
            assert_eq!(
                db.query_row("SELECT value FROM revision", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                revision
            );
            db_found = true;
        } else {
            let m: Value = serde_json::from_slice(&r.body).unwrap();
            assert_eq!(m["revision"], revision);
            assert_eq!(m["sha256"].as_str().unwrap().len(), 64);
            manifest_found = true;
        }
    }
    assert!(db_found && manifest_found);
    assert_eq!(a.status()["dropbox_revision"], revision);
    assert_eq!(a.status()["pending"], 0);
}
#[test]
fn failed_dropbox_keeps_pending_local_write_and_retry_catches_up() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    a.create(id(&u), "2026-10-01T00:00:00Z", -1, "groceries", "GBP");
    for _ in 0..6 {
        a.dropbox.reply(500, json!({"error":"unavailable"}));
    }
    let failed = a.backup("dropbox");
    assert!(!failed.status.success());
    assert_eq!(a.rows(id(&u)).len(), 1);
    assert!(a.status()["pending"].as_i64().unwrap() > 0);
    assert!(a.status()["dropbox_revision"].is_null());
    // Discard unconsumed scripted failures before the retry.
    while a.dropbox.remaining() > 0 {
        let _ = a
            .client
            .post(format!("{}/discard", a.dropbox.url))
            .send()
            .unwrap();
    }
    for _ in 0..6 {
        a.dropbox.reply(200, json!({"rev":"retry"}));
    }
    output_json(&a.backup("dropbox"));
    assert_eq!(a.status()["pending"], 0);
    assert_eq!(
        a.status()["dropbox_revision"],
        a.status()["current_revision"]
    );
}
#[test]
fn restore_rejects_hash_mismatch_without_overwriting_destination() {
    let a = App::new();
    a.user("Matteo", "personal", "UTC");
    let snapshot = a.dir.path().join("bad.sqlite");
    fs::write(&snapshot, b"not a database").unwrap();
    let manifest = a.dir.path().join("manifest.json");
    fs::write(&manifest,serde_json::to_vec(&json!({"revision":1,"schema_version":1,"sha256":"0".repeat(64),"created_at":"2026-10-01T00:00:00Z"})).unwrap()).unwrap();
    let dest = a.dir.path().join("restore.sqlite");
    fs::write(&dest, b"original").unwrap();
    let o = a.cli(&[
        "restore",
        "--db",
        dest.to_str().unwrap(),
        "--snapshot",
        snapshot.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
    ]);
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(fs::read(dest).unwrap(), b"original");
}
#[test]
fn restore_rejects_corrupt_sqlite_even_with_matching_hash() {
    let a = App::new();
    let bytes = b"SQLite format 3\0broken";
    let snapshot = a.dir.path().join("bad.sqlite");
    fs::write(&snapshot, bytes).unwrap();
    let manifest = a.dir.path().join("manifest.json");
    fs::write(&manifest,serde_json::to_vec(&json!({"revision":1,"schema_version":1,"sha256":format!("{:x}",Sha256::digest(bytes)),"created_at":"2026-10-01T00:00:00Z"})).unwrap()).unwrap();
    let dest = a.dir.path().join("restored.sqlite");
    let o = a.cli(&[
        "restore",
        "--db",
        dest.to_str().unwrap(),
        "--snapshot",
        snapshot.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
    ]);
    assert_eq!(o.status.code(), Some(2));
    assert!(!dest.exists());
}

#[test]
fn fx_duplicate_identity_and_precision_rates_fail_atomically() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let path = format!("/users/{}/settings", id(&u));
    for rates in [
        json!([{"from":"EUR","to":"GBP","rate":"0.8"},{"from":"EUR","to":"GBP","rate":"0.9"}]),
        json!([{"from":"GBP","to":"GBP","rate":"2"}]),
        json!([{"from":"EUR","to":"GBP","rate":"0.123456789"}]),
        json!([{"from":"EUR","to":"GBP","rate":"-1"}]),
    ] {
        a.api(
            "PATCH",
            &path,
            Some(json!({"expected_version":1,"rates":rates})),
            422,
        );
    }
    assert_eq!(
        a.api("GET", &format!("/users/{}", id(&u)), None, 200)["user"]["version"],
        1
    );
}
#[test]
fn report_overflow_is_explicit() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    a.create(
        id(&u),
        "2026-10-01T00:00:00Z",
        -9007199254740991,
        "groceries",
        "EUR",
    );
    a.api(
        "PATCH",
        &format!("/users/{}/settings", id(&u)),
        Some(json!({"expected_version":1,"rates":[{"from":"EUR","to":"GBP","rate":"1000000"}]})),
        200,
    );
    error(&a.report(id(&u), "2026-10", "GBP", 422), "amount_overflow");
}
#[test]
fn list_cursor_is_bound_to_user_and_filters() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let d = a.user("Papà", "dad", "UTC");
    for _ in 0..2 {
        a.create(id(&u), "2026-10-01T00:00:00Z", -1, "groceries", "GBP");
    }
    let first = a.api(
        "GET",
        &format!("/users/{}/transactions?limit=1", id(&u)),
        None,
        200,
    );
    let cursor = first["next_cursor"].as_str().unwrap();
    a.api(
        "GET",
        &format!("/users/{}/transactions?limit=1&cursor={cursor}", id(&d)),
        None,
        422,
    );
    a.api(
        "GET",
        &format!(
            "/users/{}/transactions?limit=1&category_id=rent&cursor={cursor}",
            id(&u)
        ),
        None,
        422,
    );
    for q in [
        "limit=0",
        "limit=101",
        "kind=invalid",
        "from=bad",
        "from=2026-10-02T00:00:00Z&to=2026-10-01T00:00:00Z",
    ] {
        a.api(
            "GET",
            &format!("/users/{}/transactions?{q}", id(&u)),
            None,
            422,
        );
    }
}
#[test]
fn offset_timestamps_sort_by_instant_and_latest_tie_uses_id() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let one = a.create(id(&u), "2026-10-01T12:00:00+02:00", -1, "groceries", "GBP");
    let two = a.create(id(&u), "2026-10-01T10:00:00Z", -2, "groceries", "GBP");
    a.create(id(&u), "2026-10-01T11:00:00+02:00", -3, "groceries", "GBP");
    let rows = a.rows(id(&u));
    let expected = std::cmp::max(id(&one), id(&two));
    assert_eq!(rows[0]["id"], expected);
    assert_eq!(rows[2]["amount_minor"], -3);
    assert_eq!(
        a.api(
            "GET",
            &format!("/users/{}/transactions/latest", id(&u)),
            None,
            200
        )["transaction"]["id"],
        expected
    );
}
#[test]
fn csv_second_different_batch_refused_and_wrong_user_never_writes() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let p = a.csv(&format!(
        "{HEADER}2026-10-01T00:00:00Z,groceries,SHOP,-1.00,GBP\n"
    ));
    assert_eq!(a.migrate("no-user", &p, &[]).status.code(), Some(2));
    output_json(&a.migrate(id(&u), &p, &[]));
    fs::write(
        &p,
        format!("{HEADER}2026-10-02T00:00:00Z,groceries,SHOP,-2.00,GBP\n"),
    )
    .unwrap();
    assert_eq!(a.migrate(id(&u), &p, &[]).status.code(), Some(2));
    assert_eq!(a.rows(id(&u)).len(), 1);
}
#[test]
fn legacy_cli_resolution_attaches_identity_and_replay_preserves_rows() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let row = "2026-10-01T00:00:00Z,groceries,SHOP,-1.00,GBP\n";
    let p = a.csv(&format!("{HEADER}{row}{row}"));
    output_json(&a.migrate(id(&u), &p, &["--monzo-account-id", "acc_test"]));
    let rows = a.rows(id(&u));
    let o = a.cli(&[
        "reconcile-legacy",
        "--db",
        a.db.to_str().unwrap(),
        "--user",
        id(&u),
        "--transaction",
        id(&rows[0]),
        "--monzo-id",
        "tx_one",
        "--account-id",
        "acc_test",
    ]);
    assert_eq!(output_json(&o)["reconciled"], true);
    a.monzo.page(vec![
        tx("tx_one", "2026-10-01T00:00:00Z", -100, "SHOP"),
        tx("tx_two", "2026-10-01T00:00:00Z", -100, "SHOP"),
    ]);
    a.monzo.page(vec![]);
    let r = a.import(id(&u), None, 200);
    assert_eq!(r["inserted"], 0);
    assert_eq!(r["reconciled"], 1);
    assert_eq!(r["duplicates"], 1);
    assert_eq!(a.rows(id(&u)).len(), 2);
}
#[test]
fn monzo_empty_fetch_keeps_cursor_and_revision_and_token_never_persists() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    a.monzo
        .page(vec![tx("tx_a", "2026-10-01T00:00:00Z", -1, "shop")]);
    a.monzo.page(vec![]);
    a.import(id(&u), Some("2026-09-30T00:00:00Z"), 200);
    let rev = a.status()["current_revision"].clone();
    a.monzo.page(vec![]);
    let empty = a.import(id(&u), None, 200);
    assert_eq!(empty["inserted"], 0);
    assert_eq!(empty["last_raw_id"], "tx_a");
    assert_eq!(a.status()["current_revision"], rev);
    for name in ["database.sqlite", "database.sqlite-wal", "server.log"] {
        let path = a.dir.path().join(name);
        if path.exists() {
            let bytes = fs::read(path).unwrap();
            assert!(!bytes
                .windows(b"synthetic-monzo-token".len())
                .any(|w| w == b"synthetic-monzo-token"));
        }
    }
}
#[test]
fn monzo_missing_local_fields_and_missing_issuer_do_not_guess() {
    for field in ["local_amount", "local_currency", "description"] {
        let a = App::new();
        let u = a.user("Matteo", "personal", "UTC");
        let mut bad = tx("tx_bad", "2026-10-01T00:00:00Z", -1, "shop");
        bad.as_object_mut().unwrap().remove(field);
        a.monzo.page(vec![bad]);
        error(
            &a.import(id(&u), Some("2026-09-30T00:00:00Z"), 502),
            "monzo_invalid_response",
        );
        assert!(a.rows(id(&u)).is_empty());
    }
}
#[test]
fn monzo_conflicting_duplicate_ids_abort() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    a.monzo.page(vec![
        tx("tx_same", "2026-10-01T00:00:00Z", -1, "shop"),
        tx("tx_same", "2026-10-01T00:00:00Z", -2, "shop"),
    ]);
    error(
        &a.import(id(&u), Some("2026-09-30T00:00:00Z"), 502),
        "monzo_invalid_response",
    );
    assert!(a.rows(id(&u)).is_empty());
}
#[test]
fn restore_valid_snapshot_preserves_reports() {
    let mut a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    a.create(id(&u), "2026-10-01T00:00:00Z", -321, "groceries", "GBP");
    let revision = a.status()["current_revision"].as_i64().unwrap();
    let before = a.report(id(&u), "2026-10", "GBP", 200);
    a.stop();
    let snapshot = a.dir.path().join("snapshot.sqlite");
    let src = rusqlite::Connection::open(&a.db).unwrap();
    src.execute("VACUUM INTO ?1", [snapshot.to_str().unwrap()])
        .unwrap();
    drop(src);
    let bytes = fs::read(&snapshot).unwrap();
    let manifest = a.dir.path().join("restore-manifest.json");
    fs::write(&manifest,serde_json::to_vec(&json!({"revision":revision,"schema_version":1,"sha256":format!("{:x}",Sha256::digest(&bytes)),"created_at":"2026-10-01T00:00:00Z"})).unwrap()).unwrap();
    let dest = a.dir.path().join("restored.sqlite");
    let o = a.cli(&[
        "restore",
        "--db",
        dest.to_str().unwrap(),
        "--snapshot",
        snapshot.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
    ]);
    assert_eq!(output_json(&o)["restored"], true);
    a.db = dest;
    a.start();
    assert_eq!(a.report(id(&u), "2026-10", "GBP", 200), before);
    assert_eq!(a.rows(id(&u)).len(), 1);
}
