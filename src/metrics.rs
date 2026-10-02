use crate::{config::Config, db, error::Result};
use prometheus::{
    Encoder, GaugeVec, HistogramOpts, HistogramVec, IntCounterVec, IntGaugeVec, Opts, Registry,
    TextEncoder,
};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicI64, Ordering},
        Arc,
    },
    time::Duration,
};
#[derive(Clone)]
pub struct Metrics {
    pub registry: Registry,
    pub last_duration: GaugeVec,
    pub counters: IntCounterVec,
    pub http: IntCounterVec,
    pub http_duration: HistogramVec,
    pub db_duration: HistogramVec,
    pub imports: IntCounterVec,
    pub outcomes: IntCounterVec,
    pub import_duration: HistogramVec,
    pub backup_duration: HistogramVec,
    pub refreshed: Arc<AtomicI64>,
    pub stopping: Arc<AtomicBool>,
    operational: Arc<Vec<(String, IntGaugeVec)>>,
    process: Option<ProcessMetrics>,
}
const GAUGES: &[&str] = &[
    "posserver_ready",
    "posserver_state_refresh_timestamp_seconds",
    "posserver_http_requests_in_flight",
    "posserver_db_available",
    "posserver_database_bytes",
    "posserver_wal_bytes",
    "posserver_data_free_bytes",
    "posserver_current_revision",
    "posserver_monzo_imports_in_flight",
    "posserver_monzo_last_success_timestamp_seconds",
    "posserver_backup_pending_revisions",
    "posserver_backup_oldest_pending_timestamp_seconds",
    "posserver_snapshot_revision",
    "posserver_snapshot_bytes",
];
const PROVIDER_GAUGES: &[&str] = &[
    "posserver_backup_configured",
    "posserver_backup_last_attempt_timestamp_seconds",
    "posserver_backup_last_success_timestamp_seconds",
    "posserver_backup_last_attempt_success",
    "posserver_backup_revision",
    "posserver_backup_attempts_total",
    "posserver_backup_retries_total",
];
impl Metrics {
    pub fn new() -> Self {
        let registry = Registry::new();
        let mut operational = Vec::new();
        for name in GAUGES {
            let g = IntGaugeVec::new(Opts::new(*name, name.replace('_', " ")), &[]).unwrap();
            registry.register(Box::new(g.clone())).unwrap();
            g.with_label_values::<&str>(&[]).set(0);
            operational.push((name.to_string(), g));
        }
        for name in PROVIDER_GAUGES {
            let labels = if *name == "posserver_backup_attempts_total" {
                vec!["provider", "result"]
            } else {
                vec!["provider"]
            };
            let g = IntGaugeVec::new(Opts::new(*name, name.replace('_', " ")), &labels).unwrap();
            if name.ends_with("_total") {
                registry
                    .register(Box::new(DurableCounter(g.clone())))
                    .unwrap();
            } else {
                registry.register(Box::new(g.clone())).unwrap();
            }
            for p in ["dropbox", "drive"] {
                if labels.len() == 2 {
                    for r in ["success", "failure"] {
                        g.with_label_values(&[p, r]).set(0);
                    }
                } else {
                    g.with_label_values(&[p]).set(0);
                }
            }
            operational.push((name.to_string(), g));
        }
        let last_duration = GaugeVec::new(
            Opts::new(
                "posserver_backup_last_duration_seconds",
                "Durable last backup job duration",
            ),
            &["provider"],
        )
        .unwrap();
        for p in ["dropbox", "drive"] {
            last_duration.with_label_values(&[p]).set(0.);
        }
        registry.register(Box::new(last_duration.clone())).unwrap();
        let counters = IntCounterVec::new(
            Opts::new("posserver_db_errors_total", "Database storage errors"),
            &["reason"],
        )
        .unwrap();
        for r in ["busy", "disk_full", "io", "constraint", "other"] {
            counters.with_label_values(&[r]);
        }
        registry.register(Box::new(counters.clone())).unwrap();
        let http = IntCounterVec::new(
            Opts::new(
                "posserver_http_requests_total",
                "Completed application requests",
            ),
            &["method", "route", "status_class"],
        )
        .unwrap();
        registry.register(Box::new(http.clone())).unwrap();
        let buckets = vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1., 2.5, 5., 10.];
        let http_duration = HistogramVec::new(
            HistogramOpts::new(
                "posserver_http_request_duration_seconds",
                "Complete request duration",
            )
            .buckets(buckets.clone()),
            &["method", "route"],
        )
        .unwrap();
        registry.register(Box::new(http_duration.clone())).unwrap();
        for route in crate::HTTP_ROUTES {
            for method in [
                "GET", "POST", "PATCH", "DELETE", "PUT", "HEAD", "OPTIONS", "other",
            ] {
                for class in ["2xx", "3xx", "4xx", "5xx"] {
                    http.with_label_values(&[method, route, class]);
                }
                http_duration.with_label_values(&[method, route]);
            }
        }

