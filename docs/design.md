# Implementation handover

## Scope and authority

The original design-only phase is complete. The user subsequently authorized implementation.
The API, mobile UI, CLI migration, backups and metrics are now implemented; see
[implementation status](implementation.md) and [operation/setup](operations.md). This document
remains the agreed behavior contract. No phone changes or real provider requests have been made.

Confirmed requirements:

- Rust server; local SQLite under `$PREFIX/data/posserver/database.sqlite` on Termux.
- Mobile web UI now; versioned JSON API is mandatory for a later native Android client.
- No users initially. Create users and choose any from a menu, without login/passwords.
- Per-user language (`en`/`it`), timezone, reporting currency, exchange rates and category profile.
- Dad enters transactions manually; his Italian labels remain exactly as supplied.
- Hardcoded categories, with explicit typo normalization for the existing personal CSV.
- Monthly spending, previous month, category totals, monthly category trends and all transactions.
- Monzo token pasted for one import only; no token persistence or refresh flow.
- Python importer determines local amount/currency, issuer normalization and exclusions.
- One-time CSV migration is a CLI command, never a UI feature.
- One authoritative local DB; per-write Dropbox API backup and scheduled rclone Google Drive backup.
- Prometheus metrics for app, database, imports, backups and process health, integrated with the
  existing phone Prometheus/Grafana. See [monitoring.md](monitoring.md).
- Future domains such as fitness should fit ordinary modules with central domain structs.

No category translation service, user account authentication, native Android client, bank balance
calculation, automatic FX feed, CSV UI, or generic plug-in framework in v1.

## Evidence and unresolved deployment inputs

`../pf_tools/transaction_tracker/get_transactions.py`:
`counterparty.name` when truthy, else description before first pair of spaces; uppercase;
replace literal spaces with underscores. Exclude issuer containing `212` anywhere or beginning
`POT_`. Use integer `local_amount` and `local_currency`, not settlement `amount`/`currency`.
CSV timestamps were truncated to seconds; amounts were converted via floats.
The new implementation uses exact minor units and retains upstream timestamp precision.
Do not add declined/zero/pending filtering: Python does not do that.

CSV inspected in place: 1,698 rows; header `date,category,issuer,amount,currency`; EUR and GBP;
70 observed raw category labels. No personal transaction or credential fixture was copied.
`query.py` used fixed float FX rates. Replace those with exact user-supplied rates.

`../phome_srvr/docs/handover.md` describes Android 13, Termux, ARM64, runit and a Python interval
scheduler. Vendor background kills remain unresolved; this project must not claim to fix them.
Use native Termux, not the monitoring Debian PRoot, for the requested `$PREFIX` data location.
Do not change the neighboring repository or deploy this design.

`token_backend.py` references `monzo_tokens.json`; it was not present in the inspected tree.
Only the account_id is needed for operator setup. Ask for it at setup if absent; never retrieve
previously supplied credentials or configuration from old conversation history.
Do not copy the supplied secret/access/refresh tokens into any artifact. One Monzo account is
assigned explicitly to the user's selected profile; Dad has none. Initial import requires
explicit `since` when there is neither migrated Monzo seed nor prior cursor.

Confirmed backup destinations: `/posserver` in a Dropbox App Folder; proposed rclone remote
name `gdrive:posserver` remains a deployment input. Google Drive holds **one latest backup,
overwritten every hour**, not dated history. Dropbox refresh credentials and Drive remote
configuration will be created during implementation, not this design phase.

## Architecture

Suggested product stack: axum + tokio (two runtime workers initially), reqwest with rustls,
rusqlite with bundled SQLite, serde/serde_json, chrono/chrono-tz, rust_decimal or exact integer
rational arithmetic, csv and uuid. Pin a product Cargo.lock. Do not add an ORM or frontend
build runtime without a concrete reason. Serve static HTML/CSS/JS from the binary. A small
SVG chart rendered from report data is sufficient. Test-only dependency versions in this
repository do not force identical product dependencies.

Suggested modules:

