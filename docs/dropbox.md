# Dropbox setup and recovery

This is the only remote backup provider. The phone keeps the live SQLite database; Dropbox stores checked snapshots so a lost phone can be replaced. It is not a live database sync.

## Set it up again

You need the Dropbox account owner, the `posserver-data` app key and app secret, and the existing laptop SSH alias for the phone. The app must use **App folder** access and have `files.content.write` enabled. The helper requests offline access so the phone can refresh its own credential.

1. If the old app secret may have been exposed, regenerate it in Dropbox first. Do not paste secrets into chat or save them in this repository.
2. From an interactive laptop terminal in this repository, run `scripts/setup-dropbox --app-key YOUR_APP_KEY`.
3. Approve Dropbox access in the browser. Paste the one-time authorization code into the hidden prompt. The helper sends the setup input directly to the phone over SSH; it does not save a laptop credential file.
4. Wait for the helper to finish. It configures the phone, makes a first backup, restarts posserver, and waits for the backup status check. Success means Dropbox is configured, pending revisions are zero, and the Dropbox revision matches the local snapshot revision.
5. Open **Settings → Backups** and confirm it shows current. In Dropbox, the app folder contains a manifest and revision snapshots. The service keeps the newest two completed remote snapshots.

The phone stores credentials in `$PREFIX/data/posserver/config.json` with private file permissions. Keep this file out of Git. To recover after phone loss, use the encrypted copy maintained in `../../phome_srvr`; see [phome's recovery steps](../../phome_srvr/docs/posserver-backup.md).

## If backup stops advancing

1. Check **Settings → Backups** or `GET /api/v1/backups/status`. `pending` should return to zero and `dropbox_revision` should catch up to `local_revision`.
2. Check service logs with `scripts/phone logs`. Confirm the phone has network access and that its clock is correct.
3. If the Dropbox app secret or authorization is invalid, rerun the setup helper with current app credentials. Existing database data is not removed by setup.
4. Restart posserver with `scripts/phone restart`; pending work resumes automatically. A manual retry is available in the UI.

A successful website save means the local database commit succeeded; it does not mean the remote upload has finished. Do not delete the local database or its `backups/` directory while work is pending.

## Restore a snapshot

Restore only while the service is stopped, and restore to a replacement or separate database. Download `manifest.json` and the exact revision file named by its `snapshot_path` from the Dropbox app folder. Then run on the phone, substituting paths:

```sh
"$PREFIX/apps/posserver/current/posserver" restore \
  --db "$PREFIX/data/posserver/database.sqlite" \
  --snapshot /path/to/revision.sqlite \
  --manifest /path/to/manifest.json
```

The restore command checks the file hash, database integrity, schema, and revision before replacing the destination. It preserves the replaced database as a rollback file. Start the service, then check users, transaction counts, reports, and backup status before relying on it. If restoring an older snapshot, inspect the Dropbox revision before allowing new uploads; an old database must not overwrite a newer remote copy. The full restore and rollback behavior is in [phone operations](operations.md#restore).