        let db_duration = HistogramVec::new(
            HistogramOpts::new(
                "posserver_db_operation_duration_seconds",
                "Database operation duration including wait",
            )
            .buckets(buckets),
            &["operation"],
        )
        .unwrap();
        for o in ["read", "write", "snapshot"] {
            db_duration.with_label_values(&[o]);
        }
        registry.register(Box::new(db_duration.clone())).unwrap();
        let imports = IntCounterVec::new(
            Opts::new("posserver_monzo_imports_total", "Accepted Monzo imports"),
            &["result"],
        )
        .unwrap();
        for r in [
            "success",
            "token_rejected",
            "rate_limited",
            "invalid_response",
            "pagination_stalled",
            "legacy_ambiguous",
            "unavailable",
            "other",
        ] {
            imports.with_label_values(&[r]);
        }
        registry.register(Box::new(imports.clone())).unwrap();
        let outcomes = IntCounterVec::new(
            Opts::new(
                "posserver_monzo_transactions_total",
                "Committed import outcomes",
            ),
            &["outcome"],
        )
        .unwrap();
        for o in [
            "inserted",
            "duplicate",
            "excluded",
            "reconciled",
            "unknown_category",
        ] {
            outcomes.with_label_values(&[o]);
        }
        registry.register(Box::new(outcomes.clone())).unwrap();
        let long = vec![0.1, 0.5, 1., 5., 15., 30., 60., 120., 300.];
        let import_duration = HistogramVec::new(
            HistogramOpts::new(
                "posserver_monzo_import_duration_seconds",
                "Complete import duration",
            )
            .buckets(long.clone()),
            &[],
        )
        .unwrap();
        import_duration.with_label_values::<&str>(&[]);
        registry
            .register(Box::new(import_duration.clone()))
            .unwrap();
        let backup_duration = HistogramVec::new(
            HistogramOpts::new("posserver_backup_duration_seconds", "Server backup jobs")
                .buckets(long),
            &["provider"],
        )
        .unwrap();
        for p in ["dropbox", "drive"] {
            backup_duration.with_label_values(&[p]);
        }
        registry
            .register(Box::new(backup_duration.clone()))
            .unwrap();
        let build = IntGaugeVec::new(
            Opts::new("posserver_build_info", "Application version"),
            &["version"],
        )
        .unwrap();
        build.with_label_values(&[env!("CARGO_PKG_VERSION")]).set(1);
        registry.register(Box::new(build)).unwrap();
        let process = ProcessMetrics::new(&registry);
        let start = prometheus::Gauge::with_opts(Opts::new(
            "process_start_time_seconds",
            "Process startup Unix time",
        ))
        .unwrap();
        start.set(chrono::Utc::now().timestamp() as f64);
        registry.register(Box::new(start)).unwrap();
        Self {
            registry,
            last_duration,
            counters,
            http,
            http_duration,
            db_duration,
            imports,
            outcomes,
            import_duration,
            backup_duration,
            refreshed: Arc::new(AtomicI64::new(0)),
            stopping: Arc::new(AtomicBool::new(false)),
            operational: Arc::new(operational),
            process,
        }
    }
    pub fn gauge(&self, name: &str) -> prometheus::IntGauge {
        self.operational
            .iter()
            .find(|p| p.0 == name)
            .unwrap()
            .1
            .with_label_values::<&str>(&[])
    }
    pub fn provider(&self, name: &str, labels: &[&str]) -> prometheus::IntGauge {
        self.operational
            .iter()
            .find(|p| p.0 == name)
            .unwrap()
            .1
            .with_label_values(labels)
    }
    pub fn ready(&self) -> bool {
        !self.stopping.load(Ordering::Relaxed)
            && chrono::Utc::now().timestamp() - self.refreshed.load(Ordering::Relaxed) <= 45
            && self.gauge("posserver_db_available").get() == 1
    }
    pub fn refresh(&self, path: &Path, config: &Config) -> Result<()> {
        if let Some(process) = &self.process {
            process.refresh();
        }
        let c = db::open(path)?;
        c.busy_timeout(Duration::from_millis(100))?;
        db::check_schema(&c)?;
        let revision = db::revision(&c)?;
        let schema: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if schema != 1 {
            return Err(crate::error::Error::new(503, "storage_unavailable"));
        }
        let pending: i64 = c.query_row(
            "SELECT COUNT(*) FROM backup_outbox WHERE state='pending'",
            [],
            |r| r.get(0),
        )?;
        let oldest: i64 = c.query_row(
            "SELECT COALESCE(MIN(created_at),0) FROM backup_outbox WHERE state='pending'",
            [],
            |r| r.get(0),
        )?;
        // Gather the complete state before publishing; a failure preserves the previous timestamp.
        let keys = ["snapshot_revision", "snapshot_bytes", "monzo_last_success"];
        let values = keys
            .iter()
            .map(|key| db::op_number(&c, key))
            .collect::<Result<Vec<_>>>()?;
        let mut providers = Vec::new();
        for p in ["dropbox", "drive"] {
            let keys = [
                "revision",
                "attempt_at",
                "success_at",
                "attempt_success",
                "success_total",
                "failure_total",
                "retries",
            ];
            let v = keys
                .iter()
                .map(|key| db::op_number(&c, &format!("{p}_{key}")))
                .collect::<Result<Vec<_>>>()?;
            let dur = db::op(&c, &format!("{p}_duration"))?
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.);
            providers.push((p, v, dur));
        }
        self.gauge("posserver_current_revision").set(revision);
        self.gauge("posserver_backup_pending_revisions")
            .set(pending);
        self.gauge("posserver_backup_oldest_pending_timestamp_seconds")
            .set(oldest);
        for (name, v) in [
            "posserver_snapshot_revision",
            "posserver_snapshot_bytes",
            "posserver_monzo_last_success_timestamp_seconds",
        ]
        .iter()
        .zip(values)
        {
            self.gauge(name).set(v);
        }
        self.gauge("posserver_database_bytes")
            .set(std::fs::metadata(path).map(|m| m.len() as i64).unwrap_or(0));
        self.gauge("posserver_wal_bytes").set(
            std::fs::metadata(PathBuf::from(format!("{}-wal", path.display())))
                .map(|m| m.len() as i64)
                .unwrap_or(0),
        );
        if let Ok(bytes) = fs2::available_space(path) {
            self.gauge("posserver_data_free_bytes")
                .set(bytes.min(i64::MAX as u64) as i64);
        }
        for (p, v, dur) in providers {
            self.provider("posserver_backup_configured", &[p]).set(
                if (p == "dropbox" && config.backup.dropbox.is_some())
                    || (p == "drive" && config.backup.drive.is_some())
                {
                    1
                } else {
                    0
                },
            );
            for (name, n) in [
                "posserver_backup_revision",
                "posserver_backup_last_attempt_timestamp_seconds",
                "posserver_backup_last_success_timestamp_seconds",
                "posserver_backup_last_attempt_success",
            ]
            .iter()
            .zip(v.iter())
            {
                self.provider(name, &[p]).set(*n);
            }
            self.provider("posserver_backup_attempts_total", &[p, "success"])
                .set(v[4]);
            self.provider("posserver_backup_attempts_total", &[p, "failure"])
                .set(v[5]);
            self.provider("posserver_backup_retries_total", &[p])
                .set(v[6]);
            self.last_duration.with_label_values(&[p]).set(dur);
        }
        let now = chrono::Utc::now().timestamp();
        self.gauge("posserver_state_refresh_timestamp_seconds")
            .set(now);
        self.gauge("posserver_db_available").set(1);
        self.gauge("posserver_ready").set(1);
        self.refreshed.store(now, Ordering::Relaxed);
        Ok(())
    }
    pub fn failed_refresh(&self) {
        self.gauge("posserver_db_available").set(0);
        self.gauge("posserver_ready").set(0);
    }
    pub fn text(&self) -> String {
        if !self.ready() {
            self.gauge("posserver_ready").set(0);
            self.gauge("posserver_db_available").set(0);
        }
        let mut buffer = Vec::new();
        TextEncoder::new()
            .encode(&self.registry.gather(), &mut buffer)
            .unwrap();
        String::from_utf8(buffer).unwrap()
    }
    pub fn import_result(&self, r: &Result<serde_json::Value>) {
        let label = match r {
            Ok(_) => "success",
            Err(e) => match e.code {
                "monzo_token_rejected" => "token_rejected",
                "monzo_rate_limited" => "rate_limited",
                "monzo_invalid_response" => "invalid_response",
                "monzo_pagination_stalled" => "pagination_stalled",
                "legacy_match_ambiguous" => "legacy_ambiguous",
                "monzo_unavailable" => "unavailable",
                _ => "other",
            },
        };
        self.imports.with_label_values(&[label]).inc();
        if let Ok(v) = r {
            for (field, outcome) in [
                ("inserted", "inserted"),
                ("duplicates", "duplicate"),
                ("excluded", "excluded"),
                ("reconciled", "reconciled"),
                ("unknown_categories", "unknown_category"),
            ] {
                self.outcomes
                    .with_label_values(&[outcome])
                    .inc_by(v[field].as_u64().unwrap_or(0));
            }
        }
    }
}
pub struct GaugeGuard(prometheus::IntGauge);
impl GaugeGuard {
    pub fn new(g: prometheus::IntGauge) -> Self {
        g.inc();
        Self(g)
    }
}
impl Drop for GaugeGuard {
    fn drop(&mut self) {
        self.0.dec();
    }
}

