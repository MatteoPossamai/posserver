# Prometheus monitoring contract

The user requires exported metrics to verify that the app, database, imports and backups work.
The endpoint and prepared dashboard are implemented; see [implementation.md](implementation.md).
This document specifies the contract and installs nothing on the phone.

## Endpoint and collection

Expose `GET /metrics` on the application listener, outside `/api/v1`, without login. Return 200
and `Content-Type: text/plain; version=0.0.4; charset=utf-8`, with HELP/TYPE declarations and
trailing newline. Use a Rust metrics library for escaping, counters and histograms.
[Prometheus exposition format](https://prometheus.io/docs/instrumenting/exposition_formats/).

Scrapes read counters and cached state: no provider requests, DB writes, integrity checks or
finance aggregates. Refresh DB/job/file state every 15 seconds and after relevant operations.
Use bounded reads of persistent job state so the server sees separate backup CLI results.
Failed refresh preserves previous values/timestamp; do not claim stale values are current.
Keep /metrics available when the DB is degraded; readiness becomes 0 and /healthz returns 503.
Normal phone scrape overhead should stay below one second, without waiting on the writer lock.
Exclude /metrics and /healthz from application HTTP statistics.

Request/import counters reset on server restart. Revision, backup success timestamps and backup
attempt/retry counts come from durable operational state, including separate CLI runs. Updating
that state must not create a domain revision or recursively enqueue backups. Record duration
of separate CLI jobs as a last-duration gauge; server-only histograms must not claim to include
those jobs. Sample compatible process metrics in native Termux; do not use synthetic PRoot values.
If a process collector is unsupported, document/omit that metric rather than emit fake zeros.

Use seconds/bytes and `_total` for counters. Labels are fixed enums/route templates, never concrete
URLs, user/account IDs, issuers, transactions, category labels, tokens or arbitrary error messages.
Export operational state; finance amounts/balances remain in the reporting API.
[Prometheus naming guidance](https://prometheus.io/docs/practices/naming/).

## Required metrics

Initialize bounded counter combinations at zero. `provider` is `dropbox|drive`. Timestamp gauges
are Unix seconds, 0 before the first recorded event. Revision gauges use 0 before any confirmed
snapshot/upload; check configured/success gauges to distinguish never configured from success.

| Metric | Type / labels | Meaning |
| --- | --- | --- |
| `posserver_build_info` | gauge `{version}` | Constant 1, current application version |
| `process_start_time_seconds` | gauge | Server start time, identifies restarts |
| `process_resident_memory_bytes` | gauge | Server RSS |
| `process_cpu_seconds_total` | counter | Server cumulative CPU usage |
| `posserver_ready` | gauge | Startup complete, DB available, writer accepting work: 1; else 0 |
| `posserver_state_refresh_timestamp_seconds` | gauge | Last successful cached-state refresh |
| `posserver_http_requests_total` | counter `{method,route,status_class}` | Completed application requests; class `2xx|3xx|4xx|5xx` |
| `posserver_http_request_duration_seconds` | histogram `{method,route}` | Complete application request duration, including failures |
| `posserver_http_requests_in_flight` | gauge | Active application requests |
| `posserver_db_available` | gauge | Last bounded SELECT 1/schema check succeeded and is <=45 seconds old |
| `posserver_db_operation_duration_seconds` | histogram `{operation}` | `read|write|snapshot`, includes lock wait |
| `posserver_db_errors_total` | counter `{reason}` | `busy|disk_full|io|constraint|other`; storage failures, not input validation |
| `posserver_database_bytes` | gauge | Main DB file size |
| `posserver_wal_bytes` | gauge | WAL size, 0 if absent |
| `posserver_data_free_bytes` | gauge | Available bytes on DB filesystem |
| `posserver_current_revision` | gauge | Latest committed domain revision |
| `posserver_monzo_imports_total` | counter `{result}` | `success|token_rejected|rate_limited|invalid_response|pagination_stalled|legacy_ambiguous|unavailable|other` |
| `posserver_monzo_import_duration_seconds` | histogram | Accepted imports, complete duration including failure |
| `posserver_monzo_imports_in_flight` | gauge | Active imports |
| `posserver_monzo_transactions_total` | counter `{outcome}` | Committed counts: `inserted|duplicate|excluded|reconciled|unknown_category`; unknown_category can overlap inserted |
| `posserver_monzo_last_success_timestamp_seconds` | gauge | Durable last successful manual import across links |
| `posserver_backup_configured` | gauge `{provider}` | 1 configured, else 0 |
| `posserver_backup_attempts_total` | counter `{provider,result}` | Durable job outcomes, `success|failure`; skipped hourly run is not an attempt |
| `posserver_backup_retries_total` | counter `{provider}` | Durable additional provider attempts within/replaying failed jobs |
| `posserver_backup_duration_seconds` | histogram `{provider}` | End-to-end jobs run inside server only |
| `posserver_backup_last_duration_seconds` | gauge `{provider}` | Durable last job duration, including CLI jobs |
| `posserver_backup_last_attempt_timestamp_seconds` | gauge `{provider}` | Durable latest completed job time |
| `posserver_backup_last_success_timestamp_seconds` | gauge `{provider}` | Durable time after DB and manifest both confirmed uploaded |
| `posserver_backup_last_attempt_success` | gauge `{provider}` | 1 success; 0 failure/never attempted, distinguish using attempt timestamp |
| `posserver_backup_revision` | gauge `{provider}` | Last confirmed remote revision |
| `posserver_backup_pending_revisions` | gauge | Pending durable Dropbox outbox entries |
| `posserver_backup_oldest_pending_timestamp_seconds` | gauge | Oldest pending Dropbox entry time; 0 when queue empty |
| `posserver_snapshot_revision` | gauge | Latest integrity-checked durable local snapshot revision |
| `posserver_snapshot_bytes` | gauge | Latest complete local snapshot size |

HTTP route labels use registered templates, for example `/api/v1/users/:u/transactions`; unknown
routes use `unmatched`, unknown methods use `other`. Histograms: HTTP/DB buckets 0.005, 0.01,
0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10 seconds; import/backup buckets 0.1, 0.5, 1, 5,
15, 30, 60, 120, 300 seconds; both include +Inf. Decrement in-flight gauges on errors/cancellation.
Rolled-back imports must not increment committed transaction counts. Metrics failures must not
turn a successful finance write into a failed write. No stale Monzo-import alert: imports are manual.

## Existing phone monitoring integration

At deployment read current `../phome_srvr/docs/handover.md`. Reuse its Prometheus/Grafana,
provision a `posserver` scrape job, and reload through its existing mechanism. Proposed snippet:

```yaml
- job_name: posserver
  scrape_interval: 15s
  scrape_timeout: 3s
  metrics_path: /metrics
  static_configs:
    - targets: ['127.0.0.1:8080']
```

Verify reachability from Prometheus's Debian PRoot to the native Termux app before using loopback;
otherwise use the verified app Tailscale address. Use the actual configured port. Do not add a
second Prometheus, exporter or scheduler process. This snippet is documentation, not installed config.

Provision a small Grafana app dashboard: target reachability/ready, start time/RSS/CPU, request
rate/error rate/p95 latency, DB/free bytes/WAL, current/local/remote backup revisions, pending age,
provider last success, and Monzo result counts/duration. Phone dashboard retains whole-device metrics.

Initial diagnostic thresholds, to tune against measured phone behaviour:

- Prometheus-generated `up{job="posserver"}=0` for two minutes, or absent samples: unreachable target.
- Ready/DB available=0 or state refresh age>45 seconds for one minute: unready/stuck service.
- Dropbox configured, pending>0 and oldest pending age>300 seconds: committed writes await backup.
- Drive configured and last success older than 7,200 seconds: hourly backups overdue. Never-successful
  configured Drive needs a two-hour grace period from setup/startup before warning.
- Rising DB errors, free bytes<268435456 for five minutes, or sustained RSS over measured budget:
  investigate storage/resources; allow expected import peaks.
- 5xx fraction>5% over ten minutes with >=20 requests: investigate server errors; separate expected 4xx.

Exported metrics and dashboard are required; notification setup is optional. A stopped phone or
Prometheus cannot observe itself; missing samples do not establish healthy application behaviour.

## Validation to add during implementation

Add tests for exposition/content type and baseline zeros; bounded labels; HTTP counters/duration;
in-flight cleanup; DB readiness/stale cache; successful vs rolled-back import counts; backup metrics
from CLI/restarts; never-configured vs confirmed success; failed uploads retaining pending state.
Repeated scrapes must not change domain revision/outbox, contact providers or compute reports.
Compare operational gauges with `/api/v1/backups/status`. Validate a real scrape/dashboard on ARM
and measure overhead. Local automated coverage is recorded in [implementation.md](implementation.md). A real ARM scrape,
dashboard provisioning and measured phone overhead remain deployment verification gates.
