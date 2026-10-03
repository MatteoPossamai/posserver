const $ = (s) => document.querySelector(s),
  main = $("#main");
let users = [],
  selected = null,
  categories = [],
  view = "dashboard",
  generation = 0,
  rows = [],
  nextCursor = null,
  rates = [];
const words = {
  en: {
    users: "Users",
    create: "Create user",
    name: "Name",
    language: "Language",
    timezone: "Timezone",
    currency: "Currency",
    profile: "Categories",
    save: "Save",
    transactions: "Transactions",
    dashboard: "Dashboard",
    settings: "Settings",
    monzo: "Get from Monzo",
    empty: "No transactions",
    new: "New transaction",
    date: "Date",
    kind: "Type",
    expense: "Expense",
    income: "Income",
    all: "All",
    amount: "Amount",
    category: "Category",
    issuer: "Issuer",
    edit: "Edit",
    delete: "Delete",
    cancel: "Cancel",
    confirm: "Confirm delete",
    latest: "Latest transaction",
    none: "None",
    month: "Month",
    previous: "Previous month",
    spent: "Spent",
    net: "Net",
    trend: "Monthly trend",
    rates: "Exchange rates",
    addRate: "Add rate",
    from: "From",
    to: "To",
    rate: "Rate",
    invalidAmount: "Invalid amount",
    invalidDate: "Invalid date",
    invalid: "Invalid input",
    conflict: "This entry changed. Reload it before editing.",
    missingRate: "Add the missing exchange rate in Settings.",
    tokenRejected: "Paste a fresh Monzo token.",
    unavailable: "The service is unavailable. Try again.",
    startRequired: "Add a transaction or choose a start date first.",
    import: "Get transactions",
    token: "Temporary Monzo access token",
    tokenHelp: "Kept only while choosing an account, then cleared.",
    since: "Start earlier (optional)",
    account: "Monzo account",
    accountPlaceholder: "Choose an account",
    accountChoice: "This token can access more than one account. Choose the account to import.",
    accountInvalid: "That account does not match this token. Try again.",
    accountMissing: "No Monzo account was available for this token.",
    result: "Import completed",
    pending: "Backups pending",
    backed: "Backup queue clear",
    backupError: "Backup needs retry",
    retry: "Retry",
    loadMore: "Load more",
    filter: "Filter",
    noUsers: "Create a user to begin.",
    remove: "Remove",
    notConfigured: "Backup not configured",
    unknown: "Unknown category",
    personal: "Personal",
    inserted: "Inserted",
    duplicates: "Duplicates",
    excluded: "Excluded",
    reconciled: "Reconciled",
  },
  it: {
    users: "Utenti",
    create: "Crea utente",
    name: "Nome",
    language: "Lingua",
    timezone: "Fuso orario",
    currency: "Valuta",
    profile: "Categorie",
    save: "Salva",
    transactions: "Transazioni",
    dashboard: "Riepilogo",
    settings: "Impostazioni",
    monzo: "Scarica da Monzo",
    empty: "Nessuna transazione",
    new: "Nuova transazione",
    date: "Data",
    kind: "Tipo",
    expense: "Spesa",
    income: "Entrata",
    all: "Tutte",
    amount: "Importo",
    category: "Categoria",
    issuer: "Esercente",
    edit: "Modifica",
    delete: "Elimina",
    cancel: "Annulla",
    confirm: "Conferma eliminazione",
    latest: "Ultima transazione",
    none: "Nessuna",
    month: "Mese",
    previous: "Mese precedente",
    spent: "Speso",
    net: "Saldo netto",
    trend: "Andamento mensile",
    rates: "Tassi di cambio",
    addRate: "Aggiungi tasso",
    from: "Da",
    to: "A",
    rate: "Tasso",
    invalidAmount: "Importo non valido",
    invalidDate: "Data non valida",
    invalid: "Dati non validi",
    conflict: "La transazione è cambiata. Ricaricala prima di modificarla.",
    missingRate: "Aggiungi il tasso di cambio mancante nelle Impostazioni.",
    tokenRejected: "Incolla un nuovo token Monzo.",
    unavailable: "Il servizio non è disponibile. Riprova.",
    startRequired: "Aggiungi prima una transazione o scegli una data iniziale.",
    import: "Scarica transazioni",
    token: "Token temporaneo di Monzo",
    tokenHelp: "Mantenuto solo per scegliere il conto, poi cancellato.",
    since: "Importa da una data precedente (opzionale)",
    account: "Conto Monzo",
    accountPlaceholder: "Scegli un conto",
    accountChoice: "Questo token accede a più conti. Scegli quale importare.",
    accountInvalid: "Questo conto non corrisponde al token. Riprova.",
    accountMissing: "Nessun conto Monzo disponibile per questo token.",
    result: "Importazione completata",
    pending: "Backup in attesa",
    backed: "Coda backup completata",
    backupError: "Backup da riprovare",
    retry: "Riprova",
    loadMore: "Carica altre",
    filter: "Filtra",
    noUsers: "Crea un utente per iniziare.",
    remove: "Rimuovi",
    notConfigured: "Backup non configurato",
    unknown: "Categoria sconosciuta",
    personal: "Personali",
    inserted: "Inserite",
    duplicates: "Duplicate",
    excluded: "Escluse",
    reconciled: "Riconciliate",
  },
};
const w = (k) => words[selected?.language || "en"][k] || k;
const esc = (s) =>
  String(s ?? "").replace(
    /[&<>"']/g,
    (c) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        c
      ],
  );
