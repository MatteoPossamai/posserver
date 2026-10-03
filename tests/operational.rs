//! Additional black-box fault, monitoring and concurrency checks using synthetic providers.
#![allow(dead_code)]
mod support;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    process::{Child, Command, Stdio},
    sync::Barrier,
    thread,
    time::{Duration, Instant},
};
use support::*;

fn metrics(a: &App) -> String {
    let response = a.client.get(format!("{}/metrics", a.url)).send().unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["content-type"],
        "text/plain; version=0.0.4; charset=utf-8"
    );
    let text = response.text().unwrap();
    assert!(text.ends_with('\n'));
    text
}
fn sample(text: &str, name: &str, labels: &[(&str, &str)]) -> f64 {
    let matching: Vec<_> = text
        .lines()
        .filter(|l| {
            !l.starts_with('#')
                && l.split(['{', ' ']).next() == Some(name)
                && labels
                    .iter()
                    .all(|(k, v)| l.contains(&format!("{k}=\"{v}\"")))
        })
        .collect();
    assert_eq!(matching.len(), 1, "metric {name} {labels:?}: {matching:?}");
    matching[0].rsplit_once(' ').unwrap().1.parse().unwrap()
}
fn bin() -> std::ffi::OsString {
    std::env::var_os("POSSERVER_BIN").unwrap()
}
fn config(a: &App) -> Value {
    serde_json::from_slice(&fs::read(&a.config).unwrap()).unwrap()
}
fn write_config(a: &App, c: &Value) {
    fs::write(&a.config, serde_json::to_vec(c).unwrap()).unwrap();
}
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn start(a: &App, envs: &[(&str, &str)]) -> Running {
    let child = Command::new(bin())
        .args(["serve", "--db"])
        .arg(&a.db)
        .arg("--bind")
        .arg(a.url.trim_start_matches("http://"))
        .arg("--config")
        .arg(&a.config)
        .envs(envs.iter().copied())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let run = Running(child);
    wait(|| {
        a.client
            .get(format!("{}/healthz", a.url))
            .send()
            .is_ok_and(|r| r.status() == 200)
    });
    run
}
fn wait(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(6);
    while !ready() {
        assert!(Instant::now() < deadline, "operation did not complete");
        thread::sleep(Duration::from_millis(20));
    }
}
fn backup_env(a: &App, point: &str) -> std::process::Output {
    Command::new(bin())
        .args(["backup-dropbox", "--db"])
        .arg(&a.db)
        .arg("--config")
        .arg(&a.config)
        .env("POSSERVER_FAULT", point)
        .output()
        .unwrap()
}

