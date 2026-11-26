// Riptor's web interface. Everything shown comes from torrents, peers, feeds and plugins,
// so it's only ever set as text, never parsed as HTML.

const API = "api/v2/";

// ------------------------------------------------------------------ helpers

function h(tag, props, ...kids) {
  const el = document.createElement(tag);
  if (props) {
    for (const [k, v] of Object.entries(props)) {
      if (v === undefined || v === null || v === false) continue;
      if (k === "class") el.className = v;
      else if (k === "text") el.textContent = v;
      else if (k === "style") Object.assign(el.style, v);
      else if (k === "data") Object.assign(el.dataset, v);
      else if (k.startsWith("on")) el.addEventListener(k.slice(2).toLowerCase(), v);
      else if (k === "value") el.value = v;
      else if (k === "checked") el.checked = !!v;
      else if (k === "selected") el.selected = !!v;
      else if (k === "disabled") el.disabled = !!v;
      else el.setAttribute(k, v === true ? "" : v);
    }
  }
  for (const kid of kids.flat(Infinity)) {
    if (kid === null || kid === undefined || kid === false) continue;
    el.append(kid instanceof Node ? kid : document.createTextNode(String(kid)));
  }
  return el;
}

// replaceChildren that takes nested arrays and skips empty slots, like h().
function fill(el, ...kids) {
  el?.replaceChildren(...kids.flat(Infinity).filter((k) => k !== null && k !== undefined && k !== false));
}

const ICONS = {
  down: "M12 4v14M6 12l6 6 6-6",
  up: "M12 20V6M6 12l6-6 6 6",
  play: "M7 5l12 7-12 7z",
  stop: "M7 7h10v10H7z",
  plus: "M12 5v14M5 12h14",
  trash: "M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3",
  search: "M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14zM20 20l-4-4",
  rss: "M5 5a14 14 0 0 1 14 14M5 11a8 8 0 0 1 8 8M6 19h.01",
  list: "M8 6h12M8 12h12M8 18h12M4 6h.01M4 12h.01M4 18h.01",
  log: "M4 5h16v14H4zM8 10l3 2-3 2M13 15h4",
  sliders: "M4 7h10M18 7h2M4 17h4M12 17h8M16 5v4M10 15v4",
  logout: "M15 4h4v16h-4M10 8l-4 4 4 4M6 12h11",
  sun: "M12 17a5 5 0 1 0 0-10 5 5 0 0 0 0 10zM12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4",
  moon: "M20 14.5A8 8 0 0 1 9.5 4 8 8 0 1 0 20 14.5z",
  gauge: "M12 14l4-4M4 18a9 9 0 1 1 16 0",
  menu: "M4 6h16M4 12h16M4 18h16",
  x: "M6 6l12 12M18 6L6 18",
  check: "M5 12l5 5L20 7",
  alert: "M12 9v4M12 17h.01M10.3 3.9L2 18a2 2 0 0 0 1.7 3h16.6a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z",
  info: "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM12 16v-4M12 8h.01",
  folder: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z",
  link: "M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1",
  file: "M14 3H6v18h12V7zM14 3v4h4",
  refresh: "M20 11a8 8 0 0 0-14.9-3M4 4v4h4M4 13a8 8 0 0 0 14.9 3M20 20v-4h-4",
  tag: "M3 12V3h9l9 9-9 9zM7.5 7.5h.01",
  layers: "M12 3l9 5-9 5-9-5zM3 13l9 5 9-5",
  ext: "M14 4h6v6M20 4l-9 9M18 14v6H4V6h6",
  puzzle: "M10 4a2 2 0 1 1 4 0v2h4v4h-2a2 2 0 1 0 0 4h2v4h-4v-2a2 2 0 1 0-4 0v2H6v-4H4a2 2 0 1 1 0-4h2V6h4z",
  eye: "M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12zM12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z",
  eyeoff: "M3 3l18 18M10.6 10.6a3 3 0 0 0 4.2 4.2M9.9 5.2A10 10 0 0 1 12 5c6 0 10 7 10 7a17 17 0 0 1-3.3 4M6.6 6.6C3.8 8.4 2 12 2 12s4 7 10 7a9.6 9.6 0 0 0 5.4-1.6",
  globe: "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM2 12h20M12 2a15 15 0 0 1 0 20M12 2a15 15 0 0 0 0 20",
  zap: "M13 2L4 14h7l-1 8 9-12h-7z",
  copy: "M8 8h12v12H8zM16 8V4H4v12h4",
  move: "M5 9l-3 3 3 3M9 5l3-3 3 3M15 19l-3 3-3-3M19 9l3 3-3 3M2 12h20M12 2v20",
  lock: "M6 11h12v10H6zM8 11V7a4 4 0 0 1 8 0v4",
  user: "M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM4 21a8 8 0 0 1 16 0",
  keyboard: "M3 6h18v12H3zM7 10h.01M11 10h.01M15 10h.01M7 14h10",
  check2: "M20 6L9 17l-5-5",
  chevron: "M9 6l6 6-6 6",
  sort: "M7 4v16M3 16l4 4 4-4M17 20V4M13 8l4-4 4 4",
};

function icon(name, extra) {
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("class", "icon" + (extra ? " " + extra : ""));
  svg.setAttribute("aria-hidden", "true");
  const p = document.createElementNS(ns, "path");
  p.setAttribute("d", ICONS[name] || ICONS.info);
  svg.append(p);
  return svg;
}

const UNITS = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
function bytes(n) {
  if (n === undefined || n === null || n < 0) return "–";
  let i = 0;
  let v = n;
  while (v >= 1024 && i < UNITS.length - 1) {
    v /= 1024;
    i++;
  }
  return `${i === 0 ? v : v.toFixed(v < 10 ? 2 : v < 100 ? 1 : 0)} ${UNITS[i]}`;
}
const speed = (n) => (n > 0 ? `${bytes(n)}/s` : "–");
function eta(s) {
  if (s === undefined || s < 0 || s >= 8640000) return "∞";
  if (s < 60) return `${s}s`;
  const d = Math.floor(s / 86400);
  const hr = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (d) return `${d}d ${hr}h`;
  if (hr) return `${hr}h ${m}m`;
  return `${m}m`;
}
function date(secs) {
  if (!secs || secs < 0) return "–";
  return new Date(secs * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}
function ago(secs) {
  if (!secs || secs < 0) return "–";
  const d = Date.now() / 1000 - secs;
  if (d < 60) return "just now";
  if (d < 3600) return `${Math.floor(d / 60)} min ago`;
  if (d < 86400) return `${Math.floor(d / 3600)} h ago`;
  if (d < 86400 * 30) return `${Math.floor(d / 86400)} d ago`;
  return new Date(secs * 1000).toLocaleDateString();
}
const ratio = (r) => (r < 0 ? "∞" : r.toFixed(2));
const pct = (p) => `${(Math.min(1, Math.max(0, p)) * 100).toFixed(p >= 1 || p === 0 ? 0 : 1)}%`;

function debounce(fn, ms) {
  let t;
  return (...a) => {
    clearTimeout(t);
    t = setTimeout(() => fn(...a), ms);
  };
}

const store = {
  get(k, d) {
    try {
      const v = localStorage.getItem("riptor." + k);
      return v === null ? d : JSON.parse(v);
    } catch {
      return d;
    }
  },
  set(k, v) {
    try {
      localStorage.setItem("riptor." + k, JSON.stringify(v));
    } catch {
      /* private mode */
    }
  },
};

// ---------------------------------------------------------------------- API

class SignedOut extends Error {}

async function call(path, params, opts = {}) {
  const method = opts.method || (params instanceof FormData || opts.post ? "POST" : "GET");
  let url = API + path;
  const init = { method, credentials: "same-origin", headers: {} };
  if (method === "GET" && params) {
    const q = new URLSearchParams(params).toString();
    if (q) url += "?" + q;
  } else if (params instanceof FormData) {
    init.body = params;
  } else if (params) {
    init.body = new URLSearchParams(params);
  }
  const r = await fetch(url, init);
  if (r.status === 403 && path !== "auth/login") {
    const msg = await r.text();
    if (msg === "Forbidden") {
      showLogin();
      throw new SignedOut();
    }
    throw new Error(msg || "Not allowed");
  }
  if (!r.ok) throw new Error((await r.text()) || `${r.status} ${r.statusText}`);
  const type = r.headers.get("content-type") || "";
  return type.includes("json") ? r.json() : r.text();
}
const post = (path, params) => call(path, params || {}, { post: true });

async function attempt(fn, done) {
  try {
    const r = await fn();
    if (done) toast(done, "ok");
    return r;
  } catch (e) {
    if (!(e instanceof SignedOut)) toast(e.message || String(e), "error");
    return undefined;
  }
}

// ------------------------------------------------------------------- toasts

let toastBox;
function toast(text, kind = "info") {
  if (!toastBox) {
    toastBox = h("div", { class: "toasts", role: "status", "aria-live": "polite" });
    document.body.append(toastBox);
  }
  const el = h(
    "div",
    { class: `toast ${kind}` },
    icon(kind === "ok" ? "check" : kind === "error" ? "alert" : "info"),
    h("div", { class: "grow", text }),
  );
  toastBox.append(el);
  setTimeout(() => {
    el.classList.add("leave");
    setTimeout(() => el.remove(), 300);
  }, kind === "error" ? 6000 : 3200);
}

// ------------------------------------------------------------ dialogs, menus

function dialog({ title, body, actions = [], wide = false, onClose }) {
  const close = () => {
    veil.remove();
    document.removeEventListener("keydown", onKey, true);
    onClose?.();
  };
  const onKey = (e) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      close();
    }
  };
  const footer = actions.length
    ? h(
        "footer",
        null,
        actions.map((a) =>
          h(
            "button",
            {
              class: `btn ${a.kind || ""} ${a.left ? "left" : ""}`,
              type: a.submit ? "submit" : "button",
              onclick: a.submit
                ? undefined
                : async () => {
                    if (a.run && (await a.run()) === false) return;
                    if (!a.keep) close();
                  },
            },
            a.icon && icon(a.icon),
            a.label,
          ),
        ),
      )
    : null;
  const box = h(
    "form",
    {
      class: `dialog ${wide ? "wide" : ""}`,
      onsubmit: async (e) => {
        e.preventDefault();
        const a = actions.find((x) => x.submit);
        if (a?.run && (await a.run()) === false) return;
        close();
      },
    },
    h("header", null, h("h3", { text: title }), h("button", { class: "btn ghost icon-only", type: "button", "aria-label": "Close", onclick: close }, icon("x"))),
    h("div", { class: "content" }, body),
    footer,
  );
  const veil = h("div", { class: "veil", onmousedown: (e) => e.target === veil && close() }, box);
  document.body.append(veil);
  document.addEventListener("keydown", onKey, true);
  setTimeout(() => box.querySelector("input:not([type=checkbox]), textarea, select")?.focus(), 30);
  return { close, box };
}

function confirmBox(title, text, label, kind = "primary") {
  return new Promise((resolve) => {
    let ok = false;
    dialog({
      title,
      body: h("p", { class: "muted", text }),
      actions: [
        { label: "Cancel" },
        { label, kind, submit: true, run: () => (ok = true) },
      ],
      onClose: () => resolve(ok),
    });
  });
}

function prompt(title, label, value = "", hint) {
  return new Promise((resolve) => {
    let out = null;
    const input = h("input", { class: "input", value });
    dialog({
      title,
      body: h("label", { class: "field" }, h("span", { text: label }), input, hint && h("small", { class: "hint", text: hint })),
      actions: [{ label: "Cancel" }, { label: "Save", kind: "primary", submit: true, run: () => (out = input.value.trim()) }],
      onClose: () => resolve(out),
    });
  });
}

let openMenu;
function menu(x, y, items) {
  openMenu?.remove();
  const el = h(
    "div",
    { class: "menu", role: "menu" },
    items.map((it) =>
      it === "-"
        ? h("hr")
        : h(
            "button",
            {
              class: it.danger ? "danger" : "",
              role: "menuitem",
              disabled: it.disabled,
              onclick: () => {
                el.remove();
                it.run();
              },
            },
            icon(it.icon || "chevron"),
            it.label,
            it.key && h("kbd", { text: it.key }),
          ),
    ),
  );
  document.body.append(el);
  const r = el.getBoundingClientRect();
  el.style.left = `${Math.min(x, innerWidth - r.width - 8)}px`;
  el.style.top = `${Math.min(y, innerHeight - r.height - 8)}px`;
  openMenu = el;
  setTimeout(() => {
    const off = (e) => {
      if (!el.contains(e.target)) {
        el.remove();
        document.removeEventListener("mousedown", off);
      }
    };
    document.addEventListener("mousedown", off);
  });
}

