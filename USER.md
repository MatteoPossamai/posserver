# Matteo’s posserver setup

This is the operator checklist for the personal posserver on the Android phone. Work from the laptop unless a step says **phone**. The app is public at <https://phome-public.tail1b8023.ts.net/> and also available on the private Tailnet at <http://100.108.243.40:8080>. Grafana is at <http://100.108.243.40:3000/d/posserver> privately and <https://phome-public.tail1b8023.ts.net:8443/d/posserver/posserver-application-health> through the existing Grafana Funnel. The finance app has no login: anyone who can reach its public URL can view and change its data. Share the URL only with the family members you intend to use it.

## What is live now

- The native ARM64 service runs on the phone under Termux runit; deployment and lifecycle use `scripts/phone`.
- Matteo’s `transactions.csv` was imported once (1,698 transactions). The CSV remains the current reference/source of truth while this is a pre-production system. The database is a viewing copy; website edits do not update the CSV. There is no CSV watcher or ongoing CSV synchronization. Keep the original CSV unchanged. Do not rerun migration against a changed file; review and plan a deliberate import/cutover instead.
- Dropbox backup is not configured. At the time this guide was written, seven revisions were pending backup; they remain pending until Dropbox is configured. Google Drive is paused and has no remote, credentials, or scheduled job.
- Monzo account binding and API import are not configured. The current importer accepts a short-lived access token for a manual import; it does not store tokens or refresh them in the background.
- Prometheus and Grafana now monitor the app using the phone’s existing monitoring services. See “Monitoring” below. Phone uptime and Android background reliability have not been proven over a long period or reboot.

The dashboard uses GBP reporting and the approved approximate EUR→GBP rate `0.85373` (ECB reference rate dated 2026-10-01) for all historical EUR transactions. It is a convenient estimate, not transaction-date conversion.

## 1. Keep private access working

1. Install/sign in to Tailscale on the laptop and phone using the same Tailnet account.
2. From any internet connection, open <https://phome-public.tail1b8023.ts.net/>. For private access, open <http://100.108.243.40:8080> while signed into Tailscale. If neither works, check that the phone is awake and online, then from the repository run `scripts/phone status` and `ssh phone 'curl -fsS http://100.108.243.40:8080/healthz'`.
3. The standard HTTPS Funnel route on port 443 is persistent and shares the existing Funnel node with Grafana on port 8443. Funnel makes the app reachable from the public internet; it does not add a login. Share the URL with the family members you trust.

## 2. Set up Dropbox backups

Do this from the Dropbox account that should own the backup. The app creates immutable database snapshots and a manifest under its Dropbox app folder. This is a backup copy, not a CSV import or synchronization feature.

