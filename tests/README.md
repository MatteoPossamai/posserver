# Running and extending acceptance tests

The Rust crate now contains the real product plus independent black-box test infrastructure. `support.rs` launches a supplied
service binary, uses independent temporary directories/ports for each test, and exposes synthetic
Monzo/Dropbox HTTP endpoints. It kills test child processes during cleanup. No real token or
transaction data is used.

```sh
cargo build --locked
cargo test --test contract --no-run
cargo test --test contract spec_
POSSERVER_BIN=/absolute/path/to/posserver cargo test --test contract -- --test-threads=2
```

Build the real binary first and set POSSERVER_BIN to its absolute path; missing binaries still
fail explicitly. The executable implements docs/api.md. Tests do not depend on product module names.
The original contract assertions and fixtures are preserved. `scripts/check` runs all Rust and
browser checks against an isolated service, and `tests/operational.rs` adds fault/metrics coverage.

Covered acceptance behaviour: empty users/per-user settings/categories, scoped transactions and
latest, validation, optimistic edits/deletes, tied timestamp pagination, signed accounting/refunds,
DST/month boundaries, empty trends, exact directed FX, Monzo normalization/filtering/paging/replay,
excluded-only pages, manual-history isolation, errors/rollback/stalls, edit preservation, CSV
normalization/idempotence/dry-run/quoted fields/ambiguity, restart/outbox, Dropbox WAL snapshots,
failed uploads, restore hash/corruption rejection.

The harness is Linux/Termux-oriented. It has 10-second API
timeouts; test configuration must keep upstream retry budget below that. Endpoint mocks confirm importer contract
only, not actual bank pagination. Native ARM64 deployment/lifecycle and local browser flows are verified separately; live providers
remain release gates.

Browser acceptance tests live in `tests/web/`. Run them against a disposable server; they create
users and finance entries. Install the test dependencies with `npm ci` in that directory,
then `npx playwright install chromium`, then:

```sh
POSSERVER_WEB_URL=http://127.0.0.1:8080 npm test
```

The browser server config must bind `WebMonzoSuccess` to `acc_web_success` and
`WebMonzoFailure` to `acc_web_failure` via links_by_user_name. Their import responses are mocked
in-browser; no real Monzo token is used. All other UI tests use real API persistence. UI test IDs
and translation strings are part of the implementation handover below. Chromium install is
required to execute them; source syntax checks alone do not verify the UI.

The original additional release checklist follows. Many local checks are now automated in
operational.rs and the browser suite; see ../docs/implementation.md for exact passing coverage
and external limits. Fault inputs are opt-in process environment variables, never HTTP endpoints:

- Fault hooks for crash immediately after commit/before snapshot, after snapshot/before upload,
  after upload/before acknowledgement; verify queue replay and newest-revision ordering.
- Full disk/permission denied during transaction/outbox/snapshot, busy SQLite, concurrent imports,
  concurrent edit conflicts, snapshot during a long write, outbox coalescing and stale upload.
- Dropbox expired access token refresh success/failure, Retry-After, upload session threshold,
  byte hash against manifest and stale manifest/DB publication failure. No infinite retries.
- Restore valid snapshot then compare user count and all category reports; reject
  future schema, preserve rollback file, refuse replacing a DB used by a running server.
- Legacy reconciliation CLI ownership/identity conflict;
  conflicting same Monzo ID data; replay older range without moving saved cursor backward.
- Unicode whitespace issuers (Python literal-space semantics), unsupported upstream currencies,
  decimal overflow, duplicate/identity FX rates, offset timestamp ties and cursor filter mismatch.
- Mobile browser: create Italian Dad user; labels/errors fully Italian; manual entry/edit/delete;
  switch users and ensure list/latest/chart switch; empty state, current/previous month charts;
  rate changes update totals; Monzo token clears on success/failure and never persists in storage;
  keyboard/touch entry; backup pending/failure/recovered indicator.
- Live bank: verify object-ID pagination including ties, account binding and history window using
  a fresh user-supplied token. Live Dropbox: disposable backup download/restore.
- ARM build, memory measurements, graceful shutdown, supervised restart and long unattended run.

Live-provider, ARM, actual phone monitoring and unattended-operation items remain unverified.
Do not infer them from synthetic provider/process tests.

## Archived design-only verification (2026-10-02)

- Rust suite compiles; rustfmt check passes. Full run: **2 specification tests pass, 48 service
  tests fail explicitly because POSSERVER_BIN is absent**, zero ignored.
- Playwright loads and discovers **10 browser tests**; JS syntax checks pass. No browser flow
  has been executed against an application; no application exists in this repository yet.
- JSON catalogue parses, local Markdown links resolve, git whitespace check passes and no supplied
  Monzo credential patterns appear in repository artifacts.
- Test dependency lockfiles are committed artifacts for reproducibility. A temporary Rust
  toolchain under `/tmp/posserver-cargo` and `/tmp/posserver-rustup` was used for verification;
  no system Rust installation or phone deployment was performed. Browser dependencies are local
  ignored node_modules; Chromium was not installed.

## Current implementation verification

- 50 original Rust contracts, 42 new operational checks and 4 product unit checks pass (96 total);
  zero ignored. Faults include disk full, permission/busy failures, crash/replay, concurrency,
  stale/partial publication, restore and monitoring persistence.
- All 13 Chromium browser tests pass against the real service, including the original 10 flows.
- `scripts/check` runs the whole local check suite. `scripts/browser-tests` starts/cleans up its
  own disposable real server and synthetic Monzo bindings; it never requests real bank data.
- Runtime DB/config/snapshots and browser output remain ignored. The temporary Rust toolchain
  from the design phase was also used locally; no phone toolchain or service was installed.

Read [implementation status](../docs/implementation.md) and [operations](../docs/operations.md)
before setup. Local validation does not establish live backups or ARM/phone reliability.

## Deployment tooling checks

`python3 tests/deployment.py` runs six isolated packaging/configuration/shell checks.
They do not contact the phone or prove ARM compatibility. `scripts/check` includes them.
Actual native build and phone lifecycle checks are recorded in `docs/implementation.md`.