function switchEl(label, checked, onchange, hint) {
  const input = h("input", { type: "checkbox", checked, onchange: (e) => onchange?.(e.target.checked) });
  return h("label", { class: "switch" }, input, h("i"), h("div", null, h("div", { text: label }), hint && h("small", { class: "hint", text: hint })));
}

function field(label, control, hint) {
  return h("label", { class: "field" }, h("span", { text: label }), control, hint && h("small", { class: "hint", text: hint }));
}

function select(options, value, onchange) {
  return h(
    "select",
    { class: "input", onchange: (e) => onchange?.(e.target.value) },
    options.map(([v, label]) => h("option", { value: v, selected: v === value, text: label })),
  );
}

// ---------------------------------------------------------------- the state

const S = {
  torrents: {},
  server: {},
  categories: {},
  tags: [],
  selected: new Set(),
  anchor: null,
  focus: null,
  status: store.get("status", "all"),
  category: null,
  tag: null,
  text: "",
  sort: store.get("sort", { key: "added_on", asc: false }),
  tab: store.get("tab", "transfers"),
  detailsTab: store.get("detailsTab", "general"),
  detailsOpen: store.get("detailsOpen", true),
  history: { down: [], up: [] },
  version: "",
  timers: [],
};

// ----------------------------------------------------------------- the shell

const root = () => document.getElementById("app");

function applyTheme() {
  const t = store.get("theme", "auto");
  const light = t === "light" || (t === "auto" && matchMedia("(prefers-color-scheme: light)").matches);
  document.documentElement.dataset.theme = light ? "light" : "dark";
}
applyTheme();
matchMedia("(prefers-color-scheme: light)").addEventListener("change", applyTheme);

function stopTimers() {
  S.timers.forEach(clearInterval);
  S.timers = [];
}

function every(ms, fn) {
  const tick = () => document.visibilityState === "visible" && fn();
  S.timers.push(setInterval(tick, ms));
  fn();
}

function showLogin(message) {
  stopTimers();
  document.title = "Riptor · Sign in";
  const user = h("input", { class: "input", name: "username", autocomplete: "username", value: store.get("user", "admin"), required: true });
  const pass = h("input", { class: "input", name: "password", type: "password", autocomplete: "current-password", required: true, maxlength: 256 });
  const eye = h("button", { class: "btn ghost icon-only", type: "button", "aria-label": "Show password", onclick: () => {
    const show = pass.type === "password";
    pass.type = show ? "text" : "password";
    fill(eye, icon(show ? "eyeoff" : "eye"));
  } }, icon("eye"));
  const error = h("div", { class: "error", text: message || "" });
  const submit = h("button", { class: "btn primary", type: "submit" }, icon("lock"), "Sign in");
  const form = h(
    "form",
    {
      onsubmit: async (e) => {
        e.preventDefault();
        submit.disabled = true;
        fill(submit, h("span", { class: "spinner" }), "Signing in…");
        error.textContent = "";
        try {
          const r = await fetch(API + "auth/login", {
            method: "POST",
            credentials: "same-origin",
            body: new URLSearchParams({ username: user.value.trim(), password: pass.value }),
          });
          const t = await r.text();
          if (r.ok && t === "Ok.") {
            store.set("user", user.value.trim());
            pass.value = "";
            start();
            return;
          }
          error.textContent =
            r.status === 403
              ? "Too many failed sign-ins. Wait a while before trying again."
              : "Wrong user name or password.";
          pass.value = "";
          pass.focus();
        } catch {
          error.textContent = "Can't reach Riptor.";
        }
        submit.disabled = false;
        fill(submit, icon("lock"), "Sign in");
      },
    },
    h("div", { class: "brand" }, h("img", { src: "web/icon.svg", alt: "" }), h("h1", { text: "Riptor" }), h("div", { class: "muted", text: "Sign in to manage your downloads" })),
    error,
    field("User name", h("div", { class: "search-box" }, icon("user"), user)),
    field("Password", h("div", { class: "search-box" }, icon("lock"), pass, h("div", { style: { position: "absolute", right: "2px" } }, eye))),
    submit,
  );
  root().className = "";
  fill(root(), h("div", { class: "login" }, form));
  setTimeout(() => (user.value ? pass : user).focus(), 30);
}

async function start() {
  try {
    S.version = await call("app/version");
  } catch (e) {
    if (!(e instanceof SignedOut)) showLogin();
    return;
  }
  stopTimers();
  buildShell();
}

const TABS = [
  ["transfers", "Transfers", "list"],
  ["search", "Search", "search"],
  ["rss", "RSS", "rss"],
  ["log", "Log", "log"],
];

let UI = {};

function sparkline(canvas, data, color) {
  const dpr = devicePixelRatio || 1;
  const w = canvas.clientWidth || 64;
  const hgt = canvas.clientHeight || 22;
  canvas.width = w * dpr;
  canvas.height = hgt * dpr;
  const c = canvas.getContext("2d");
  c.scale(dpr, dpr);
  const max = Math.max(1, ...data);
  c.beginPath();
  data.forEach((v, i) => {
    const x = (i / Math.max(1, data.length - 1)) * w;
    const y = hgt - 2 - (v / max) * (hgt - 4);
    i ? c.lineTo(x, y) : c.moveTo(x, y);
  });
  c.strokeStyle = color;
  c.lineWidth = 1.5;
  c.stroke();
  c.lineTo(w, hgt);
  c.lineTo(0, hgt);
  c.closePath();
  const g = c.createLinearGradient(0, 0, 0, hgt);
  g.addColorStop(0, color + "55");
  g.addColorStop(1, color + "00");
  c.fillStyle = g;
  c.fill();
}

function buildShell() {
  document.title = "Riptor";
  const tabs = h(
    "nav",
    { class: "tabs" },
    TABS.map(([id, label, ic]) =>
      h("button", { data: { tab: id }, title: label, onclick: () => setTab(id) }, icon(ic), h("span", { class: "label", text: label })),
    ),
  );
  const speedEl = (kind) => {
    const b = h("b", { text: "–" });
    const small = h("small");
    const canvas = h("canvas");
    return { el: h("div", { class: `speed ${kind}`, title: kind === "down" ? "Download speed" : "Upload speed" }, icon(kind), h("div", null, b, h("br"), small), canvas), b, small, canvas };
  };
  const down = speedEl("down");
  const up = speedEl("up");
  const alt = h("button", { class: "btn icon-only wide-only", title: "Alternative speed limits", onclick: toggleAlt }, icon("gauge"));
  const theme = h("button", { class: "btn icon-only ghost wide-only", title: "Switch theme", onclick: cycleTheme }, icon(document.documentElement.dataset.theme === "light" ? "moon" : "sun"));
  const top = h(
    "header",
    { class: "topbar" },
    h("button", { class: "btn ghost icon-only menu-toggle", "aria-label": "Filters", onclick: () => UI.sidebar.classList.toggle("open") }, icon("menu")),
    h("div", { class: "logo" }, h("img", { src: "web/icon.svg", alt: "" }), h("span", { text: "Riptor" })),
    tabs,
    h("div", { class: "speeds" }, down.el, up.el),
    alt,
    theme,
    h("button", { class: "btn icon-only ghost", title: "Options", onclick: openSettings }, icon("sliders")),
    h("button", { class: "btn icon-only ghost wide-only", title: "Keyboard shortcuts", onclick: showShortcuts }, icon("keyboard")),
    h("button", { class: "btn icon-only ghost", title: "Sign out", onclick: signOut }, icon("logout")),
  );
  const sidebar = h("aside", { class: "sidebar" });
  const main = h("main", { class: "main" });
  const body = h("div", { class: "body" }, sidebar, main);
  const status = h("footer", { class: "statusbar" });
  const veil = h("div", { class: "drop-veil" }, h("div", null, icon("down"), "Drop .torrent files to add them"));
  root().className = "";
  fill(root(), h("div", { class: "shell" }, top, body, status), veil);
  UI = { tabs, down, up, alt, theme, sidebar, main, body, status, veil };
  setTab(S.tab, true);
  every(1500, poll);
}

async function signOut() {
  await attempt(() => post("auth/logout"));
  showLogin("Signed out.");
}

function cycleTheme() {
  const now = document.documentElement.dataset.theme;
  store.set("theme", now === "light" ? "dark" : "light");
  applyTheme();
  fill(UI.theme, icon(document.documentElement.dataset.theme === "light" ? "moon" : "sun"));
}

async function toggleAlt() {
  await attempt(() => post("transfer/toggleSpeedLimitsMode"));
  poll();
}

const pageTimers = [];
function setTab(id, first) {
  if (!TABS.some((t) => t[0] === id)) id = "transfers";
  S.tab = id;
  store.set("tab", id);
  pageTimers.splice(0).forEach(clearInterval);
  for (const b of UI.tabs.children) b.classList.toggle("active", b.dataset.tab === id);
  UI.body.classList.toggle("no-side", id !== "transfers");
  UI.sidebar.classList.remove("open");
  UI.tx = UI.sr = UI.rss = UI.log = null;
  clearInterval(detailsTimer);
  detailsFor = null;
  fill(UI.main, );
  if (id === "transfers") transfersPage();
  else if (id === "search") searchPage();
  else if (id === "rss") rssPage();
  else logPage();
  if (!first) poll();
}

function pageEvery(ms, fn) {
  const t = setInterval(() => document.visibilityState === "visible" && fn(), ms);
  pageTimers.push(t);
  fn();
}

// --------------------------------------------------------------- polling

async function poll() {
  let d;
  try {
    d = await call("sync/maindata");
  } catch (e) {
    if (!(e instanceof SignedOut)) fill(UI.status, h("span", { class: "row" }, icon("alert"), "Connection lost, retrying…"));
    return;
  }
  S.torrents = d.torrents || {};
  S.server = d.server_state || {};
  S.categories = d.categories || {};
  S.tags = d.tags || [];
  for (const id of [...S.selected]) if (!S.torrents[id]) S.selected.delete(id);
  const s = S.server;
  S.history.down.push(s.dl_info_speed || 0);
  S.history.up.push(s.up_info_speed || 0);
  for (const k of ["down", "up"]) if (S.history[k].length > 60) S.history[k].shift();
  UI.down.b.textContent = speed(s.dl_info_speed) === "–" ? "0 B/s" : speed(s.dl_info_speed);
  UI.up.b.textContent = speed(s.up_info_speed) === "–" ? "0 B/s" : speed(s.up_info_speed);
  UI.down.small.textContent = s.dl_rate_limit ? `limit ${bytes(s.dl_rate_limit)}/s` : bytes(s.dl_info_data) + " this session";
  UI.up.small.textContent = s.up_rate_limit ? `limit ${bytes(s.up_rate_limit)}/s` : bytes(s.up_info_data) + " this session";
  sparkline(UI.down.canvas, S.history.down, getComputedStyle(document.documentElement).getPropertyValue("--down").trim() || "#38bdf8");
  sparkline(UI.up.canvas, S.history.up, getComputedStyle(document.documentElement).getPropertyValue("--up").trim() || "#c084fc");
  UI.alt.classList.toggle("on", !!s.use_alt_speed_limits);
  const n = Object.keys(S.torrents).length;
  const active = Object.values(S.torrents).filter((t) => t.dlspeed > 0 || t.upspeed > 0).length;
  document.title = s.dl_info_speed > 0 ? `↓ ${speed(s.dl_info_speed)} · Riptor` : "Riptor";
  renderStatus(n, active);
  const tb = UI.tabs.querySelector('[data-tab="transfers"]');
  tb.querySelector(".count")?.remove();
  if (n) tb.append(h("span", { class: "count", text: n }));
  if (S.tab === "transfers") renderTransfers();
}