| Module | Responsibility |
| --- | --- |
| `models.rs` | Domain structs/enums: User, UserSettings, Money, Transaction, Category, FxRate, Summary, TrendPoint, MonzoLink, ImportResult, BackupStatus |
| `api/` | Request/response DTOs, validation, error mapping, routes |
| `db/` | Versioned SQL migrations, queries, transactions and revision management |
| `finance/` | Calendar boundaries, category lookup, exact conversion and aggregations |
| `imports/monzo.rs` | Fetch paging, validate, normalize, deduplicate, atomic commit |
| `imports/legacy.rs` | CLI parse/preview, idempotent CSV import and overlap reconciliation |
| `backup/` | Snapshot/outbox, Dropbox worker, scheduled Drive command and recovery |
| `config.rs` | Runtime paths/provider endpoints and configuration |
| `main.rs` | CLI subcommands and process lifecycle |
| `web/` | Bilingual static interface |

Keep domain types in models.rs as requested. Wire DTOs can be separate so future fitness structs
need not carry finance API details. Add fitness tables/routes/modules through explicit migrations;
avoid an untyped universal data table. All synchronous SQLite work runs on bounded blocking
workers, never blocking async request threads. One writer serializes mutations; short-lived reads
use separate connections. No network operation occurs inside a SQLite write transaction.

## Persistence model

Use UUID strings for API identifiers; 64-bit signed minor-unit amounts internally. JSON amounts
are integers limited to +/-9,007,199,254,740,991 to remain exact in browser JS. GBP/EUR/USD are
v1 currencies (two decimal places); reject other currencies explicitly. Monzo pages containing
unsupported currencies fail atomically rather than guessing their exponent.

Tables and invariants (actual SQL left to implementer):

- `users(id, name, language, timezone, reporting_currency, category_profile, version)`;
  name trimmed, nonempty <=100 chars; duplicate names allowed; enum profiles `personal`/`dad`.
- `fx_rates(user_id, from_currency, to_currency, decimal_rate)` unique directed pair;
  positive decimal strings, <=8 fractional digits; no inverse/chained derivation. Identity is 1.
- `transactions(id, user_id, occurred_at, category_id, issuer, amount_minor, currency,
  source, version, deleted_at, monzo_account_id, monzo_transaction_id, legacy_batch_id,
  legacy_row_ordinal)`; FK user; sources `manual`, `legacy_csv`, `monzo`.
- Partial unique `(user_id, monzo_account_id, monzo_transaction_id)` where ID not null,
  including tombstoned rows, to prevent deleted imported items reappearing.
- `monzo_links(user_id PRIMARY KEY, account_id UNIQUE)`; refuse the same account for two users.
- `monzo_sync(user_id, account_id, last_transaction_id, last_created_at, seed_since)`;
  cursor scoped to link and derived from raw fetched data including excluded transfers.
- `legacy_batches(id, user_id, source_sha256, row_count)` unique `(user_id, source_sha256)`;
  preserve identical rows within a source file as distinct rows, using ordinal identity.
- `revision(singleton, value)` increments once per domain commit, including settings and imports.
- `backup_outbox(revision PRIMARY KEY, state, snapshot_path, sha256, attempts, error_code)`;
  written in same transaction as revision; backup worker bookkeeping does not increment revision.

Indexes: `(user_id, occurred_at, id)` for stable list/cursor reads and latest transaction;
`(user_id, category_id, occurred_at)` for reports. Every query includes user_id; selecting users
is only a client preference, never a mutable global "current user" on the server.

PRAGMAs: foreign_keys ON, WAL, synchronous FULL, bounded busy_timeout. Store UTC RFC3339 with
normalized fractional precision or numeric microseconds; avoid text comparisons of mixed formats.
Validate IANA timezone names. Date-only manual entry is local noon converted to UTC by UI;
API accepts only explicit-offset RFC3339 instants. Reject nonexistent/ambiguous local times in UI
unless a concrete offset is selected. Reports use real local midnight boundaries (DST aware).

Transactions PATCH/DELETE require integer `expected_version`; stale updates return 409.
DELETE soft-deletes; lists/reports/latest exclude tombstones. Provider identity is immutable;
user edits remain on reimport. Monzo duplicates are counted, not overwritten. This preserves
Python append semantics; a new provider ID is required for a new row.

## Category and language contract

