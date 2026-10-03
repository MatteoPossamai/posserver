// Browser acceptance tests only: require a disposable implementation server.
import { test, expect } from '@playwright/test';

let base;
test.beforeAll(() => {
  base = process.env.POSSERVER_WEB_URL;
  if (!base) throw new Error('Set POSSERVER_WEB_URL to a disposable future posserver instance. No UI implementation exists yet.');
});

async function user(request, profile = 'personal', name = `browser-${profile}-${crypto.randomUUID()}`) {
  const response = await request.post(`${base}/api/v1/users`, { data: {
    name,
    language: profile === 'dad' ? 'it' : 'en',
    timezone: 'UTC', reporting_currency: profile === 'dad' ? 'EUR' : 'GBP',
    category_profile: profile,
  } });
  expect(response.status()).toBe(201);
  return (await response.json()).user;
}

async function select(page, u) {
  await page.goto(base);
  await page.getByTestId('user-menu').click();
  await page.getByTestId(`user-option-${u.id}`).click();
  await expect(page.getByTestId('selected-user')).toHaveText(u.name);
}

async function create(request, u, amount = -123, category = 'groceries') {
  const response = await request.post(`${base}/api/v1/users/${u.id}/transactions`, { data: {
    occurred_at: '2026-10-01T12:00:00Z', category_id: category,
    issuer: 'BROWSER_SHOP', amount_minor: amount, currency: u.reporting_currency,
  } });
  expect(response.status()).toBe(201);
  return (await response.json()).transaction;
}

test('create a user from the mobile menu', async ({ page }) => {
  await page.goto(base);
  await page.getByTestId('user-menu').click();
  await page.getByTestId('create-user').click();
  const name = `UI Papà ${crypto.randomUUID()}`;
  await page.getByTestId('user-name').fill(name);
  await page.getByTestId('user-language').selectOption('it');
  await page.getByTestId('user-timezone').fill('Europe/Rome');
  await page.getByTestId('user-currency').selectOption('EUR');
  await page.getByTestId('user-profile').selectOption('dad');
  await page.getByTestId('user-save').click();
  await expect(page.getByTestId('selected-user')).toHaveText(name);
  await expect(page.getByTestId('transactions-nav')).toHaveText('Transazioni');
});

test('Italian manual entry converts positive expense input to signed minor units', async ({ page, request }) => {
  const u = await user(request, 'dad');
  await select(page, u);
  await page.getByTestId('transactions-nav').click();
  await expect(page.getByTestId('transaction-empty')).toHaveText('Nessuna transazione');
  await expect(page.getByTestId('new-transaction')).toHaveText('Nuova transazione');
  await page.getByTestId('new-transaction').click();
  await page.getByTestId('transaction-date').fill('2026-10-02');
  await page.getByTestId('transaction-kind').selectOption('expense');
  await page.getByTestId('transaction-amount').fill('12.34');
  await page.getByTestId('transaction-category').selectOption('dad_groceries');
  await page.getByTestId('transaction-issuer').fill('SUPERMERCATO');
  await expect(page.getByTestId('transaction-save')).toHaveText('Salva');
  await page.getByTestId('transaction-save').click();
  const response = await request.get(`${base}/api/v1/users/${u.id}/transactions`);
  const rows = (await response.json()).transactions;
  expect(rows).toHaveLength(1);
  expect(rows[0].amount_minor).toBe(-1234);
  expect(rows[0].occurred_at).toBe('2026-10-02T12:00:00Z');
  await expect(page.getByTestId(`transaction-row-${rows[0].id}`)).toContainText('SUPERMERCATO');
});

test('switching users changes transactions and latest preview', async ({ page, request }) => {
  const u = await user(request); const dad = await user(request, 'dad');
  const t = await create(request, u);
  await select(page, u);
  await page.getByTestId('transactions-nav').click();
  await expect(page.getByTestId(`transaction-row-${t.id}`)).toBeVisible();
  await page.getByTestId('new-transaction').click();
  await expect(page.getByTestId('latest-transaction')).toContainText('BROWSER_SHOP');
  await select(page, dad);
  await page.getByTestId('transactions-nav').click();
  await expect(page.getByTestId(`transaction-row-${t.id}`)).toHaveCount(0);
  await expect(page.getByTestId('transaction-empty')).toBeVisible();
});

test('edit and delete target the displayed transaction', async ({ page, request }) => {
  const u = await user(request); const t = await create(request, u);
  await select(page, u); await page.getByTestId('transactions-nav').click();
  await page.getByTestId(`transaction-edit-${t.id}`).click();
  await page.getByTestId('transaction-amount').fill('2.50');
  await page.getByTestId('transaction-save').click();
  const edited = await request.get(`${base}/api/v1/users/${u.id}/transactions/${t.id}`);
  expect((await edited.json()).transaction.amount_minor).toBe(-250);
  await page.getByTestId(`transaction-delete-${t.id}`).click();
  await expect(page.getByTestId('delete-confirmation')).toContainText('BROWSER_SHOP');
  await page.getByTestId('delete-confirm').click();
  await expect(page.getByTestId(`transaction-row-${t.id}`)).toHaveCount(0);
});

