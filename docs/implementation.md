# Implementation handover — 2026-10-02

The user authorized implementation after the design-only phase. The Rust service, embedded
mobile UI, migration/reconciliation/setup CLIs, backups and monitoring are implemented locally.
Native ARM64 phone deployment and basic lifecycle checks are verified below. No real bank request, Dropbox
authorization has been performed.

## Implemented behavior

- Schema-v1 SQLite migrations with WAL, FULL synchronous durability, foreign keys, provider
  identity uniqueness, soft deletion, optimistic versions and atomic revision/outbox commits.
- Explicit user-scoped API, bilingual categories and controls, stable keyset pagination,
  directed exact decimal FX, per-transaction rounding, signed accounting and DST calendar reports.
  Reports query the relevant indexed range and aggregate categories/months in one pass.
- Monzo paging through empty termination, ID cursors and microsecond instants; literal-space
  Python issuer normalization/exclusions; atomic validation, deduplication, scoped cursors,
  edit/tombstone preservation, legacy ambiguity stops and explicit wider replay.
- Strict full-file CSV preview/migration with original byte hashes, aliases, provenance and
  ordinal identities; identity reconciliation CLI and immutable account-binding CLI.
- Verified/fsynced immutable online SQLite snapshots, per-commit durable outbox, serialized
  Dropbox jobs, bounded refresh/retries/timeouts, durable operational counters, restart
  recovery, newer-revision ordering, local/remote retention and validated atomic restore/rollback.
- Embedded English/Italian mobile HTML/CSS/JS with exact decimal entry, local-noon dates,
  reports/charts, directed FX settings, conflict handling, one-use token clearing and backup state.
  Response generation checks prevent old requests from changing a newly selected user's view.
- Cached Prometheus endpoint, HTTP/DB/import/job metrics, readiness/stale-state handling and
  native cached process CPU/RSS. The scrape job and Grafana dashboard are installed in the
  existing phome monitoring stack. Operator setup steps are in USER.md.

## Automated verification

The unchanged original Rust contract suite has **50 passing tests**, including 48 tests exercised
against a real service and two specification tests. Product unit tests add **4 passing checks**.
`tests/operational.rs` adds **42 passing black-box checks**; none are ignored. Original browser
flows remain intact: all **13 browser tests pass**, including the original 10 and three new
large-money, local-timezone and late-response checks.

`scripts/check` builds the binary and runs formatting, warning-free product/new-test Clippy,
all 96 Rust checks, JS syntax and an isolated Chromium browser server. Both debug and optimized
service binaries passed all 92 independent Rust contracts/operational checks and all 13 browser
flows; the four product unit checks also pass. The preserved contract
file has one pre-existing style-only Clippy warning, so its runtime behavior is tested without
rewriting it for lint preferences. Cargo/npm lockfiles pin dependency resolution.

Fault/operational coverage includes:

- Process exit after commit, snapshot and upload, exact snapshot replay and durable queue recovery.
- Real SQLite page exhaustion for domain writes and snapshots; failed outbox insertion;
  snapshot permission denial, external busy writer and consistent snapshot during an uncommitted write.
- Simultaneous versioned edits and imports, byte/manifest hashing, late writes during upload,
  coalescing, pending protection, latest-two retention, stale-known-revision rejection and recovery.
