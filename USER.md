# Matteo’s posserver setup

This is the operator checklist for the personal posserver on the Android phone. Work from the laptop unless a step says **phone**. The app is public at <https://phome-public.tail1b8023.ts.net/> and also available on the private Tailnet at <http://100.108.243.40:8080>. Grafana is at <http://100.108.243.40:3000/d/posserver> privately and <https://phome-public.tail1b8023.ts.net:8443/d/posserver/posserver-application-health> through the existing Grafana Funnel. The finance app has no login: anyone who can reach its public URL can view and change its data. Share the URL only with the family members you intend to use it.

## What is live now

- The native ARM64 service runs on the phone under Termux runit; deployment and lifecycle use `scripts/phone`.
- SQLite on the phone is now the source of truth. Matteo’s `transactions.csv` was imported as the initial 1,698-transaction history and is kept as an archive; website changes are saved to SQLite and are not written back to the CSV. Do not rerun the migration.
- Dropbox backup is configured and the first backup completed (revision 7; no pending revisions at the last check).
- Monzo imports are manual. Matteo can paste a short-lived access token in **Get from Monzo**; the server discovers the account, starts from the latest saved transaction, and imports pages. Tokens are not stored or refreshed.
- Prometheus and Grafana now monitor the app using the phone’s existing monitoring services. See “Monitoring” below. Phone uptime and Android background reliability have not been proven over a long period or reboot.

The dashboard uses GBP reporting and the approved approximate EUR→GBP rate `0.85373` (ECB reference rate dated 2026-10-01) for all historical EUR transactions. It is a convenient estimate, not transaction-date conversion.

## 1. Keep private access working

1. Install/sign in to Tailscale on the laptop and phone using the same Tailnet account.
2. From any internet connection, open <https://phome-public.tail1b8023.ts.net/>. For private access, open <http://100.108.243.40:8080> while signed into Tailscale. If neither works, check that the phone is awake and online, then from the repository run `scripts/phone status` and `ssh phone 'curl -fsS http://100.108.243.40:8080/healthz'`.
3. The standard HTTPS Funnel route on port 443 is persistent and shares the existing Funnel node with Grafana on port 8443. Funnel makes the app reachable from the public internet; it does not add a login. Share the URL with the family members you trust.

## What happens when you save a transaction

1. The website sends the new transaction to posserver.
2. Posserver saves the transaction and a “needs backup” entry together in one SQLite commit. When the website reports success, the transaction is safely saved on the phone; Dropbox may still be catching up.
3. The background backup worker wakes and creates an integrity-checked snapshot of the database. It uploads that snapshot and then its manifest to Dropbox. Several quick saves may be covered by one newer snapshot rather than producing one Dropbox file per tap.
4. After Dropbox confirms the files, posserver marks the covered backup entries complete. **Settings → Backups** then shows no pending revisions. If Dropbox is unavailable, the transaction remains saved on the phone, the backup remains pending, and posserver retries; a restart also resumes pending work.

This backs up the SQLite database, including transactions entered on the website. The original CSV is an archive and is not kept synchronized.

## 2. Set up Dropbox backups

Do this from the Dropbox account that should own the backup. The app creates immutable database snapshots and a manifest under its Dropbox app folder. This is a backup copy, not a CSV import or synchronization feature.