function renderStatus(n, active) {
  const s = S.server;
  const conn = s.connection_status === "connected";
  fill(UI.status, 
    h("span", { class: "row" }, h("span", { class: "dot", style: { background: conn ? "var(--ok)" : "var(--danger)" } }), conn ? "Connected" : "Held: no traffic"),
    h("span", { class: "row" }, icon("globe"), `${s.dht_nodes || 0} DHT nodes`),
    h("span", { class: "row" }, icon("folder"), `${bytes(s.free_space_on_disk)} free`),
    h("span", { class: "row" }, icon("layers"), `${n} torrents, ${active} active`),
    h("span", { class: "row push" }, icon("down"), `${bytes(s.alltime_dl)} all time`),
    h("span", { class: "row" }, icon("up"), `${bytes(s.alltime_ul)} · ratio ${s.global_ratio || "0.00"}`),
    h("span", { class: "faint", text: `Riptor (API ${S.version})` }),
  );
}

// ------------------------------------------------------------- transfers

const DOWN_STATES = ["downloading", "forcedDL", "metaDL", "stalledDL", "queuedDL", "checkingDL", "allocating", "moving"];
const SEED_STATES = ["uploading", "forcedUP", "stalledUP", "queuedUP", "checkingUP"];

function kind(t) {
  const s = t.state;
  if (s === "error" || s === "missingFiles") return "error";
  if (s.startsWith("stopped") || s.startsWith("paused")) return "stopped";
  if (s.startsWith("checking")) return "checking";
  if (s.startsWith("queued")) return "queued";
  if (s === "metaDL") return "meta";
  if (s.startsWith("stalled")) return "stalled";
  if (SEED_STATES.includes(s)) return "seeding";
  return "downloading";
}

const STATE_LABEL = {
  downloading: ["Downloading", "down"],
  forcedDL: ["Downloading (forced)", "down"],
  metaDL: ["Getting metadata", "info"],
  stalledDL: ["Stalled", "warn"],
  queuedDL: ["Queued", ""],
  checkingDL: ["Checking", "info"],
  uploading: ["Seeding", "up"],
  forcedUP: ["Seeding (forced)", "up"],
  stalledUP: ["Seeding", "ok"],
  queuedUP: ["Queued to seed", ""],
  checkingUP: ["Checking", "info"],
  stoppedDL: ["Stopped", ""],
  stoppedUP: ["Completed", "ok"],
  pausedDL: ["Stopped", ""],
  pausedUP: ["Completed", "ok"],
  error: ["Error", "danger"],
  missingFiles: ["Files missing", "danger"],
  moving: ["Moving", "info"],
};

const FILTERS = [
  ["all", "All", "layers", () => true],
  ["downloading", "Downloading", "down", (t) => DOWN_STATES.includes(t.state)],
  ["seeding", "Seeding", "up", (t) => SEED_STATES.includes(t.state)],
  ["completed", "Completed", "check", (t) => t.progress >= 1],
  ["stopped", "Stopped", "stop", (t) => kind(t) === "stopped"],
  ["active", "Active", "zap", (t) => t.dlspeed > 0 || t.upspeed > 0],
  ["inactive", "Inactive", "gauge", (t) => !(t.dlspeed > 0 || t.upspeed > 0)],
  ["stalled", "Stalled", "alert", (t) => t.state.startsWith("stalled")],
  ["checking", "Checking", "refresh", (t) => t.state.startsWith("checking")],
  ["errored", "Errored", "alert", (t) => kind(t) === "error"],
];

const COLUMNS = [
  { key: "name", label: "Name", width: "auto", render: nameCell },
  { key: "size", label: "Size", width: "92px", num: true, opt: true, render: (t) => bytes(t.size) },
  { key: "progress", label: "Progress", width: "150px", render: progressCell },
  { key: "state", label: "Status", width: "140px", opt: true, render: stateCell },
  { key: "num_complete", label: "Seeds", width: "72px", num: true, opt: true, render: (t) => `${t.num_complete ?? 0}` },
  { key: "num_leechs", label: "Peers", width: "72px", num: true, opt: true, render: (t) => `${t.num_leechs ?? 0}` },
  { key: "dlspeed", label: "Down", width: "100px", num: true, render: (t) => speed(t.dlspeed) },
  { key: "upspeed", label: "Up", width: "100px", num: true, opt: true, render: (t) => speed(t.upspeed) },
  { key: "eta", label: "ETA", width: "80px", num: true, opt: true, render: (t) => (t.progress >= 1 ? "–" : eta(t.eta)) },
  { key: "ratio", label: "Ratio", width: "70px", num: true, opt: true, render: (t) => ratio(t.ratio) },
  { key: "added_on", label: "Added", width: "120px", opt: true, render: (t) => ago(t.added_on) },
];

function nameCell(t) {
  const sub = [t.category, t.tags].filter(Boolean).join(" · ");
  return [h("span", { text: t.name, title: t.name }), sub && h("span", { class: "sub", text: sub })];
}

function progressCell(t) {
  const k = kind(t);
  const cls = t.progress >= 1 ? "done" : k === "stopped" ? "stopped" : k === "error" ? "error" : k === "checking" || k === "meta" ? "busy" : "";
  return bar(t.progress, cls);
}

function bar(p, cls = "") {
  return h("div", { class: "bar-cell" }, h("div", { class: `progress ${p >= 1 ? "done" : cls}` }, h("i", { style: { width: pct(p) } })), h("span", { text: pct(p) }));
}

function stateCell(t) {
  const [label, cls] = STATE_LABEL[t.state] || [t.state, ""];
  return h("span", { class: `badge ${cls}` }, label);
}

function visible() {
  const f = FILTERS.find((x) => x[0] === S.status)?.[3] || (() => true);
  const words = S.text.toLowerCase().split(/\s+/).filter(Boolean);
  const list = Object.entries(S.torrents)
    .map(([hash, t]) => ({ ...t, hash }))
    .filter(f)
    .filter((t) => S.category === null || (t.category || "") === S.category)
    .filter((t) => S.tag === null || (S.tag === "" ? !t.tags : t.tags.split(", ").includes(S.tag)))
    .filter((t) => words.every((w) => t.name.toLowerCase().includes(w)));
  const { key, asc } = S.sort;
  list.sort((a, b) => {
    let x = a[key];
    let y = b[key];
    if (key === "eta") {
      x = a.progress >= 1 ? Infinity : x;
      y = b.progress >= 1 ? Infinity : y;
    }
    const c = typeof x === "string" ? x.localeCompare(y, undefined, { numeric: true, sensitivity: "base" }) : (x ?? 0) - (y ?? 0);
    return asc ? c : -c;
  });
  return list;
}

function transfersPage() {
  const filterInput = h("input", {
    class: "input",
    placeholder: "Filter by name   /",
    value: S.text,
    oninput: debounce((e) => {
      S.text = e.target.value;
      renderTransfers();
    }, 120),
  });
  const btn = (ic, label, run, extra = "") => h("button", { class: `btn ${extra}`, title: label, onclick: run }, icon(ic), h("span", { class: "label", text: label }));
  const selectionButtons = [
    btn("play", "Start", () => act("torrents/start")),
    btn("stop", "Stop", () => act("torrents/stop")),
    btn("trash", "Remove", () => removeSelected(), "danger"),
  ];
  const toolbar = h(
    "div",
    { class: "toolbar" },
    h("button", { class: "btn primary", onclick: () => addDialog() }, icon("plus"), "Add"),
    h("span", { class: "sep" }),
    selectionButtons,
    h("button", { class: "btn icon-only", title: "More actions", onclick: (e) => rowMenu(e.clientX, e.clientY) }, icon("menu")),
    h("div", { class: "search-box" }, icon("search"), filterInput),
  );
  const thead = h("thead");
  const tbody = h("tbody");
  const table = h("table", { class: "grid wide" }, h("colgroup", null, COLUMNS.map((c) => h("col", { class: `c-${c.key} ${c.opt ? "opt" : ""}`, style: { width: c.width } }))), thead, tbody);
  const wrap = h("div", { class: "table-wrap", tabindex: 0 }, table);
  const details = h("section", { class: `details ${S.detailsOpen ? "" : "closed"}` });
  const resizer = h("div", { class: "resizer" });
  resizer.addEventListener("mousedown", (e) => {
    const startY = e.clientY;
    const startH = details.getBoundingClientRect().height;
    const move = (ev) => (details.style.height = `${Math.max(140, Math.min(innerHeight - 220, startH - (ev.clientY - startY)))}px`);
    const up = () => {
      removeEventListener("mousemove", move);
      removeEventListener("mouseup", up);
      store.set("detailsH", details.style.height);
    };
    addEventListener("mousemove", move);
    addEventListener("mouseup", up);
  });
  if (store.get("detailsH")) details.style.height = store.get("detailsH");
  UI.main.append(toolbar, wrap, resizer, details);
  UI.tx = { tbody, thead, wrap, details, filterInput, selectionButtons, resizer };
  wrap.addEventListener("contextmenu", (e) => {
    const tr = e.target.closest("tr[data-id]");
    if (!tr) return;
    e.preventDefault();
    if (!S.selected.has(tr.dataset.id)) selectOnly(tr.dataset.id);
    rowMenu(e.clientX, e.clientY);
  });
  wrap.addEventListener("click", (e) => {
    const tr = e.target.closest("tr[data-id]");
    if (!tr) {
      if (e.target === wrap) {
        S.selected.clear();
        renderTransfers();
      }
      return;
    }
    clickRow(tr.dataset.id, e);
  });
  wrap.addEventListener("dblclick", (e) => {
    const tr = e.target.closest("tr[data-id]");
    if (!tr) return;
    const t = S.torrents[tr.dataset.id];
    act(kind(t) === "stopped" ? "torrents/start" : "torrents/stop", [tr.dataset.id]);
  });
  renderHead();
  renderTransfers();
  renderDetails(true);
}

function renderHead() {
  fill(UI.tx.thead, 
    h(
      "tr",
      null,
      COLUMNS.map((c) =>
        h(
          "th",
          {
            class: [c.num && "num", c.opt && "opt", S.sort.key === c.key && "sorted", S.sort.key === c.key && S.sort.asc && "asc"].filter(Boolean).join(" "),
            onclick: () => {
              S.sort = { key: c.key, asc: S.sort.key === c.key ? !S.sort.asc : c.key === "name" };
              store.set("sort", S.sort);
              renderHead();
              renderTransfers();
            },
          },
          c.label,
        ),
      ),
    ),
  );
}

function renderSidebar() {
  const all = Object.values(S.torrents);
  const item = (label, ic, count, active, onclick, color) =>
    h(
      "button",
      { class: `nav-item ${active ? "active" : ""}`, onclick },
      color ? h("i", { class: "dot", style: { background: color } }) : icon(ic),
      h("span", { text: label }),
      h("em", { text: count }),
    );
  const pick = (fn) => () => {
    fn();
    UI.sidebar.classList.remove("open");
    renderTransfers();
    renderSidebar();
  };
  const cats = Object.keys(S.categories).sort((a, b) => a.localeCompare(b));
  const palette = ["#ff6b2c", "#38bdf8", "#c084fc", "#34d399", "#fbbf24", "#f472b6", "#60a5fa", "#a3e635"];
  const tagCount = (tag) => all.filter((t) => t.tags && t.tags.split(", ").includes(tag)).length;
  fill(UI.sidebar, 
    h("h4", { text: "Status" }),
    FILTERS.map(([id, label, ic, f]) => item(label, ic, all.filter(f).length, S.status === id, pick(() => {
      S.status = id;
      store.set("status", id);
    }))),
    h("h4", null, "Categories", h("button", { class: "btn ghost icon-only small", title: "New category", onclick: newCategory }, icon("plus"))),
    item("All", "folder", all.length, S.category === null, pick(() => (S.category = null))),
    item("Uncategorized", "folder", all.filter((t) => !t.category).length, S.category === "", pick(() => (S.category = ""))),
    cats.map((c, i) => {
      const el = item(c, "folder", all.filter((t) => t.category === c).length, S.category === c, pick(() => (S.category = c)), palette[i % palette.length]);
      el.addEventListener("contextmenu", (e) => {
        e.preventDefault();
        menu(e.clientX, e.clientY, [
          { label: "Edit save folder", icon: "folder", run: () => editCategory(c) },
          { label: "Remove category", icon: "trash", danger: true, run: () => attempt(() => post("torrents/removeCategories", { categories: c }), "Category removed").then(poll) },
        ]);
      });
      return el;
    }),
    S.tags.length ? h("h4", { text: "Tags" }) : null,
    S.tags.length ? item("All", "tag", all.length, S.tag === null, pick(() => (S.tag = null))) : null,
    S.tags.length ? item("Untagged", "tag", all.filter((t) => !t.tags).length, S.tag === "", pick(() => (S.tag = ""))) : null,
    S.tags.map((t) => item(t, "tag", tagCount(t), S.tag === t, pick(() => (S.tag = t)))),
  );
}