- Dropbox refresh success/failure, bounded Retry-After, partial manifest publication, interrupted
  job retry state, offline OAuth setup and private config persistence. Already-removed retention
  objects are idempotent according to the [Dropbox API specification](https://github.com/dropbox/dropbox-api-spec/blob/main/files.stone).
- Running-server/symlink restore locks, future/incomplete schema/corruption/hash rejection and rollback files.
- Prometheus content type/types/baseline zeros, bounded labels, request duration/in-flight cleanup,
  commit-only import counts, degraded readiness, stale cache, read-only scrapes, durable CLI/restart
  counters and status agreement. Unsupported-process omission is implemented but remains a
  platform-specific verification item.
- Legacy CLI ownership/identity conflicts, unsupported upstream currencies, raw Unicode issuers,
  page ordering/caps, older replay, malformed quoting and non-creating dry-run.

## SSH deployment tooling

`scripts/phone` uploads build sources over the existing SSH alias, builds natively with one
Cargo job, installs versioned releases under `$PREFIX/apps/posserver`, and controls the app
through the existing Termux runit daemon. It preserves runtime data, checks readiness, supports
binary rollback, and optionally installs Ansible Vault encrypted JSON through SSH stdin.
Six local deployment checks cover source exclusions, shell syntax, build-failure preservation,
invalid listener rejection, plaintext rejection and the secret configuration pipeline. The UI remains a website, not an APK.

## Native phone verification — 2026-10-02

The release binary built natively on the existing aarch64 Termux phone using locked Cargo
resolution and one job. Initial dependency build took 10m46s; repeat build reused the cache
and completed in 0.67s. First activation exposed runit's new-service registration delay;
the deploy script now waits for the supervise FIFO before enabling the service.

The app is installed under `$PREFIX/apps/posserver`, supervised as `posserver`, and listening
on private Tailscale `100.108.243.40:8080`. Verified repeat deployment, stop with listener removal,
start, restart, rollback and reverse rollback; UI/JS/CSS, health, metrics, users and backup status
all returned HTTP 200 afterward. Native CPU/RSS metrics are present; one empty-database idle
RSS sample was 10,809,344 bytes (10.3 MiB), not an import/load measurement. The database was
empty at initial deployment; Matteo's CSV was imported afterward (see below). No bank request or
cloud upload was made. Data directory is mode 700, config/database 600. Existing phome-monitoring
and sshd supervisor PIDs stayed unchanged. No backup provider credentials were
installed at that point. No reboot or overnight test ran.
The optional Vault workflow has local synthetic pipeline coverage, not a live operator secret test.

## Remaining release gates and limits

- Measured import memory/load and long-running phone reliability.
  Native ARM64 build and basic app lifecycle are verified; the laptop binary remains x86_64.
- Dropbox app/OAuth, first live upload and restore drill. Never retrieve old credentials from
  conversation history.
- Live Monzo chronology, object-ID pagination including timestamp ties, provider history window
  and deliberate wider replay. Mocks establish the implemented contract, not the bank's behavior.
- Disposable live Dropbox upload/download/hash/integrity/restore, app-folder permissions and
  OAuth refresh. Dropbox files above 150 MiB fail visibly; upload sessions are not implemented.
- Long-running phone operation, vendor Android background kills, supervised/physical reboot,
  real disk exhaustion/permissions under Termux and notification setup. These are not established
  by local process crash injection or Chromium tests.
- Remote freshness protection uses the last confirmed revision in durable local state. It does
  not query a live provider manifest before every upload. After restoring an older DB, inspect
  remote state before re-enabling uploads; do not treat a historical local state as proof of the
  provider's current revision. Manifests and downloaded bytes must always agree at restore.

The test fault inputs are process-local opt-in environment variables, never HTTP endpoints:
`POSSERVER_FAULT=after_commit|after_snapshot|after_upload` exits with status 86;
`POSSERVER_TEST_DB_MAX_PAGES` and `POSSERVER_TEST_SNAPSHOT_MAX_PAGES` set real SQLite capacity
limits. Leave them unset in normal operation. File/provider/process fixtures use only synthetic data.

Remote account configuration remains a setup step. No CSV sync is planned.
Repository changes are left reviewable without creating a commit.


## Dropbox CSV import — 2026-10-02

Imported Matteo's `/home/mpossamai/Dropbox/personal_data/transactions.csv` through the native
Termux CLI after a full 1,698-row dry run. The source had its expected header, two supported
currencies and 70 mapped categories; amounts used one or two decimal digits. The opt-in
`--legacy-decimals` parser converts these exactly to minor units and keeps the SHA-256 of the
original bytes for the batch record. Import committed 1,698 rows as source `legacy_csv`, revision 2.
The temporary phone copy was mode 600 and removed after successful commit. Dashboard now opens on
the latest transaction month and filters its category table to rows with positive spending.


Matteo approved a current-rate approximation for the mixed-currency dashboard. Saved directed
rate EUR→GBP `0.85373`, the ECB reference rate dated 2026-10-01. It applies uniformly to the
historical EUR rows, so older converted totals are estimates rather than transaction-date rates.
The September 2026 dashboard report now loads with nine categories that contain spending.


## Matteo's UI recovery — 2026-10-02

Reproduced the reported “service unavailable” page in a mobile browser. The dashboard asked
for `/users/:id/latest`; the API route is `/users/:id/transactions/latest`, causing an HTTP 404
and the generic UI error. Corrected the request and improved backup status to say “Backup not
configured” when neither cloud provider is configured, rather than showing an endless pending
state. Dropbox remains unconfigured.


## Website icon — 2026-10-02

Embedded `assets/cyberpunk_p_icon.svg` as the website's `/icon.svg`, registered it as the browser
tab icon, and added the mark beside the header title. The deploy archive now includes the asset.


## Monthly chart update — 2026-10-02

Replaced monthly spending bars with a responsive line chart with labeled axes, currency ticks,
month labels, keyboard-focusable points, and an exact per-month amount tooltip on hover/focus.


## Current month and income categories — 2026-10-02

Dashboard now defaults to the current month in Matteo's timezone so October updates as the month
progresses. Category rows now appear when either expense or income totals are nonzero, preserving
salary and other income-only categories in their monthly summaries.


## Public Funnel access — 2026-10-02

At Matteo's request, the existing Tailscale Funnel node now publishes the app on
`https://phome-public.tail1b8023.ts.net/` (HTTPS port 443 → private Tailscale
`100.108.243.40:8080`). The existing Grafana Funnel remains on port 8443. Verified the public
health endpoint and app page, plus the Grafana API for the posserver dashboard (16 panels). This
v1 has no authentication, so the public URL permits anyone to view and change data. Funnel
availability still depends on the phone and Termux remaining online.