test('dashboard shows month comparison and category trend', async ({ page, request }) => {
  const u = await user(request); await create(request, u, -1000);
  await select(page, u); await page.getByTestId('dashboard-nav').click();
  await page.getByTestId('report-month').fill('2026-10');
  await expect(page.getByTestId('current-expense')).toHaveAttribute('data-minor', '1000');
  await expect(page.getByTestId('previous-expense')).toHaveAttribute('data-minor', '0');
  await expect(page.getByTestId('category-total-groceries')).toHaveAttribute('data-minor', '1000');
  await page.getByTestId('trend-category').selectOption('groceries');
  await expect(page.getByTestId('trend-chart')).toBeVisible();
  await expect(page.getByTestId('trend-point-2026-10')).toHaveAttribute('data-minor', '1000');
});

test('settings rate changes affect converted reports', async ({ page, request }) => {
  const u = await user(request);
  const response = await request.post(`${base}/api/v1/users/${u.id}/transactions`, { data: {
    occurred_at: '2026-10-01T12:00:00Z', category_id: 'groceries', issuer: 'EUR_SHOP', amount_minor: -1000, currency: 'EUR',
  } });
  expect(response.status()).toBe(201);
  await select(page, u); await page.getByTestId('settings-nav').click();
  await page.getByTestId('rate-add').click();
  await page.getByTestId('rate-from-0').selectOption('EUR');
  await page.getByTestId('rate-to-0').selectOption('GBP');
  await page.getByTestId('rate-value-0').fill('0.87');
  await page.getByTestId('settings-save').click();
  await page.getByTestId('dashboard-nav').click();
  await page.getByTestId('report-month').fill('2026-10');
  await expect(page.getByTestId('current-expense')).toHaveAttribute('data-minor', '870');
});

test('Italian validation is visible and keyboard accessible', async ({ page, request }) => {
  const u = await user(request, 'dad'); await select(page, u);
  await page.getByTestId('transactions-nav').click(); await page.getByTestId('new-transaction').click();
  await page.getByTestId('transaction-date').fill('2026-10-02');
  await page.getByTestId('transaction-amount').fill('0.001');
  await page.getByTestId('transaction-issuer').fill('NEGOZIO');
  await page.getByTestId('transaction-category').selectOption('dad_groceries');
  await page.getByTestId('transaction-save').focus(); await page.keyboard.press('Enter');
  await expect(page.getByTestId('transaction-error')).toContainText('Importo non valido');
  const response = await request.get(`${base}/api/v1/users/${u.id}/transactions`);
  expect((await response.json()).transactions).toHaveLength(0);
});

for (const success of [true, false]) {
  test(`Monzo token clears after ${success ? 'success' : 'failure'} and stays out of browser storage`, async ({ page, request }) => {
    const name = 'Matteo';
    const u = await user(request, 'personal', name);
    const token = 'synthetic-browser-monzo-token';
    await page.route(`**/api/v1/users/${u.id}/imports/monzo`, async route => {
      expect(route.request().postDataJSON().access_token).toBe(token);
      await route.fulfill({ status: success ? 200 : 422, contentType: 'application/json', body: JSON.stringify(success
        ? { inserted: 0, duplicates: 0, excluded: 0, reconciled: 0, unknown_categories: 0, last_raw_id: null, revision: 1 }
        : { error: { code: 'monzo_token_rejected', message: 'Paste a fresh token', details: {} } }) });
    });
    await select(page, u);
    await page.getByTestId('monzo-nav').click();
    await page.getByTestId('monzo-token').fill(token);
    await page.getByTestId('monzo-since').fill('2026-10-01');
    await page.getByTestId('monzo-import').click();
    await expect(page.getByTestId(success ? 'monzo-result' : 'monzo-error')).toBeVisible();
    await expect(page.getByTestId('monzo-token')).toHaveValue('');
    const stored = await page.evaluate(() => JSON.stringify({ local: { ...localStorage }, session: { ...sessionStorage } }));
    expect(stored).not.toContain(token);
  });
}

