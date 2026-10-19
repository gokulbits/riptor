import { open } from "@tauri-apps/plugin-dialog";
import clsx from "clsx";
import {
  AlertTriangle,
  CheckCircle2,
  Download,
  ExternalLink,
  FileCode2,
  Globe,
  KeyRound,
  Loader2,
  Plus,
  RefreshCw,
  ShieldAlert,
  Trash2,
  XCircle,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Dialog, Field, Toggle } from "../../components/ui";
import { api, errorText } from "../../lib/ipc";
import { formatBytes } from "../../lib/format";
import type { CatalogEntry, PluginCatalog, PluginPreview, ProviderInfo, PythonStatus } from "../../lib/types";
import { useApp } from "../../stores/app";

const KIND_LABEL = { builtin: "Built in", torznab: "Torznab", plugin: "Plugin" } as const;
const PYTHON_DOWNLOADS = "https://www.python.org/downloads/";
const COMMUNITY_PAGE = "https://github.com/qbittorrent/search-plugins/wiki/Unofficial-search-plugins";
/** Plugins reviewed and installed together at most. */
const MAX_BATCH = 20;

const openUrl = (url: string) => api.openWebUrl(url).catch(() => {});
const host = (url: string | null) => {
  try {
    return url ? new URL(url).host : null;
  } catch {
    return null;
  }
};

