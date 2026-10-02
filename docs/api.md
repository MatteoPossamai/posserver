# HTTP and CLI contract v1

All API paths begin `/api/v1`. UTF-8 JSON, explicit user IDs, no login or session required.
Unknown user/transaction returns 404, including another user's transaction. JSON unknown fields
are rejected (422). Error shape: `{"error":{"code":"invalid_input","message":"...","details":{}}}`.
Do not expose upstream token/body in errors. Domain write JSON includes `revision`.
GET health is `/healthz` -> 200 `{"status":"ok","schema_version":1}` after migrations.
Return 503 with status `not_ready` if database/service readiness fails. `GET /metrics` is outside
`/api/v1`, returns Prometheus text exposition, and stays available for diagnosing degraded service.
See [the monitoring contract](monitoring.md) for required metrics and collection semantics.

## Users and settings

| Method/path | Request | Response |
| --- | --- | --- |
| GET `/users` | none | 200 `{users:[User]}` ordered name then id |
| POST `/users` | `{name,language,timezone,reporting_currency,category_profile}` | 201 `{user:User,revision}` |
| GET `/users/:u` | none | 200 `{user:User}` |
| PATCH `/users/:u/settings` | `{expected_version,language?,timezone?,reporting_currency?,rates?}` | 200 `{user:User,revision}` |
| GET `/users/:u/categories` | none | 200 `{categories:[{id,en,it}]}` sorted id |

User: `{id,name,language,timezone,reporting_currency,category_profile,version,rates,monzo_linked}`.
monzo_linked is boolean; the UI uses it to show the import card only for linked users.
rates is array `{from:"EUR",to:"GBP",rate:"0.87"}`; PATCH replaces all rates if present.
Initial version 1. Each effective PATCH increments version. Invalid rates/timezones/language
return 422; stale version 409 `version_conflict`. Identity pairs need not be stored;
duplicate pairs/identity values other than 1 rejected. Profile is immutable in v1.
No user delete endpoint in v1; it avoids unspecified cascading finance deletion.

## Transactions

POST `/users/:u/transactions` -> 201 `{transaction:Transaction,revision}`.
Body `{occurred_at,category_id,issuer,amount_minor,currency}`. Timestamp explicit offset;
issuer trimmed nonempty <=500 characters. Manual issuer otherwise preserved, unlike Monzo.
Category must belong to user's profile; currencies GBP/EUR/USD; signed integer amount.

GET `/users/:u/transactions` -> 200 `{transactions:[Transaction],next_cursor:string|null}`.
Query `from`/`to` are UTC/offset RFC3339 instants [from,to); `category_id`; `kind` in
`income|expense|all` (zero only in all); `limit` 1..100 default 50; opaque `cursor`.
Order occurred_at DESC then id DESC. Cursor binds user and filters; incompatible cursor 422.
Newer inserts between pages cannot repeat items. Invalid query/time bounds return 422.

GET `/users/:u/transactions/latest` -> 200 `{transaction:Transaction|null}`.
GET `/users/:u/transactions/:t` -> 200 `{transaction:Transaction}`.
PATCH same path -> 200 `{transaction,revision}` with `{expected_version,...editable fields}`;
editable fields are the POST fields. DELETE same path, JSON `{expected_version}` -> 200
`{deleted:true,revision}`. Initial version 1; PATCH increments; all routes hide tombstones.

Transaction: `{id,user_id,occurred_at,category_id,issuer,amount_minor,currency,source,version,
monzo_transaction_id:null|string}`. Source and identity cannot be patched. Monetary source
values remain unchanged when reporting rates change. Missing field/type/fractional integer,
unsupported currency/category, invalid dates return 422. Stale edits/deletes return 409.

## Reports

GET `/users/:u/reports/month?month=2026-10&currency=GBP` -> 200:

```json
{
  "month":"2026-10", "timezone":"Europe/London", "currency":"GBP",
  "settings_version":1, "rates":[],
  "current":{"expense_minor":1000,"income_minor":200,"net_minor":-800,"count":2},
  "previous":{"month":"2026-09","expense_minor":0,"income_minor":0,"net_minor":0,"count":0},
  "categories":[{"category_id":"groceries","expense_minor":1000,"income_minor":0,"net_minor":-1000,"count":1}]
}
```

categories actually contains every profile category, including zero entries; example abbreviated.
`currency` defaults to reporting currency. Explicit month required; UI chooses current month using
user timezone. January previous is December of preceding year. Count includes zero rows.

GET `/users/:u/reports/trend?from_month=2026-01&to_month=2026-12&category_id=groceries&currency=GBP`
-> 200 `{currency,timezone,settings_version,rates,points:[{month,expense_minor,income_minor,net_minor,count}]}`.
Inclusive month endpoints; maximum 120 months; category optional (all if omitted), validates profile.
Zero-fill missing months. Unknown currency/bad month/inverted ranges 422. Missing directed rate
422 `missing_exchange_rate` with `{from,to}`; no report is returned partially.

## Monzo