`categories.json` is the canonical hardcoded input for implementation and specification tests.
It contains all observed personal CSV labels mapped to stable IDs and bilingual labels, plus
all 17 Dad labels verbatim. Category selection is profile-specific; no custom category CRUD.
Normalize by trim, lowercase and whitespace-to-underscore, then apply explicit aliases. Do not
use fuzzy matching. Closely related but distinct concepts (food/groceries, bills/utilities,
study/university, bonus/salary) stay distinct. Dress/clothes, known spelling mistakes and singular
gift/present variants normalize explicitly. Preserve original category in migration provenance
for audit. `mondo` maps to income; unknown nonempty Monzo category maps to general and increments
`unknown_categories`; unknown CSV category aborts migration with row diagnostics.

Dad labels are not corrected even when tax terminology seems inconsistent. English labels are
best effort; categories use `dad_*` IDs so Italian labels do not merge with personal categories.
UI must translate all controls, empty states, error messages, date/currency formatting and chart
labels with a static en/it dictionary. API returns stable codes plus optional localized text;
clients can localize codes. Transaction issuers/notes are never translated.

## Accounting and reporting

Negative amount = expense; positive = incoming money; zero remains valid. Transaction category
does not change sign semantics. Expense is sum of absolute negative values; income is sum of
positive values; net = income - expense. "Spent" is gross expense, so a positive refund appears
in income rather than silently reducing spending. UI shows both gross spending and net.

Rates are per-user directed current rates. Historical reports are recomputed using the current
settings; v1 does not store historical FX quotations. Return the applied rates/settings version
with each report. Missing conversion returns 422 `missing_exchange_rate`, never partial totals.
Multiply each source transaction's minor units by the exact decimal rate; round each transaction
to nearest target minor unit, ties away from zero, then aggregate. No float math. Detect overflow
and return 422 `amount_overflow`. Example EUR -101 at EUR->GBP 0.5 = GBP -51.

User's timezone determines local month membership. Report intervals are start-inclusive,
end-exclusive. API month is YYYY-MM. Previous month is calendar previous month, including January
to December transition. Zero-fill every month of trend range and every profile category in
category summaries; totals include zero transactions in counts. Latest sorts instant descending,
then id descending and is unrelated to the Monzo cursor.

## Monzo fetch and commit algorithm

Pasted token goes in POST JSON body, upstream Bearer header only, and lives for that request.
No refresh endpoint calls, token files, URL parameters, browser storage or token logging.
Bounded request timeouts; 401/403 maps to `monzo_token_rejected` and asks user to paste a token.
429 returns `monzo_rate_limited` with bounded Retry-After; other upstream failures return
`monzo_unavailable`. Do not advance cursor on any failure.

1. Resolve the selected user's configured link; absent link returns 409. Do not discover/select
   arbitrary accounts with the token. Account ID comes from runtime config/setup, as requested.
2. Serialize imports per link; reject overlapping imports with 409 `import_in_progress`.
3. Initial request: user/account's `seed_since` minus one second, or caller's explicit `since`.
   Manual transactions and Dad's history never affect it. A successful established import starts
   one second before last raw upstream timestamp, to cover the entire CSV second boundary.
4. GET `/transactions?account_id=...&since=...&limit=100`. Validate chronological page order (preserve provider order for ties); use last raw returned
   object ID as subsequent `since` cursor. Reject incompatible
   ordering rather than guessing. Timestamp +1 second is forbidden. ID cursor handles >100 items
   at exactly the same timestamp. Continue until empty, even on a short page. Terminate on repeated
   cursor/nonempty page with no new IDs, with `monzo_pagination_stalled` and no commit.
5. Validate every object before ignoring it: ID, timestamp, category, local integer amount,
   supported local currency and issuer inputs. Empty counterparty falls back to description;
   missing both fails. Accept timestamps with/without fractions and explicit offsets, retaining
   microseconds. Only the issuer transformation/exclusions are byte-for-byte Python-compatible.
6. Stage normalized results in memory (bounded 100,000 objects / configurable cap); exceeding
   cap aborts without cursor advance. Count unique raw IDs; repeated IDs with conflicting data
   fail validation. Excluded transfers count and advance raw cursor but never create finance rows.
7. In one DB transaction: reconcile migrated rows as below, insert unseen IDs, preserve edited
   and deleted rows, save raw cursor, increment revision and enqueue backup. Network failures,
   malformed later pages or ambiguous legacy matches leave finance rows/cursor unchanged.
