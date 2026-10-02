# Fresh-agent handover

Read README.md, docs/design.md, docs/api.md, docs/categories.json, docs/monitoring.md and
tests/README.md before implementation. They contain the agreed behaviour and verification limits.

This repository now contains the implemented service; read docs/implementation.md for verified status.
Follow the current user's task: implement
only when asked to implement; a documentation request does not authorize coding/deployment.
The original design-only restriction applies to that completed phase.

For implementation, preserve contracts/synthetic fixtures. Make tests pass against a real Rust
service, not a fake; add the documented fault/monitoring tests. Root Cargo.toml defines the product and independent tests. Preserve `cargo test --test contract`.
Do not silently skip tests or weaken them to match implementation bugs.

Production account binding, Dropbox credentials and rclone remote are operator setup inputs,
not prerequisites for local coding/tests. Use synthetic providers first. Do not retrieve/copy
credentials from old conversation history. Keep runtime DB/config/credentials out of Git.
Read neighboring pf_tools/phome_srvr when available; the documented contracts stand alone if
absent. Before deployment changes read phome_srvr's current instructions and handover.

Use documented defaults and judgment for routine engineering choices. Missing operator values
need only be requested at setup. Do not claim live backup, ARM deployment or unattended reliability
from local tests. Keep documentation and implemented/tested/deployed status current.
