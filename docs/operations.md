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
has user selection without authentication. The phone deployment is reachable on private Tailscale
and through its public HTTPS Funnel route; anyone who can reach that URL can view and change data.
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

Use the human [Dropbox setup and recovery guide](dropbox.md). It covers app permissions, offline OAuth, phone setup and first-backup verification. For API/config details, see [api.md](api.md). The setup command accepts OAuth input on stdin:

For the phone deployment, prefer the interactive [`scripts/setup-dropbox`](../scripts/setup-dropbox)
helper documented in [USER.md](../USER.md). It prompts for credentials locally and performs the
authorization, configuration, restart, first upload, and status check without a temporary secret file.

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

## Move the server

The app runs on the Termux phone at `$PREFIX/apps/posserver`; its live data is under `$PREFIX/data/posserver`. Monitoring, Tailscale Funnel, and the SSH alias are managed in `../../phome_srvr`. Moving the app means updating both repositories.

1. Set up Termux, SSH, Tailscale, and the `phone` SSH alias on the replacement using [phome setup](../../phome_srvr/docs/setup.md). Keep the existing Dropbox app credentials available through the encrypted config backup.
2. If retaining the existing finance history, deploy posserver, stop it, restore a verified Dropbox snapshot using [the restore steps](dropbox.md#restore-a-snapshot), install the runtime config, then start it. For a new empty database, deploy and install config without restoring. Do not copy a live SQLite file while the old service is running.
3. Deploy from this repository with the replacement phone's current private Tailscale IPv4 address: `scripts/phone deploy --bind PHONE_TAILSCALE_IP:8080`. Check `scripts/phone status` and `/healthz`.
4. In `../../phome_srvr`, update the Prometheus posserver target, then reload its existing configuration. Update the existing Funnel route only if the public hostname/phone changes. Confirm Grafana and public/private health after changes.
5. Keep the old phone and its data until the new service, reports, and Dropbox backup status are confirmed.

Do not add Prometheus, Grafana, an extra scheduler, or a second SSH listener in this repository. Current addresses and installed state belong in the phome handover, not in this generic guide.

## Restore

Stop the service. Download the manifest and the immutable DB named by its `snapshot_path` from
Dropbox. Restore checks exact SHA-256, schema version,
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

The posserver scrape job is installed in phome's existing Prometheus configuration, targeting
private Tailscale `100.108.243.40:8080` every 15 seconds. The provisioned Grafana dashboard is
installed at `/d/posserver` and uses the existing Prometheus datasource. Prometheus also alerts
through the existing Telegram Alertmanager on target-down, app-not-ready, Dropbox backup failure,
and a configured Dropbox queue stuck for 15 minutes. The backup alert is silent until Dropbox is
configured. No second monitoring service was created. See [monitoring.md](monitoring.md) for
metric semantics and diagnostic thresholds.

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
rotation is not yet configured. The current phone's Funnel publishes port 443 to the app; this is
an explicit personal deployment exception. It provides no authentication and is open to the internet.

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

Monitoring and dashboard are installed in phome's existing stack; do not install another scheduler or monitoring stack.

The UI is a responsive website, served by this binary. There is currently no APK, Android
wrapper, web app manifest or offline service worker. Use a mobile browser at the public Funnel URL
or on private Tailscale.
Phone build/deployment verification is recorded separately in `implementation.md`; supervisor
restart does not demonstrate survival of Android vendor kills, reboot or overnight operation.