function renderTransfers() {
  if (!UI.tx) return;
  renderSidebar();
  const list = visible();
  const rows = list.map((t) => {
    const tr = h(
      "tr",
      { data: { id: t.hash }, class: S.selected.has(t.hash) ? "selected" : "" },
      COLUMNS.map((c) => h("td", { class: [c.num && "num", c.opt && "opt", c.key === "name" && "name"].filter(Boolean).join(" ") }, c.render(t))),
    );
    return tr;
  });
  if (!rows.length) {
    const none = Object.keys(S.torrents).length === 0;
    fill(UI.tx.tbody, 
      h(
        "tr",
        null,
        h(
          "td",
          { colspan: COLUMNS.length, style: { height: "auto", border: 0 } },
          h(
            "div",
            { class: "empty" },
            icon(none ? "down" : "search"),
            h("b", { text: none ? "No torrents yet" : "Nothing matches" }),
            h("span", { text: none ? "Add a magnet link or a .torrent file, or drop files anywhere on this page." : "Try another filter." }),
            none && h("button", { class: "btn primary", onclick: () => addDialog() }, icon("plus"), "Add a torrent"),
          ),
        ),
      ),
    );
  } else {
    fill(UI.tx.tbody, ...rows);
  }
  const has = S.selected.size > 0;
  UI.tx.selectionButtons.forEach((b) => (b.disabled = !has));
  renderDetails();
}

function clickRow(id, e) {
  const list = visible().map((t) => t.hash);
  if (e.shiftKey && S.anchor) {
    const a = list.indexOf(S.anchor);
    const b = list.indexOf(id);
    if (a >= 0 && b >= 0) {
      if (!(e.ctrlKey || e.metaKey)) S.selected.clear();
      list.slice(Math.min(a, b), Math.max(a, b) + 1).forEach((x) => S.selected.add(x));
    }
  } else if (e.ctrlKey || e.metaKey) {
    S.selected.has(id) ? S.selected.delete(id) : S.selected.add(id);
    S.anchor = id;
  } else {
    selectOnly(id);
  }
  S.focus = id;
  renderTransfers();
}

function selectOnly(id) {
  S.selected.clear();
  S.selected.add(id);
  S.anchor = id;
  S.focus = id;
}

const hashes = (ids) => (ids || [...S.selected]).join("|");

async function act(path, ids, extra = {}) {
  const list = ids || [...S.selected];
  if (!list.length) return;
  await attempt(() => post(path, { hashes: hashes(list), ...extra }));
  poll();
}

function rowMenu(x, y) {
  const ids = [...S.selected];
  const none = !ids.length;
  const one = ids.length === 1 ? S.torrents[ids[0]] : null;
  menu(x, y, [
    { label: "Start", icon: "play", disabled: none, run: () => act("torrents/start") },
    { label: "Stop", icon: "stop", disabled: none, run: () => act("torrents/stop") },
    { label: "Force start", icon: "zap", disabled: none, run: () => act("torrents/setForceStart", null, { value: "true" }) },
    "-",
    { label: "Set category…", icon: "folder", disabled: none, run: setCategoryDialog },
    { label: "Tags…", icon: "tag", disabled: none, run: tagsDialog },
    { label: "Speed limits…", icon: "gauge", disabled: none, run: limitsDialog },
    { label: "Move files…", icon: "move", disabled: none, run: moveDialog },
    { label: one?.seq_dl ? "Download rarest first" : "Download in order", icon: "sort", disabled: none, run: () => act("torrents/toggleSequentialDownload") },
    "-",
    { label: "Force recheck", icon: "refresh", disabled: none, run: () => act("torrents/recheck") },
    { label: "Ask trackers for peers", icon: "globe", disabled: none, run: () => act("torrents/reannounce") },
    { label: "Move to top of queue", icon: "up", disabled: none, run: () => act("torrents/topPrio") },
    { label: "Move to bottom of queue", icon: "down", disabled: none, run: () => act("torrents/bottomPrio") },
    "-",
    { label: "Copy magnet link", icon: "copy", disabled: !one, run: () => copy(one.magnet_uri, "Magnet link copied") },
    { label: "Save .torrent file", icon: "file", disabled: !one, run: () => exportTorrent(ids[0], one.name) },
    "-",
    { label: "Remove…", icon: "trash", danger: true, disabled: none, key: "Del", run: removeSelected },
  ]);
}

async function copy(text, done) {
  try {
    await navigator.clipboard.writeText(text);
    toast(done, "ok");
  } catch {
    prompt("Copy", "Copy this:", text);
  }
}

function exportTorrent(id, name) {
  const a = h("a", { href: `${API}torrents/export?hash=${encodeURIComponent(id)}`, download: `${name.replace(/[\\/:*?"<>|]/g, "_")}.torrent` });
  document.body.append(a);
  a.click();
  a.remove();
}

function removeSelected() {
  const ids = [...S.selected];
  if (!ids.length) return;
  const files = h("input", { type: "checkbox" });
  const names = ids.slice(0, 5).map((id) => h("li", { text: S.torrents[id]?.name || id }));
  dialog({
    title: ids.length === 1 ? "Remove torrent" : `Remove ${ids.length} torrents`,
    body: [
      h("ul", { class: "muted", style: { margin: "0 0 12px", paddingLeft: "18px" } }, names, ids.length > 5 && h("li", { text: `and ${ids.length - 5} more` })),
      h("label", { class: "check" }, files, "Also delete the downloaded files"),
    ],
    actions: [
      { label: "Cancel" },
      {
        label: "Remove",
        kind: "danger solid",
        submit: true,
        run: async () => {
          await attempt(() => post("torrents/delete", { hashes: hashes(ids), deleteFiles: files.checked ? "true" : "false" }), files.checked ? "Removed with files" : "Removed");
          ids.forEach((id) => S.selected.delete(id));
          poll();
        },
      },
    ],
  });
}

function categoryOptions(current) {
  return [["", "No category"], ...Object.keys(S.categories).sort().map((c) => [c, c])].map(([v, l]) => h("option", { value: v, text: l, selected: v === (current || "") }));
}

function setCategoryDialog() {
  const first = S.torrents[[...S.selected][0]];
  const sel = h("select", { class: "input" }, categoryOptions(first?.category));
  const fresh = h("input", { class: "input", placeholder: "Or type a new one" });
  dialog({
    title: "Set category",
    body: [field("Category", sel), field("New category", fresh)],
    actions: [
      { label: "Cancel" },
      {
        label: "Apply",
        kind: "primary",
        submit: true,
        run: async () => {
          const name = fresh.value.trim() || sel.value;
          if (fresh.value.trim() && !S.categories[name]) await attempt(() => post("torrents/createCategory", { category: name, savePath: "" }));
          await act("torrents/setCategory", null, { category: name });
        },
      },
    ],
  });
}

async function newCategory() {
  const name = await prompt("New category", "Name");
  if (name) {
    await attempt(() => post("torrents/createCategory", { category: name, savePath: "" }), "Category added");
    poll();
  }
}

async function editCategory(c) {
  const path = await prompt(`Save folder for ${c}`, "Folder", S.categories[c]?.savePath || "", "Leave empty to use the default download folder.");
  if (path !== null) {
    await attempt(() => post("torrents/editCategory", { category: c, savePath: path }), "Category saved");
    poll();
  }
}

function tagsDialog() {
  const ids = [...S.selected];
  const current = new Set(ids.flatMap((id) => (S.torrents[id]?.tags ? S.torrents[id].tags.split(", ") : [])));
  const input = h("input", { class: "input", placeholder: "Comma-separated, like linux, iso" });
  const chips = h(
    "div",
    { class: "chips" },
    [...new Set([...S.tags, ...current])].map((t) => {
      const b = h("button", { type: "button", class: `btn small ${current.has(t) ? "on" : ""}`, text: t, onclick: () => b.classList.toggle("on") });
      return b;
    }),
  );
  dialog({
    title: "Tags",
    body: [chips.children.length ? field("Tap to turn on or off", chips) : null, field("Add new tags", input)],
    actions: [
      { label: "Cancel" },
      {
        label: "Apply",
        kind: "primary",
        submit: true,
        run: async () => {
          const on = [...chips.children].filter((b) => b.classList.contains("on")).map((b) => b.textContent);
          const off = [...chips.children].filter((b) => !b.classList.contains("on")).map((b) => b.textContent);
          const added = input.value.split(",").map((s) => s.trim()).filter(Boolean);
          if (off.length) await attempt(() => post("torrents/removeTags", { hashes: hashes(ids), tags: off.join(",") }));
          if (on.length + added.length) await attempt(() => post("torrents/addTags", { hashes: hashes(ids), tags: [...on, ...added].join(",") }));
          poll();
        },
      },
    ],
  });
}

function limitsDialog() {
  const ids = [...S.selected];
  const t = S.torrents[ids[0]] || {};
  const kib = (b) => (b > 0 ? Math.round(b / 1024) : "");
  const dl = h("input", { class: "input", type: "number", min: 0, placeholder: "No limit", value: kib(t.dl_limit) });
  const ul = h("input", { class: "input", type: "number", min: 0, placeholder: "No limit", value: kib(t.up_limit) });
  dialog({
    title: "Speed limits",
    body: [h("div", { class: "grid2" }, field("Download (KiB/s)", dl), field("Upload (KiB/s)", ul)), h("small", { class: "hint", text: "Leave empty or 0 for no limit." })],
    actions: [
      { label: "Cancel" },
      {
        label: "Apply",
        kind: "primary",
        submit: true,
        run: async () => {
          await attempt(() => post("torrents/setDownloadLimit", { hashes: hashes(ids), limit: (Number(dl.value) || 0) * 1024 }));
          await attempt(() => post("torrents/setUploadLimit", { hashes: hashes(ids), limit: (Number(ul.value) || 0) * 1024 }), "Limits saved");
          poll();
        },
      },
    ],
  });
}

function moveDialog() {
  const ids = [...S.selected];
  const loc = h("input", { class: "input mono", value: S.torrents[ids[0]]?.save_path || "" });
  dialog({
    title: "Move files",
    body: field("New folder on the Riptor computer", loc, "Files are moved there and the torrent keeps seeding from the new place."),
    actions: [{ label: "Cancel" }, { label: "Move", kind: "primary", submit: true, run: () => act("torrents/setLocation", ids, { location: loc.value.trim() }) }],
  });
}

// ------------------------------------------------------------------ add

let defaultSave = null;

