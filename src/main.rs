pub const HTTP_ROUTES: &[&str] = &[
    "/",
    "/app.js",
    "/style.css",
    "/api/v1/users",
    "/api/v1/users/:u",
    "/api/v1/users/:u/settings",
    "/api/v1/users/:u/categories",
    "/api/v1/users/:u/transactions",
    "/api/v1/users/:u/transactions/latest",
    "/api/v1/users/:u/transactions/:t",
    "/api/v1/users/:u/reports/month",
    "/api/v1/users/:u/reports/trend",
    "/api/v1/users/:u/imports/monzo",
    "/api/v1/backups/status",
    "/api/v1/backups/retry",
    "unmatched",
];
mod api;
mod backup;
mod config;
mod db;
mod error;
mod finance;
mod imports;
mod metrics;
mod models;
use axum::{
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::any,
    Json, Router,
};
use clap::{Parser, Subcommand};
use config::Config;
use error::{Error, Result};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
#[derive(Parser)]
#[command(version, about = "Local family finance server")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Serve {
        #[arg(long)]
        db: Option<PathBuf>,
        #[arg(long, default_value = "0.0.0.0:8080")]
        bind: String,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    MigrateCsv {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        user: String,
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        monzo_account_id: Option<String>,
        #[arg(long)]
        dry_run: bool,
        /// Import legacy amounts with one or two fractional digits, exactly scaled to minor units.
        #[arg(long)]
        legacy_decimals: bool,
    },
    ReconcileLegacy {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        user: String,
        #[arg(long)]
        transaction: String,
        #[arg(long)]
        monzo_id: String,
        #[arg(long)]
        account_id: String,
    },
    BackupDropbox {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        config: PathBuf,
    },
    BackupDrive {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        force: bool,
    },
    /// Exchange an offline OAuth authorization code from stdin and save private configuration.
    SetupDropbox {
        #[arg(long)]
        config: PathBuf,
    },
    LinkMonzo {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        user: String,
        #[arg(long)]
        account_id: String,
    },
    Restore {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        snapshot: PathBuf,
        #[arg(long)]
        manifest: PathBuf,
    },
}
#[derive(Clone)]
struct App {
    path: PathBuf,
    config: Config,
    metrics: metrics::Metrics,
    capacity: Arc<tokio::sync::Semaphore>,
    wake: Arc<tokio::sync::Notify>,
    retry_wake: Arc<tokio::sync::Notify>,
}
pub fn fault(point: &str) {
    // Explicit opt-in process fault injection, never exposed through HTTP.
    if std::env::var("POSSERVER_FAULT").as_deref() == Ok(point) {
        std::process::exit(86);
    }
}
fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Serve { db, bind, config } => {
            let db = db
                .or_else(|| {
                    std::env::var_os("POSSERVER_DATA_DIR")
                        .map(|p| PathBuf::from(p).join("database.sqlite"))
                })
                .or_else(|| {
                    std::env::var_os("PREFIX")
                        .map(|p| PathBuf::from(p).join("data/posserver/database.sqlite"))
                });
            match db {
                Some(path) => Config::load(config.as_deref()).and_then(|config| {
                    let rt = tokio::runtime::Builder::new_multi_thread()
                        .worker_threads(2)
                        .max_blocking_threads(8)
                        .enable_all()
                        .build()?;
                    let result = rt.block_on(serve(path, bind, config));
                    rt.shutdown_timeout(Duration::from_secs(5));
                    result
                }),
                None => Err(Error::new(422, "database_path_required")),
            }
        }
        Command::MigrateCsv {
            db,
            user,
            file,
            monzo_account_id,
            dry_run,
            legacy_decimals,
        } => imports::migrate(
            &db,
            &user,
            &file,
            monzo_account_id.as_deref(),
            dry_run,
            legacy_decimals,
        ),
        Command::ReconcileLegacy {
            db,
            user,
            transaction,
            monzo_id,
            account_id,
        } => imports::reconcile(&db, &user, &transaction, &monzo_id, &account_id),
        Command::BackupDropbox { db, config } => {
            Config::load(Some(&config)).and_then(|c| backup::run(&db, &c, "dropbox", true, None))
        }
        Command::BackupDrive { db, config, force } => {
            Config::load(Some(&config)).and_then(|c| backup::run(&db, &c, "drive", force, None))
        }
        Command::SetupDropbox { config } => backup::setup_dropbox(&config),
        Command::LinkMonzo {
            db,
            user,
            account_id,
        } => imports::link(&db, &user, &account_id),
        Command::Restore {
            db,
            snapshot,
            manifest,
        } => backup::restore(&db, &snapshot, &manifest),
    };
    match result {
        Ok(v) => {
            if !v.is_null() {
                println!("{v}");
            }
        }
        Err(e) => {
            println!("{}", e.json());
            std::process::exit(2);
        }
    }
}
async fn serve(path: PathBuf, bind: String, config: Config) -> Result<Value> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }
    let path = db::resolve(&path)?;
    let _lock = backup::Lock::acquire(&path.with_extension("server.lock"), false)
        .map_err(|_| Error::new(409, "server_running"))?;
    db::init(&path)?;
    {
        let mut c = db::open(&path)?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut changed = false;
        for (user, account) in &config.monzo.links_by_user_id {
            if !db::user(&tx, user)?.monzo_linked {
                changed = true;
            }
            imports::bind(&tx, user, account)?;
        }
        if changed {
            db::commit_revision(&tx)?;
        }
        tx.commit()?;
    }
    let metrics = metrics::Metrics::new();
    metrics.refresh(&path, &config)?;
    let app = App {
        path,
        config,
        metrics,
        capacity: Arc::new(tokio::sync::Semaphore::new(6)),
        wake: Arc::new(tokio::sync::Notify::new()),
        retry_wake: Arc::new(tokio::sync::Notify::new()),
    };
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let refresh = app.clone();
    let refresh_task = tokio::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(15));
        loop {
            timer.tick().await;
            let a = refresh.clone();
            let _ = tokio::task::spawn_blocking(move || {
                if a.metrics.refresh(&a.path, &a.config).is_err() {
                    a.metrics.failed_refresh();
                }
            })
            .await;
        }
    });
    let worker = app.clone();
    let backup_task = tokio::spawn(async move {
        // Hourly Drive scheduling belongs to phome's existing scheduler. Catch up once on startup.
        if worker.config.backup.automatic && worker.config.backup.drive.is_some() {
            server_backup(worker.clone(), "drive", false).await;
        }
        let mut timer = tokio::time::interval(Duration::from_secs(15));
        loop {
            let manual = tokio::select! {_=timer.tick()=>false,_=worker.wake.notified()=>false,_=worker.retry_wake.notified()=>true};
            if !worker.config.backup.automatic && !manual {
                continue;
            }
            if manual && worker.config.backup.drive.is_some() {
                server_backup(worker.clone(), "drive", true).await;
            }
            if worker.config.backup.dropbox.is_some() {
                server_backup(worker.clone(), "dropbox", false).await;
            } else {
                let a = worker.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    let result = (|| -> Result<()> {
                        let c = db::open(&a.path)?;
                        let current = db::revision(&c)?;
                        if current == 0 || current <= db::op_number(&c, "snapshot_revision")? {
                            return Ok(());
                        }
                        let _lock =
                            backup::Lock::acquire(&a.path.with_extension("backup.lock"), false)?;
                        let start = Instant::now();
                        let result = backup::snapshot(&a.path, &a.config);
                        a.metrics
                            .db_duration
                            .with_label_values(&["snapshot"])
                            .observe(start.elapsed().as_secs_f64());
                        result?;
                        Ok(())
                    })();
                    if let Err(e) = result {
                        if e.code != "job_in_progress" {
                            eprintln!("local snapshot: {}", e.code);
                        }
                    }
                    if a.metrics.refresh(&a.path, &a.config).is_err() {
                        a.metrics.failed_refresh();
                    }
                })
                .await;
            }
        }
    });
    let stopping = app.metrics.stopping.clone();
    let router = Router::new().fallback(any(handler)).with_state(app);
    let shutdown = async move {
        let mut term =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
        tokio::select! {_=tokio::signal::ctrl_c()=>{},_=term.recv()=>{}}
        stopping.store(true, std::sync::atomic::Ordering::Relaxed);
    };
    // Stop accepting immediately on TERM, allow bounded request draining.
    let server = axum::serve(listener, router).with_graceful_shutdown(shutdown);
    // axum's shutdown signal starts draining; an outer signal-based timeout bounds it.
    let mut term =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
    let server = std::future::IntoFuture::into_future(server);
    tokio::pin!(server);
    tokio::select! {
        result=&mut server=>{result?;},
        _=async{tokio::select!{_=term.recv()=>{},_=tokio::signal::ctrl_c()=>{}}}=>{let _=tokio::time::timeout(Duration::from_secs(8),&mut server).await;},
    }
    refresh_task.abort();
    backup_task.abort();
    Ok(Value::Null)
}
async fn server_backup(app: App, provider: &'static str, force: bool) {
    let _ = tokio::task::spawn_blocking(move || {
        let started = Instant::now();
        let result = backup::run(&app.path, &app.config, provider, force, Some(&app.metrics));
        if !result.as_ref().is_ok_and(|v| v["skipped"] == true)
            && !result.as_ref().is_err_and(|e| e.code == "job_in_progress")
        {
            app.metrics
                .backup_duration
                .with_label_values(&[provider])
                .observe(started.elapsed().as_secs_f64());
        }
        if let Err(e) = result {
            eprintln!("{provider} backup: {}", e.code);
        }
        if app.metrics.refresh(&app.path, &app.config).is_err() {
            app.metrics.failed_refresh();
        }
    })
    .await;
}
fn route_label(path: &str) -> &'static str {
    let p: Vec<_> = path.trim_matches('/').split('/').collect();
    match p.as_slice() {
        ["api", "v1", "users"] => "/api/v1/users",
        ["api", "v1", "users", _] => "/api/v1/users/:u",
        ["api", "v1", "users", _, "settings"] => "/api/v1/users/:u/settings",
        ["api", "v1", "users", _, "categories"] => "/api/v1/users/:u/categories",
        ["api", "v1", "users", _, "transactions"] => "/api/v1/users/:u/transactions",
        ["api", "v1", "users", _, "transactions", "latest"] => {
            "/api/v1/users/:u/transactions/latest"
        }
        ["api", "v1", "users", _, "transactions", _] => "/api/v1/users/:u/transactions/:t",
        ["api", "v1", "users", _, "reports", "month"] => "/api/v1/users/:u/reports/month",
        ["api", "v1", "users", _, "reports", "trend"] => "/api/v1/users/:u/reports/trend",
        ["api", "v1", "users", _, "imports", "monzo"] => "/api/v1/users/:u/imports/monzo",
        ["api", "v1", "backups", "status"] => "/api/v1/backups/status",
        ["api", "v1", "backups", "retry"] => "/api/v1/backups/retry",
        [""] => "/",
        ["app.js"] => "/app.js",
        ["style.css"] => "/style.css",
        _ => "unmatched",
    }
}
async fn handler(State(app): State<App>, req: Request) -> Response {
    let path = req.uri().path().to_string();
    if path == "/metrics" && req.method() == "GET" {
        return (
            [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
            app.metrics.text(),
        )
            .into_response();
    }
    if path == "/healthz" && req.method() == "GET" {
        return if app.metrics.ready() {
            Json(json!({"status":"ok","schema_version":1})).into_response()
        } else {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"status":"not_ready"})),
            )
                .into_response()
        };
    }
    let method = match req.method().as_str() {
        "GET" => "GET",
        "POST" => "POST",
        "PATCH" => "PATCH",
        "DELETE" => "DELETE",
        "PUT" => "PUT",
        "HEAD" => "HEAD",
        "OPTIONS" => "OPTIONS",
        _ => "other",
    };
    let route = route_label(&path);
    let _guard = metrics::GaugeGuard::new(app.metrics.gauge("posserver_http_requests_in_flight"));
    let timer = Instant::now();
    let response = handle(app.clone(), req, path, method).await;
    let class = match response.status().as_u16() {
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        _ => "5xx",
    };
    app.metrics
        .http
        .with_label_values(&[method, route, class])
        .inc();
    app.metrics
        .http_duration
        .with_label_values(&[method, route])
        .observe(timer.elapsed().as_secs_f64());
    response
}
async fn handle(app: App, req: Request, path: String, method: &'static str) -> Response {
    if method == "GET" {
        match path.as_str() {
            "/" => {
                return (
                    [("content-type", "text/html; charset=utf-8")],
                    include_str!("../web/index.html"),
                )
                    .into_response()
            }
            "/app.js" => {
                return (
                    [("content-type", "text/javascript; charset=utf-8")],
                    include_str!("../web/app.js"),
                )
                    .into_response()
            }
            "/style.css" => {
                return (
                    [("content-type", "text/css; charset=utf-8")],
                    include_str!("../web/style.css"),
                )
                    .into_response()
            }
            _ => {}
        }
    }
    if !path.starts_with("/api/v1/") {
        return Error::new(404, "not_found").into_response();
    }
    let query = match req.uri().query() {
        Some(s) => match reqwest::Url::parse(&format!("http://localhost/?{s}")) {
            Ok(url) => {
                let mut q = BTreeMap::new();
                for (k, v) in url.query_pairs() {
                    if q.insert(k.into_owned(), v.into_owned()).is_some() {
                        return error::invalid().into_response();
                    }
                }
                q
            }
            Err(_) => return error::invalid().into_response(),
        },
        None => BTreeMap::new(),
    };
    let bytes = match axum::body::to_bytes(req.into_body(), 64 * 1024).await {
        Ok(b) => b,
        Err(_) => return error::invalid().into_response(),
    };
    let body: Value = if bytes.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(_) => return error::invalid().into_response(),
        }
    };
    let permit = match app.capacity.clone().acquire_owned().await {
        Ok(p) => p,
        Err(_) => return Error::new(503, "not_ready").into_response(),
    };
    let a = app.clone();
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let parts: Vec<_> = path.trim_start_matches("/api/v1/").split('/').collect();
        let is_import = parts.len() == 4
            && parts[0] == "users"
            && parts[2] == "imports"
            && parts[3] == "monzo"
            && method == "POST";
        let result = if is_import {
            let r = imports::monzo(&a.path, &a.config, parts[1], &body, Some(&a.metrics));
            r.map(|v| (200, v))
        } else {
            let start = Instant::now();
            let r = api::dispatch(&a.path, &a.config, method, &parts, &query, &body);
            a.metrics
                .db_duration
                .with_label_values(&[if method == "GET" { "read" } else { "write" }])
                .observe(start.elapsed().as_secs_f64());
            r
        };
        if let Err(e) = &result {
            if let Some(reason) = e.db_reason {
                a.metrics.counters.with_label_values(&[reason]).inc();
            }
        }
        if a.metrics.refresh(&a.path, &a.config).is_err() {
            a.metrics.failed_refresh();
        }
        if result.is_ok() && method == "POST" && parts == ["backups", "retry"] {
            a.retry_wake.notify_one();
        } else if result.is_ok() && method != "GET" && a.config.backup.automatic {
            a.wake.notify_one();
        }
        result
    })
    .await;
    match result {
        Ok(Ok((status, v))) => (StatusCode::from_u16(status).unwrap(), Json(v)).into_response(),
        Ok(Err(e)) => e.into_response(),
        Err(_) => Error::new(500, "internal_error").into_response(),
    }
}