#[test]
fn metrics_baseline_types_and_scrapes_are_read_only_and_bounded() {
    let a = App::new();
    let text = metrics(&a);
    for (name, kind) in [
        ("posserver_build_info", "gauge"),
        ("posserver_monzo_imports_total", "counter"),
        ("posserver_backup_attempts_total", "counter"),
        ("posserver_backup_retries_total", "counter"),
        ("posserver_http_request_duration_seconds", "histogram"),
        ("posserver_db_operation_duration_seconds", "histogram"),
    ] {
        assert!(text.contains(&format!("# HELP {name} ")));
        assert!(text.contains(&format!("# TYPE {name} {kind}\n")));
    }
    assert_eq!(sample(&text, "posserver_ready", &[]), 1.);
    assert_eq!(sample(&text, "posserver_current_revision", &[]), 0.);
    {
        let p = "dropbox";
        assert_eq!(
            sample(&text, "posserver_backup_configured", &[("provider", p)]),
            1.
        );
        assert_eq!(
            sample(
                &text,
                "posserver_backup_last_success_timestamp_seconds",
                &[("provider", p)]
            ),
            0.
        );
        assert_eq!(
            sample(
                &text,
                "posserver_backup_attempts_total",
                &[("provider", p), ("result", "success")]
            ),
            0.
        );
    }
    let before = a.status();
    let requests = sample(
        &metrics(&a),
        "posserver_http_requests_total",
        &[
            ("method", "GET"),
            ("route", "/api/v1/backups/status"),
            ("status_class", "2xx"),
        ],
    );
    for _ in 0..5 {
        let m = metrics(&a);
        assert_eq!(
            sample(
                &m,
                "posserver_http_requests_total",
                &[
                    ("method", "GET"),
                    ("route", "/api/v1/backups/status"),
                    ("status_class", "2xx")
                ]
            ),
            requests
        );
    }
    assert_eq!(a.status(), before);
    assert!(a.monzo.requests().is_empty());
    assert!(a.dropbox.requests().is_empty());
    assert!(!text.contains("/healthz"));
    assert!(!text.contains("route=\"/metrics\""));
}
#[test]
fn http_metrics_use_templates_and_clean_in_flight_after_errors() {
    let a = App::new();
    let u = a.user("Sensitive issuer-name", "personal", "UTC");
    for n in 0..20 {
        a.api(
            "GET",
            &format!("/users/{}/transactions/missing-{n}", id(&u)),
            None,
            404,
        );
        a.api("GET", &format!("/unrecognized-{n}"), None, 404);
    }
    let text = metrics(&a);
    assert!(!text.contains(id(&u)));
    assert!(!text.contains("missing-"));
    assert!(!text.contains("Sensitive"));
    assert_eq!(
        sample(
            &text,
            "posserver_http_requests_total",
            &[
                ("method", "GET"),
                ("route", "/api/v1/users/:u/transactions/:t"),
                ("status_class", "4xx")
            ]
        ),
        20.
    );
    assert_eq!(
        sample(
            &text,
            "posserver_http_request_duration_seconds_count",
            &[("method", "GET"), ("route", "unmatched")]
        ),
        20.
    );
    assert_eq!(sample(&text, "posserver_http_requests_in_flight", &[]), 0.);
}
#[test]
fn metrics_import_outcomes_only_count_commits() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    a.monzo
        .page(vec![tx("ok", "2026-10-01T00:00:00Z", -1, "shop")]);
    a.monzo.raw(500, vec![]);
    a.import(id(&u), Some("2026-09-01T00:00:00Z"), 502);
    let m = metrics(&a);
    assert_eq!(
        sample(
            &m,
            "posserver_monzo_transactions_total",
            &[("outcome", "inserted")]
        ),
        0.
    );
    assert_eq!(sample(&m, "posserver_monzo_imports_in_flight", &[]), 0.);
    assert_eq!(
        sample(
            &m,
            "posserver_monzo_imports_total",
            &[("result", "unavailable")]
        ),
        1.
    );
    a.monzo
        .page(vec![tx("ok", "2026-10-01T00:00:00Z", -1, "shop")]);
    a.monzo.page(vec![]);
    a.import(id(&u), Some("2026-09-01T00:00:00Z"), 200);
    let m = metrics(&a);
    assert_eq!(
        sample(
            &m,
            "posserver_monzo_transactions_total",
            &[("outcome", "inserted")]
        ),
        1.
    );
    assert!(sample(&m, "posserver_monzo_last_success_timestamp_seconds", &[]) > 0.);
}
#[test]
fn cli_backup_metrics_survive_restart_and_match_status() {
    let mut a = App::new();
    a.user("Matteo", "personal", "UTC");
    for _ in 0..3 {
        a.dropbox.reply(200, json!({}));
    }
    output_json(&a.backup("dropbox"));
    a.stop();
    a.start();
    let status = a.status();
    let m = metrics(&a);
    assert_eq!(
        sample(&m, "posserver_backup_revision", &[("provider", "dropbox")]),
        status["dropbox_revision"].as_f64().unwrap()
    );
    assert_eq!(sample(&m, "posserver_backup_pending_revisions", &[]), 0.);
    assert_eq!(
        sample(
            &m,
            "posserver_backup_attempts_total",
            &[("provider", "dropbox"), ("result", "success")]
        ),
        1.
    );
    assert_eq!(
        sample(
            &m,
            "posserver_backup_duration_seconds_count",
            &[("provider", "dropbox")]
        ),
        0.
    );
    assert!(
        sample(
            &m,
            "posserver_backup_last_duration_seconds",
            &[("provider", "dropbox")]
        ) > 0.
    );
}
#[test]
fn unconfigured_backup_is_distinct_from_confirmed_success() {
    let mut a = App::new();
    a.stop();
    let mut c = config(&a);
    c["backup"].as_object_mut().unwrap().remove("dropbox");
    write_config(&a, &c);
    a.start();
    assert!(a.status()["dropbox_revision"].is_null());
    let m = metrics(&a);
    {
        let p = "dropbox";
        assert_eq!(
            sample(&m, "posserver_backup_configured", &[("provider", p)]),
            0.
        );
        assert_eq!(
            sample(
                &m,
                "posserver_backup_last_attempt_success",
                &[("provider", p)]
            ),
            0.
        );
    }
}
#[test]
fn unavailable_database_keeps_metrics_and_reports_unready() {
    let a = App::new();
    let old = metrics(&a);
    let timestamp = sample(&old, "posserver_state_refresh_timestamp_seconds", &[]);
    let original = a.db.with_extension("saved");
    fs::rename(&a.db, &original).unwrap();
    fs::write(&a.db, b"broken SQLite file").unwrap();
    a.api("GET", "/users", None, 503);
    let m = metrics(&a);
    assert_eq!(sample(&m, "posserver_ready", &[]), 0.);
    assert_eq!(sample(&m, "posserver_db_available", &[]), 0.);
    assert_eq!(
        sample(&m, "posserver_state_refresh_timestamp_seconds", &[]),
        timestamp
    );
    assert_eq!(
        a.client
            .get(format!("{}/healthz", a.url))
            .send()
            .unwrap()
            .status(),
        503
    );
    fs::remove_file(&a.db).unwrap();
    fs::rename(original, &a.db).unwrap();
    a.api("GET", "/users", None, 200);
    assert_eq!(sample(&metrics(&a), "posserver_ready", &[]), 1.);
}
#[test]
fn busy_writer_does_not_block_scrapes_or_lose_atomicity() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let before = a.status();
    let c = rusqlite::Connection::open(&a.db).unwrap();
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    thread::scope(|scope| {
        let request = scope.spawn(|| {
            a.api(
                "POST",
                &format!("/users/{}/transactions", id(&u)),
                Some(manual("2026-10-01T00:00:00Z", -1, "groceries", "GBP")),
                503,
            )
        });
        thread::sleep(Duration::from_millis(80));
        let now = Instant::now();
        let m = metrics(&a);
        assert!(now.elapsed() < Duration::from_secs(1));
        assert!(sample(&m, "posserver_http_requests_in_flight", &[]) >= 1.);
        error(&request.join().unwrap(), "storage_unavailable");
    });
    c.execute_batch("ROLLBACK").unwrap();
    assert_eq!(a.status(), before);
    assert!(a.rows(id(&u)).is_empty());
    assert_eq!(
        sample(
            &metrics(&a),
            "posserver_db_errors_total",
            &[("reason", "busy")]
        ),
        1.
    );
}
#[test]
fn simultaneous_edits_have_one_winner_and_one_revision() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let t = a.create(id(&u), "2026-10-01T00:00:00Z", -1, "groceries", "GBP");
    let before = a.status()["current_revision"].as_i64().unwrap();
    let barrier = Barrier::new(3);
    let statuses = thread::scope(|scope| {
        let tasks: Vec<_> = (0..2)
            .map(|n| {
                let (barrier, a, u, t) = (&barrier, &a, &u, &t);
                scope.spawn(move || {
                    barrier.wait();
                    a.client
                        .patch(format!(
                            "{}/api/v1/users/{}/transactions/{}",
                            a.url,
                            id(u),
                            id(t)
                        ))
                        .json(&json!({"expected_version":1,"amount_minor":-2-n}))
                        .send()
                        .unwrap()
                        .status()
                        .as_u16()
                })
            })
            .collect();
        barrier.wait();
        tasks
            .into_iter()
            .map(|t| t.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(statuses.contains(&200) && statuses.contains(&409));
    assert_eq!(a.status()["current_revision"], before + 1);
    assert_eq!(a.rows(id(&u))[0]["version"], 2);
}
#[test]
fn crash_after_commit_replays_durable_queue() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let file=a.csv("date,category,issuer,amount,currency\n2026-10-01T00:00:00Z,groceries,CRASH_SHOP,-1.00,GBP\n");
    let o = Command::new(bin())
        .args(["migrate-csv", "--db"])
        .arg(&a.db)
        .arg("--user")
        .arg(id(&u))
        .arg("--file")
        .arg(file)
        .env("POSSERVER_FAULT", "after_commit")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(86));
    assert_eq!(a.rows(id(&u)).len(), 1);
    assert_eq!(a.status()["pending"], 2);
    for _ in 0..2 {
        a.dropbox.reply(200, json!({}));
    }
    output_json(&a.backup("dropbox"));
    assert_eq!(a.status()["pending"], 0);
}
#[test]
fn crashes_after_snapshot_and_upload_replay_same_immutable_bytes() {
    for point in ["after_snapshot", "after_upload"] {
        let a = App::new();
        a.user("Matteo", "personal", "UTC");
        for _ in 0..4 {
            a.dropbox.reply(200, json!({}));
        }
        assert_eq!(backup_env(&a, point).status.code(), Some(86));
        assert_eq!(a.status()["pending"], 1);
        let snapshot = a.dir.path().join("backups/1.sqlite");
        let bytes = fs::read(&snapshot).unwrap();
        output_json(&a.backup("dropbox"));
        assert_eq!(fs::read(snapshot).unwrap(), bytes);
        assert_eq!(a.status()["pending"], 0);
        let db = rusqlite::Connection::open(&a.db).unwrap();
        assert!(
            db.query_row(
                "SELECT CAST(value AS INTEGER) FROM operational WHERE key='dropbox_retries'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap()
                >= 1
        );
        let uploads: Vec<_> = a
            .dropbox
            .requests()
            .into_iter()
            .filter(|r| r.body.starts_with(b"SQLite format 3\0"))
            .collect();
        for upload in uploads {
            assert_eq!(upload.body, bytes);
        }
    }
}
#[test]
fn dropbox_expired_token_refreshes_once_and_never_persists_access_token() {
    let a = App::new();
    a.user("Matteo", "personal", "UTC");
    let mut conf = config(&a);
    conf["backup"]["dropbox"]["refresh_token"] = json!("synthetic-refresh");
    conf["backup"]["dropbox"]["app_key"] = json!("key");
    conf["backup"]["dropbox"]["app_secret"] = json!("secret");
    write_config(&a, &conf);
    a.dropbox.reply(401, json!({}));
    a.dropbox
        .reply(200, json!({"access_token":"fresh-synthetic-access"}));
    a.dropbox.reply(200, json!({}));
    a.dropbox.reply(200, json!({}));
    output_json(&a.backup("dropbox"));
    let requests = a.dropbox.requests();
    assert_eq!(requests[1].path, "/oauth2/token");
    assert_eq!(
        requests[2].header("Authorization"),
        Some("Bearer fresh-synthetic-access")
    );
    assert_eq!(a.status()["pending"], 0);
    for p in [&a.db, &a.config] {
        let b = fs::read(p).unwrap();
        assert!(!b.windows(22).any(|b| b == b"fresh-synthetic-access"));
    }
}
#[test]
fn rejected_dropbox_refresh_retains_pending_state() {
    let a = App::new();
    a.user("Matteo", "personal", "UTC");
    let mut conf = config(&a);
    for (key, v) in [
        ("refresh_token", "refresh"),
        ("app_key", "key"),
        ("app_secret", "secret"),
    ] {
        conf["backup"]["dropbox"][key] = json!(v);
    }
    write_config(&a, &conf);
    a.dropbox.reply(401, json!({}));
    a.dropbox
        .reply(400, json!({"error":"secret-upstream-body"}));
    let o = a.backup("dropbox");
    assert!(!o.status.success());
    assert!(!String::from_utf8_lossy(&o.stdout).contains("secret-upstream-body"));
    assert_eq!(a.dropbox.requests().len(), 2);
    assert_eq!(a.status()["pending"], 1);
    assert!(a.status()["dropbox_revision"].is_null());
}
#[test]
fn manifest_publication_failure_does_not_acknowledge_database_upload() {
    let a = App::new();
    a.user("Matteo", "personal", "UTC");
    a.dropbox.reply(200, json!({}));
    a.dropbox.reply(400, json!({}));
    assert!(!a.backup("dropbox").status.success());
    assert_eq!(a.status()["pending"], 1);
    assert!(a.status()["dropbox_revision"].is_null());
    let reqs = a.dropbox.requests();
    assert!(reqs[0].body.starts_with(b"SQLite format 3\0"));
    let manifest: Value = serde_json::from_slice(&reqs[1].body).unwrap();
    assert_eq!(
        manifest["sha256"],
        format!("{:x}", Sha256::digest(&reqs[0].body))
    );
}
#[test]
fn snapshot_permission_failure_preserves_finance_and_outbox() {
    use std::os::unix::fs::PermissionsExt;
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    a.create(id(&u), "2026-10-01T00:00:00Z", -1, "groceries", "GBP");
    let before = a.status()["current_revision"].clone();
    let dir = a.dir.path().join("backups");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o500)).unwrap();
    let o = a.backup("dropbox");
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(!o.status.success());
    assert_eq!(a.status()["current_revision"], before);
    assert_eq!(a.rows(id(&u)).len(), 1);
    assert_eq!(a.status()["pending"], 2);
    assert!(a.dropbox.requests().is_empty());
}
#[test]
fn sqlite_disk_full_rolls_back_transaction_revision_and_outbox() {
    let mut a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    a.stop();
    let running = start(&a, &[("POSSERVER_TEST_DB_MAX_PAGES", "1")]);
    let mut failed = false;
    for n in 0..100 {
        let before = a.status();
        let rows = a.rows(id(&u)).len();
        let mut body = manual("2026-10-01T00:00:00Z", -1, "groceries", "GBP");
        body["issuer"] = json!(format!("{n:04}{}", "X".repeat(490)));
        let response = a
            .client
            .post(format!("{}/api/v1/users/{}/transactions", a.url, id(&u)))
            .json(&body)
            .send()
            .unwrap();
        if response.status() == 503 {
            assert_eq!(a.status()["current_revision"], before["current_revision"]);
            assert_eq!(a.status()["pending"], before["pending"]);
            assert_eq!(a.rows(id(&u)).len(), rows);
            failed = true;
            break;
        }
        assert_eq!(response.status(), 201);
    }
    assert!(failed);
    assert!(
        sample(
            &metrics(&a),
            "posserver_db_errors_total",
            &[("reason", "disk_full")]
        ) >= 1.
    );
    drop(running);
}
#[test]
fn restore_rejects_future_schema_and_running_destination_and_keeps_rollback() {
    let a = App::new();
    a.user("Matteo", "personal", "UTC");
    for _ in 0..2 {
        a.dropbox.reply(200, json!({}));
    }
    output_json(&a.backup("dropbox"));
    let source = a.dir.path().join("backups/1.sqlite");
    let mf = a.dir.path().join("backups/1.json");
    let args = [
        "restore",
        "--db",
        a.db.to_str().unwrap(),
        "--snapshot",
        source.to_str().unwrap(),
        "--manifest",
        mf.to_str().unwrap(),
    ];
    assert!(!a.cli(&args).status.success());
    let dest = a.dir.path().join("candidate.sqlite");
    let original = b"previous local database";
    fs::write(&dest, original).unwrap();
    let args = [
        "restore",
        "--db",
        dest.to_str().unwrap(),
        "--snapshot",
        source.to_str().unwrap(),
        "--manifest",
        mf.to_str().unwrap(),
    ];
    output_json(&a.cli(&args));
    let rollback: Vec<_> = fs::read_dir(a.dir.path())
        .unwrap()
        .map(|p| p.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("candidate.rollback-")
        })
        .collect();
    assert_eq!(rollback.len(), 1);
    assert_eq!(fs::read(&rollback[0]).unwrap(), original);
    let mut m: Value = serde_json::from_slice(&fs::read(&mf).unwrap()).unwrap();
    m["schema_version"] = json!(99);
    fs::write(&mf, serde_json::to_vec(&m).unwrap()).unwrap();
    let before = fs::read(&dest).unwrap();
    assert!(!a.cli(&args).status.success());
    assert_eq!(fs::read(dest).unwrap(), before);
}
#[test]
fn monzo_older_replay_never_moves_saved_cursor_backwards() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    a.monzo
        .page(vec![tx("newer", "2026-10-01T00:00:00Z", -1, "SHOP")]);
    a.monzo.page(vec![]);
    a.import(id(&u), Some("2026-09-01T00:00:00Z"), 200);
    a.monzo
        .page(vec![tx("older", "2026-09-01T00:00:00Z", -1, "SHOP")]);
    a.monzo.page(vec![]);
    let r = a.import(id(&u), Some("2026-08-01T00:00:00Z"), 200);
    assert_eq!(r["inserted"], 1);
    assert_eq!(r["last_raw_id"], "newer");
}
#[test]
fn monzo_unicode_literal_space_semantics_and_unsupported_currency() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let mut v = tx("unicode", "2026-10-01T00:00:00Z", -1, "unused");
    v["counterparty"] = json!({"name":"caffè\t shop\u{a0}one"});
    a.monzo.page(vec![v]);
    a.monzo.page(vec![]);
    a.import(id(&u), Some("2026-09-01T00:00:00Z"), 200);
    assert_eq!(a.rows(id(&u))[0]["issuer"], "CAFFÈ\t_SHOP\u{a0}ONE");
    let before = a.status()["current_revision"].clone();
    let mut bad = tx("unsupported", "2026-10-01T00:00:01Z", -1, "POT excluded");
    bad["local_currency"] = json!("JPY");
    a.monzo.page(vec![bad]);
    error(&a.import(id(&u), None, 502), "monzo_invalid_response");
    assert_eq!(a.status()["current_revision"], before);
}
#[test]
fn monzo_objects_cap_and_unsorted_pages_abort() {
    for capped in [false, true] {
        let a = App::new();
        let u = a.user("Matteo", "personal", "UTC");
        if capped {
            let mut c = config(&a);
            c["monzo"]["max_objects"] = json!(1);
            write_config(&a, &c);
            let mut a = a;
            a.stop();
            a.start();
            a.monzo.page(vec![
                tx("a", "2026-10-01T00:00:00Z", -1, "shop"),
                tx("b", "2026-10-01T00:00:01Z", -1, "shop"),
            ]);
            a.import(id(&u), Some("2026-09-01T00:00:00Z"), 502);
            assert!(a.rows(id(&u)).is_empty());
        } else {
            a.monzo.page(vec![
                tx("a", "2026-10-02T00:00:00Z", -1, "shop"),
                tx("b", "2026-10-01T00:00:00Z", -1, "shop"),
            ]);
            a.import(id(&u), Some("2026-09-01T00:00:00Z"), 502);
            assert!(a.rows(id(&u)).is_empty());
        }
    }
}
#[test]
fn legacy_resolution_checks_ownership_and_identity_conflicts() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let d = a.user("Other", "personal", "UTC");
    let p = a.csv(
        "date,category,issuer,amount,currency\n2026-10-01T00:00:00Z,groceries,SHOP,-1.00,GBP\n",
    );
    output_json(&a.migrate(id(&u), &p, &[]));
    let row = a.rows(id(&u))[0].clone();
    let wrong = a.cli(&[
        "reconcile-legacy",
        "--db",
        a.db.to_str().unwrap(),
        "--user",
        id(&d),
        "--transaction",
        id(&row),
        "--monzo-id",
        "provider-a",
        "--account-id",
        "acc_test",
    ]);
    assert!(!wrong.status.success());
    let args = [
        "reconcile-legacy",
        "--db",
        a.db.to_str().unwrap(),
        "--user",
        id(&u),
        "--transaction",
        id(&row),
        "--monzo-id",
        "provider-a",
        "--account-id",
        "acc_test",
    ];
    output_json(&a.cli(&args));
    let before = a.status()["current_revision"].clone();
    let mut conflict = args;
    conflict[8] = "provider-b";
    assert!(!a.cli(&conflict).status.success());
    assert_eq!(a.status()["current_revision"], before);
}