async function addDialog(files = [], links = "") {
  if (defaultSave === null) defaultSave = (await attempt(() => call("app/defaultSavePath"))) || "";
  const picked = [...files];
  const urls = h("textarea", { class: "input", placeholder: "Magnet links or http(s) links to .torrent files, one per line", value: links });
  const list = h("div", { class: "chips" });
  const fileInput = h("input", { type: "file", accept: ".torrent,application/x-bittorrent", multiple: true, style: { display: "none" }, onchange: (e) => addFiles(e.target.files) });
  const showFiles = () => fill(list, ...picked.map((f, i) => h("span", { class: "badge accent" }, icon("file"), f.name, h("button", { type: "button", class: "btn ghost icon-only small", "aria-label": "Remove", onclick: () => {
    picked.splice(i, 1);
    showFiles();
  } }, icon("x")))));
  const addFiles = (fl) => {
    for (const f of fl) if (f.name.toLowerCase().endsWith(".torrent") || f.type === "application/x-bittorrent") picked.push(f);
    showFiles();
  };
  const zone = h("div", { class: "dropzone", onclick: () => fileInput.click() }, icon("file"), h("b", { text: "Choose .torrent files" }), h("small", { class: "hint", text: "or drop them here" }));
  zone.addEventListener("dragover", (e) => {
    e.preventDefault();
    zone.classList.add("over");
  });
  zone.addEventListener("dragleave", () => zone.classList.remove("over"));
  zone.addEventListener("drop", (e) => {
    e.preventDefault();
    e.stopPropagation();
    zone.classList.remove("over");
    addFiles(e.dataTransfer.files);
  });
  const save = h("input", { class: "input mono", value: defaultSave, placeholder: "Default download folder" });
  const cat = h("select", { class: "input" }, categoryOptions(S.category || ""));
  const tags = h("input", { class: "input", placeholder: "Optional, comma-separated" });
  const stopped = h("input", { type: "checkbox" });
  const seq = h("input", { type: "checkbox" });
  showFiles();
  dialog({
    title: "Add torrents",
    body: [
      field("Links", urls),
      zone,
      fileInput,
      list,
      h("div", { style: { height: "14px" } }),
      field("Save to", save),
      h("div", { class: "grid2" }, field("Category", cat), field("Tags", tags)),
      h("label", { class: "check" }, stopped, "Add stopped"),
      h("label", { class: "check" }, seq, "Download in order (good for playing while it downloads)"),
    ],
    actions: [
      { label: "Cancel" },
      {
        label: "Add",
        kind: "primary",
        icon: "plus",
        submit: true,
        run: async () => {
          const fd = new FormData();
          const links = urls.value.trim();
          if (!links && !picked.length) {
            toast("Paste a link or choose a file first.", "error");
            return false;
          }
          if (links) fd.append("urls", links);
          picked.forEach((f) => fd.append("torrents", f, f.name));
          if (save.value.trim() && save.value.trim() !== defaultSave) fd.append("savepath", save.value.trim());
          if (cat.value) fd.append("category", cat.value);
          if (tags.value.trim()) fd.append("tags", tags.value.trim());
          fd.append("stopped", stopped.checked ? "true" : "false");
          if (seq.checked) fd.append("sequentialDownload", "true");
          const r = await attempt(() => call("torrents/add", fd));
          if (r === undefined) return false;
          if (r !== "Ok.") {
            toast("Riptor couldn't add that. Check the link or file.", "error");
            return false;
          }
          const n = picked.length + links.split("\n").filter((l) => l.trim()).length;
          toast(n === 1 ? "Adding the torrent…" : `Adding ${n} torrents…`, "ok");
          setTimeout(poll, 800);
        },
      },
    ],
  });
}

let dragDepth = 0;
addEventListener("dragenter", (e) => {
  if (!UI.veil || ![...(e.dataTransfer?.types || [])].includes("Files") || document.querySelector(".veil")) return;
  dragDepth++;
  UI.veil.classList.add("show");
});
addEventListener("dragleave", () => {
  if (--dragDepth <= 0) {
    dragDepth = 0;
    UI.veil?.classList.remove("show");
  }
});
addEventListener("dragover", (e) => e.preventDefault());
addEventListener("drop", (e) => {
  e.preventDefault();
  dragDepth = 0;
  UI.veil?.classList.remove("show");
  if (UI.veil && e.dataTransfer?.files?.length && !document.querySelector(".veil")) addDialog([...e.dataTransfer.files]);
});
addEventListener("paste", (e) => {
  if (!UI.veil || e.target.closest?.("input, textarea") || document.querySelector(".veil")) return;
  const text = e.clipboardData?.getData("text") || "";
  if (/^(magnet:\?|https?:\/\/)/i.test(text.trim())) addDialog([], text.trim());
});

// -------------------------------------------------------------- details

let detailsFor = null;
let detailsTimer = null;

function renderDetails(force) {
  const d = UI.tx?.details;
  if (!d) return;
  const id = S.selected.size === 1 ? [...S.selected][0] : null;
  d.classList.toggle("closed", !S.detailsOpen || !id);
  UI.tx.resizer.style.display = d.classList.contains("closed") ? "none" : "";
  if (!id) {
    detailsFor = null;
    clearInterval(detailsTimer);
    return;
  }
  if (id === detailsFor && !force) {
    const title = d.querySelector(".subtabs .title");
    if (title) title.textContent = S.torrents[id]?.name || "";
    return;
  }
  detailsFor = id;
  const body = h("div", { class: "subtab-body" });
  const tabs = [
    ["general", "General"],
    ["files", "Files"],
    ["trackers", "Trackers"],
    ["peers", "Peers"],
  ];
  const bar = h(
    "div",
    { class: "subtabs" },
    tabs.map(([k, label]) =>
      h("button", { class: S.detailsTab === k ? "active" : "", onclick: () => {
        S.detailsTab = k;
        store.set("detailsTab", k);
        renderDetails(true);
      } }, label),
    ),
    h("span", { class: "title", text: S.torrents[id]?.name || "" }),
    h("button", { class: "btn ghost icon-only small", title: "Hide details", onclick: () => {
      S.detailsOpen = false;
      store.set("detailsOpen", false);
      renderDetails(true);
    } }, icon("x")),
  );
  fill(d, bar, body);
  clearInterval(detailsTimer);
  const load = () => document.visibilityState === "visible" && S.tab === "transfers" && detailsFor === id && fillDetails(id, body);
  load();
  detailsTimer = setInterval(load, 2500);
}

async function fillDetails(id, body) {
  const tab = S.detailsTab;
  try {
    if (tab === "general") await generalTab(id, body);
    else if (tab === "files") await filesTab(id, body);
    else if (tab === "trackers") await trackersTab(id, body);
    else await peersTab(id, body);
  } catch (e) {
    if (!(e instanceof SignedOut)) fill(body, h("div", { class: "empty", text: e.message }));
  }
}

async function generalTab(id, body) {
  const [p, pieces] = await Promise.all([call("torrents/properties", { hash: id }), call("torrents/pieceStates", { hash: id }).catch(() => [])]);
  const t = S.torrents[id] || {};
  const prop = (label, value) => h("div", { class: "prop" }, h("span", { text: label }), h("b", { text: value, title: value }));
  const canvas = h("canvas");
  fill(body, 
    pieces.length ? h("div", { class: "pieces", title: `${p.pieces_have} of ${p.pieces_num} pieces` }, canvas) : null,
    h(
      "div",
      { class: "props" },
      prop("Progress", `${pct(t.progress || 0)} of ${bytes(p.total_size)}`),
      prop("Downloaded", `${bytes(p.total_downloaded)} (${bytes(p.total_downloaded_session)} this session)`),
      prop("Uploaded", `${bytes(p.total_uploaded)} (${bytes(p.total_uploaded_session)} this session)`),
      prop("Ratio", ratio(p.share_ratio)),
      prop("Download speed", speed(p.dl_speed)),
      prop("Upload speed", speed(p.up_speed)),
      prop("Time left", t.progress >= 1 ? "Done" : eta(p.eta)),
      prop("Peers", `${p.peers} connected, ${p.peers_total} seen`),
      prop("Seeds in swarm", `${p.seeds_total}`),
      prop("Seeding time", eta(p.seeding_time)),
      prop("Pieces", p.pieces_num > 0 ? `${p.pieces_have} / ${p.pieces_num}` : "–"),
      prop("Added", date(p.addition_date)),
      prop("Completed", date(p.completion_date)),
      prop("Save path", p.save_path),
      prop("Category", t.category || "None"),
      prop("Tags", t.tags || "None"),
      prop("Order", t.seq_dl ? "In order" : "Rarest first"),
      prop("Info hash", id),
    ),
  );
  if (pieces.length) {
    requestAnimationFrame(() => {
      const w = canvas.clientWidth;
      const hh = canvas.clientHeight;
      canvas.width = w;
      canvas.height = hh;
      const c = canvas.getContext("2d");
      const cols = Math.min(w, pieces.length);
      const per = pieces.length / cols;
      for (let x = 0; x < cols; x++) {
        let have = 0;
        const from = Math.floor(x * per);
        const to = Math.max(from + 1, Math.floor((x + 1) * per));
        for (let i = from; i < to; i++) if (pieces[i] === 2) have++;
        const f = have / (to - from);
        if (!f) continue;
        c.fillStyle = f === 1 ? "#ff6b2c" : `rgba(255,107,44,${0.25 + f * 0.6})`;
        c.fillRect((x * w) / cols, 0, Math.ceil(w / cols), hh);
      }
    });
  }
}

const PRIO = [
  ["0", "Don't download"],
  ["1", "Normal"],
  ["7", "High"],
];

async function filesTab(id, body) {
  if (document.activeElement?.closest?.(".subtab-body select") && body.contains(document.activeElement)) return;
  const files = await call("torrents/files", { hash: id });
  const rows = files.map((f) =>
    h(
      "tr",
      { class: "compact" },
      h("td", { class: "name", title: f.name }, f.name),
      h("td", { class: "num", text: bytes(f.size) }),
      h("td", null, bar(f.progress)),
      h(
        "td",
        null,
        h(
          "select",
          {
            class: "input",
            style: { height: "28px" },
            onchange: async (e) => {
              await attempt(() => post("torrents/filePrio", { hash: id, id: f.index, priority: e.target.value }), "Priority saved");
            },
          },
          PRIO.map(([v, l]) => h("option", { value: v, text: l, selected: String(f.priority === 6 ? 7 : f.priority) === v })),
        ),
      ),
    ),
  );
  const prioAll = (v) => attempt(() => post("torrents/filePrio", { hash: id, id: files.map((f) => f.index).join("|"), priority: v }), "Priority saved").then(() => fillDetails(id, body));
  fill(body, 
    h("div", { class: "toolbar" }, h("span", { class: "muted", text: `${files.length} files` }), h("span", { class: "grow" }), h("button", { class: "btn small", onclick: () => prioAll("1") }, "Download all"), h("button", { class: "btn small", onclick: () => prioAll("0") }, "Skip all")),
    h(
      "table",
      { class: "grid" },
      h("colgroup", null, h("col"), h("col", { style: { width: "100px" } }), h("col", { style: { width: "150px" } }), h("col", { style: { width: "170px" } })),
      h("thead", null, h("tr", null, h("th", { text: "Name" }), h("th", { class: "num", text: "Size" }), h("th", { text: "Progress" }), h("th", { text: "Priority" }))),
      h("tbody", null, rows),
    ),
  );
}

async function trackersTab(id, body) {
  if (body.contains(document.activeElement) && document.activeElement.tagName === "TEXTAREA") return;
  const list = await call("torrents/trackers", { hash: id });
  const urls = h("textarea", { class: "input", placeholder: "Add trackers, one per line", style: { minHeight: "60px" } });
  fill(body, 
    h(
      "table",
      { class: "grid" },
      h("colgroup", null, h("col"), h("col", { style: { width: "60px" } }), h("col", { style: { width: "110px" } })),
      h("thead", null, h("tr", null, h("th", { text: "Tracker" }), h("th", { class: "num", text: "Tier" }), h("th", { text: "" }))),
      h(
        "tbody",
        null,
        list.map((t) =>
          h("tr", { class: "compact" }, h("td", { class: "mono", text: t.url, title: t.url }), h("td", { class: "num", text: t.tier }), h("td", null, h("button", { class: "btn small danger", onclick: async () => {
            await attempt(() => post("torrents/removeTrackers", { hash: id, urls: t.url }), "Tracker removed");
            fillDetails(id, body);
          } }, icon("trash"), "Remove"))),
        ),
      ),
    ),
    h("div", { class: "toolbar", style: { borderBottom: 0 } }, h("div", { class: "grow" }, urls), h("button", { class: "btn", onclick: async () => {
      if (!urls.value.trim()) return;
      await attempt(() => post("torrents/addTrackers", { hash: id, urls: urls.value.trim() }), "Trackers added");
      urls.value = "";
      fillDetails(id, body);
    } }, icon("plus"), "Add")),
  );
}

function flag(cc) {
  if (!cc || cc.length !== 2) return "";
  return String.fromCodePoint(...[...cc.toUpperCase()].map((c) => 0x1f1e6 + c.charCodeAt(0) - 65));
}

async function peersTab(id, body) {
  const d = await call("sync/torrentPeers", { hash: id });
  const peers = Object.values(d.peers || {});
  fill(body, 
    peers.length
      ? h(
          "table",
          { class: "grid" },
          h("thead", null, h("tr", null, ["Address", "Client", "Connection", "Downloaded", "Uploaded"].map((x, i) => h("th", { class: i > 2 ? "num" : "", text: x })))),
          h(
            "tbody",
            null,
            peers.map((p) =>
              h(
                "tr",
                { class: "compact" },
                h("td", { class: "mono" }, flag(p.country_code) && h("span", { text: flag(p.country_code) + " ", title: p.country_code.toUpperCase() }), `${p.ip}:${p.port}`),
                h("td", { text: p.client || "–" }),
                h("td", { text: p.connection }),
                h("td", { class: "num", text: bytes(p.downloaded) }),
                h("td", { class: "num", text: bytes(p.uploaded) }),
              ),
            ),
          ),
        )
      : h("div", { class: "empty" }, icon("globe"), h("span", { text: "No peers connected right now." })),
  );
}