const test = (id, tag, content = "", attrs = "") =>
  `<${tag} data-testid="${id}" ${attrs}>${content}</${tag}>`;
const button = (id, text, attrs = "") => test(id, "button", esc(text), attrs);
const input = (id, type, value = "", attrs = "") =>
  `<input data-testid="${id}" id="${id}" type="${type}" value="${esc(value)}" ${attrs}>`;
const select = (id, options, value) =>
  test(
    id,
    "select",
    options
      .map(
        ([v, t]) =>
          `<option value="${esc(v)}" ${v === value ? "selected" : ""}>${esc(t)}</option>`,
      )
      .join(""),
    `id="${id}"`,
  );
const label = (key, control) => `<label>${esc(w(key))}${control}</label>`;
const currencies = ["GBP", "EUR", "USD"].map((c) => [c, c]);
const categoryOptions = () =>
  categories.map((c) => [c.id, c[selected.language]]);
const val = (id) => $(`[data-testid="${id}"]`).value;
function magnitude(amount) {
  const a = BigInt(amount) < 0n ? -BigInt(amount) : BigInt(amount);
  return `${a / 100n}.${String(a % 100n).padStart(2, "0")}`;
}
function money(amount, curr = selected.reporting_currency) {
  const minor = BigInt(amount),
    absolute = minor < 0n ? -minor : minor;
  const whole = absolute / 100n;
  const signed = minor < 0n ? (whole === 0n ? -0 : -whole) : whole;
  return new Intl.NumberFormat(selected.language, {
    style: "currency",
    currency: curr,
  })
    .formatToParts(signed)
    .map((p) =>
      p.type === "fraction"
        ? String(absolute % 100n).padStart(2, "0")
        : p.value,
    )
    .join("");
}
function dateInZone(date = new Date()) {
  const parts = new Intl.DateTimeFormat("en-CA", {
    timeZone: selected?.timezone || "UTC",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).formatToParts(date);
  return ["year", "month", "day"]
    .map((k) => parts.find((p) => p.type === k).value)
    .join("-");
}
function localInstant(day, hour = 12) {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(day)) throw Error(w("invalidDate"));
  const target = Date.parse(`${day}T${String(hour).padStart(2, "0")}:00:00Z`);
  if (
    !Number.isFinite(target) ||
    new Date(target).toISOString().slice(0, 10) !== day
  )
    throw Error(w("invalidDate"));
  const format = new Intl.DateTimeFormat("en-CA", {
    timeZone: selected.timezone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hourCycle: "h23",
  });
  const wall = (time) => {
    const parts = format.formatToParts(new Date(time));
    const p = (k) => parts.find((p) => p.type === k).value;
    return Date.parse(
      `${p("year")}-${p("month")}-${p("day")}T${p("hour")}:${p("minute")}:${p("second")}Z`,
    );
  };
  const offsets = new Set(
    [-36, 0, 36].map((h) => {
      const probe = target + h * 3600000;
      return wall(probe) - probe;
    }),
  );
  const candidates = [...offsets]
    .map((offset) => target - offset)
    .filter((time) => wall(time) === target);
  // Date-only entry uses local noon. Reject skipped or repeated wall times.
  if (candidates.length !== 1) throw Error(w("invalidDate"));
  return new Date(candidates[0]).toISOString().replace(".000Z", "Z");
}
const localNoon = (day) => localInstant(day, 12);
function errorText(code) {
  return w(
    {
      missing_exchange_rate: "missingRate",
      amount_overflow: "invalidAmount",
      version_conflict: "conflict",
      monzo_token_rejected: "tokenRejected",
      import_start_required: "startRequired",
      monzo_account_invalid: "accountInvalid",
      monzo_account_not_found: "accountMissing",
      invalid_input: "invalid",
    }[code] || "unavailable",
  );
}
async function api(path, method = "GET", body) {
  const response = await fetch(`/api/v1${path}`, {
    method,
    headers: body ? { "Content-Type": "application/json" } : {},
    body: body ? JSON.stringify(body) : undefined,
  });
  const data = await response.json();
  if (!response.ok) {
    const error = Error(errorText(data.error?.code));
    error.code = data.error?.code;
    error.details = data.error?.details || {};
    throw error;
  }
  return data;
}
const base = () => `/users/${selected.id}`;
function showError(id, error) {
  const el = $(`[data-testid="${id}"]`);
  if (el) {
    el.hidden = false;
    el.textContent = error.message || w("invalid");
  }
}
async function loadUsers() {
  users = (await api("/users")).users;
}
$("#user-menu").onclick = async () => {
  await loadUsers();
  const menu = $("#menu");
  menu.hidden = !menu.hidden;
  menu.innerHTML =
    users.map((u) => button(`user-option-${u.id}`, u.name)).join("") +
    button("create-user", w("create"));
  for (const u of users)
    $(`[data-testid="user-option-${u.id}"]`).onclick = () => choose(u);
  $('[data-testid="create-user"]').onclick = createUser;
};
async function choose(u) {
  const g = ++generation;
  selected = u;
  localStorage.setItem("selectedUser", u.id);
  document.documentElement.lang = u.language;
  $("#selected-user").textContent = u.name;
  $("#user-menu").textContent = w("users");
  $("#menu").hidden = true;
  const loaded = (await api(`/users/${u.id}/categories`)).categories;
  if (g !== generation) return;
  categories = loaded;
  nav();
  await render();
  await backupStatus();
}
function nav() {
  $("#nav").innerHTML = [
    "dashboard",
    "transactions",
    "settings",
    ...(selected.name === "Matteo" && selected.category_profile === "personal"
      ? ["monzo"]
      : []),
  ]
    .map((k) => button(`${k}-nav`, w(k)))
    .join("");
  for (const k of ["dashboard", "transactions", "settings", "monzo"]) {
    const b = $(`[data-testid="${k}-nav"]`);
    if (b)
      b.onclick = () => {
        view = k;
        render();
      };
  }
}
function createUser() {
  main.innerHTML = `<form id="user-form"><h2>${esc(w("create"))}</h2>${label("name", input("user-name", "text", "", 'required maxlength="100"'))}${label(
    "language",
    select(
      "user-language",
      [
        ["en", "English"],
        ["it", "Italiano"],
      ],
      "en",
    ),
  )}${label("timezone", input("user-timezone", "text", "Europe/London", "required"))}${label("currency", select("user-currency", currencies, "GBP"))}${label(
    "profile",
    select(
      "user-profile",
      [
        ["personal", w("personal")],
        ["dad", "Papà"],
      ],
      "personal",
    ),
  )}${test("user-error", "p", "", 'class="error" role="alert" hidden')}${button("user-save", w("save"), 'type="submit"')}</form>`;
  $("#menu").hidden = true;
  $("#user-form").onsubmit = async (e) => {
    e.preventDefault();
    try {
      const r = await api("/users", "POST", {
        name: val("user-name"),
        language: val("user-language"),
        timezone: val("user-timezone"),
        reporting_currency: val("user-currency"),
        category_profile: val("user-profile"),
      });
      view = "dashboard";
      await loadUsers();
      await choose(r.user);
    } catch (err) {
      showError("user-error", err);
    }
  };
}
async function render() {
  if (!selected) {
    main.innerHTML = `<p>${w("noUsers")}</p>${button("create-user-empty", w("create"))}`;
    $('[data-testid="create-user-empty"]').onclick = createUser;
    return;
  }
  generation++;
  try {
    if (view === "transactions") await transactions();
    else if (view === "settings") settings();
    else if (view === "monzo") monzo();
    else await dashboard();
  } catch (e) {
    main.innerHTML = `<p class="error" role="alert">${esc(e.message)}</p>`;
  }
}
async function transactions() {
  const g = generation;
  const u = base();
  const r = await api(`${u}/transactions`);
  if (g !== generation) return;
  rows = r.transactions;
  nextCursor = r.next_cursor;
  main.innerHTML = `<h2>${w("transactions")}</h2>${button("new-transaction", w("new"))}<form id="filters"><div class="row">${label("from", input("filter-from", "date"))}${label("to", input("filter-to", "date"))}${label("category", select("filter-category", [["", w("all")], ...categoryOptions()], ""))}${label(
    "kind",
    select(
      "filter-kind",
      [
        ["all", w("all")],
        ["expense", w("expense")],
        ["income", w("income")],
      ],
      "all",
    ),
  )}</div>${button("filter-submit", w("filter"), 'type="submit"')}</form><div id="rows"></div>${button("load-more", w("loadMore"))}`;
  drawRows();
  $('[data-testid="new-transaction"]').onclick = () => transactionForm();
  let listSequence = 0;
  $("#filters").onsubmit = async (e) => {
    e.preventDefault();
    const sequence = ++listSequence;
    try {
      const r = await api(`${u}/transactions?${filters()}`);
      if (g !== generation || sequence !== listSequence) return;
      rows = r.transactions;
      nextCursor = r.next_cursor;
      drawRows();
    } catch (e) {
      if (g === generation) alert(e.message);
    }
  };
  $('[data-testid="load-more"]').onclick = async () => {
    const sequence = ++listSequence;
    try {
      const r = await api(
        `${u}/transactions?${filters()}&cursor=${encodeURIComponent(nextCursor)}`,
      );
      if (g !== generation || sequence !== listSequence) return;
      rows.push(...r.transactions);
      nextCursor = r.next_cursor;
      drawRows();
    } catch (e) {
      if (g === generation) alert(e.message);
    }
  };
}
function filters() {
  const q = new URLSearchParams();
  const from = val("filter-from"),
    to = val("filter-to");
  if (from) q.set("from", localInstant(from, 0));
  if (to) q.set("to", localInstant(to, 0));
  if (val("filter-category")) q.set("category_id", val("filter-category"));
  q.set("kind", val("filter-kind"));
  return q;
}
function drawRows() {
  $("#rows").innerHTML = rows.length
    ? rows
        .map((t) =>
          test(
            `transaction-row-${t.id}`,
            "article",
            `<div class="row"><span><strong>${esc(t.issuer)}</strong><br>${esc(new Intl.DateTimeFormat(selected.language, { timeZone: selected.timezone, dateStyle: "medium" }).format(new Date(t.occurred_at)))} · ${esc(categories.find((c) => c.id === t.category_id)?.[selected.language] || t.category_id)} · ${esc(money(t.amount_minor, t.currency))}</span>${button(`transaction-edit-${t.id}`, w("edit"))}${button(`transaction-delete-${t.id}`, w("delete"))}</div>`,
          ),
        )
        .join("")
    : test("transaction-empty", "p", w("empty"));
  for (const t of rows) {
    $(`[data-testid="transaction-edit-${t.id}"]`).onclick = () =>
      transactionForm(t);
    $(`[data-testid="transaction-delete-${t.id}"]`).onclick = () =>
      deleteForm(t);
  }
  $('[data-testid="load-more"]').hidden = !nextCursor;
}
async function transactionForm(t) {
  const g = generation;
  const latest = await api(`${base()}/transactions/latest`);
  if (g !== generation) return;
  const d = t ? dateInZone(new Date(t.occurred_at)) : dateInZone();
  main.innerHTML = `<form id="transaction-form" novalidate><h2>${w(t ? "edit" : "new")}</h2>${test("latest-transaction", "p", `${w("latest")}: ${esc(latest.transaction?.issuer || w("none"))}`)}${label("date", input("transaction-date", "date", d, "required"))}${label(
    "kind",
    select(
      "transaction-kind",
      [
        ["expense", w("expense")],
        ["income", w("income")],
      ],
      t?.amount_minor > 0 ? "income" : "expense",
    ),
  )}${label("amount", input("transaction-amount", "text", t ? magnitude(t.amount_minor) : "", 'inputmode="decimal" required'))}${label("currency", select("transaction-currency", currencies, t?.currency || selected.reporting_currency))}${label("category", select("transaction-category", categoryOptions(), t?.category_id || categories[0]?.id))}${label("issuer", input("transaction-issuer", "text", t?.issuer || "", 'required maxlength="500"'))}${test("transaction-error", "p", "", 'class="error" role="alert" hidden')}${button("transaction-save", w("save"), 'type="submit"')}${button("transaction-cancel", w("cancel"), 'type="button"')}</form>`;
  $('[data-testid="transaction-cancel"]').onclick = () => render();
  $("#transaction-form").onsubmit = async (e) => {
    e.preventDefault();
    try {
      const amount = val("transaction-amount");
      if (!/^\d+(?:[.,]\d{1,2})?$/.test(amount))
        throw Error(w("invalidAmount"));
      const [whole, fraction = ""] = amount.replace(",", ".").split(".");
      const magnitude = BigInt(whole) * 100n + BigInt(fraction.padEnd(2, "0"));
      if (magnitude > 9007199254740991n) throw Error(w("invalidAmount"));
      const b = {
        occurred_at:
          t && val("transaction-date") === d
            ? t.occurred_at
            : localNoon(val("transaction-date")),
        category_id: val("transaction-category"),
        issuer: val("transaction-issuer"),
        amount_minor:
          Number(magnitude) * (val("transaction-kind") === "expense" ? -1 : 1),
        currency: val("transaction-currency"),
      };
      if (t) b.expected_version = t.version;
      await api(
        `${base()}/transactions${t ? "/" + t.id : ""}`,
        t ? "PATCH" : "POST",
        b,
      );
      if (g !== generation) return;
      view = "transactions";
      await render();
      await backupStatus();
    } catch (e) {
      showError("transaction-error", e);
    }
  };
}
function deleteForm(t) {
  const g = generation;
  main.innerHTML = test(
    "delete-confirmation",
    "article",
    `<h2>${w("delete")}</h2><p>${esc(t.issuer)} · ${esc(money(t.amount_minor, t.currency))}</p>${test("delete-error", "p", "", 'role="alert" class="error" hidden')}${button("delete-confirm", w("confirm"))}${button("delete-cancel", w("cancel"))}`,
  );
  $('[data-testid="delete-cancel"]').onclick = () => render();
  $('[data-testid="delete-confirm"]').onclick = async () => {
    try {
      await api(`${base()}/transactions/${t.id}`, "DELETE", {
        expected_version: t.version,
      });
      if (g !== generation) return;
      await render();
      await backupStatus();
    } catch (e) {
      showError("delete-error", e);
    }
  };
}
async function dashboard() {
  const month = dateInZone().slice(0, 7);
  main.innerHTML = `<h2>${w("dashboard")}</h2>${label("month", input("report-month", "month", month))}${label("currency", select("report-currency", currencies, selected.reporting_currency))}${test("report-error", "p", "", 'class="error" role="alert" hidden')}<div id="report"></div>${label("category", select("trend-category", [["", w("all")], ...categoryOptions()], ""))}<div id="chart"></div>`;
  let reportSequence = 0;
  const update = async () => {
    const n = ++reportSequence,
      g = generation,
      u = base();
    try {
      const month = val("report-month"),
        currency = val("report-currency");
      const r = await api(
        `${u}/reports/month?month=${month}&currency=${currency}`,
      );
      const start = new Date(`${month}-01T12:00:00Z`);
      start.setUTCMonth(start.getUTCMonth() - 11);
      const q = new URLSearchParams({
        from_month: start.toISOString().slice(0, 7),
        to_month: month,
        currency,
      });
      if (val("trend-category")) q.set("category_id", val("trend-category"));
      const trend = await api(`${u}/reports/trend?${q}`);
      if (n !== reportSequence || g !== generation) return;
      $('[data-testid="report-error"]').hidden = true;
      $("#report").innerHTML =
        `<div class="totals"><article><h3>${w("spent")}</h3>${test("current-expense", "strong", esc(money(r.current.expense_minor, currency)), `data-minor="${r.current.expense_minor}"`)}</article><article><h3>${w("income")}</h3>${esc(money(r.current.income_minor, currency))}</article><article><h3>${w("net")}</h3>${esc(money(r.current.net_minor, currency))}</article><article><h3>${w("previous")} ${esc(r.previous.month)}</h3>${test("previous-expense", "strong", esc(money(r.previous.expense_minor, currency)), `data-minor="${r.previous.expense_minor}"`)}</article></div><table><thead><tr><th>${w("category")}</th><th>${w("spent")}</th><th>${w("income")}</th></tr></thead><tbody>${r.categories.filter(
          (c) =>
            BigInt(c.expense_minor) > 0n || BigInt(c.income_minor) > 0n,
        ).map((c) => `<tr><td>${esc(categories.find((cat) => cat.id === c.category_id)[selected.language])}</td><td data-testid="category-total-${c.category_id}" data-minor="${c.expense_minor}">${esc(money(c.expense_minor, currency))}</td><td>${esc(money(c.income_minor, currency))}</td></tr>`).join("")}</tbody></table>`;
      const chartWidth = 720,
        chartHeight = 350,
        left = 92,
        right = 20,
        top = 20,
        bottom = 62,
        plotWidth = chartWidth - left - right,
        plotHeight = chartHeight - top - bottom,
        maxMinor = Math.max(
          1,
          ...trend.points.map((p) => Number(p.expense_minor)),
        ),
        points = trend.points.map((p, i) => ({
          ...p,
          x:
            left +
            (trend.points.length < 2
              ? plotWidth / 2
              : (i * plotWidth) / (trend.points.length - 1)),
          y: top + plotHeight - (Number(p.expense_minor) / maxMinor) * plotHeight,
        })),
        grid = Array.from({ length: 5 }, (_, i) => {
          const fraction = i / 4,
            y = top + fraction * plotHeight,
            value = BigInt(Math.round((maxMinor * (4 - i)) / 4)),
            label = new Intl.NumberFormat(selected.language, {
              style: "currency",
              currency,
              maximumFractionDigits: 0,
            }).format(value);
          return `<g class="chart-tick"><line class="chart-grid" x1="${left}" y1="${y}" x2="${chartWidth - right}" y2="${y}" /><text class="chart-y-label" x="${left - 12}" y="${y}" text-anchor="end" dominant-baseline="middle">${esc(label)}</text></g>`;
        }).join(""),
        line = points.map((p) => `${p.x},${p.y}`).join(" "),
        markers = points
          .map((p) => {
            const month = new Intl.DateTimeFormat(selected.language, {
                timeZone: selected.timezone,
                month: "short",
              }).format(new Date(`${p.month}-01T12:00:00Z`)),
              amount = money(p.expense_minor, currency),
              tip = `${month} ${p.month.slice(0, 4)}: ${amount} · ${w("spent")}`;
            return `<g class="chart-point" data-testid="trend-point-${p.month}" data-minor="${p.expense_minor}" data-chart-tip="${esc(tip)}" tabindex="0" role="img" aria-label="${esc(tip)}"><circle cx="${p.x}" cy="${p.y}" r="6"><title>${esc(tip)}</title></circle><text class="chart-x-label" x="${p.x}" y="${top + plotHeight + 28}" text-anchor="middle">${esc(month)}</text></g>`;
          })
          .join("");
      $("#chart").innerHTML =
        `<h3>${w("trend")}</h3><div class="chart-shell"><svg data-testid="trend-chart" viewBox="0 0 ${chartWidth} ${chartHeight}" role="img" aria-label="${esc(w("trend"))}" preserveAspectRatio="xMidYMid meet">${grid}<line class="chart-axis" x1="${left}" y1="${top}" x2="${left}" y2="${top + plotHeight}"/><line class="chart-axis" x1="${left}" y1="${top + plotHeight}" x2="${chartWidth - right}" y2="${top + plotHeight}"/><text class="chart-axis-title" transform="translate(20 ${top + plotHeight / 2}) rotate(-90)" text-anchor="middle">${esc(w("spent"))}</text><polyline class="chart-line" points="${line}"/>${markers}</svg><output class="chart-tooltip" data-testid="chart-tooltip" hidden></output></div>`;
      const tooltip = $('[data-testid="chart-tooltip"]');
      for (const point of $("#chart").querySelectorAll("[data-chart-tip]")) {
        const show = () => {
          tooltip.textContent = point.dataset.chartTip;
          tooltip.hidden = false;
        };
        const hide = () => {
          tooltip.hidden = true;
        };
        point.addEventListener("pointerenter", show);
        point.addEventListener("pointerleave", hide);
        point.addEventListener("focus", show);
        point.addEventListener("blur", hide);
      }
    } catch (e) {
      if (n === reportSequence && g === generation) {
        $("#report").innerHTML = "";
        $("#chart").innerHTML = "";
        showError("report-error", e);
      }
    }
  };
  for (const id of ["report-month", "report-currency", "trend-category"])
    $(`[data-testid="${id}"]`).onchange = update;
  await update();
}
function settings() {
  const g = generation;
  rates = selected.rates.map((r) => ({ ...r }));
  main.innerHTML = `<form id="settings-form"><h2>${w("settings")}</h2>${label(
    "language",
    select(
      "settings-language",
      [
        ["en", "English"],
        ["it", "Italiano"],
      ],
      selected.language,
    ),
  )}${label("timezone", input("settings-timezone", "text", selected.timezone, "required"))}${label("currency", select("settings-currency", currencies, selected.reporting_currency))}<h3>${w("rates")}</h3><div id="rates"></div>${button("rate-add", w("addRate"), 'type="button"')}${test("settings-error", "p", "", 'class="error" role="alert" hidden')}${button("settings-save", w("save"), 'type="submit"')}</form>`;
  drawRates();
  $('[data-testid="rate-add"]').onclick = () => {
    readRates();
    rates.push({ from: "EUR", to: selected.reporting_currency, rate: "" });
    drawRates();
  };
  $("#settings-form").onsubmit = async (e) => {
    e.preventDefault();
    try {
      readRates();
      const r = await api(`${base()}/settings`, "PATCH", {
        expected_version: selected.version,
        language: val("settings-language"),
        timezone: val("settings-timezone"),
        reporting_currency: val("settings-currency"),
        rates,
      });
      if (g !== generation) return;
      selected = r.user;
      document.documentElement.lang = selected.language;
      $("#user-menu").textContent=w("users");
      nav();
      settings();
      await backupStatus();
    } catch (e) {
      showError("settings-error", e);
    }
  };
}
function readRates() {
  rates = rates.map((_, i) => ({
    from: val(`rate-from-${i}`),
    to: val(`rate-to-${i}`),
    rate: val(`rate-value-${i}`),
  }));
}
function drawRates() {
  $("#rates").innerHTML = rates
    .map(
      (r, i) =>
        `<div class="row">${label("from", select(`rate-from-${i}`, currencies, r.from))}${label("to", select(`rate-to-${i}`, currencies, r.to))}${label("rate", input(`rate-value-${i}`, "text", r.rate, 'inputmode="decimal"'))}${button(`rate-remove-${i}`, w("remove"), 'type="button"')}</div>`,
    )
    .join("");
  rates.forEach((_, i) => {
    $(`[data-testid="rate-remove-${i}"]`).onclick = () => {
      readRates();
      rates.splice(i, 1);
      drawRates();
    };
  });
}
function monzo() {
  const g = generation;
  main.innerHTML = `<form id="monzo-form"><h2>${w("monzo")}</h2>${label("token", input("monzo-token", "password", "", 'autocomplete="off" required'))}<p>${esc(w("tokenHelp"))}</p>${label("since", input("monzo-since", "date"))}${test("monzo-account-choice", "div", "", "hidden")}${button("monzo-import", w("import"), 'type="submit"')}${test("monzo-result", "p", "", 'role="status" hidden')}${test("monzo-error", "p", "", 'role="alert" class="error" hidden')}</form>`;
  $("#monzo-form").onsubmit = async (e) => {
    e.preventDefault();
    const token = $('[data-testid="monzo-token"]'),
      btn = $('[data-testid="monzo-import"]');
    const accountSelect = $('[data-testid="monzo-account-select"]');
    if (accountSelect && !accountSelect.value) {
      accountSelect.reportValidity();
      return;
    }
    const error = $('[data-testid="monzo-error"]');
    error.hidden = true;
    error.textContent = "";
    btn.disabled = true;
    let keepToken = false;
    try {
      const b = { access_token: token.value };
      if (accountSelect) b.account_id = accountSelect.value;
      if (val("monzo-since")) b.since = `${val("monzo-since")}T00:00:00Z`;
      const r = await api(`${base()}/imports/monzo`, "POST", b);
      if (g !== generation) return;
      $("[data-testid=monzo-account-choice]").hidden = true;
      const result = $('[data-testid="monzo-result"]');
      result.hidden = false;
      result.textContent = `${w("result")}: ${r.inserted} ${w("inserted")}, ${r.duplicates} ${w("duplicates")}, ${r.excluded} ${w("excluded")}, ${r.reconciled} ${w("reconciled")}`;
    } catch (e) {
      if (e.code === "monzo_account_selection_required" && Array.isArray(e.details?.accounts)) {
        const options = [["", w("accountPlaceholder")], ...e.details.accounts.map((a) => [a.id, `${a.description} (${a.id})`])];
        const choice = $("[data-testid=monzo-account-choice]");
        choice.innerHTML = `<p>${esc(w("accountChoice"))}</p>${label("account", select("monzo-account-select", options, ""))}`;
        choice.hidden = false;
        keepToken = true;
      } else {
        showError("monzo-error", e);
      }
    } finally {
      if (!keepToken) token.value = "";
      btn.disabled = false;
      await backupStatus();
    }
  };
}
async function backupStatus() {
  try {
    const s = await api("/backups/status");
    $("#backup").innerHTML =
      test(
        "backup-status",
        "span",
        esc(
          !(s.dropbox_configured ?? s.dropbox_revision !== null)
            ? w("notConfigured")
            : s.pending
              ? w("pending")
              : w("backed"),
        ),
        `data-pending="${s.pending}"`,
      ) +
      (s.last_error
        ? test(
            "backup-error",
            "span",
            ` · ${esc(w("backupError"))} ${button("backup-retry", w("retry"))}`,
          )
        : "");
    const retry = $('[data-testid="backup-retry"]');
    if (retry)
      retry.onclick = async () => {
        await api("/backups/retry", "POST", {});
        await backupStatus();
      };
  } catch {}
}
await loadUsers();
const remembered = users.find(
  (u) => u.id === localStorage.getItem("selectedUser"),
);
if (remembered) await choose(remembered);
else await render();
setInterval(backupStatus, 15000);