#[test]
fn concurrent_import_is_rejected_without_a_second_provider_request() {
    use tiny_http::{Response, Server};
    let mut a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    a.stop();
    let server = Server::http("127.0.0.1:0").unwrap();
    let upstream = format!("http://{}", server.server_addr());
    let mut c = config(&a);
    c["monzo"]["base_url"] = json!(upstream);
    write_config(&a, &c);
    a.start();
    let (entered_tx, entered) = std::sync::mpsc::channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let provider = thread::spawn(move || {
        let req = server
            .recv_timeout(Duration::from_secs(4))
            .unwrap()
            .unwrap();
        entered_tx.send(()).unwrap();
        let _ = release_rx.recv_timeout(Duration::from_secs(4));
        req.respond(Response::from_string("{\"transactions\":[]}"))
            .unwrap();
        assert!(server
            .recv_timeout(Duration::from_millis(100))
            .unwrap()
            .is_none());
    });
    thread::scope(|scope| {
        let first = scope.spawn(|| a.import(id(&u), Some("2026-09-01T00:00:00Z"), 200));
        entered.recv_timeout(Duration::from_secs(4)).unwrap();
        let r = a.import(id(&u), Some("2026-09-01T00:00:00Z"), 409);
        error(&r, "import_in_progress");
        assert_eq!(
            sample(&metrics(&a), "posserver_monzo_imports_in_flight", &[]),
            1.
        );
        release.send(()).unwrap();
        first.join().unwrap();
    });
    provider.join().unwrap();
    assert_eq!(
        sample(&metrics(&a), "posserver_monzo_imports_in_flight", &[]),
        0.
    );
}
#[test]
fn dropbox_rate_limit_respects_bounded_retry_after() {
    use tiny_http::{Header, Response, Server, StatusCode};
    let a = App::new();
    a.user("Matteo", "personal", "UTC");
    let server = Server::http("127.0.0.1:0").unwrap();
    let mut c = config(&a);
    c["backup"]["dropbox"]["content_base_url"] = json!(format!("http://{}", server.server_addr()));
    write_config(&a, &c);
    let provider = thread::spawn(move || {
        let req = server
            .recv_timeout(Duration::from_secs(4))
            .unwrap()
            .unwrap();
        req.respond(
            Response::from_string("{}")
                .with_status_code(StatusCode(429))
                .with_header(Header::from_bytes("Retry-After", "999").unwrap()),
        )
        .unwrap();
        let now = Instant::now();
        for _ in 0..2 {
            let req = server
                .recv_timeout(Duration::from_secs(4))
                .unwrap()
                .unwrap();
            req.respond(Response::from_string("{}")).unwrap();
        }
        assert!(now.elapsed() >= Duration::from_millis(900));
        assert!(now.elapsed() < Duration::from_secs(4));
    });
    output_json(&a.backup("dropbox"));
    provider.join().unwrap();
    assert_eq!(a.status()["pending"], 0);
}
#[test]
fn snapshot_during_long_uncommitted_write_contains_only_committed_revision() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    a.create(id(&u), "2026-10-01T00:00:00Z", -1, "groceries", "GBP");
    let c = rusqlite::Connection::open(&a.db).unwrap();
    c.execute_batch("BEGIN IMMEDIATE;UPDATE transactions SET amount_minor=-999")
        .unwrap();
    let command = Command::new(bin())
        .args(["backup-dropbox", "--db"])
        .arg(&a.db)
        .arg("--config")
        .arg(&a.config)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let mut process = Running(command);
    let snapshot = a.dir.path().join("backups/2.sqlite");
    wait(|| snapshot.exists());
    let saved = rusqlite::Connection::open(&snapshot).unwrap();
    assert_eq!(
        saved
            .query_row("SELECT amount_minor FROM transactions", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        -1
    );
    assert_eq!(
        saved
            .query_row("SELECT value FROM revision", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    c.execute_batch("ROLLBACK").unwrap();
    for _ in 0..2 {
        a.dropbox.reply(200, json!({}));
    }
    assert!(process.0.wait().unwrap().success());
}
#[test]
fn local_snapshots_coalesce_bursts_and_keep_latest_two() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    for round in 0..3 {
        for n in 0..4 {
            a.create(
                id(&u),
                "2026-10-01T00:00:00Z",
                -1 - n - round,
                "groceries",
                "GBP",
            );
        }
        for _ in 0..3 {
            a.dropbox.reply(200, json!({}));
        }
        output_json(&a.backup("dropbox"));
    }
    let snapshots: Vec<_> = fs::read_dir(a.dir.path().join("backups"))
        .unwrap()
        .map(|p| p.unwrap().path())
        .filter(|p| p.extension().is_some_and(|s| s == "sqlite"))
        .collect();
    assert_eq!(snapshots.len(), 2);
    assert_eq!(a.status()["pending"], 0);
    assert_eq!(a.status()["local_revision"], a.status()["current_revision"]);
}
#[test]
fn csv_malformed_quotes_and_missing_database_dry_run_do_not_write() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    for issuer in ["bad\"quote", "\"unterminated", "\"quoted\"extra"] {
        let p=a.csv(&format!("date,category,issuer,amount,currency\n2026-10-01T00:00:00Z,groceries,{issuer},-1.00,GBP\n"));
        assert!(!a.migrate(id(&u), &p, &[]).status.success());
        assert!(a.rows(id(&u)).is_empty());
    }
    let absent = a.dir.path().join("absent.sqlite");
    let p = a.csv("date,category,issuer,amount,currency\n");
    let o = a.cli(&[
        "migrate-csv",
        "--db",
        absent.to_str().unwrap(),
        "--user",
        id(&u),
        "--file",
        p.to_str().unwrap(),
        "--dry-run",
    ]);
    assert!(!o.status.success());
    assert!(!absent.exists());
}
#[test]
fn graceful_sigterm_releases_server_lock_and_preserves_commits() {
    let mut a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    a.stop();
    let mut run = start(&a, &[]);
    a.create(id(&u), "2026-10-01T00:00:00Z", -1, "groceries", "GBP");
    let start = Instant::now();
    unsafe { libc::kill(run.0.id() as i32, libc::SIGTERM) };
    assert!(run.0.wait().unwrap().success());
    assert!(start.elapsed() < Duration::from_secs(15));
    a.start();
    assert_eq!(a.rows(id(&u)).len(), 1);
}