// --------------------------------------------------------------- search

const SEARCH = { jobs: [], current: null, results: {}, plugins: [], sort: { key: "nbSeeders", asc: false }, text: "", seededOnly: false };
const CATS = ["all", "anime", "books", "games", "movies", "music", "pictures", "software", "tv"];
const catName = (c) => (c === "all" ? "All categories" : c === "tv" ? "TV shows" : c[0].toUpperCase() + c.slice(1));

async function searchPage() {
  SEARCH.plugins = (await attempt(() => call("search/plugins"))) || [];
  const q = h("input", { class: "input", placeholder: "Search for torrents", autocomplete: "off" });
  const cat = select(CATS.map((c) => [c, catName(c)]), "all");
  const usable = SEARCH.plugins.filter((p) => !p.error);
  const eng = select([["enabled", "Turned-on engines"], ["all", "All engines"], ...usable.map((p) => [p.name, p.fullName])], "enabled");
  const go = h("button", { class: "btn primary", type: "submit" }, icon("search"), "Search");
  const form = h(
    "form",
    {
      class: "hero-search",
      onsubmit: async (e) => {
        e.preventDefault();
        const pattern = q.value.trim();
        if (!pattern) return;
        go.disabled = true;
        const r = await attempt(() => post("search/start", { pattern, category: cat.value, plugins: eng.value }));
        go.disabled = false;
        if (r?.id === undefined) return;
        SEARCH.jobs.push({ id: r.id, query: pattern, status: "Running", total: 0 });
        SEARCH.current = r.id;
        SEARCH.results[r.id] = [];
        renderSearch();
      },
    },
    h("div", { class: "search-box" }, icon("search"), q),
    cat,
    eng,
    go,
    h("button", { class: "btn", type: "button", onclick: enginesDialog }, icon("puzzle"), "Engines"),
  );
  const jobs = h("div", { class: "toolbar" });
  const wrap = h("div", { class: "table-wrap" });
  UI.main.append(h("div", { class: "pane" }, form, jobs, wrap));
  UI.sr = { jobs, wrap, q };
  setTimeout(() => q.focus(), 30);
  renderSearch();
  pageEvery(1200, pollSearch);
}

async function pollSearch() {
  const running = SEARCH.jobs.filter((j) => j.status === "Running" || j.id === SEARCH.current);
  if (!running.length) return;
  for (const j of running) {
    const have = SEARCH.results[j.id]?.length || 0;
    const r = await call("search/results", { id: j.id, offset: have, limit: 500 }).catch(() => null);
    if (!r) {
      j.status = "Stopped";
      continue;
    }
    SEARCH.results[j.id] = (SEARCH.results[j.id] || []).concat(r.results);
    // Seed counts can arrive after a result; refresh the first ones now and then.
    if (r.status === "Running" && have > 0 && Math.random() < 0.25) {
      const all = await call("search/results", { id: j.id }).catch(() => null);
      if (all) SEARCH.results[j.id] = all.results;
    }
    j.status = r.status;
    j.total = r.total;
  }
  if (S.tab === "search") renderSearch();
}

function renderSearch() {
  if (!UI.sr) return;
  fill(UI.sr.jobs, 
    SEARCH.jobs.length ? null : h("span", { class: "muted", text: "Results from every turned-on engine show up here. Seed counts are filled in from the trackers." }),
    SEARCH.jobs.map((j) =>
      h(
        "button",
        { class: `btn small ${j.id === SEARCH.current ? "on" : ""}`, onclick: () => {
          SEARCH.current = j.id;
          renderSearch();
        } },
        j.status === "Running" ? h("span", { class: "spinner", style: { width: "12px", height: "12px" } }) : icon("search"),
        `${j.query} · ${j.total}`,
        h("span", { role: "button", "aria-label": "Close", title: j.status === "Running" ? "Stop and close" : "Close", onclick: async (e) => {
          e.stopPropagation();
          await call("search/delete", { id: j.id }, { post: true }).catch(() => {});
          SEARCH.jobs = SEARCH.jobs.filter((x) => x !== j);
          delete SEARCH.results[j.id];
          if (SEARCH.current === j.id) SEARCH.current = SEARCH.jobs.at(-1)?.id ?? null;
          renderSearch();
        } }, icon("x")),
      ),
    ),
    SEARCH.current !== null &&
      h("div", { class: "row", style: { marginLeft: "auto" } },
        switchEl("Only with seeds", SEARCH.seededOnly, (v) => {
          SEARCH.seededOnly = v;
          renderSearch();
        }),
        h("div", { class: "search-box", style: { width: "220px" } }, icon("search"), h("input", { class: "input", placeholder: "Filter results", value: SEARCH.text, oninput: debounce((e) => {
          SEARCH.text = e.target.value;
          renderResults();
        }, 120) })),
        SEARCH.jobs.find((j) => j.id === SEARCH.current)?.status === "Running" &&
          h("button", { class: "btn small", onclick: async () => {
            await attempt(() => post("search/stop", { id: SEARCH.current }));
            pollSearch();
          } }, icon("stop"), "Stop"),
      ),
  );
  renderResults();
}

function renderResults() {
  const wrap = UI.sr.wrap;
  if (SEARCH.current === null) {
    const none = !SEARCH.plugins.some((p) => p.enabled && !p.error);
    fill(wrap, 
      h("div", { class: "empty" }, icon("search"), h("b", { text: none ? "No search engines are on" : "Search the web for torrents" }), h("span", { text: none ? "Open Engines to turn one on or add a plugin." : "Type what you're looking for above." })),
    );
    return;
  }
  const words = SEARCH.text.toLowerCase().split(/\s+/).filter(Boolean);
  const list = (SEARCH.results[SEARCH.current] || [])
    .filter((r) => !SEARCH.seededOnly || r.nbSeeders > 0)
    .filter((r) => words.every((w) => r.fileName.toLowerCase().includes(w)));
  const { key, asc } = SEARCH.sort;
  list.sort((a, b) => {
    const c = typeof a[key] === "string" ? a[key].localeCompare(b[key]) : a[key] - b[key];
    return asc ? c : -c;
  });
  const cols = [
    ["fileName", "Name", "auto"],
    ["fileSize", "Size", "96px", true],
    ["nbSeeders", "Seeds", "76px", true],
    ["nbLeechers", "Peers", "76px", true],
    ["engineName", "Engine", "150px"],
    ["pubDate", "Published", "110px"],
    ["", "", "110px"],
  ];
  const job = SEARCH.jobs.find((j) => j.id === SEARCH.current);
  if (!list.length) {
    fill(wrap, 
      h("div", { class: "empty" }, job?.status === "Running" ? h("span", { class: "spinner", style: { width: "28px", height: "28px" } }) : icon("search"), h("b", { text: job?.status === "Running" ? "Searching…" : "No results" })),
    );
    return;
  }
  fill(wrap, 
    h(
      "table",
      { class: "grid" },
      h("colgroup", null, cols.map(([, , w], i) => h("col", { class: i === 3 || i === 5 ? "opt" : "", style: { width: w } }))),
      h(
        "thead",
        null,
        h(
          "tr",
          null,
          cols.map(([k, label, , num], i) =>
            h("th", { class: [num && "num", (i === 3 || i === 5) && "opt", SEARCH.sort.key === k && k && "sorted", SEARCH.sort.key === k && SEARCH.sort.asc && "asc"].filter(Boolean).join(" "), onclick: () => {
              if (!k) return;
              SEARCH.sort = { key: k, asc: SEARCH.sort.key === k ? !SEARCH.sort.asc : k === "fileName" };
              renderResults();
            } }, label),
          ),
        ),
      ),
      h(
        "tbody",
        null,
        list.slice(0, 1000).map((r) =>
          h(
            "tr",
            { ondblclick: () => downloadResult(r) },
            h("td", { class: "name", title: r.fileName }, r.descrLink && /^https?:/i.test(r.descrLink) ? h("a", { href: r.descrLink, target: "_blank", rel: "noopener noreferrer", text: r.fileName }) : r.fileName),
            h("td", { class: "num", text: bytes(r.fileSize) }),
            h("td", { class: "num" }, h("span", { class: r.nbSeeders > 0 ? "badge ok" : "faint", text: r.nbSeeders >= 0 ? r.nbSeeders : "–" })),
            h("td", { class: "num opt", text: r.nbLeechers >= 0 ? r.nbLeechers : "–" }),
            h("td", { class: "muted", text: r.engineName || r.pluginName }),
            h("td", { class: "muted opt", text: r.pubDate > 0 ? new Date(r.pubDate * 1000).toLocaleDateString() : "–" }),
            h("td", null, h("button", { class: "btn small", onclick: () => downloadResult(r) }, icon("down"), "Get")),
          ),
        ),
      ),
    ),
  );
}

async function downloadResult(r) {
  const viaPlugin = SEARCH.plugins.find((p) => p.name === r.pluginName)?.kind === "plugin" && !r.fileUrl.startsWith("magnet:");
  if (viaPlugin) await attempt(() => post("search/downloadTorrent", { torrentUrl: r.fileUrl, pluginName: r.pluginName }), `Adding "${r.fileName}"`);
  else addDialog([], r.fileUrl);
}

async function enginesDialog() {
  const list = h("div");
  const fill = async () => {
    SEARCH.plugins = (await attempt(() => call("search/plugins"))) || [];
    fill(list, 
      ...SEARCH.plugins.map((p) =>
        h(
          "div",
          { class: "engine" },
          icon(p.kind === "plugin" ? "puzzle" : p.kind === "torznab" ? "layers" : "globe"),
          h(
            "div",
            { class: "grow" },
            h("div", null, h("b", { text: p.fullName }), " ", h("span", { class: "badge", text: p.kind === "builtin" ? "Built in" : p.kind === "torznab" ? "Torznab" : "Plugin" })),
            p.error ? h("small", { class: "err", text: p.error }) : h("small", { text: p.url || p.supportedCategories.map((c) => c.name).join(", ") }),
          ),
          switchEl("", p.enabled, async (on) => {
            await attempt(() => post("search/enablePlugin", { names: p.name, enable: on ? "true" : "false" }));
          }),
          p.kind !== "builtin" &&
            h("button", { class: "btn small ghost danger", title: "Remove", onclick: async () => {
              if (await confirmBox("Remove engine", `Remove ${p.fullName}?`, "Remove", "danger solid")) {
                await attempt(() => post("search/uninstallPlugin", { names: p.name }), "Removed");
                fill();
              }
            } }, icon("trash")),
        ),
      ),
    );
  };
  const url = h("input", { class: "input mono", placeholder: "https://…/plugin.py" });
  await fill();
  dialog({
    title: "Search engines",
    wide: true,
    body: [
      list,
      h("h5", { class: "muted", text: "Install a plugin", style: { margin: "18px 0 8px" } }),
      h("div", { class: "row" }, h("div", { class: "grow" }, url), h("button", { class: "btn", type: "button", onclick: async () => {
        if (!url.value.trim()) return;
        const ok = await attempt(() => post("search/installPlugin", { sources: url.value.trim() }), "Plugin installed");
        if (ok !== undefined) {
          url.value = "";
          fill();
        }
      } }, icon("plus"), "Install")),
      h("small", { class: "hint", text: "Plugins are Python programs that run on the Riptor computer. Installing them remotely has to be allowed in Riptor's remote control settings." }),
    ],
    actions: [
      { label: "Update plugins", left: true, icon: "refresh", keep: true, run: async () => {
        await attempt(() => post("search/updatePlugins"), "Plugins updated");
        fill();
      } },
      { label: "Done", kind: "primary", run: () => S.tab === "search" && setTab("search") },
    ],
  });
}

// ------------------------------------------------------------------ RSS

const RSS = { feeds: {}, current: null, text: "" };

