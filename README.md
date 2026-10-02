# posserver

Rust family finance service with a mobile English/Italian web interface, SQLite storage,
exact currency conversion, a versioned API, one-use Monzo imports, CSV migration, durable
Dropbox backups and Prometheus metrics. Google Drive support remains paused.

```sh
cargo build --locked
mkdir -p data/posserver
cp config/example.json data/posserver/config.json
./target/debug/posserver serve --db "$PWD/data/posserver/database.sqlite" \
  --bind 127.0.0.1:8080 --config "$PWD/data/posserver/config.json"
```

Open `http://127.0.0.1:8080` and create a user. No bank or backup credentials are needed
for manual entries and reports. Desktop requires `--db` or `POSSERVER_DATA_DIR`; Termux
also defaults to `$PREFIX/data/posserver/database.sqlite`. The UI is embedded in the binary.

Read [Matteo's operator setup](USER.md), [setup and operation](docs/operations.md), [API/CLI contracts](docs/api.md),
[design](docs/design.md), [categories](docs/categories.json), [monitoring](docs/monitoring.md),
and [implementation status](docs/implementation.md).

```sh
POSSERVER_BIN="$PWD/target/debug/posserver" cargo test --locked --all-targets -- --test-threads=2
cd tests/web
npm ci
npx playwright install chromium
cd ../..
scripts/check
```

`scripts/check` builds and tests the real service, including a disposable browser server.
The original `cargo test --test contract` interface is preserved. Missing binaries or
browser dependencies fail explicitly. Fixtures are synthetic; tests do not establish live
provider correctness or phone reliability. Native ARM64 deployment and basic lifecycle checks
are verified separately in [implementation status](docs/implementation.md).

Build and control the phone app with `scripts/phone deploy`, `start`, `stop`, `status` and
`rollback`; see [SSH deployment](docs/operations.md#ssh-deployment-and-supervision).
