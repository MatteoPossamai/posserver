# Setup and operation

## Local and Termux runtime

Build with `cargo build --release --locked`. On Termux install Rust, clang and build tools
through `pkg`, confirm `uname -m` is `aarch64`, and build natively. The laptop binary is x86_64;
it is not an Android executable. Native ARM64 build and basic lifecycle are verified in
[implementation.md](implementation.md); long-running phone operation remains a release gate.

Keep the database, config and snapshots together in private, ignored storage:
`$PREFIX/data/posserver` on Termux, or this repository's `data/posserver` for local use.
Copy `config/example.json` there, then run:

```sh
posserver serve --db PATH/database.sqlite --config PATH/config.json --bind 127.0.0.1:8080
```

The default listener is `0.0.0.0:8080`; use an explicit bind for local tests. This v1 intentionally
has user selection without authentication. Use the agreed private Tailscale access for phone setup.
Created DB/snapshots/config files use mode 600 and new data directories use mode 700.
Do not share a backup directory between different databases.

The server uses two async runtime workers, bounded blocking database/provider work, SQLite WAL,
foreign keys, FULL synchronous writes and a 1.5-second busy timeout. A commit and its backup outbox
entry share one SQLite transaction. GET reports hold a coherent read snapshot. SIGTERM/SIGINT stop
accepting work, signal jobs to stop, allow eight seconds of request draining and at most five
additional seconds of runtime shutdown. Committed outbox work survives interruption.

## Monzo binding and migration

Create the personal user first, obtain its immutable `id` from `GET /api/v1/users`, then bind:

```sh
posserver link-monzo --db PATH/database.sqlite --user USER_UUID --account-id ACCOUNT_ID
```

Alternatively configure `monzo.links_by_user_id`; startup persists new bindings in one domain
commit. Name mappings are only bootstrap/test convenience and are evaluated on user creation.
The same account cannot belong to two users. Dad needs no Monzo link.

Preview the original CSV before migration; source bytes are never rewritten:

```sh
posserver migrate-csv --db PATH/database.sqlite --user USER_UUID --file SOURCE.csv \
  --monzo-account-id ACCOUNT_ID --dry-run
posserver migrate-csv --db PATH/database.sqlite --user USER_UUID --file SOURCE.csv \
  --monzo-account-id ACCOUNT_ID
```

Amounts follow the agreed strict format with exactly two fractional decimal digits. Errors identify
CSV lines; inspect them before proceeding. A second distinct file is rejected. Identical-looking
rows remain separate. Preserve the original file for audit. Ambiguous matches stop the whole
Monzo import; resolve provider identity explicitly with `reconcile-legacy`, then repeat the fetch.
The Dropbox `personal_data/transactions.csv` source uses one or two fractional digits. For that
source only, opt into `--legacy-decimals`; values are scaled exactly with integer arithmetic, and
the importer still hashes the untouched original CSV bytes. The default importer remains strict.

Paste a current access token into the linked user's import form. Choose an explicit start date
when there is no migrated seed or prior cursor. It is sent only in that request and is cleared
from the form on completion/error. No Monzo refresh or token persistence exists. Wider replay can
be requested explicitly; it does not move the saved cursor into an older range. The provider's
history window and object-ID pagination must be checked live before claiming a complete import.

## Dropbox OAuth and backups