1. Sign in to the [Dropbox App Console](https://www.dropbox.com/developers/apps) and create a scoped API app with **App folder** access. Give it a recognizable name such as `Matteo posserver`. App-folder access limits it to this app’s folder.
2. In the app’s Permissions tab, enable `files.content.write` (needed to upload snapshots and remove old revisions). Apply/save the permission changes.
3. In OAuth settings, choose the code flow. A redirect URI is optional for Dropbox; omitting it makes Dropbox show a one-time code to copy. If you choose a redirect URI, register it exactly and use that exact value below.
4. Copy the app key and app secret from the console. Build the Dropbox authorization URL in your browser, replacing `APP_KEY`:

   `https://www.dropbox.com/oauth2/authorize?client_id=APP_KEY&response_type=code&token_access_type=offline`

   If using a redirect URI, URL-encode it and add `&redirect_uri=ENCODED_URI`. Approve access and copy the fresh authorization code from the redirect/page. The `token_access_type=offline` parameter is required so posserver can refresh access without asking you to sign in each time. Dropbox’s [OAuth guide](https://docs.dropboxapi.com/dropbox-api/docs/oauth) describes app creation, redirect matching, code flow, and offline refresh tokens.
5. From the repository on the laptop, run the following Python prompt helper. It keeps the app secret and code out of shell history, command arguments, and a temporary file, then sends the JSON over SSH stdin directly to the phone’s setup command. Enter the redirect URI exactly as used above, or press Enter if none was used.

   ```sh
   python3 - <<'PY'
   import getpass, json, subprocess

   with open('/dev/tty', 'r+') as tty:
       tty.write('Dropbox app key: '); tty.flush()
       app_key = tty.readline().strip()
       app_secret = getpass.getpass('Dropbox app secret: ', stream=tty)
       code = getpass.getpass('Fresh Dropbox authorization code: ', stream=tty)
       tty.write('Registered redirect URI (blank if none): '); tty.flush()
       redirect_uri = tty.readline().strip()
   payload = {'app_key': app_key, 'app_secret': app_secret,
              'authorization_code': code}
   if redirect_uri:
       payload['redirect_uri'] = redirect_uri
   command = '"$PREFIX/apps/posserver/current/posserver" setup-dropbox --config "$PREFIX/data/posserver/config.json"'
   subprocess.run(['ssh', 'phone', command], input=json.dumps(payload).encode(), check=True)
   PY
   ```

   The command prints setup status only. The app key, secret, and refresh token are saved in the phone’s private runtime config (`$PREFIX/data/posserver/config.json`, mode 600); that file is not in Git. Do not send credentials, authorization codes, or the config file in chat or email.
6. Restart the app to load the new config: `scripts/phone restart`. Check `scripts/phone status`, then open **Settings → Backups** in the website. It should stop saying “Backup not configured.”
7. Trigger a first upload from the laptop:

   ```sh
   ssh phone '"$PREFIX/apps/posserver/current/posserver" backup-dropbox --db "$PREFIX/data/posserver/database.sqlite" --config "$PREFIX/data/posserver/config.json"'
   ```

   Check **Settings → Backups** again. Confirm the Dropbox revision is current and pending/error counts are zero. Open the Dropbox app folder and verify the manifest and a revision snapshot exist. The CLI’s setup success alone does not prove a backup uploaded.
8. Keep the newest two completed Dropbox snapshots. For a restore, download the matching snapshot and manifest from Dropbox to a private temporary directory, then follow [Restore](docs/operations.md#restore); never restore over the live database as a first check. Do a restore drill to a separate database before relying on the backup.

Dropbox’s single-upload path currently rejects files over 150 MiB; resumable upload sessions are not implemented.

## 3. Set up Monzo when ready to make it the live transaction source

Do not start a historical backfill blindly. The CSV is still the source of truth today; select a clear cutover date and import only the intended range. The installed importer is currently manual: it stores the bound account ID and cursor, but accepts an access token per import and does not keep OAuth refresh credentials or run unattended sync.

1. In [Monzo Developer Tools](https://developers.monzo.com/), create a client for your own account. Use a **confidential** client only if you need refresh tokens; keep its secret on the phone/server side. Monzo says its Developer API is for your own account or a small set of explicitly allowed users, not a public app.
2. For the current manual importer, the shortest route is Monzo’s API Playground: authorize your client there, approve the access request in the Monzo app (including its security approval), and copy the temporary access token. You do not need a refresh token for a one-off import. If you use your own OAuth callback instead, register its exact redirect URI, create a random unguessable `state`, authorize with `response_type=code`, and check the returned state before exchanging the code. See Monzo’s [OAuth documentation](https://docs.monzo.com/#authentication).
3. If using your own OAuth callback (skip this step when using the API Playground), exchange the one-time code at `https://api.monzo.com/oauth2/token` using the client ID, client secret, exact redirect URI, and `grant_type=authorization_code`. Keep the returned access/refresh tokens private. Monzo access tokens expire after several hours; a refresh token is issued only to confidential clients and is single-use/rotating. The current posserver importer does not automate refresh.
4. Confirm the token with Monzo’s `/ping/whoami`, then list `/accounts` and choose the account whose transactions you intend to import. Use the account `id` (starts `acc_…`), not the description. Monzo’s [account endpoint](https://docs.monzo.com/#list-accounts) returns accounts for the authorized user.
5. On the laptop, get Matteo’s user UUID from `http://100.108.243.40:8080/api/v1/users`. Bind the selected account on the phone (substitute the values):

   ```sh
   ssh phone '"$PREFIX/apps/posserver/current/posserver" link-monzo --db "$PREFIX/data/posserver/database.sqlite" --user USER_UUID --account-id ACCOUNT_ID'
   ```

6. In the website, select Matteo and open the Monzo import form. Paste a fresh access token only for that request. Choose the agreed cutover date. The CSV currently ends on 2026-09-28; if the intention is to start after that imported history, use `2026-09-29` as the initial date. Review the import result for inserted, duplicate, reconciled, and excluded counts. Resolve any reported ambiguous legacy matches before retrying. The token is cleared after the request and is not persisted.
7. Compare a few days of imported transactions and totals with the Monzo app before treating the database as the live source. Monzo history availability and cursor pagination must be verified against your account. Do not claim the import is complete until that comparison is satisfactory.
8. A future unattended Monzo integration needs a separate implementation decision: safely store/rotate a confidential client’s refresh token, refresh access automatically, schedule imports, report failures, and define how Monzo becomes authoritative. None of that background work is installed now.

## 4. Monitoring

The existing phone Prometheus scrapes posserver every 15 seconds at `100.108.243.40:8080/metrics`. Grafana serves the app dashboard at <http://100.108.243.40:3000/d/posserver> and publicly at <https://phome-public.tail1b8023.ts.net:8443/d/posserver/posserver-application-health>. The phone-health dashboard’s **Service and collection health** panel also shows whether posserver is up; snapshot age has been removed from that panel. Other panels cover uptime, request rate/latency, database size, transaction/import counters, and backup attempts. The existing Telegram alerting sends notices for a down target, app readiness failure, Dropbox backup failure, or a configured Dropbox queue stuck for 15 minutes. The backup alerts stay quiet until Dropbox is configured.

For a quick check, open the dashboard and look for **Reachability and readiness** (scrape and ready should be 1), then **Backup attempts** after Dropbox is configured. Grafana/Prometheus monitor the app from the same phone, so they cannot report if Android kills Termux or the phone loses power/network. Phone background-kill and reboot recovery still need an operator check; do not treat this as independent outage alerting.

## 5. What remains for production

1. Configure Dropbox and complete the first upload/restore drill.
2. Choose a Monzo cutover date and perform a small, reviewed import. Decide later whether background OAuth refresh and scheduled import are wanted; that is not implemented.
3. Decide when the database becomes the source of truth, record the CSV migration as the historical baseline, and keep the original CSV as an archive. No CSV synchronization is planned.
4. Check phone uptime after several days and after a supervised reboot. In Android Settings, allow Termux to run in the background (battery use set to unrestricted where available, and auto-launch/background activity enabled if your phone offers those controls). Keep the phone powered and online. Android vendor controls can still stop Termux and its children, so arrange a periodic human check or independent external monitor if unattended availability is required.
5. Google Drive is paused. Nothing needs to be done for it unless you choose to resume it later.
6. Review who has the public URL before sharing it more widely. There is no app login/authorization in this version; the URL is open to anyone on the internet.

For commands and limits, see [operations](docs/operations.md). For the tested-versus-unverified status, see [implementation status](docs/implementation.md).
