# posserver

A small family finance server. It stores transactions in one SQLite database on the phone and serves a mobile website plus a JSON API.

## How it works

```text
Browser / API client
        │ HTTP
        ▼
   posserver (Rust) ── writes ──► SQLite database
        │                              │
        ├── /metrics                   └── backup queue
        │                                      │
        ▼                                      ▼
phome Prometheus/Grafana                 Dropbox snapshots
```

The database on the phone is the live copy. Each successful change and its backup-queue entry commit together. A background worker makes a checked SQLite snapshot and uploads it to Dropbox. A saved transaction remains saved if Dropbox is offline; the queue retries later.

## API at a glance

The versioned API is under `/api/v1`: users and settings, transactions, monthly reports, Monzo imports, and backup status/retry. `/healthz` reports readiness; `/metrics` serves operational metrics. There is no login. Anyone who can reach the server can view and change its data.

For Monzo, Matteo starts a manual import from the website with a temporary access token. Posserver considers account IDs beginning `acc_`; it uses the account automatically when there is one, and asks Matteo to choose only when there are several. It then requests pages and saves them to SQLite. The first import starts from Matteo's latest saved transaction; later imports use the saved Monzo cursor. The token is not stored and there is no automatic refresh.

## Run locally

```sh
cargo build --locked
mkdir -p data/posserver
cp config/example.json data/posserver/config.json
./target/debug/posserver serve --db "$PWD/data/posserver/database.sqlite" \
  --bind 127.0.0.1:8080 --config "$PWD/data/posserver/config.json"
```

Open `http://127.0.0.1:8080`. On Termux, the default database is `$PREFIX/data/posserver/database.sqlite`. The UI is embedded in the binary.

## Set up the phone

Use [`scripts/phone`](scripts/phone) to deploy and manage the native Termux service. Follow [Dropbox setup and recovery](docs/dropbox.md) to configure and recover backups. If the phone or address changes, follow [move the server](docs/operations.md#move-the-server). Monitoring and public routes are in the neighboring `phome_srvr` repository; do not copy its configuration here.

## More detail

- [Dropbox setup and recovery](docs/dropbox.md) - human operator guide.
- [Phone operation and restore](docs/operations.md) - commands and deployment details.
- [API contract](docs/api.md), [monitoring contract](docs/monitoring.md), and [categories](docs/categories.json) - implementation references.
- [Design](docs/design.md) - original behavior and Monzo planning; preserved as agreed.
- [Implementation status](docs/implementation.md) - what was built and verified.
- [AGENTS.md](AGENTS.md) - instructions for future agents.