function rssPage() {
  const list = h("div", { class: "list" });
  const right = h("div", { class: "pane" });
  UI.main.append(
    h(
      "div",
      { class: "toolbar" },
      h("button", { class: "btn primary", onclick: addFeed }, icon("plus"), "Add feed"),
      h("button", { class: "btn", onclick: () => attempt(() => post("rss/refreshItem", { itemPath: "" }), "Refreshing all feeds").then(() => setTimeout(loadRss, 1500)) }, icon("refresh"), "Refresh all"),
      h("button", { class: "btn", onclick: rulesDialog }, icon("zap"), "Download rules"),
      h("div", { class: "search-box" }, icon("search"), h("input", { class: "input", placeholder: "Filter articles", value: RSS.text, oninput: debounce((e) => {
        RSS.text = e.target.value;
        renderArticles();
      }, 120) })),
    ),
    h("div", { class: "split" }, list, right),
  );
  UI.rss = { list, right };
  pageEvery(10000, loadRss);
}

async function loadRss() {
  const items = await call("rss/items", { withData: "true" }).catch(() => null);
  if (!items) return;
  RSS.feeds = items;
  if (RSS.current === null || !RSS.feeds[RSS.current]) RSS.current = Object.keys(items)[0] ?? null;
  renderFeeds();
  renderArticles();
}

function renderFeeds() {
  const names = Object.keys(RSS.feeds);
  fill(UI.rss.list, 
    names.length
      ? names.map((n) => {
          const f = RSS.feeds[n];
          const unread = (f.articles || []).filter((a) => !a.isRead).length;
          return h(
            "div",
            { class: `list-item ${n === RSS.current ? "active" : ""}`, onclick: () => {
              RSS.current = n;
              renderFeeds();
              renderArticles();
            } },
            icon(f.hasError ? "alert" : "rss"),
            h("div", { class: "grow" }, h("div", { text: n }), h("small", { text: f.hasError ? f.error || "Couldn't read this feed" : f.lastBuildDate ? `Checked ${new Date(f.lastBuildDate).toLocaleString()}` : f.url })),
            unread ? h("span", { class: "badge accent", text: unread }) : null,
          );
        })
      : h("div", { class: "empty" }, icon("rss"), h("b", { text: "No feeds" }), h("span", { text: "Add a feed to follow new releases." })),
  );
}