/// Operational counters live in SQLite and may already be nonzero at startup.
/// Export their cached absolute values through the standard Prometheus collector API.
struct DurableCounter<T: prometheus::core::Collector>(T);
impl<T: prometheus::core::Collector> prometheus::core::Collector for DurableCounter<T> {
    fn desc(&self) -> Vec<&prometheus::core::Desc> {
        self.0.desc()
    }
    fn collect(&self) -> Vec<prometheus::proto::MetricFamily> {
        self.0
            .collect()
            .into_iter()
            .map(|mut family| {
                family.set_field_type(prometheus::proto::MetricType::COUNTER);
                for metric in family.mut_metric() {
                    let value = metric.gauge.take().unwrap().value();
                    let mut counter = prometheus::proto::Counter::default();
                    counter.set_value(value);
                    metric.set_counter(counter);
                }
                family
            })
            .collect()
    }
}

#[derive(Clone)]
struct ProcessMetrics {
    rss: prometheus::Gauge,
    cpu: GaugeVec,
}
impl ProcessMetrics {
    fn sample() -> Option<(f64, f64)> {
        let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
        let fields: Vec<_> = stat.rsplit_once(')')?.1.split_whitespace().collect();
        let user = fields.get(11)?.parse::<f64>().ok()?;
        let system = fields.get(12)?.parse::<f64>().ok()?;
        let resident = fields.get(21)?.parse::<f64>().ok()?;
        // sysconf reads native kernel constants; no PRoot substitutions are used.
        let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if ticks <= 0 || page <= 0 {
            return None;
        }
        Some((resident * page as f64, (user + system) / ticks as f64))
    }
    fn new(registry: &Registry) -> Option<Self> {
        let (rss_value, cpu_value) = Self::sample()?;
        let rss = prometheus::Gauge::with_opts(Opts::new(
            "process_resident_memory_bytes",
            "Native process resident bytes",
        ))
        .unwrap();
        rss.set(rss_value);
        registry.register(Box::new(rss.clone())).unwrap();
        let cpu = GaugeVec::new(
            Opts::new("process_cpu_seconds_total", "Native cumulative CPU seconds"),
            &[],
        )
        .unwrap();
        cpu.with_label_values::<&str>(&[]).set(cpu_value);
        registry
            .register(Box::new(DurableCounter(cpu.clone())))
            .unwrap();
        Some(Self { rss, cpu })
    }
    fn refresh(&self) {
        if let Some((rss, cpu)) = Self::sample() {
            self.rss.set(rss);
            self.cpu.with_label_values::<&str>(&[]).set(cpu);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_cache_is_unready_without_advancing_timestamp() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("db.sqlite");
        db::init(&path).unwrap();
        let metrics = Metrics::new();
        metrics.refresh(&path, &Config::default()).unwrap();
        let recorded = metrics
            .gauge("posserver_state_refresh_timestamp_seconds")
            .get();
        metrics.refreshed.store(recorded - 46, Ordering::Relaxed);
        assert!(!metrics.ready());
        let text = metrics.text();
        assert!(text.contains("posserver_ready 0\n"));
        assert_eq!(
            metrics
                .gauge("posserver_state_refresh_timestamp_seconds")
                .get(),
            recorded
        );
    }
}