#[test]
fn newer_write_during_upload_remains_pending_and_remote_order_is_monotonic() {
    use tiny_http::{Response, Server};
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    a.create(id(&u), "2026-10-01T00:00:00Z", -1, "groceries", "GBP");
    let server = Server::http("127.0.0.1:0").unwrap();
    let mut c = config(&a);
    c["backup"]["dropbox"]["content_base_url"] = json!(format!("http://{}", server.server_addr()));
    write_config(&a, &c);
    let (entered_tx, entered) = std::sync::mpsc::channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let provider = thread::spawn(move || {
        let mut manifests = Vec::new();
        for i in 0..4 {
            let mut request = server
                .recv_timeout(Duration::from_secs(4))
                .unwrap()
                .unwrap();
            let mut bytes = Vec::new();
            request.as_reader().read_to_end(&mut bytes).unwrap();
            if i == 0 {
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(4)).unwrap();
            }
            if i % 2 == 1 {
                let m: Value = serde_json::from_slice(&bytes).unwrap();
                manifests.push(m["revision"].as_i64().unwrap());
            }
            request.respond(Response::from_string("{}")).unwrap();
        }
        manifests
    });
    thread::scope(|scope| {
        let first = scope.spawn(|| output_json(&a.backup("dropbox")));
        entered.recv_timeout(Duration::from_secs(4)).unwrap();
        a.create(id(&u), "2026-10-01T00:00:01Z", -2, "groceries", "GBP");
        release.send(()).unwrap();
        assert_eq!(first.join().unwrap()["revision"], 2);
    });
    assert_eq!(a.status()["pending"], 1);
    assert_eq!(a.status()["dropbox_revision"], 2);
    output_json(&a.backup("dropbox"));
    assert_eq!(provider.join().unwrap(), vec![2, 3]);
    assert_eq!(a.status()["pending"], 0);
}
#[test]
fn failed_outbox_insert_rolls_back_finance_mutation() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    let before = a.status();
    let c = rusqlite::Connection::open(&a.db).unwrap();
    c.execute_batch("CREATE TRIGGER reject_outbox BEFORE INSERT ON backup_outbox BEGIN SELECT RAISE(ABORT,'outbox denied'); END;").unwrap();
    a.api(
        "POST",
        &format!("/users/{}/transactions", id(&u)),
        Some(manual("2026-10-01T00:00:00Z", -1, "groceries", "GBP")),
        503,
    );
    assert!(a.rows(id(&u)).is_empty());
    assert_eq!(a.status(), before);
}
#[test]
fn snapshot_disk_full_is_visible_and_retains_queue() {
    let a = App::new();
    a.user("Matteo", "personal", "UTC");
    let o = Command::new(bin())
        .args(["backup-dropbox", "--db"])
        .arg(&a.db)
        .arg("--config")
        .arg(&a.config)
        .env("POSSERVER_TEST_SNAPSHOT_MAX_PAGES", "1")
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert_eq!(a.status()["pending"], 1);
    assert!(a.status()["dropbox_revision"].is_null());
    assert!(a.dropbox.requests().is_empty());
}
#[test]
fn automatic_worker_recovers_pending_queue_after_server_restart() {
    let mut a = App::new();
    a.user("Matteo", "personal", "UTC");
    a.stop();
    let mut c = config(&a);
    c["backup"]["automatic"] = json!(true);
    write_config(&a, &c);
    for _ in 0..2 {
        a.dropbox.reply(200, json!({}));
    }
    a.start();
    wait(|| a.status()["pending"] == 0);
    let m = metrics(&a);
    assert_eq!(
        sample(
            &m,
            "posserver_backup_duration_seconds_count",
            &[("provider", "dropbox")]
        ),
        1.
    );
    assert!(
        sample(
            &m,
            "posserver_db_operation_duration_seconds_count",
            &[("operation", "snapshot")]
        ) >= 1.
    );
}
#[test]
fn month_start_midnight_gap_uses_first_real_instant() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "America/Asuncion");
    a.create(id(&u), "2023-10-01T03:59:59Z", -1, "groceries", "GBP");
    a.create(id(&u), "2023-10-01T04:00:00Z", -2, "groceries", "GBP");
    let r = a.report(id(&u), "2023-10", "GBP", 200);
    assert_eq!(r["current"]["expense_minor"], 2);
    assert_eq!(r["previous"]["expense_minor"], 1);
}
#[test]
fn server_and_restore_locks_cover_database_symlink_aliases() {
    use std::os::unix::fs::symlink;
    let a = App::new();
    a.user("Matteo", "personal", "UTC");
    let alias = a.dir.path().join("alias.sqlite");
    symlink(&a.db, &alias).unwrap();
    for _ in 0..2 {
        a.dropbox.reply(200, json!({}));
    }
    output_json(&a.backup("dropbox"));
    let source = a.dir.path().join("backups/1.sqlite");
    let mf = a.dir.path().join("backups/1.json");
    let o = a.cli(&[
        "restore",
        "--db",
        alias.to_str().unwrap(),
        "--snapshot",
        source.to_str().unwrap(),
        "--manifest",
        mf.to_str().unwrap(),
    ]);
    error(
        &serde_json::from_slice(&o.stdout).unwrap(),
        "server_running",
    );
    assert!(!o.status.success());
}