function TorznabForm({ onAdded }: { onAdded: () => void }) {
  const toast = useApp((s) => s.toast);
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const add = async () => {
    setBusy(true);
    try {
      await api.addTorznab(name, url, key);
      setName("");
      setUrl("");
      setKey("");
      onAdded();
      toast("success", "Indexer added");
    } catch (e) {
      toast("error", errorText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-[1fr_2fr] gap-2">
        <Field label="Name">
          <input className="input" value={name} onChange={(e) => setName(e.target.value)} placeholder="My indexer" maxLength={64} />
        </Field>
        <Field label="Torznab feed URL" hint="The indexer's Torznab feed URL. https, or http on this computer/your network.">
          <input
            className="input font-mono text-xs"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder="http://127.0.0.1:9117/api/v2.0/indexers/all/results/torznab/"
          />
        </Field>
      </div>
      <Field label="API key" hint="Stored in the system credential store, not in Riptor's files or backups.">
        <input className="input font-mono text-xs" type="password" value={key} onChange={(e) => setKey(e.target.value)} autoComplete="off" />
      </Field>
      <button className="btn-primary" disabled={busy || !name.trim() || !url.trim() || !key.trim()} onClick={add}>
        {busy ? <Loader2 size={14} className="animate-spin" /> : <KeyRound size={14} />} Add indexer
      </button>
    </div>
  );
}

/** Where plugins get their Python from, and what to do when there is none. */
function PythonPanel({ python, onChange }: { python: PythonStatus | null; onChange: (p: PythonStatus) => void }) {
  const toast = useApp((s) => s.toast);
  const [busy, setBusy] = useState<"check" | "install" | "remove" | null>(null);
  const run = async (kind: "check" | "install" | "remove") => {
    setBusy(kind);
    try {
      const p = kind === "check" ? await api.pythonStatus(true) : kind === "install" ? await api.installPython() : await api.removePython();
      onChange(p);
      if (kind === "install") toast("success", `Python ${p.bundle_version} is ready for plugins`);
    } catch (e) {
      toast("error", errorText(e));
    } finally {
      setBusy(null);
    }
  };
  if (!python) return null;

  const checkAgain = (
    <button className="btn-ghost px-1.5 py-0.5 text-xs" onClick={() => run("check")} disabled={busy != null}>
      <RefreshCw size={12} className={busy === "check" ? "animate-spin" : ""} /> Check again
    </button>
  );

  if (python.available) {
    return (
      <div className="flex flex-wrap items-center gap-2 text-xs text-muted">
        <CheckCircle2 size={14} className="text-ok" />
        <span>
          Plugins run with Python {python.version}
          {python.bundled ? " (set up by Riptor)" : ""}
          {python.sandboxed ? " · sandboxed" : ""}
        </span>
        <span className="flex-1" />
        {python.bundled_installed && (
          <button className="btn-ghost px-1.5 py-0.5 text-xs hover:text-danger" onClick={() => run("remove")} disabled={busy != null}>
            {busy === "remove" ? <Loader2 size={12} className="animate-spin" /> : <Trash2 size={12} />} Remove Riptor's Python
          </button>
        )}
        {checkAgain}
      </div>
    );
  }

  return (
    <div className="space-y-2 rounded-md border border-warn/40 bg-warn/10 p-3 text-[13px]">
      <p className="flex items-start gap-2 font-medium text-warn">
        <AlertTriangle size={16} className="mt-0.5 shrink-0" />
        Search plugins need Python 3.9 or newer, and none was found on this computer.
      </p>
      <p className="text-xs text-muted">
        The Internet Archive and Torznab indexers work without it. Plugins can be installed now and start working once Python is
        available.
      </p>
      {python.can_bundle && (
        <div className="space-y-1">
          <button className="btn-primary" onClick={() => run("install")} disabled={busy != null}>
            {busy === "install" ? <Loader2 size={14} className="animate-spin" /> : <Download size={14} />}
            {busy === "install" ? "Setting up Python…" : `Set up Python ${python.bundle_version} for Riptor`}
          </button>
          <p className="text-xs text-muted">
            Downloads the official embeddable Python{python.bundle_size ? ` (${formatBytes(python.bundle_size)})` : ""} from python.org,
            checks its SHA-256 fingerprint, and keeps it in Riptor's data folder. Nothing is installed system-wide and no administrator
            rights are needed.
          </p>
        </div>
      )}
      <p className="text-xs">
        {python.can_bundle ? "Or install it yourself" : navigator.userAgent.includes("Linux") ? "Install python3 with your package manager or" : "Install it"}{" "}
        from{" "}
        <button className="underline" onClick={() => openUrl(PYTHON_DOWNLOADS)}>
          python.org
        </button>
        , then {checkAgain}
      </p>
    </div>
  );
}

/** Review fetched plugins (source + SHA-256) and install the confirmed ones. */
function ReviewPanel({
  previews,
  failures,
  onDone,
  onCancel,
}: {
  previews: PluginPreview[];
  failures: { source: string; error: string }[];
  onDone: () => void;
  onCancel: () => void;
}) {
  const toast = useApp((s) => s.toast);
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const install = async () => {
    setBusy(true);
    const done: string[] = [];
    for (const p of previews) {
      try {
        await api.installPlugin(p.sha256);
        done.push(p.declared_name ?? p.id);
      } catch (e) {
        toast("error", `${p.declared_name ?? p.id}: ${errorText(e)}`);
      }
    }
    setBusy(false);
    if (done.length) toast("success", done.length === 1 ? `Installed ${done[0]}` : `Installed ${done.length} plugins`);
    onDone();
  };
  return (
    <div className="card space-y-3 border-warn/40 p-3 text-[13px]">
      <p className="flex items-center gap-2 font-medium text-warn">
        <ShieldAlert size={16} /> Plugins are programs. Only install ones you trust.
      </p>
      {previews.map((p) => (
        <dl key={p.sha256} className="grid grid-cols-[6rem_1fr] gap-x-3 gap-y-0.5 border-t border-border pt-2 first-of-type:border-0 first-of-type:pt-0">
          <dt className="text-muted">Name</dt>
          <dd className="font-medium">
            {p.declared_name ?? p.id}
            {p.replaces_existing && <span className="ml-2 text-xs font-normal text-warn">replaces the installed copy</span>}
          </dd>
          <dt className="text-muted">Site</dt>
          <dd className="truncate">{p.declared_url ?? "–"}</dd>
          <dt className="text-muted">Source</dt>
          <dd className="select-text truncate font-mono text-xs" title={p.source}>
            {p.source}
          </dd>
          <dt className="text-muted">SHA-256</dt>
          <dd className="select-text break-all font-mono text-xs">{p.sha256}</dd>
          <dt className="text-muted">Size</dt>
          <dd>{formatBytes(p.size)}</dd>
        </dl>
      ))}
      {failures.map((f) => (
        <p key={f.source} className="flex items-start gap-2 text-xs text-danger">
          <XCircle size={13} className="mt-0.5 shrink-0" />
          <span className="min-w-0">
            <span className="block truncate font-mono" title={f.source}>
              {f.source}
            </span>
            {f.error}
          </span>
        </p>
      ))}
      {previews.length > 0 && (
        <Toggle
          checked={confirmed}
          onChange={setConfirmed}
          label={previews.length === 1 ? "I trust this plugin and its source" : `I trust these ${previews.length} plugins and their sources`}
          hint="Each runs on your computer whenever you search with it, without administrator rights."
        />
      )}
      <div className="flex gap-2">
        {previews.length > 0 && (
          <button className="btn-primary" disabled={!confirmed || busy} onClick={install}>
            {busy && <Loader2 size={14} className="animate-spin" />} Install{previews.length > 1 ? ` ${previews.length}` : ""}
          </button>
        )}
        <button className="btn-ghost" onClick={onCancel} disabled={busy}>
          {previews.length ? "Cancel" : "Close"}
        </button>
      </div>
    </div>
  );
}

type Review = { previews: PluginPreview[]; failures: { source: string; error: string }[] };

async function fetchReviews(sources: string[]): Promise<Review> {
  const settled = await Promise.allSettled(sources.map((s) => api.previewPlugin(s)));
  const review: Review = { previews: [], failures: [] };
  settled.forEach((r, i) => {
    if (r.status === "fulfilled") review.previews.push(r.value);
    else review.failures.push({ source: sources[i], error: errorText(r.reason) });
  });
  return review;
}

const HEALTH: Record<CatalogEntry["health"], { label: string; className: string } | null> = {
  working: null,
  issues: { label: "Has problems", className: "bg-warn/15 text-warn" },
  broken: { label: "Broken", className: "bg-danger/15 text-danger" },
  unknown: null,
};

function CatalogRow({ e, selected, onToggle }: { e: CatalogEntry; selected: boolean; onToggle: () => void }) {
  const site = host(e.site);
  const health = HEALTH[e.health];
  return (
    <li className={clsx("flex items-start gap-3 px-3 py-2 text-[13px]", e.problem && "opacity-60")}>
      <input
        type="checkbox"
        className="mt-1"
        checked={selected}
        disabled={!!e.problem}
        onChange={onToggle}
        aria-label={`Select ${e.name}`}
        title={e.problem ?? undefined}
      />
      <div className="min-w-0 flex-1">
        <p className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
          <span className="font-medium">{e.name}</span>
          {e.version && <span className="text-xs text-muted">v{e.version}</span>}
          {e.installed && <span className="rounded bg-ok/15 px-1.5 py-0.5 text-xxs text-ok">Installed</span>}
          {health && <span className={clsx("rounded px-1.5 py-0.5 text-xxs", health.className)}>{health.label}</span>}
          {e.private && <span className="rounded bg-raised px-1.5 py-0.5 text-xxs text-muted">Needs an account</span>}
        </p>
        <p className="truncate text-xs text-muted" title={e.note ?? undefined}>
          {[site, e.author && `by ${e.author}`, e.problem ?? e.note].filter(Boolean).join(" · ")}
        </p>
      </div>
      {e.site && (
        <button className="btn-ghost p-1" title={`Open ${site}`} onClick={() => openUrl(e.site!)}>
          <ExternalLink size={13} />
        </button>
      )}
    </li>
  );
}

/** Plugins from the official search-plugins repository and community list. */
function PluginCatalogView({ onInstalled }: { onInstalled: () => void }) {
  const toast = useApp((s) => s.toast);
  const [catalog, setCatalog] = useState<PluginCatalog | null>(null);
  const [loading, setLoading] = useState(false);
  const [text, setText] = useState("");
  const [community, setCommunity] = useState(false);
  const [showPrivate, setShowPrivate] = useState(false);
  const [showBroken, setShowBroken] = useState(false);
  const [selected, setSelected] = useState<CatalogEntry[]>([]);
  const [review, setReview] = useState<Review | null>(null);
  const [reviewing, setReviewing] = useState(false);

  const load = useCallback(
    (refresh: boolean) => {
      setLoading(true);
      api
        .pluginCatalog(refresh)
        .then(setCatalog)
        .catch((e) => toast("error", errorText(e)))
        .finally(() => setLoading(false));
    },
    [toast],
  );
  useEffect(() => load(false), [load]);

  const visible = useMemo(() => {
    const t = text.trim().toLowerCase();
    return (catalog?.entries ?? []).filter(
      (e) =>
        (e.origin === "official" || community) &&
        (showPrivate || !e.private) &&
        (showBroken || e.health !== "broken") &&
        (!t || e.name.toLowerCase().includes(t) || (e.site ?? "").toLowerCase().includes(t)),
    );
  }, [catalog, text, community, showPrivate, showBroken]);

  const toggle = (e: CatalogEntry) => {
    setSelected((cur) => {
      if (cur.some((x) => x.download_url === e.download_url)) return cur.filter((x) => x.download_url !== e.download_url);
      // Two plugins with the same file name would overwrite each other.
      const next = cur.filter((x) => x.id !== e.id);
      if (next.length >= MAX_BATCH) {
        toast("error", `Review up to ${MAX_BATCH} plugins at a time`);
        return cur;
      }
      return [...next, e];
    });
  };
  const startReview = async () => {
    setReviewing(true);
    try {
      setReview(await fetchReviews(selected.map((e) => e.download_url)));
    } finally {
      setReviewing(false);
    }
  };
  const finish = () => {
    setReview(null);
    setSelected([]);
    onInstalled();
    load(false);
  };

  if (review) return <ReviewPanel {...review} onDone={finish} onCancel={() => setReview(null)} />;

  const official = visible.filter((e) => e.origin === "official");
  const others = visible.filter((e) => e.origin === "community");
  const isSelected = (e: CatalogEntry) => selected.some((x) => x.download_url === e.download_url);

  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2">
        <input className="input w-56 py-1 text-xs" placeholder="Filter plugins" value={text} onChange={(e) => setText(e.target.value)} />
        <label className="flex items-center gap-1 text-xs text-muted">
          <input type="checkbox" checked={community} onChange={(e) => setCommunity(e.target.checked)} /> Community plugins
        </label>
        {community && (
          <>
            <label className="flex items-center gap-1 text-xs text-muted">
              <input type="checkbox" checked={showPrivate} onChange={(e) => setShowPrivate(e.target.checked)} /> Private sites
            </label>
            <label className="flex items-center gap-1 text-xs text-muted">
              <input type="checkbox" checked={showBroken} onChange={(e) => setShowBroken(e.target.checked)} /> Broken
            </label>
          </>
        )}
        <span className="flex-1" />
        <button className="btn-ghost px-1.5 py-0.5 text-xs" onClick={() => load(true)} disabled={loading} title="Fetch the lists again from GitHub">
          <RefreshCw size={12} className={loading ? "animate-spin" : ""} /> Refresh
        </button>
      </div>
      {catalog?.errors.map((e) => (
        <p key={e} className="flex items-start gap-2 text-xs text-danger">
          <AlertTriangle size={13} className="mt-0.5 shrink-0" /> {e}
        </p>
      ))}
      {community && (
        <p className="text-xs text-muted">
          Community plugins are written by volunteers and listed on{" "}
          <button className="underline" onClick={() => openUrl(COMMUNITY_PAGE)}>
            the plugins wiki
          </button>
          . Riptor doesn't check them; read the source before installing.
        </p>
      )}
      <div className="card max-h-80 overflow-auto">
        {!catalog && loading ? (
          <p className="flex items-center gap-2 px-3 py-4 text-xs text-muted">
            <Loader2 size={13} className="animate-spin" /> Loading plugin lists from GitHub…
          </p>
        ) : visible.length === 0 ? (
          <p className="px-3 py-4 text-xs text-muted">No plugins match.</p>
        ) : (
          <>
            {official.length > 0 && <h4 className="px-3 pt-2 text-xxs font-semibold uppercase tracking-wider text-muted">Official</h4>}
            <ul className="divide-y divide-border">
              {official.map((e) => (
                <CatalogRow key={e.download_url} e={e} selected={isSelected(e)} onToggle={() => toggle(e)} />
              ))}
            </ul>
            {others.length > 0 && <h4 className="px-3 pt-3 text-xxs font-semibold uppercase tracking-wider text-muted">Community</h4>}
            <ul className="divide-y divide-border">
              {others.map((e) => (
                <CatalogRow key={e.download_url} e={e} selected={isSelected(e)} onToggle={() => toggle(e)} />
              ))}
            </ul>
          </>
        )}
      </div>
      <div className="flex items-center gap-2">
        <button className="btn-primary" disabled={selected.length === 0 || reviewing} onClick={startReview}>
          {reviewing ? <Loader2 size={14} className="animate-spin" /> : <ShieldAlert size={14} />}
          Review {selected.length || ""} selected…
        </button>
        {selected.length > 0 && (
          <button className="btn-ghost text-xs" onClick={() => setSelected([])}>
            Clear selection
          </button>
        )}
        <span className="text-xs text-muted">You'll see each plugin's source and SHA-256 before anything is installed.</span>
      </div>
    </div>
  );
}

/** Install one plugin from a URL or a file. */
function PluginInstaller({ onInstalled }: { onInstalled: () => void }) {
  const [url, setUrl] = useState("");
  const [review, setReview] = useState<Review | null>(null);
  const [busy, setBusy] = useState(false);

  const inspect = async (source: string) => {
    setBusy(true);
    setReview(null);
    try {
      setReview(await fetchReviews([source]));
    } finally {
      setBusy(false);
    }
  };
  const pickFile = async () => {
    const f = await open({ filters: [{ name: "Python search plugin", extensions: ["py"] }] });
    if (typeof f === "string") inspect(f);
  };

  return (
    <div className="space-y-2">
      <div className="flex gap-2">
        <input
          className="input font-mono text-xs"
          placeholder="https://raw.githubusercontent.com/…/engine.py"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
        />
        <button className="btn-ghost border border-border" disabled={busy || !url.startsWith("https://")} onClick={() => inspect(url.trim())}>
          {busy ? <Loader2 size={14} className="animate-spin" /> : <Globe size={14} />} From URL
        </button>
        <button className="btn-ghost border border-border" disabled={busy} onClick={pickFile}>
          <FileCode2 size={14} /> From file
        </button>
      </div>
      {review && (
        <ReviewPanel
          {...review}
          onDone={() => {
            setReview(null);
            setUrl("");
            onInstalled();
          }}
          onCancel={() => setReview(null)}
        />
      )}
    </div>
  );
}

function LookupSetting() {
  const toast = useApp((s) => s.toast);
  const [on, setOn] = useState<boolean | null>(null);
  useEffect(() => {
    api
      .searchOptions()
      .then((o) => setOn(o.scrape))
      .catch(() => {});
  }, []);
  const change = async (v: boolean) => {
    try {
      await api.searchSetScrape(v);
      setOn(v);
    } catch (e) {
      toast("error", errorText(e));
    }
  };
  if (on == null) return null;
  return (
    <Toggle
      checked={on}
      onChange={change}
      label="Ask trackers for missing seed and peer counts"
      hint="For results that come without them, such as the Internet Archive's. Those trackers learn which torrents you looked at. Addresses on your own network are never contacted."
    />
  );
}

export function EnginesDialog({ onClose }: { onClose: () => void }) {
  const toast = useApp((s) => s.toast);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [python, setPython] = useState<PythonStatus | null>(null);

  const reload = useCallback(() => {
    api.searchProviders().then(setProviders).catch((e) => toast("error", errorText(e)));
  }, [toast]);
  useEffect(() => {
    reload();
    api.pythonStatus(false).then(setPython).catch(() => {});
  }, [reload]);

  const pythonChanged = (p: PythonStatus) => {
    setPython(p);
    reload();
  };
  const toggle = async (p: ProviderInfo, enabled: boolean) => {
    try {
      await api.searchSetEnabled(p.id, enabled);
      reload();
    } catch (e) {
      toast("error", errorText(e));
    }
  };
  const remove = async (p: ProviderInfo) => {
    try {
      if (p.kind === "torznab") await api.removeTorznab(p.id);
      else await api.uninstallPlugin(p.id);
      reload();
    } catch (e) {
      toast("error", errorText(e));
    }
  };

  return (
    <Dialog title="Search engines" onClose={onClose} width="max-w-3xl">
      <div className="space-y-6">
        <section>
          <h3 className="mb-2 text-xs font-semibold uppercase tracking-wider text-muted">Installed</h3>
          <ul className="card divide-y divide-border">
            {providers.map((p) => (
              <li key={p.id} className="flex items-center gap-3 px-3 py-2 text-[13px]">
                <div className="min-w-0 flex-1">
                  <p className="flex items-center gap-2">
                    <span className="truncate font-medium">{p.name}</span>
                    <span className="rounded bg-raised px-1.5 py-0.5 text-xxs text-muted">{KIND_LABEL[p.kind]}</span>
                  </p>
                  {p.error ? (
                    <p className="truncate text-xs text-danger" title={p.error}>
                      {p.error}
                    </p>
                  ) : (
                    <p className="truncate text-xs text-muted">{p.url}</p>
                  )}
                  {p.id === "archive" && (
                    <p className="text-xs text-muted">
                      Archive items are mostly served by web seeds, which Riptor does not support yet, so some downloads may be slow.
                    </p>
                  )}
                </div>
                {p.url.startsWith("https://") && (
                  <button className="btn-ghost p-1" title="Open site" onClick={() => openUrl(p.url)}>
                    <ExternalLink size={13} />
                  </button>
                )}
                {p.kind !== "builtin" && (
                  <button className="btn-ghost p-1 hover:text-danger" title="Remove" onClick={() => remove(p)}>
                    <Trash2 size={13} />
                  </button>
                )}
                <Toggle checked={p.enabled} onChange={(v) => toggle(p, v)} label="" disabled={!!p.error} />
              </li>
            ))}
          </ul>
          <div className="mt-2">
            <LookupSetting />
          </div>
        </section>

        <section className="space-y-2">
          <h3 className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-muted">
            <Plus size={12} /> Search plugins
          </h3>
          <PythonPanel python={python} onChange={pythonChanged} />
          <PluginCatalogView onInstalled={reload} />
        </section>

        <section>
          <h3 className="mb-2 flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-muted">
            <Plus size={12} /> Plugin from a URL or file
          </h3>
          <PluginInstaller onInstalled={reload} />
        </section>

        <section>
          <h3 className="mb-2 flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-muted">
            <Plus size={12} /> Torznab indexer
          </h3>
          <TorznabForm onAdded={reload} />
        </section>
      </div>
    </Dialog>
  );
}