function renderArticles() {
  const right = UI.rss?.right;
  if (!right) return;
  const name = RSS.current;
  const f = name !== null ? RSS.feeds[name] : null;
  if (!f) {
    fill(right, );
    return;
  }
  const words = RSS.text.toLowerCase().split(/\s+/).filter(Boolean);
  const arts = (f.articles || []).filter((a) => words.every((w) => a.title.toLowerCase().includes(w)));
  fill(right, 
    h(
      "div",
      { class: "toolbar" },
      h("b", { class: "grow", text: name, style: { overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" } }),
      h("button", { class: "btn small", onclick: () => attempt(() => post("rss/refreshItem", { itemPath: name }), "Refreshing").then(() => setTimeout(loadRss, 1500)) }, icon("refresh"), "Refresh"),
      h("button", { class: "btn small", onclick: () => attempt(() => post("rss/markAsRead", { itemPath: name })).then(loadRss) }, icon("check"), "Mark all read"),
      h("button", { class: "btn small", onclick: async () => {
        const n = await prompt("Rename feed", "Name", name);
        if (n) attempt(() => post("rss/moveItem", { itemPath: name, destPath: n }), "Renamed").then(() => {
          RSS.current = n;
          loadRss();
        });
      } }, "Rename"),
      h("button", { class: "btn small danger", onclick: async () => {
        if (await confirmBox("Remove feed", `Remove ${name}? Its rules stop using it.`, "Remove", "danger solid")) attempt(() => post("rss/removeItem", { path: name }), "Feed removed").then(() => {
          RSS.current = null;
          loadRss();
        });
      } }, icon("trash")),
    ),
    f.hasError && h("div", { class: "toolbar" }, h("span", { class: "badge warn" }, icon("alert"), f.error || "Couldn't read this feed")),
    h(
      "div",
      { class: "table-wrap" },
      arts.length
        ? h(
            "table",
            { class: "grid" },
            h("colgroup", null, h("col"), h("col", { class: "opt", style: { width: "100px" } }), h("col", { class: "opt", style: { width: "150px" } }), h("col", { style: { width: "120px" } })),
            h("thead", null, h("tr", null, h("th", { text: "Title" }), h("th", { class: "num opt", text: "Size" }), h("th", { class: "opt", text: "Date" }), h("th", { text: "" }))),
            h(
              "tbody",
              null,
              arts.map((a) =>
                h(
                  "tr",
                  { class: `article ${a.isRead ? "" : "unread"}`, onclick: () => !a.isRead && post("rss/markAsRead", { itemPath: name, articleId: a.id }).then(() => (a.isRead = true) && renderFeeds()).catch(() => {}) },
                  h("td", { class: "name", title: a.title }, a.link && /^https?:/i.test(a.link) && a.link !== a.torrentURL ? h("a", { href: a.link, target: "_blank", rel: "noopener noreferrer", text: a.title }) : a.title),
                  h("td", { class: "num opt", text: a.size ? bytes(a.size) : "–" }),
                  h("td", { class: "muted opt", text: a.date ? new Date(a.date).toLocaleString() : "–" }),
                  h("td", null, h("button", { class: "btn small", onclick: (e) => {
                    e.stopPropagation();
                    addDialog([], a.torrentURL);
                  } }, icon("down"), "Get")),
                ),
              ),
            ),
          )
        : h("div", { class: "empty" }, icon("rss"), h("span", { text: words.length ? "No articles match." : "No articles yet." })),
    ),
  );
}

function addFeed() {
  const url = h("input", { class: "input mono", placeholder: "https://example.org/feed.xml", type: "url", required: true });
  const name = h("input", { class: "input", placeholder: "Optional, the feed's title otherwise" });
  dialog({
    title: "Add RSS feed",
    body: [field("Feed address", url), field("Name", name)],
    actions: [
      { label: "Cancel" },
      {
        label: "Add",
        kind: "primary",
        submit: true,
        run: async () => {
          const r = await attempt(() => post("rss/addFeed", { url: url.value.trim(), path: name.value.trim() }), "Feed added");
          if (r === undefined) return false;
          setTimeout(loadRss, 1500);
          loadRss();
        },
      },
    ],
  });
}

async function rulesDialog() {
  let rules = (await attempt(() => call("rss/rules"))) || {};
  const feeds = Object.entries(RSS.feeds);
  const left = h("div", { class: "list", style: { minHeight: "360px" } });
  const right = h("div", { style: { padding: "0 0 0 16px" } });
  let current = Object.keys(rules)[0] ?? null;
  const renderList = () => {
    fill(left, 
      h("button", { class: "btn small primary", type: "button", style: { width: "100%", marginBottom: "8px" }, onclick: async () => {
        const n = await prompt("New rule", "Rule name");
        if (!n) return;
        await attempt(() => post("rss/setRule", { ruleName: n, ruleDef: JSON.stringify({ enabled: true }) }));
        rules = (await attempt(() => call("rss/rules"))) || {};
        current = n;
        renderList();
        renderRule();
      } }, icon("plus"), "New rule"),
      Object.keys(rules).map((n) => h("div", { class: `list-item ${n === current ? "active" : ""}`, onclick: () => {
        current = n;
        renderList();
        renderRule();
      } }, icon(rules[n].enabled ? "zap" : "stop"), h("div", { class: "grow" }, h("div", { text: n }), h("small", { text: rules[n].mustContain || "Matches everything" })))),
    );
  };
  const renderRule = () => {
    if (current === null || !rules[current]) {
      fill(right, h("div", { class: "empty" }, icon("zap"), h("b", { text: "No rules" }), h("span", { text: "Rules add new articles that match automatically." })));
      return;
    }
    const r = rules[current];
    const enabled = h("input", { type: "checkbox", checked: r.enabled });
    const must = h("input", { class: "input", value: r.mustContain });
    const not = h("input", { class: "input", value: r.mustNotContain });
    const regex = h("input", { type: "checkbox", checked: r.useRegex });
    const cat = h("select", { class: "input" }, categoryOptions(r.assignedCategory || r.torrentParams?.category));
    const save = h("input", { class: "input mono", value: r.savePath || r.torrentParams?.save_path || "", placeholder: "Default" });
    const paused = h("input", { type: "checkbox", checked: r.torrentParams?.stopped ?? r.addPaused });
    const affected = new Set(r.affectedFeeds || []);
    const boxes = feeds.map(([n, f]) => {
      const b = h("input", { type: "checkbox", checked: affected.has(f.url), value: f.url });
      return h("label", { class: "check" }, b, n);
    });
    const matches = h("div", { class: "hint" });
    fill(right, 
      h("label", { class: "check", style: { marginBottom: "10px" } }, enabled, h("b", { text: current }), " is on"),
      field("Must contain", must, "All words must appear; | separates alternatives."),
      field("Must not contain", not),
      h("label", { class: "check" }, regex, "Use regular expressions"),
      h("div", { class: "grid2", style: { marginTop: "12px" } }, field("Category", cat), field("Save to", save)),
      h("label", { class: "check" }, paused, "Add stopped"),
      h("h5", { class: "muted", text: "Feeds", style: { margin: "14px 0 6px" } }),
      boxes.length ? boxes : h("span", { class: "hint", text: "Add a feed first." }),
      h("div", { class: "row", style: { marginTop: "16px" } },
        h("button", { class: "btn primary", type: "button", onclick: async () => {
          const def = {
            enabled: enabled.checked,
            mustContain: must.value,
            mustNotContain: not.value,
            useRegex: regex.checked,
            affectedFeeds: boxes.map((l) => l.querySelector("input")).filter((b) => b.checked).map((b) => b.value),
            assignedCategory: cat.value,
            savePath: save.value.trim(),
            torrentParams: { category: cat.value, save_path: save.value.trim(), stopped: paused.checked },
          };
          const ok = await attempt(() => post("rss/setRule", { ruleName: current, ruleDef: JSON.stringify(def) }), "Rule saved");
          if (ok === undefined) return;
          rules = (await attempt(() => call("rss/rules"))) || {};
          renderList();
          showMatches();
        } }, icon("check"), "Save rule"),
        h("button", { class: "btn", type: "button", onclick: async () => {
          const n = await prompt("Rename rule", "Name", current);
          if (!n || n === current) return;
          await attempt(() => post("rss/renameRule", { ruleName: current, newRuleName: n }));
          rules = (await attempt(() => call("rss/rules"))) || {};
          current = n;
          renderList();
          renderRule();
        } }, "Rename"),
        h("button", { class: "btn danger", type: "button", onclick: async () => {
          await attempt(() => post("rss/removeRule", { ruleName: current }), "Rule removed");
          rules = (await attempt(() => call("rss/rules"))) || {};
          current = Object.keys(rules)[0] ?? null;
          renderList();
          renderRule();
        } }, icon("trash")),
      ),
      h("h5", { class: "muted", text: "Already in the feeds", style: { margin: "18px 0 6px" } }),
      matches,
    );
    const showMatches = async () => {
      const m = await call("rss/matchingArticles", { ruleName: current }).catch(() => ({}));
      const titles = Object.values(m).flat();
      fill(matches, titles.length ? h("ul", { style: { margin: 0, paddingLeft: "18px" } }, titles.slice(0, 30).map((t) => h("li", { text: t }))) : "Nothing matches yet.");
    };
    showMatches();
  };
  renderList();
  renderRule();
  dialog({ title: "RSS download rules", wide: true, body: h("div", { class: "settings", style: { gridTemplateColumns: "230px 1fr" } }, left, right), actions: [{ label: "Done", kind: "primary" }] });
}

// ------------------------------------------------------------------ log

const LOG = { lines: [], last: -1, kinds: store.get("logKinds", { 1: true, 2: true, 4: true, 8: true }), text: "", follow: true };
const LOG_KIND = { 1: ["Info", ""], 2: ["Debug", "info"], 4: ["Warning", "warn"], 8: ["Error", "danger"] };

function logPage() {
  const tbody = h("tbody");
  const wrap = h("div", { class: "table-wrap" }, h("table", { class: "grid log" }, h("colgroup", null, h("col", { style: { width: "170px" } }), h("col", { style: { width: "100px" } }), h("col")), h("thead", null, h("tr", null, h("th", { text: "Time" }), h("th", { text: "Level" }), h("th", { text: "Message" }))), tbody));
  wrap.addEventListener("scroll", () => (LOG.follow = wrap.scrollTop + wrap.clientHeight >= wrap.scrollHeight - 30));
  const chips = Object.entries(LOG_KIND).map(([k, [label]]) =>
    h("button", { class: `btn small ${LOG.kinds[k] ? "on" : ""}`, onclick: (e) => {
      LOG.kinds[k] = !LOG.kinds[k];
      store.set("logKinds", LOG.kinds);
      e.currentTarget.classList.toggle("on");
      renderLog();
    } }, label),
  );
  UI.main.append(
    h("div", { class: "pane" },
      h("div", { class: "toolbar" }, chips, h("button", { class: "btn small", onclick: () => {
        LOG.lines = [];
        renderLog();
      } }, icon("trash"), "Clear view"), h("div", { class: "search-box" }, icon("search"), h("input", { class: "input", placeholder: "Filter messages", value: LOG.text, oninput: debounce((e) => {
        LOG.text = e.target.value;
        renderLog();
      }, 120) }))),
      wrap,
    ),
  );
  UI.log = { tbody, wrap };
  pageEvery(2000, loadLog);
}

async function loadLog() {
  const fresh = await call("log/main", { last_known_id: LOG.last }).catch(() => null);
  if (!fresh?.length) {
    if (!LOG.lines.length) renderLog();
    return;
  }
  LOG.last = fresh[fresh.length - 1].id;
  LOG.lines = LOG.lines.concat(fresh).slice(-2000);
  renderLog();
}

function renderLog() {
  if (!UI.log) return;
  const words = LOG.text.toLowerCase().split(/\s+/).filter(Boolean);
  const lines = LOG.lines.filter((l) => LOG.kinds[l.type]).filter((l) => words.every((w) => l.message.toLowerCase().includes(w)));
  fill(UI.log.tbody, 
    ...lines.map((l) => {
      const [label, cls] = LOG_KIND[l.type] || ["Info", ""];
      return h("tr", null, h("td", { class: "muted", text: new Date(l.timestamp * 1000).toLocaleString() }), h("td", { class: "level" }, h("span", { class: `badge ${cls}`, text: label })), h("td", { class: "msg", text: l.message }));
    }),
  );
  if (LOG.follow) UI.log.wrap.scrollTop = UI.log.wrap.scrollHeight;
}

// ------------------------------------------------------------- settings

async function openSettings() {
  const prefs = await attempt(() => call("app/preferences"));
  if (!prefs) return;
  const changed = {};
  const set = (k) => (v) => (changed[k] = v);
  const num = (k, opts = {}) => {
    const scale = opts.kib ? 1024 : 1;
    const v = prefs[k];
    const shown = opts.kib ? (v > 0 ? Math.round(v / scale) : "") : v < 0 && opts.none ? "" : v;
    return h("input", { class: "input", type: "number", min: opts.min ?? 0, max: opts.max, step: opts.step, placeholder: opts.none || "", value: shown, oninput: (e) => {
      const raw = e.target.value.trim();
      changed[k] = raw === "" ? (opts.kib ? 0 : -1) : Number(raw) * scale;
    } });
  };
  const text = (k, ph) => h("input", { class: "input mono", value: prefs[k] || "", placeholder: ph, oninput: (e) => (changed[k] = e.target.value) });
  const sw = (k, label, hint) => switchEl(label, !!prefs[k], set(k), hint);
  const pages = {
    Downloads: [
      field("Default download folder", text("save_path")),
      sw("temp_path_enabled", "Keep unfinished downloads in another folder"),
      field("Incomplete downloads folder", text("temp_path", "Choose a folder")),
      sw("add_stopped_enabled", "Add new torrents stopped"),
      h("small", { class: "hint", text: "Folders are on the Riptor computer and must be ones Riptor may write to." }),
    ],
    Speed: [
      h("h5", { text: "Limits" }),
      h("div", { class: "grid2" }, field("Download (KiB/s)", num("dl_limit", { kib: true, none: "No limit" })), field("Upload (KiB/s)", num("up_limit", { kib: true, none: "No limit" }))),
      h("h5", { text: "Alternative limits" }),
      h("div", { class: "grid2" }, field("Download (KiB/s)", num("alt_dl_limit", { kib: true, none: "No limit" })), field("Upload (KiB/s)", num("alt_up_limit", { kib: true, none: "No limit" }))),
      sw("scheduler_enabled", "Use the alternative limits on a schedule"),
      h("div", { class: "grid2" },
        field("From", h("div", { class: "row" }, num("schedule_from_hour", { max: 23 }), ":", num("schedule_from_min", { max: 59 }))),
        field("To", h("div", { class: "row" }, num("schedule_to_hour", { max: 23 }), ":", num("schedule_to_min", { max: 59 }))),
      ),
      field("Days", select([["0", "Every day"], ["1", "Weekdays"], ["2", "Weekends"], ...["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"].map((d, i) => [String(i + 3), d])], String(prefs.scheduler_days), (v) => (changed.scheduler_days = Number(v)))),
    ],
    Connection: [
      field("Listening port", num("listen_port", { max: 65535 }), "0 picks a random port at each start."),
      sw("upnp", "Forward the port with UPnP / NAT-PMP"),
      sw("dht", "DHT (find peers without trackers)"),
      sw("lsd", "Find peers on the local network"),
      field("Encryption", select([["0", "Prefer encryption"], ["1", "Require encryption"], ["2", "Off"]], String(prefs.encryption), (v) => (changed.encryption = Number(v)))),
      sw("anonymous_mode", "Anonymous mode", "Hides the client name from peers and trackers."),
      field("Peer limit", num("max_connec", { none: "No limit" })),
    ],
    Queue: [
      sw("queueing_enabled", "Limit how many torrents run at once"),
      h("div", { class: "grid2" }, field("Active downloads", num("max_active_downloads", { none: "No limit" })), field("Active uploads", num("max_active_uploads", { none: "No limit" }))),
      field("Active torrents", num("max_active_torrents", { none: "No limit" })),
      sw("dont_count_slow_torrents", "Don't count slow torrents"),
      h("div", { class: "grid2" }, field("Slow below, download (KiB/s)", num("slow_torrent_dl_rate_threshold")), field("Slow below, upload (KiB/s)", num("slow_torrent_ul_rate_threshold"))),
    ],
    Seeding: [
      sw("max_ratio_enabled", "Stop seeding at a ratio"),
      field("Ratio", num("max_ratio", { step: 0.1, none: "None" })),
      sw("max_seeding_time_enabled", "Stop seeding after a time"),
      field("Minutes", num("max_seeding_time", { none: "None" })),
      sw("max_inactive_seeding_time_enabled", "Stop seeding after a time without uploads"),
      field("Minutes", num("max_inactive_seeding_time", { none: "None" })),
      field("Then", select([["0", "Stop the torrent"], ["1", "Remove the torrent"], ["3", "Remove the torrent and its files"]], String(prefs.max_ratio_act), (v) => (changed.max_ratio_act = Number(v)))),
    ],
    RSS: [
      sw("rss_auto_downloading_enabled", "Download rules add torrents automatically"),
      field("Check feeds every (minutes)", num("rss_refresh_interval", { min: 5, max: 1440 })),
    ],
    "Remote control": [
      h("p", { class: "muted", text: "These are changed in the Riptor app itself, never from here." }),
      h(
        "div",
        { class: "props", style: { padding: 0 } },
        [
          ["Address", `${prefs.web_ui_address}:${prefs.web_ui_port}`],
          ["User name", prefs.web_ui_username],
          ["HTTPS", prefs.use_https ? "On" : "Off"],
          ["Ban after failures", String(prefs.web_ui_max_auth_fail_count)],
          ["Ban length", eta(prefs.web_ui_ban_duration)],
          ["Session timeout", eta(prefs.web_ui_session_timeout)],
        ].map(([k, v]) => h("div", { class: "prop" }, h("span", { text: k }), h("b", { text: v }))),
      ),
      h("h5", { text: "Appearance" }),
      field("Theme", select([["auto", "Match this device"], ["dark", "Dark"], ["light", "Light"]], store.get("theme", "auto"), (v) => {
        store.set("theme", v);
        applyTheme();
      })),
    ],
  };
  const section = h("section");
  const nav = h("nav");
  const show = (name) => {
    for (const b of nav.children) b.classList.toggle("active", b.dataset.page === name);
    fill(section, ...pages[name]);
  };
  for (const name of Object.keys(pages)) nav.append(h("button", { class: "nav-item", type: "button", data: { page: name }, onclick: () => show(name) }, h("span", { text: name })));
  show("Downloads");
  dialog({
    title: "Options",
    wide: true,
    body: h("div", { class: "settings" }, nav, section),
    actions: [
      { label: "Cancel" },
      {
        label: "Save",
        kind: "primary",
        submit: true,
        run: async () => {
          if (!Object.keys(changed).length) return;
          if ("max_ratio" in changed && changed.max_ratio >= 0 && !("max_ratio_enabled" in changed)) changed.max_ratio_enabled = true;
          if ("max_seeding_time" in changed && changed.max_seeding_time >= 0 && !("max_seeding_time_enabled" in changed)) changed.max_seeding_time_enabled = true;
          const r = await attempt(() => post("app/setPreferences", { json: JSON.stringify(changed) }), "Options saved");
          if (r === undefined) return false;
          defaultSave = null;
          poll();
        },
      },
    ],
  });
}

// ------------------------------------------------------------ keyboard

function showShortcuts() {
  const rows = [
    ["1 – 4", "Switch between Transfers, Search, RSS and Log"],
    ["A", "Add torrents"],
    ["/", "Filter the list"],
    ["Ctrl + A", "Select every torrent shown"],
    ["Space", "Start or stop the selection"],
    ["Delete", "Remove the selection"],
    ["Enter", "Show details"],
    ["↑ ↓", "Move the selection"],
    ["Esc", "Clear the selection or close a window"],
    ["Ctrl + V", "Paste a magnet link to add it"],
  ];
  dialog({
    title: "Keyboard shortcuts",
    body: h("table", { class: "grid" }, h("tbody", null, rows.map(([k, d]) => h("tr", { class: "compact" }, h("td", { style: { width: "130px" } }, h("kbd", { text: k })), h("td", { text: d }))))),
    actions: [{ label: "Close", kind: "primary" }],
  });
}

addEventListener("keydown", (e) => {
  if (e.key === "Escape" && openMenu?.isConnected) {
    openMenu.remove();
    return;
  }
  if (!UI.veil || document.querySelector(".veil, .login") || e.target.closest?.("input, textarea, select")) return;
  const k = e.key;
  if (["1", "2", "3", "4"].includes(k) && !e.ctrlKey && !e.metaKey && !e.altKey) {
    e.preventDefault();
    setTab(TABS[Number(k) - 1][0]);
    return;
  }
  if (S.tab !== "transfers") return;
  if (k === "/") {
    e.preventDefault();
    UI.tx.filterInput.focus();
  } else if ((k === "a" || k === "A") && (e.ctrlKey || e.metaKey)) {
    e.preventDefault();
    visible().forEach((t) => S.selected.add(t.hash));
    renderTransfers();
  } else if (k === "a" || k === "A") {
    e.preventDefault();
    addDialog();
  } else if (k === "Delete" || k === "Backspace") {
    e.preventDefault();
    removeSelected();
  } else if (k === " ") {
    e.preventDefault();
    const ids = [...S.selected];
    if (!ids.length) return;
    const anyRunning = ids.some((id) => kind(S.torrents[id]) !== "stopped");
    act(anyRunning ? "torrents/stop" : "torrents/start");
  } else if (k === "Escape") {
    S.selected.clear();
    renderTransfers();
  } else if (k === "Enter") {
    S.detailsOpen = true;
    store.set("detailsOpen", true);
    renderDetails(true);
  } else if (k === "ArrowDown" || k === "ArrowUp") {
    e.preventDefault();
    const list = visible().map((t) => t.hash);
    if (!list.length) return;
    const i = list.indexOf(S.focus);
    const next = list[Math.max(0, Math.min(list.length - 1, i < 0 ? 0 : i + (k === "ArrowDown" ? 1 : -1)))];
    if (e.shiftKey) {
      S.selected.add(next);
      S.focus = next;
    } else selectOnly(next);
    renderTransfers();
    UI.tx.tbody.querySelector(`tr[data-id="${next}"]`)?.scrollIntoView({ block: "nearest" });
  }
});

document.addEventListener("visibilitychange", () => document.visibilityState === "visible" && UI.veil && poll());

start();