1. In the [Dropbox App Console](https://www.dropbox.com/developers/apps), open your existing **posserver-data** app. Confirm it uses **App folder** access and enable `files.content.write` in Permissions. Save the change. Dropbox’s [OAuth guide](https://docs.dropboxapi.com/dropbox-api/docs/oauth) explains app permissions and offline OAuth.
2. Regenerate the app secret in the console before continuing: the previous secret was pasted into chat. Do not send the replacement here.
3. From the repository on your laptop, run:

   ```sh
   scripts/setup-dropbox --app-key 4dtr67zqh7xtwq2
   ```

   The helper prompts for the new secret without displaying it, opens Dropbox authorization, and requests offline access. Since you did not set a redirect URI, Dropbox will show a one-time code after you approve access. Paste that code into the helper’s hidden prompt. It sends the code and secret directly to the phone over SSH stdin, configures Dropbox, uploads the pending revisions, restarts posserver, and checks backup status. It does not save a local credential file or print the secret/code.
4. Success ends with `pending revisions: 0`. Then open **Settings → Backups** and confirm Dropbox is configured and current. The phone stores the app credentials and refresh token in its private runtime config (`$PREFIX/data/posserver/config.json`, mode 600); that file is not in Git.
5. Open the Dropbox app folder and confirm the manifest and revision snapshot exist. The service retains the newest five completed Dropbox snapshots, deleting older ones after a newer backup succeeds. Nearby changes may be coalesced; otherwise each completed change may produce a snapshot, but remote storage is capped at five completed revisions. Before relying on these backups, do a restore drill to a separate database: download the matching snapshot and manifest to a private temporary directory, then follow [Restore](docs/operations.md#restore). Never test restore over the live database.

Dropbox’s single-upload path currently rejects files over 150 MiB; resumable upload sessions are not implemented.

### Keep a recovery copy of the phone config

The database snapshot is in Dropbox, but the Dropbox refresh credential and other runtime settings are on the phone. Save an encrypted copy from the neighboring phome repository on the laptop:

```sh
cd ../phome_srvr
scripts/backup-posserver-config
```

This asks for an Ansible Vault password and saves only encrypted data in phome. Store that password in your password manager. Repeat after changing posserver credentials or runtime settings. The phone’s live config remains plaintext with private permissions so the service can use it; the encrypted copy protects the recovery copy at rest. Full replacement-phone steps are in [phome’s recovery guide](../phome_srvr/docs/posserver-backup.md).

If the phone dies, redeploy posserver to its replacement, restore config with `scripts/phone configure --vault ../phome_srvr/config/posserver-runtime.json.vault`, restart the service, then restore the latest Dropbox database snapshot using [the restore procedure](docs/operations.md#restore). The encrypted config does not contain transaction data.

## 3. Import from Monzo

SQLite is the source of truth. The CSV is retained as an archive. For a manual import:

1. Open the app, select Matteo, and choose **Get from Monzo**.
2. In [Monzo API Playground](https://developers.monzo.com/), authorize your account and approve the request in the Monzo app. Copy the temporary access token.
3. Paste the token into the form. The server finds the account; if there is more than one, choose the intended account. The initial import starts one second before Matteo's latest saved transaction. To import earlier, set **Start earlier**.
4. Check the inserted, duplicate, reconciled, and excluded counts. Compare imported dates and totals with the Monzo app before treating the import as complete.

The token is sent to the server and cleared after the import. If you must choose an account, it stays in the form for that second request. It is not saved or refreshed. Make the initial fetch promptly: [Monzo may limit older history to 90 days after the first five minutes of authentication](https://docs.monzo.com/#list-transactions). If the token expires or is rejected, create a fresh one and retry. Background sync and automatic token refresh are not implemented.

## 4. Monitoring

The existing phone Prometheus scrapes posserver every 15 seconds at `100.108.243.40:8080/metrics`. Grafana serves the app dashboard at <http://100.108.243.40:3000/d/posserver> and publicly at <https://phome-public.tail1b8023.ts.net:8443/d/posserver/posserver-application-health>. The phone-health dashboard’s **Service and collection health** panel also shows whether posserver is up; snapshot age has been removed from that panel. Other panels cover uptime, request rate/latency, database size, transaction/import counters, and backup attempts. The existing Telegram alerting sends notices for a down target, app readiness failure, Dropbox backup failure, or a configured Dropbox queue stuck for 15 minutes.

For a quick check, open the dashboard and look for **Reachability and readiness** (scrape and ready should be 1), then **Backup attempts** after Dropbox is configured. Grafana/Prometheus monitor the app from the same phone, so they cannot report if Android kills Termux or the phone loses power/network. Phone background-kill and reboot recovery still need an operator check; do not treat this as independent outage alerting.

## 5. What remains for production

1. Complete a Dropbox restore drill to a separate database.
2. Compare the latest Monzo import's date range and totals with Monzo. Imports remain manual; background OAuth refresh and scheduled imports are not implemented.
3. Keep the original CSV as an archive. No CSV synchronization is planned.
4. Check phone uptime after several days and after a supervised reboot. In Android Settings, allow Termux to run in the background (battery use set to unrestricted where available, and auto-launch/background activity enabled if your phone offers those controls). Keep the phone powered and online. Android vendor controls can still stop Termux and its children, so arrange a periodic human check or independent external monitor if unattended availability is required.
5. Review who has the public URL before sharing it more widely. There is no app login/authorization in this version; the URL is open to anyone on the internet.

For commands and limits, see [operations](docs/operations.md). For the tested-versus-unverified status, see [implementation status](docs/implementation.md).