#[test]
fn dropbox_setup_reads_credentials_from_stdin_and_writes_private_config() {
    use std::{io::Write, os::unix::fs::PermissionsExt};
    let a = App::new();
    a.dropbox.reply(200,json!({"refresh_token":"setup-synthetic-refresh","access_token":"transient-synthetic-access"}));
    let mut child = Command::new(bin())
        .arg("setup-dropbox")
        .arg("--config")
        .arg(&a.config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(br#"{"app_key":"synthetic-key","app_secret":"synthetic-secret","authorization_code":"synthetic-code"}"#).unwrap();
    let o = child.wait_with_output().unwrap();
    let result = output_json(&o);
    assert_eq!(result["configured"], true);
    assert_eq!(result["verified"], false);
    let c = config(&a);
    assert_eq!(
        c["backup"]["dropbox"]["refresh_token"],
        "setup-synthetic-refresh"
    );
    assert!(c["backup"]["dropbox"]["access_token"].is_null());
    assert_eq!(
        fs::metadata(&a.config).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(!String::from_utf8_lossy(&o.stdout).contains("synthetic-secret"));
    assert!(!String::from_utf8_lossy(&o.stdout).contains("synthetic-refresh"));
    assert_eq!(a.dropbox.requests()[0].path, "/oauth2/token");
    assert_eq!(c["monzo"]["links_by_user_name"]["Matteo"], "acc_test");
}
#[test]
fn link_monzo_cli_is_scoped_unique_and_idempotent() {
    let a = App::new();
    let u = a.user("Other", "personal", "UTC");
    let before = a.status()["current_revision"].as_i64().unwrap();
    let run = |user: &str, account: &str| {
        a.cli(&[
            "link-monzo",
            "--db",
            a.db.to_str().unwrap(),
            "--user",
            user,
            "--account-id",
            account,
        ])
    };
    assert_eq!(
        output_json(&run(id(&u), "account-new"))["revision"],
        before + 1
    );
    assert_eq!(
        output_json(&run(id(&u), "account-new"))["revision"],
        before + 1
    );
    let other = a.user("Other2", "personal", "UTC");
    assert!(!run(id(&other), "account-new").status.success());
    assert_eq!(
        a.api("GET", &format!("/users/{}", id(&u)), None, 200)["user"]["monzo_linked"],
        true
    );
}
#[test]
fn explicit_retry_wakes_worker_with_automatic_disabled() {
    let mut a = App::new();
    a.user("Matteo", "personal", "UTC");
    a.stop();
    let c = config(&a);
    write_config(&a, &c);
    a.start();
    assert_eq!(a.status()["pending"], 1);
    assert!(a.dropbox.requests().is_empty());
    for _ in 0..2 {
        a.dropbox.reply(200, json!({}));
    }
    a.api("POST", "/backups/retry", Some(json!({})), 202);
    wait(|| a.status()["pending"] == 0);
    assert_eq!(a.status()["current_revision"], 1);
}
#[test]
fn dropbox_retains_five_revisions_and_retries_failed_retention() {
    let a = App::new();
    let u = a.user("Matteo", "personal", "UTC");
    for revision in 1..=6 {
        if revision > 1 {
            a.create(
                id(&u),
                "2026-10-01T00:00:00Z",
                -revision,
                "groceries",
                "GBP",
            );
        }
        a.dropbox.reply(200, json!({}));
        a.dropbox.reply(200, json!({}));
        if revision == 6 {
            a.dropbox.reply(400, json!({}));
        }
        output_json(&a.backup("dropbox"));
    }
    assert_eq!(a.status()["pending"], 0);
    assert_eq!(a.status()["dropbox_revision"], 3);
    assert_eq!(a.status()["last_error"]["code"], "retention_failed");
    for _ in 0..2 {
        a.dropbox.reply(200, json!({}));
    }
    a.dropbox.reply(
        409,
        json!({"error":{".tag":"path_lookup","path_lookup":{".tag":"not_found"}}}),
    );
    output_json(&a.backup("dropbox"));
    assert!(a.status()["last_error"].is_null());
    let requests = a.dropbox.requests();
    let deletions: Vec<_> = requests
        .iter()
        .filter(|r| r.path == "/2/files/delete_v2")
        .collect();
    assert_eq!(deletions.len(), 2);
    for d in deletions {
        assert_eq!(
            serde_json::from_slice::<Value>(&d.body).unwrap()["path"],
            "/posserver/revisions/1.sqlite"
        );
    }
}
#[test]
fn known_newer_remote_revision_refuses_stale_snapshot() {
    let a = App::new();
    a.user("Matteo", "personal", "UTC");
    let c = rusqlite::Connection::open(&a.db).unwrap();
    c.execute(
        "INSERT INTO operational VALUES('dropbox_revision','99')",
        [],
    )
    .unwrap();
    let o = a.backup("dropbox");
    assert!(!o.status.success());
    error(
        &serde_json::from_slice(&o.stdout).unwrap(),
        "stale_snapshot",
    );
    assert!(a.dropbox.requests().is_empty());
    assert_eq!(a.status()["pending"], 1);
}

#[test]
fn duplicate_user_names_are_allowed_and_bootstrap_account_binds_once() {
    let a = App::new();
    let one = a.user("Matteo", "personal", "UTC");
    let two = a.user("Matteo", "personal", "UTC");
    assert_ne!(id(&one), id(&two));
    assert_eq!(one["monzo_linked"], true);
    assert_eq!(two["monzo_linked"], false);
    assert!(a.rows(id(&two)).is_empty());
}

#[test]
fn restore_rejects_incomplete_schema_even_with_valid_hash_and_sqlite_integrity() {
    let a = App::new();
    let source = a.dir.path().join("incomplete.sqlite");
    let c = rusqlite::Connection::open(&source).unwrap();
    c.execute_batch("CREATE TABLE revision(value INTEGER);INSERT INTO revision VALUES(1);PRAGMA user_version=1;").unwrap();
    drop(c);
    let mf = a.dir.path().join("incomplete.json");
    fs::write(&mf,serde_json::to_vec(&json!({"revision":1,"schema_version":1,"sha256":format!("{:x}",Sha256::digest(fs::read(&source).unwrap())),"created_at":"2026-10-01T00:00:00Z"})).unwrap()).unwrap();
    let dest = a.dir.path().join("destination.sqlite");
    fs::write(&dest, b"existing destination").unwrap();
    let o = a.cli(&[
        "restore",
        "--db",
        dest.to_str().unwrap(),
        "--snapshot",
        source.to_str().unwrap(),
        "--manifest",
        mf.to_str().unwrap(),
    ]);
    assert!(!o.status.success());
    assert_eq!(fs::read(dest).unwrap(), b"existing destination");
}