Use the app creation/offline OAuth instructions in [design.md](design.md#backup-durability-and-provider-setup).
Enable content read/write and metadata write for retention. Request offline access when obtaining
an authorization code. Feed the following JSON **on stdin**, from a private terminal/input file,
to `posserver setup-dropbox --config PATH/config.json`:

```json
{"app_key":"YOUR_APP_KEY","app_secret":"YOUR_APP_SECRET","authorization_code":"FRESH_CODE","redirect_uri":"YOUR_REGISTERED_REDIRECT_URI"}
```

Omit `redirect_uri` if your authorization flow omitted it. Never put this input in a committed
file or command arguments. The CLI exchanges the code, stores refresh credentials in mode-600
runtime JSON, and prints only setup status. It does not mark a remote backup verified. This uses
one private JSON config rather than a separate `dropbox.env`; plaintext credentials stay outside Git.

The resulting `backup.dropbox` config contains `app_key`, `app_secret`, `refresh_token` and optional
`access_token`, along with default API/content URLs and root `/posserver`. A console access token
alone is useful for a synthetic first check; offline refresh is needed for unattended operation.
Changed runtime configuration takes effect on server restart; explicit CLI jobs read it each run.

```sh
posserver backup-dropbox --db PATH/database.sqlite --config PATH/config.json
```

Automatic mode drains pending Dropbox revisions on startup/after writes and retries pending work
on a 15-second tick. Each job has at most three attempts per upload, one token refresh, bounded
Retry-After/backoff and three-second HTTP request timeouts. Uploads stream from immutable files.
Above 150 MiB the single-upload path fails visibly with `dropbox_upload_too_large`; upload sessions
are not implemented. This limit is tested without allocating a large body or contacting Dropbox.

Snapshots include committed WAL contents via SQLite online backup, are integrity checked, hashed,
fsynced and atomically published. Their revision is read from the snapshot itself. One cross-process
lock serializes snapshot/upload jobs. The immutable revision DB is uploaded before the manifest;
only then are covered outbox entries acknowledged. A newer write during upload stays queued.
Same-revision retries reuse the exact verified snapshot bytes. Keep the newest two completed local
and Dropbox snapshots; pending local snapshots are protected. Failed remote retention is visible
and retried on a later job. Cleanup failure does not invalidate a confirmed upload.

`GET /api/v1/backups/status` reports local/current/provider revisions and pending/error state.
`POST /api/v1/backups/retry` explicitly retries configured providers even when timed workers are
disabled. With providers absent, automatic mode still produces local verified snapshots.
`automatic:false` disables startup/timed/write-triggered backup work; it still records every outbox
entry. Do not call retry in deterministic tests unless provider responses have been scripted.

## Drive and the existing phone scheduler

Configure rclone outside Git, then add to runtime config:

```json
{"remote":"gdrive:posserver","interval_seconds":3600,"timeout_seconds":30}
```

This object goes under `backup.drive`. Register **one** job in the existing phome interval scheduler:

```sh
posserver backup-drive --db PATH/database.sqlite --config PATH/config.json
```

Run it every 3,600 seconds. Read phome's current handover before installing or changing jobs.
The server catches up once at startup in automatic mode; subsequent hourly work belongs to the
existing scheduler. The command persists its last success, skips calls within the interval,
and uses the same cross-process backup lock. `--force` deliberately bypasses the interval.
`POSSERVER_RCLONE_BIN` overrides the executable for testing.

Drive stores only `REMOTE/database.sqlite` and `REMOTE/manifest.json`, overwritten with `copyto`.
Each subprocess has a bounded timeout (default 30 seconds, clamped to 1–300), one transfer/checker
and bounded rclone retries. Timeout/shutdown kills its process group. Success requires both files;
a partial upload retains the previous confirmed revision. Two-file publication is not atomic:
downloaded bytes must always be checked against the manifest.

## Restore

Stop the service. Download the manifest and the immutable DB named by its `snapshot_path` from
Dropbox, or the fixed DB/manifest pair from Drive. Restore checks exact SHA-256, schema version,
SQLite integrity and DB/manifest revision agreement before modifying the destination:

```sh
posserver restore --db PATH/database.sqlite --snapshot DOWNLOADED.sqlite --manifest manifest.json
```

Running-server locks cover relative paths and symlink aliases. Existing destination bytes are kept
as a `*.rollback-UUID.sqlite` file before atomic replacement. Retain that rollback until checking
users, transaction counts and reports after restart. Do not restore into a running authoritative DB.
A restored older DB must not overwrite a provider revision already recorded as newer.

## Monitoring integration

`/healthz` reports readiness; `/metrics` uses cached state and remains available during DB failure.
Operational state refreshes every 15 seconds and after relevant operations. Scrapes perform no DB
operations or provider requests. CPU/RSS are sampled from the native process; unsupported platforms
omit those metrics rather than fabricate zero values. Start time is always exported.

Merge the job from `config/prometheus.yml` into the **existing** Prometheus configuration only after
verifying listener reachability from Debian PRoot. Import/provision `config/grafana-dashboard.json`
using the existing Prometheus datasource. Set the actual app port/address; loopback is a proposal.
Reuse phome's reload mechanism. These files are prepared artifacts, not installed configuration.
See [monitoring.md](monitoring.md) for metric semantics and diagnostic thresholds.

## SSH deployment and supervision

Use the laptop script with the existing `phone` SSH alias. It uploads only build sources,
builds natively on Android with one Cargo job, and installs versioned binaries under
`$PREFIX/apps/posserver`. Runtime config, database and backups remain under
`$PREFIX/data/posserver`; repeat deployments preserve them. Rust, clang, curl, flock and
termux-services must already be installed on the phone (`pkg install rust clang curl util-linux termux-services`).
The existing Termux runit daemon must be running.

```sh
scripts/phone deploy --bind 100.108.243.40:8080
scripts/phone status
scripts/phone logs
scripts/phone stop
scripts/phone start
scripts/phone restart
scripts/phone rollback
```

The bind above is this phone's private Tailscale address; use the actual address for another
phone. Default deployment binds `127.0.0.1:8080`; access it through
`ssh -L 8080:127.0.0.1:8080 phone`. Start detaches through runit and restarts a crashed
process. Stop persists across supervisor restarts. Build failure leaves the old service running;
activation checks readiness and restarts the previous release if available on failure. Rollback selects the previous
binary without restoring the database. Release sources and build cache remain for incremental
builds; prune unused releases manually. Logs append to private `data/posserver/server.log`;
rotation is not yet configured. Never expose this unauthenticated service through public Funnel.

### Optional encrypted configuration

Full Ansible orchestration is unnecessary for one phone. Ansible Vault can still store an
encrypted runtime JSON file in the repository. Create `config/production.json.vault` with
`ansible-vault create config/production.json.vault`, entering JSON matching `config/example.json`
with the provider credentials. Keep the Vault password outside Git (or enter it interactively).

```sh
scripts/phone configure --vault config/production.json.vault
scripts/phone restart
```

Only Vault ciphertext belongs in Git. This command decrypts into laptop memory and sends JSON
through SSH stdin to an atomic mode-600 phone config file. It does not print credentials.
[Vault protects the committed file at rest](https://docs.ansible.com/projects/ansible/latest/vault_guide/vault.html); the running phone necessarily holds decrypted
credentials. Monzo access tokens remain request-only and must not be added to this file.
Ansible Vault is an optional laptop dependency; no Ansible installation is needed on Android.

Merge `config/scheduler-job.json` as one entry in phome's existing `jobs` array only when Drive
is configured; preserve existing jobs. Monitoring/dashboard/scheduler installation is separate
from app deployment. Do not install another scheduler or monitoring stack.

The UI is a responsive website, served by this binary. There is currently no APK, Android
wrapper, web app manifest or offline service worker. Use a mobile browser on private Tailscale.
Phone build/deployment verification is recorded separately in `implementation.md`; supervisor
restart does not demonstrate survival of Android vendor kills, reboot or overnight operation.