POST `/users/:u/imports/monzo`, body `{access_token, since?:RFC3339}` -> 200
`{inserted,duplicates,excluded,reconciled,unknown_categories,last_raw_id:null|string,revision}`.
If since omitted use cursor/seed scoped to selected link; absent both -> 422 `import_start_required`.
Configured link absent -> 409 `monzo_not_linked`; concurrent import -> 409 `import_in_progress`.
Token empty -> 422. 401/403 upstream -> 422 `monzo_token_rejected`; 429 -> 503
`monzo_rate_limited`; timeout/5xx -> 502 `monzo_unavailable`; malformed JSON/object -> 502
`monzo_invalid_response`; cursor stall -> 502 `monzo_pagination_stalled`; legacy collision -> 409
`legacy_match_ambiguous`. Provider error messages/bodies are not forwarded.

No endpoint returns token. Configuring link is operator config/CLI, not arbitrary account choice
in UI. Test config format below fixes synthetic account/user mapping by exact user name; production
setup should assign immutable user IDs once creation is complete. Do not auto-bind all new users.

## Backup status

GET `/backups/status` -> 200 `{current_revision,local_revision,dropbox_revision,drive_revision,
pending,last_error:null|{provider,code},drive_last_success_at:null|string}`.
Unconfigured providers have null revisions and no claimed successful backup. GET does not write.
POST `/backups/retry` -> 202 `{queued:true}` wakes worker, does not increment domain revision.

## Runtime/CLI expected by executable tests

`posserver serve --db PATH --bind 127.0.0.1:PORT --config PATH`
starts until SIGTERM. Config JSON:

```json
{
  "monzo":{"base_url":"http://127.0.0.1:MOCK","links_by_user_name":{"Matteo":"acc_test"}},
  "backup":{"directory":"/tmp/isolated/backups","automatic":false,
    "dropbox":{"content_base_url":"http://127.0.0.1:MOCK","api_base_url":"http://127.0.0.1:MOCK",
      "access_token":"synthetic-dropbox-token","root":"/posserver"},
    "drive":{"remote":"gdrive:posserver","interval_seconds":3600}}
}
```

Endpoint overrides support local mock tests. Config lives outside committed source; example
values are synthetic. `automatic:false` suppresses timed workers for deterministic CLI tests;
domain writes still record backup outbox. Production automatic defaults true. `links_by_user_name`
is test/bootstrap convenience: bind the first matching personal user once; later duplicate names
remain unlinked, and account uniqueness is enforced;
persist resulting immutable link, do not reevaluate name-based binding on each request.

`migrate-csv --db PATH --user ID --file PATH [--monzo-account-id ID] [--dry-run]`:
exit 0 JSON `{inserted,duplicates,dry_run}`; repeated exact source returns inserted 0, duplicates
original row count. Validation exit 2 JSON error, DB unchanged. Dry-run must not write/create
DB if none exists; tests operate on existing DB with known user. Invalid user -> exit 2.

`backup-dropbox --db PATH --config PATH` and `backup-drive --db PATH --config PATH`:
one bounded attempt (with bounded refresh/retry), exit 0 JSON `{revision}` or nonzero JSON error.
Drive runs inside the interval return exit 0 `{revision,skipped:true}` without invoking rclone.
Drive command uses executable from `POSSERVER_RCLONE_BIN` if set (otherwise `rclone` on PATH),
and fixed destination `REMOTE/database.sqlite` then `REMOTE/manifest.json`. It must respect
interval_seconds; `--force` is available for an operator/test run. Lock covers concurrent calls.

`restore --db DEST --snapshot SOURCE --manifest PATH`: stopped-server operation, exit 0 JSON
`{restored:true,revision}` or exit 2 JSON error; failure preserves existing destination. Manifest:
`{revision,schema_version:1,sha256,created_at}`; lowercase SHA-256 hex of exact snapshot bytes.
Test DB revision table: `revision` with integer `value` column, one row. This is the only SQL
storage contract used by tests beyond PRAGMA integrity_check and verifying snapshot readability.

`reconcile-legacy --db PATH --user ID --transaction ID --monzo-id ID --account-id ID`:
bind identity only after checking ownership/uniqueness, one domain commit; exit 0 JSON
`{reconciled:true,revision}` or exit 2 JSON error. It never changes the financial fields.

Unknown CLI option returns nonzero. CLI must not run server automatically. No migration/import
performs real network traffic except the explicit Monzo import endpoint or backup commands.

## Implemented setup extensions

`link-monzo --db PATH --user ID --account-id ID`: persist a unique account binding;
exit 0 `{linked:true,revision}`. An identical repeat is a no-op; conflicts fail with exit 2.

`setup-dropbox --config PATH`: read stdin JSON
`{app_key,app_secret,authorization_code,redirect_uri?}`; exchange for offline OAuth credentials,
atomically save private runtime JSON, exit 0 `{configured:true,provider:"dropbox",verified:false}`.
No credential appears in output or process arguments. Synthetic endpoint overrides remain supported.

`backup.drive.timeout_seconds` bounds each rclone subprocess (default 30, range 1–300).
`automatic:false` suppresses timed/startup/write-triggered work; an explicit HTTP retry still runs.
Status can report `last_error.provider:"local"` for snapshot retention errors; metrics provider
labels remain the fixed `dropbox|drive` set. See operations.md for job bounds and setup limits.