test('Monzo is shown only to Matteo and supports account choice', async ({ page, request }) => {
  const other = await user(request, 'personal', 'Other');
  await select(page, other);
  await expect(page.getByTestId('monzo-nav')).toHaveCount(0);

  const matteo = await user(request, 'personal', 'Matteo');
  const token = 'synthetic-account-choice-token';
  let calls = 0;
  await page.route(`**/api/v1/users/${matteo.id}/imports/monzo`, async route => {
    const body = route.request().postDataJSON();
    expect(body.access_token).toBe(token);
    calls++;
    if (calls === 1) {
      expect(body.account_id).toBeUndefined();
      await route.fulfill({ status: 409, contentType: 'application/json', body: JSON.stringify({ error: {
        code: 'monzo_account_selection_required', message: 'Choose an account', details: { accounts: [
          { id: 'acc_current', description: 'Current <img src=x>' }, { id: 'acc_joint', description: 'Joint' },
        ] },
      } }) });
    } else {
      expect(body.account_id).toBe('acc_joint');
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({
        inserted: 0, duplicates: 0, excluded: 0, reconciled: 0, unknown_categories: 0, last_raw_id: null, revision: 1,
      }) });
    }
  });
  await select(page, matteo);
  await expect(page.getByTestId('monzo-nav')).toBeVisible();
  await page.getByTestId('monzo-nav').click();
  await page.getByTestId('monzo-token').fill(token);
  await page.getByTestId('monzo-import').click();
  await expect(page.getByTestId('monzo-account-select')).toBeVisible();
  await expect(page.getByTestId('monzo-account-choice').locator('img')).toHaveCount(0);
  await page.getByTestId('monzo-account-select').selectOption('acc_joint');
  await page.getByTestId('monzo-import').click();
  await expect(page.getByTestId('monzo-result')).toBeVisible();
  await expect(page.getByTestId('monzo-token')).toHaveValue('');
  expect(calls).toBe(2);
});

test('backup indicator distinguishes pending and recovered revisions', async ({ page, request }) => {
  const u = await user(request); let pending = true;
  await page.route('**/api/v1/backups/status', route => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({
    current_revision: 8, local_revision: 8, dropbox_revision: pending ? 6 : 8,
    pending: pending ? 2 : 0, last_error: pending ? { provider: 'dropbox', code: 'unavailable' } : null,
  }) }));
  await select(page, u);
  await expect(page.getByTestId('backup-status')).toHaveAttribute('data-pending', '2');
  await expect(page.getByTestId('backup-error')).toBeVisible();
  pending = false; await page.reload();
  await expect(page.getByTestId('backup-status')).toHaveAttribute('data-pending', '0');
  await expect(page.getByTestId('backup-error')).toHaveCount(0);
});

test('large exact minor-unit amount survives opening and saving an edit', async ({ page, request }) => {
  const u = await user(request); const t = await create(request, u, -9007199254740991);
  await select(page, u); await page.getByTestId('transactions-nav').click();
  await page.getByTestId(`transaction-edit-${t.id}`).click();
  await expect(page.getByTestId('transaction-amount')).toHaveValue('90071992547409.91');
  await page.getByTestId('transaction-save').click();
  await expect(page.getByTestId(`transaction-row-${t.id}`)).toBeVisible();
  const response = await request.get(`${base}/api/v1/users/${u.id}/transactions/${t.id}`);
  expect((await response.json()).transaction.amount_minor).toBe(-9007199254740991);
});

test('date-only entry uses selected timezone noon across DST', async ({ page, request }) => {
  const response = await request.post(`${base}/api/v1/users`, { data: {
    name: `Rome-${crypto.randomUUID()}`, language: 'it', timezone: 'Europe/Rome',
    reporting_currency: 'EUR', category_profile: 'dad',
  } });
  const u = (await response.json()).user;
  await select(page, u); await page.getByTestId('transactions-nav').click();
  await page.getByTestId('new-transaction').click();
  await page.getByTestId('transaction-date').fill('2026-07-01');
  await page.getByTestId('transaction-amount').fill('12,34');
  await page.getByTestId('transaction-issuer').fill('CAFFÈ');
  await page.getByTestId('transaction-category').selectOption('dad_groceries');
  await page.getByTestId('transaction-save').click();
  await expect(page.getByTestId('transaction-empty')).toHaveCount(0);
  const result = await request.get(`${base}/api/v1/users/${u.id}/transactions`);
  const rows = (await result.json()).transactions;
  expect(rows[0].occurred_at).toBe('2026-07-01T10:00:00Z');
  expect(rows[0].amount_minor).toBe(-1234);
});

test('late settings response does not switch back to the old user', async ({ page, request }) => {
  const first = await user(request); const second = await user(request, 'dad');
  let release; const gate = new Promise(resolve => { release = resolve; });
  let entered; const started = new Promise(resolve => { entered = resolve; });
  await page.route(`**/api/v1/users/${first.id}/settings`, async route => {
    const response = await route.fetch(); entered(); await gate;
    await route.fulfill({ response });
  });
  await select(page, first); await page.getByTestId('settings-nav').click();
  await page.getByTestId('settings-language').selectOption('it');
  await page.getByTestId('settings-save').click(); await started;
  await page.getByTestId('user-menu').click();
  await page.getByTestId(`user-option-${second.id}`).click();
  await expect(page.getByTestId('selected-user')).toHaveText(second.name);
  release();
  await page.waitForResponse(r => r.url().endsWith(`/users/${first.id}/settings`));
  await expect(page.getByTestId('selected-user')).toHaveText(second.name);
});