8. Return inserted/duplicates/excluded/reconciled counts, unknown_categories and last_raw_id.
   Empty initial fetch returns null cursor; empty established fetch preserves old cursor. An
   import with no state change does not increment revision or enqueue a backup.

Re-fetching the boundary handles ties and repeats; it cannot promise access to arbitrarily old
late-arriving transactions. Provide explicit `since` on a later import for deliberate wider replay,
without replacing the saved cursor with an older cursor. Document provider history constraints.
[Monzo documents ID/timestamp pagination and a maximum page size of 100](https://docs.monzo.com/#pagination).
[Historical access may become limited to 90 days five minutes after authentication](https://docs.monzo.com/#list-transactions).
Do not silently claim complete migration from an API history window that cannot be fetched.
Live verification of chronological ordering and object-ID tie pagination is a release gate;
mocks encode the proposed provider contract, not proof of live behaviour.

## One-off legacy migration and duplicates

CLI: `posserver migrate-csv --db PATH --user ID --file PATH [--monzo-account-id ID]
[--dry-run]`. Never touch or rewrite the source CSV. Strict header, RFC3339 date, exact signed
two-decimal amount, supported currency, nonempty issuer and known category. Allow BOM and CRLF;
reject extra/missing columns, fractional pennies, NaN and malformed quoting with line details.
Parse and validate the entire file before one atomic commit. Hash original file bytes; identical
file reimport is a no-op. A different file for a user with a prior batch returns 409 at CLI exit
level; this is a one-off migration, not ongoing incremental CSV ingestion.

Preserve duplicate-looking original rows as distinct ordinal rows. Do not deduplicate solely by
date/amount/issuer; legitimate identical purchases exist. With `--monzo-account-id`, bind the
account and seed from latest imported row only (the legacy boundary, not arbitrary manual rows).
Normal first fetch includes the full boundary second. For wider replay, match provider normalized
issuer, signed minor amount, local currency and timestamp truncated to UTC seconds against that
user's legacy rows; category is not a key because user corrections may differ from provider.
Exactly one matching unbound row: attach provider ID, preserve original finance fields. A bound
row with same provider ID is duplicate. Multiple candidates, or a conflicting provider ID on
the sole candidate: abort whole import with 409 `legacy_match_ambiguous` and a diagnostic row ID
list (never merge or discard automatically). Resolve via CLI
`posserver reconcile-legacy --db PATH --user ID --transaction ID --monzo-id ID --account-id ID`;
repeat fetch afterward. Not a UI migration workflow.

CSV-to-provider identity is not recoverable with certainty from five fields. This explicit
ambiguity stop is necessary to satisfy no unintended duplicates without erasing valid rows.

## Backup durability and provider setup

Every successful domain write increments a global revision and enqueues that revision atomically.
The HTTP response includes revision; it means local commit, not remote upload completion.
Offline Dropbox must never roll back a user's entry. Status exposes current/local/Dropbox/Drive
revisions, pending count and last error. UI shows pending/retry status.

One serialized backup worker creates immutable SQLite snapshots using the SQLite online backup
API (never copy just the live main file while WAL is active). Snapshot integrity_check must be
`ok`, plus SHA-256 recorded in a manifest. Rename staged snapshot atomically on the same filesystem
and fsync file and directory before advertising it. A queued revision can be covered by a newer
snapshot after a burst/restart; every commit gets a durable queue item but snapshots may coalesce.
Mark all covered revisions delivered only after successful upload. Drain newest valid state
serially; never let an older upload overwrite a newer remote state. Reject stale manifests.
Snapshot revision is read from snapshot DB, not assumed from a pre-copy counter.

Dropbox default path `/posserver/database.sqlite` relative to the chosen App Folder, plus
`/posserver/manifest.json`. Upload immutable revision snapshot first, then manifest, then update
latest DB if retaining both forms; status advances only after DB and manifest are confirmed.
For v1 minimal storage, use immutable `/posserver/revisions/REV.sqlite` plus manifest pointing
to it; `database.sqlite` is convenience only and never a restore authority without matching hash.
Keep latest two verified Dropbox revisions, pending local snapshots and the latest two completed local snapshots. Delete no pending file. Quota/disk-full errors remain visible; reject new write with
503 `storage_unavailable` if durable local commit/outbox cannot be recorded.

Upload endpoint is `/2/files/upload`, binary body, Bearer header and `Dropbox-API-Arg` JSON with
path, `mode: overwrite`, `autorename: false`, `mute: true`. File sizes above upload endpoint limit
must use upload sessions or fail visibly; never truncate. Retry network/5xx/429 with bounded
exponential backoff + Retry-After; 401 permits one Dropbox refresh then retry; invalid refresh
stays pending with actionable status. This Dropbox refresh is independent of Monzo's deliberately
absent refresh. [Dropbox upload reference](https://docs.dropboxapi.com/dropbox-api/api-reference/user-endpoints/files/upload).

Dropbox setup (implementation phase, no credentials needed now):

1. Create a Scoped access app in the [Dropbox App Console](https://www.dropbox.com/developers/apps),
   choose App Folder, and enable `files.content.write` and `files.content.read`; add metadata
   read/write if retention code calls list/delete endpoints.
2. A generated console access token is useful for an initial upload check but expires. For
   unattended operation, authorize via OAuth code flow with `token_access_type=offline` and the
   app's redirect URI; exchange the code at `https://api.dropboxapi.com/oauth2/token`.
3. Store resulting refresh token/app credentials outside source in mode-600 ignored phone
   `$PREFIX/data/posserver/config.json`; worker gets short-lived access tokens via refresh grant.
   The implemented `setup-dropbox` CLI reads OAuth inputs on stdin into private runtime JSON;
   see [operations.md](operations.md). Do not paste credentials into docs.
4. Upload a disposable synthetic snapshot and verify download/hash/integrity before marking
   configuration complete. Retain the local snapshot through retries.

[Dropbox recommends refresh tokens for background access](https://docs.dropboxapi.com/dropbox-api/docs/oauth).
Do not confuse its working refresh flow with Monzo's broken refresh flow reported by the user.

Drive is `gdrive:posserver` (remote name configurable), every 3,600 seconds, one latest
`database.sqlite` overwritten plus a matching `manifest.json`, no historical directories.
Configure through `rclone config`; use laptop-assisted browser auth if needed on headless phone.
[Official Drive setup](https://rclone.org/drive/). Register `posserver backup-drive` with the
existing phone interval scheduler. Persist last successful time and a process lock; do not run
jobs concurrently. After restart catch up once if overdue, without replaying every missed hour.
Use rclone `copyto` (never mirror/sync/delete local DB), explicit arguments, bounded timeout,
transfers and checkers. Upload a fresh consistent local snapshot to the fixed DB path, then its
manifest. Nonzero exit retains local snapshot and reports failure; last successful revision/time
advances only after both commands succeed. A failure between DB and manifest leaves a detectable
hash mismatch until retry; restore must reject that pair. Strict atomic two-file publication
is not promised by rclone/Drive. Only one completed DB is retained on Drive, as requested.

Restore is operator CLI only while server stopped: download revision + manifest from either
provider, verify hash/schema/integrity, restore atomically to a separate candidate, keep old DB
as timestamped rollback, then start and verify user/transaction/report counts. Do not pull remote
changes into a running authoritative DB. [SQLite backup API](https://www.sqlite.org/backup.html).

## UI behaviour and extensibility

First load: empty user list with Create user action. User creation requires explicit language,
timezone, currency and category profile. Menu lists all users; selection scopes every URL to ID.
Manual entry displays latest selected-user transaction and signed amount controls expressed
as Income/Expense; users enter positive decimal magnitude. Edit/delete must display affected
row and use version conflict handling. No global mutable selected user on server.

Dashboard: current/previous month, gross spent/income/net, category totals and zero-filled monthly
trend; currency selector uses configured rates. Transaction page supports date/category/sign
filters and stable pagination. Settings edits language, timezone, currency and directed rates;
changing rates immediately changes converted reports, never source amounts. Monzo import card
only for linked users, clears token after completion/failure, shows counts and import errors.
Dad's UI remains fully Italian. Backup indicator shows pending/offline and last successful dates.
Use semantic controls, labels, keyboard navigation and mobile tap targets. No JS framework
required. The original UI acceptance tests now pass against the embedded mobile interface; see tests/README.md.

## Build, supervision and release gates

Phone/Termux implementation instructions: confirm `uname -m` is aarch64; install Termux rust,
clang and build tools using pkg, then `cargo build --release` natively on the phone. Native
Termux packages handle Android linker paths. Laptop x86 binaries cannot be copied as executable
ARM builds. Do not build `aarch64-unknown-linux-gnu` for native Termux; it belongs in Debian PRoot.
Optional laptop cross-build requires Android NDK/linker and
[`aarch64-linux-android`](https://doc.rust-lang.org/rustc/platform-support/android.html), separately tested.

Runtime: one Rust process supervised by a new Termux runit service, graceful SIGTERM, bounded
workers and upload concurrency one; startup reconciles pending backup queues. Bind address/port
are configurable (`0.0.0.0:8080` proposed); use existing Tailscale access. Never disturb monitoring
or existing SSH supervision. Add deployment integration only during implementation, with phone
instructions and rollback. Data stays on Termux internal storage, not Android shared storage.
Desktop requires explicit --db unless POSSERVER_DATA_DIR is set; do not expand literal `$PREFIX`
in paths when unset. Shutdown drains current local transaction and persists retry state, bounded
grace period; remote availability does not block shutdown forever.

Measure idle/peak memory and 10k-row import on ARM; initial goal idle RSS <50 MiB, bounded import
peak <150 MiB, label results measured rather than promised. Confirm restart, disk full, WAL
snapshot integrity, provider timeout, Dropbox replay, hourly Drive retry, restore and UI Italian
flow. Physical reboot/long unattended operation require coordination with phome work.

Implementation order: data/migrations + exact accounting; user/transaction API; legacy CLI;
Monzo paging/identity tests; snapshot/outbox and provider retries; UI; phone compilation and
deployment. Keep tests failing until behaviour is implemented; do not relax tests to match bugs.
Before deployment, bind the supplied account ID to the created user and provision Dropbox/Drive. Contract tests
provide broad acceptance coverage; fault injection and live provider limits remain separate
release checks. See tests/README.md for exact commands and verification limits.

## Browser contract for implementation

`tests/web/ui.spec.mjs` defines stable `data-testid` hooks. Keep these hooks out of visible
product text; they express semantic controls, not framework choices. Forms use native select
controls for language/currency/profile/category/sign/rate pairs, text timezone input, date inputs
for manual date/import start, month input for report month, and numeric/decimal text amount.
User options and row edit/delete hooks include immutable IDs. Selection displays selected-user;
user-menu must allow creating a user. The dashboard initially shows a 12-month range ending at
selected report month, so trend-point-YYYY-MM exists for that month. Financial metric hooks carry
`data-minor` exact target amount, including SVG trend points (accessible equivalents acceptable).
Backup-status carries data-pending and removes backup-error after recovery. Delete confirmation
names the issuer. Token input is emptied after both results; do not store it in browser storage.

Required Italian strings asserted by tests: Transazioni, Nessuna transazione, Nuova transazione,
Salva, Importo non valido. Translate remaining labels/errors by the same static dictionary.
Manual date-only UTC profile entry produces local noon `12:00:00Z` in the test fixture. Avoid
forcing the user to handle timestamps/negative signs; convert UI magnitude/sign before API write.
Browser tests need a disposable server and configured synthetic links for WebMonzoSuccess and
WebMonzoFailure; they never reach the actual Monzo API. Runtime/browser tests now run against the real service. Test hooks do not justify adding
production-only diagnostic endpoints.

## Fresh-agent readiness

Read AGENTS.md and all linked contracts. They suffice to begin local implementation without the
original conversation, using synthetic providers. Metrics are implemented alongside the service. Root Cargo.toml now defines the product and
independent acceptance tests, preserving the contract command. Read implementation.md for current
verification, and keep the documented external release gates separate from local test results.

Actual Monzo account binding, Dropbox app credentials, rclone remote, initial import start date
and production CSV path are operator setup inputs. They are not necessary to code/test locally.
At setup ask for missing values; never retrieve the previously pasted tokens/client secret.
No fully automatic live deployment can be promised without these inputs and live verification.

Aim to complete the agreed app autonomously, but do not promise one-shot success: live Monzo
pagination/history behaviour and phone build/runtime still need verification. Preserve money and
identity invariants when resolving genuine discrepancies; do not invent missing operator values.
Report implementation, automated tests and actual deployment separately.
