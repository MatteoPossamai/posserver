# posserver instructions

## Read first

Start with README.md for the human model and docs/handover.md if present. This repository has no handover file currently. Read docs/implementation.md for installed and verified state before changing operational claims.

## Human docs and implementation references

- README.md and docs/dropbox.md are for the operator: explain only the moving parts, setup, checks, and recovery in plain language.
- docs/operations.md covers phone commands, deployment, and restoring a database. Keep it actionable.
- docs/api.md and docs/monitoring.md are detailed contracts for implementation work. They are intentionally denser than the human guides.
- docs/design.md preserves the original design and Monzo plan; do not edit it unless the user explicitly asks to change the plan. USER.md is the concise operator guide and should reflect the current workflow.
- docs/implementation.md records actual changes and verification. Separate local tests from phone/provider checks.

Do not copy detailed monitoring configuration from `../phome_srvr`; that repository owns Prometheus, Grafana, and the scheduler. This repo owns only posserver and its application config.

## Runtime boundaries

- The Rust binary and SQLite database run natively in Termux. The phone path is `$PREFIX/data/posserver`; releases are under `$PREFIX/apps/posserver`.
- `scripts/phone` builds on the phone and controls its existing runit service. Preserve runtime config, database, snapshots, and release rollback behavior during deployment changes.
- Dropbox is the only remote backup provider. Google Drive support was removed. Do not reintroduce it without a new request.
- Dropbox OAuth credentials, database files, tokens, and real financial data stay out of Git. Use synthetic fixtures for development.
- Only the personal user named Matteo has the Monzo import UI/API. This is a user-scope rule, not login protection; the app has no authentication.
- Monzo uses a temporary access token supplied per import. First use discovers accounts; only IDs starting `acc_` qualify. A single eligible account is automatic; multiple eligible accounts need a user choice. Start from one second before Matteo's latest saved transaction, then keep the provider cursor. Commit account binding and imported rows only after all pages validate.
- Never persist, log, or put a Monzo token in a URL. Do not add background OAuth refresh or scheduled imports unless requested.
- Public access has no application authentication. Say so clearly when describing the live URL.

## Changes and checks

- Preserve API contracts and synthetic fixtures unless the requested change requires a contract update. Use the real Rust service in acceptance tests. Do not weaken tests to hide defects.
- Do not deploy or claim device verification unless the user asks for it and it is actually completed. Do not reboot the phone unexpectedly.
- Keep examples generic; put device-specific live values in USER.md only when they are needed by the operator. Never include secrets.
- Use `scripts/check` for the full local verification when the user asks to test or verify. Report exactly what ran and what remains unverified.
- Monzo contracts live in `docs/api.md`; black-box importer cases are in `tests/contract.rs`, operational cursor/failure cases in `tests/operational.rs`, and UI cases in `tests/web/ui.spec.mjs`. Use synthetic accounts/tokens only.
- Update relevant human guidance when setup or recovery changes. Prefer one authoritative guide over duplicated instructions.
