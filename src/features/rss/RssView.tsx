import { ask, open } from "@tauri-apps/plugin-dialog";
import clsx from "clsx";
import { AlertTriangle, CheckCheck, Download, ExternalLink, FolderOpen, ListFilter, Pencil, Plus, RefreshCw, Rss, Trash2 } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Dialog, Field, Toggle } from "../../components/ui";
import { api, errorText, events } from "../../lib/ipc";
import { formatBytes, formatDate } from "../../lib/format";
import type { RssArticle, RssRule, RssRuleMatch, RssView as View, Source } from "../../lib/types";
import { useApp } from "../../stores/app";

const NEW_RULE: RssRule = {
  id: 0,
  name: "",
  enabled: true,
  must_contain: "",
  must_not_contain: "",
  use_regex: false,
  feeds: [],
  category: null,
  save_path: null,
  start_paused: false,
  last_match: null,
};

const sourceOf = (link: string): Source => (link.startsWith("magnet:") ? { kind: "magnet", value: link } : { kind: "url", value: link });

function AddFeedDialog({ onClose, onAdded }: { onClose: () => void; onAdded: (id: number) => void }) {
  const toast = useApp((s) => s.toast);
  const [url, setUrl] = useState("");
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const add = async () => {
    setBusy(true);
    try {
      onAdded(await api.rssAddFeed(url.trim(), name.trim() || null));
      onClose();
    } catch (e) {
      toast("error", errorText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog
      title="Add RSS Feed"
      onClose={onClose}
      width="max-w-lg"
      footer={
        <>
          <button className="btn-ghost" onClick={onClose}>
            Cancel
          </button>
          <button className="btn-primary" disabled={!url.trim() || busy} onClick={add}>
            Add Feed
          </button>
        </>
      }
    >
      <div className="space-y-3">
        <Field label="Feed URL" hint="RSS or Atom, http(s). Fetched through the VPN when it's on.">
          <input
            className="input font-mono text-xs"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && url.trim() && add()}
            placeholder="https://…"
          />
        </Field>
        <Field label="Name" hint="Optional; the feed's own title is used otherwise.">
          <input className="input" value={name} onChange={(e) => setName(e.target.value)} />
        </Field>
      </div>
    </Dialog>
  );
}

function RenameDialog({ current, onClose, onSave }: { current: string; onClose: () => void; onSave: (name: string) => void }) {
  const [name, setName] = useState(current);
  const ok = name.trim().length > 0;
  return (
    <Dialog
      title="Rename Feed"
      onClose={onClose}
      width="max-w-sm"
      footer={
        <>
          <button className="btn-ghost" onClick={onClose}>
            Cancel
          </button>
          <button className="btn-primary" disabled={!ok} onClick={() => onSave(name.trim())}>
            Rename
          </button>
        </>
      }
    >
      <input className="input" value={name} onChange={(e) => setName(e.target.value)} onKeyDown={(e) => e.key === "Enter" && ok && onSave(name.trim())} />
    </Dialog>
  );
}

function RulesDialog({ view, categories, onClose, onChanged }: { view: View; categories: string[]; onClose: () => void; onChanged: () => void }) {
  const toast = useApp((s) => s.toast);
  const [rule, setRule] = useState<RssRule>(view.rules[0] ?? NEW_RULE);
  const [matches, setMatches] = useState<RssRuleMatch[] | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const set = (p: Partial<RssRule>) => setRule((r) => ({ ...r, ...p }));

  useEffect(() => {
    if (!rule.must_contain.trim()) {
      setMatches(null);
      setProblem(null);
      return;
    }
    const t = setTimeout(() => {
      api
        .rssPreviewRule(rule)
        .then((m) => {
          setMatches(m);
          setProblem(null);
        })
        .catch((e) => {
          setMatches(null);
          setProblem(errorText(e));
        });
    }, 250);
    return () => clearTimeout(t);
  }, [rule]);

  const save = async () => {
    try {
      const id = await api.rssSaveRule(rule);
      set({ id });
      onChanged();
      toast("success", "Rule saved");
    } catch (e) {
      toast("error", errorText(e));
    }
  };
  const remove = async () => {
    if (!rule.id) return setRule(NEW_RULE);
    if (!(await ask(`Delete the rule “${rule.name}”?`, { title: "Delete rule", kind: "warning" }))) return;
    try {
      await api.rssDeleteRule(rule.id);
      setRule(NEW_RULE);
      onChanged();
    } catch (e) {
      toast("error", errorText(e));
    }
  };
  const pickFolder = async () => {
    const dir = await open({ directory: true, defaultPath: rule.save_path ?? undefined });
    if (typeof dir === "string") set({ save_path: dir });
  };

  return (
    <Dialog
      title="RSS Download Rules"
      onClose={onClose}
      width="max-w-4xl"
      footer={
        <>
          <button className="btn-ghost hover:text-danger" onClick={remove}>
            <Trash2 size={14} /> {rule.id ? "Delete" : "Discard"}
          </button>
          <span className="flex-1" />
          <button className="btn-ghost" onClick={onClose}>
            Close
          </button>
          <button className="btn-primary" disabled={!rule.name.trim() || !rule.must_contain.trim()} onClick={save}>
            Save Rule
          </button>
        </>
      }
    >
      <div className="flex gap-4">
        <nav className="w-48 shrink-0 space-y-0.5">
          {view.rules.map((r) => (
            <button
              key={r.id}
              onClick={() => setRule(r)}
              className={clsx(
                "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm",
                rule.id === r.id ? "bg-accent/15 text-fg" : "text-muted hover:bg-raised hover:text-fg",
              )}
            >
              <span className={clsx("h-2 w-2 shrink-0 rounded-full", r.enabled ? "bg-ok" : "bg-border")} />
              <span className="truncate">{r.name}</span>
            </button>
          ))}
          <button className="btn-ghost w-full justify-start text-sm" onClick={() => setRule(NEW_RULE)}>
            <Plus size={14} /> New Rule
          </button>
        </nav>
        <div className="min-w-0 flex-1 space-y-3">
          <div className="grid grid-cols-2 gap-3">
            <Field label="Name">
              <input className="input" value={rule.name} onChange={(e) => set({ name: e.target.value })} />
            </Field>
            <div className="pt-5">
              <Toggle checked={rule.enabled} onChange={(v) => set({ enabled: v })} label="Enabled" />
            </div>
          </div>
          <Field
            label="Must Contain"
            hint={rule.use_regex ? "A regular expression, case-insensitive." : "Every word must appear (any order, any case). Use | between alternatives, e.g. show 1080p | show 2160p."}
          >
            <input className="input font-mono text-xs" value={rule.must_contain} onChange={(e) => set({ must_contain: e.target.value })} />
          </Field>
          <Field label="Must Not Contain" hint="Titles matching this are skipped.">
            <input className="input font-mono text-xs" value={rule.must_not_contain} onChange={(e) => set({ must_not_contain: e.target.value })} />
          </Field>
          <Toggle checked={rule.use_regex} onChange={(v) => set({ use_regex: v })} label="Use Regular Expressions" />
          <div className="grid grid-cols-2 gap-3">
            <Field label="Category">
              <select className="input" value={rule.category ?? ""} onChange={(e) => set({ category: e.target.value || null })}>
                <option value="">None</option>
                {categories.map((c) => (
                  <option key={c} value={c}>
                    {c}
                  </option>
                ))}
              </select>
            </Field>
            <Field label="Save Folder" hint="Empty: the category's folder or the default.">
              <div className="flex gap-1">
                <input className="input min-w-0 flex-1 text-xs" value={rule.save_path ?? ""} onChange={(e) => set({ save_path: e.target.value.trim() || null })} />
                <button type="button" className="btn-ghost px-2" onClick={pickFolder} aria-label="Choose folder">
                  <FolderOpen size={14} />
                </button>
              </div>
            </Field>
          </div>
          <Toggle checked={rule.start_paused} onChange={(v) => set({ start_paused: v })} label="Add Stopped" hint="Torrents containing programs are always added stopped." />
          <div>
            <span className="label">Feeds</span>
            <div className="mt-1 flex flex-wrap gap-x-4 gap-y-1">
              <label className="flex items-center gap-1.5 text-sm">
                <input type="checkbox" checked={rule.feeds.length === 0} onChange={() => set({ feeds: [] })} /> All feeds
              </label>
              {view.feeds.map((f) => (
                <label key={f.id} className="flex items-center gap-1.5 text-sm">
                  <input
                    type="checkbox"
                    checked={rule.feeds.includes(f.id)}
                    onChange={(e) => set({ feeds: e.target.checked ? [...rule.feeds, f.id] : rule.feeds.filter((x) => x !== f.id) })}
                  />
                  {f.name}
                </label>
              ))}
            </div>
          </div>
          <div className="rounded-md bg-raised p-2">
            <p className="mb-1 text-xs text-muted">
              Rules add new articles as they arrive. {problem ? "" : matches ? `${matches.length} current articles match:` : "Type something to see what it matches."}
            </p>
            {problem && <p className="text-xs text-danger">{problem}</p>}
            <ul className="max-h-40 overflow-auto text-xs">
              {matches?.map((m, i) => (
                <li key={i} className="truncate">
                  <span className="text-muted">{m.feed} · </span>
                  {m.title}
                </li>
              ))}
            </ul>
          </div>
        </div>
      </div>
    </Dialog>
  );
}

export function RssView({ categories }: { categories: string[] }) {
  const { toast, queueAdds } = useApp.getState();
  const [view, setView] = useState<View | null>(null);
  const [feed, setFeed] = useState<number | null>(null);
  const [articles, setArticles] = useState<RssArticle[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [modal, setModal] = useState<"add" | "rules" | "rename" | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(() => {
    api.rssView().then(setView).catch((e) => toast("error", errorText(e)));
  }, [toast]);
  const loadArticles = useCallback(() => {
    if (feed == null) return setArticles([]);
    api.rssArticles(feed).then(setArticles).catch(() => setArticles([]));
  }, [feed]);

  useEffect(() => {
    load();
    const un = events.rss(() => load());
    return () => {
      un.then((f) => f());
    };
  }, [load]);
  useEffect(() => {
    loadArticles();
  }, [loadArticles, view]);
  useEffect(() => {
    if (view && feed == null && view.feeds.length) setFeed(view.feeds[0].id);
  }, [view, feed]);

  const run = (p: Promise<unknown>) =>
    p.then(load).catch((e) => toast("error", errorText(e)));
  const refresh = async (ids: number[] | null) => {
    setBusy(true);
    await run(api.rssRefresh(ids));
    setBusy(false);
  };
  const markRead = (a: RssArticle) => {
    if (!a.read && feed != null) run(api.rssMarkRead(feed, a.id));
  };
  const download = (a: RssArticle) => {
    markRead(a);
    queueAdds([sourceOf(a.torrent)]);
  };
  const remove = async (id: number, name: string) => {
    if (!(await ask(`Remove the feed “${name}”? Rules stop using it.`, { title: "Remove feed", kind: "warning" }))) return;
    if (feed === id) setFeed(null);
    run(api.rssRemoveFeed(id));
  };

  if (!view) return <div className="flex h-full items-center justify-center text-sm text-muted">Loading feeds…</div>;
  const current = view.feeds.find((f) => f.id === feed);
  const article = articles.find((a) => a.id === selected);

  return (
    <div className="flex h-full">
      <aside className="flex w-64 shrink-0 flex-col border-r border-border bg-panel">
        <div className="flex items-center gap-1 border-b border-border p-2">
          <button className="btn-primary flex-1" onClick={() => setModal("add")}>
            <Plus size={15} /> Feed
          </button>
          <button className="btn-ghost" onClick={() => refresh(null)} disabled={busy || !view.feeds.length} title="Refresh all feeds">
            <RefreshCw size={15} className={busy ? "animate-spin" : undefined} />
          </button>
          <button className="btn-ghost" onClick={() => setModal("rules")} title="Download rules">
            <ListFilter size={15} />
          </button>
        </div>
        <ul className="min-h-0 flex-1 overflow-auto p-1">
          {view.feeds.map((f) => (
            <li key={f.id}>
              <button
                onClick={() => setFeed(f.id)}
                className={clsx(
                  "group flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm",
                  feed === f.id ? "bg-accent/15 text-fg" : "text-muted hover:bg-raised hover:text-fg",
                )}
                title={f.error ?? f.url}
              >
                {f.error ? <AlertTriangle size={14} className="shrink-0 text-warn" /> : <Rss size={14} className="shrink-0" />}
                <span className="flex-1 truncate">{f.name}</span>
                {f.unread > 0 && <span className="rounded-full bg-accent/20 px-1.5 text-xs tabular-nums text-fg">{f.unread}</span>}
              </button>
            </li>
          ))}
          {!view.feeds.length && <li className="p-3 text-xs text-muted">No feeds yet. Add one from a site or indexer you use.</li>}
        </ul>
        <div className="space-y-2 border-t border-border p-3">
          <Toggle
            checked={view.options.auto_download}
            onChange={(v) => run(api.rssSetOptions({ ...view.options, auto_download: v }))}
            label="Download Rules"
            hint={`${view.rules.filter((r) => r.enabled).length} enabled`}
          />
          <label className="flex items-center gap-2 text-xs text-muted">
            Refresh every
            <select
              className="input w-auto py-0.5 text-xs"
              value={view.options.refresh_minutes}
              onChange={(e) => run(api.rssSetOptions({ ...view.options, refresh_minutes: Number(e.target.value) }))}
            >
              {[5, 15, 30, 60, 120, 360, 720, 1440].map((m) => (
                <option key={m} value={m}>
                  {m < 60 ? `${m} min` : `${m / 60} h`}
                </option>
              ))}
            </select>
          </label>
        </div>
      </aside>

      <main className="flex min-w-0 flex-1 flex-col">
        {current ? (
          <>
            <div className="flex items-center gap-2 border-b border-border px-3 py-2">
              <div className="min-w-0 flex-1">
                <h2 className="truncate text-sm font-semibold">{current.name}</h2>
                <p className={clsx("truncate text-xs", current.error ? "text-warn" : "text-muted")}>
                  {current.error ?? (current.last_refresh ? `Updated ${formatDate(current.last_refresh)}` : "Not fetched yet")}
                </p>
              </div>
              <button className="btn-ghost" onClick={() => refresh([current.id])} disabled={busy} title="Refresh">
                <RefreshCw size={14} />
              </button>
              <button className="btn-ghost" onClick={() => run(api.rssMarkRead(current.id, null))} title="Mark all read">
                <CheckCheck size={14} />
              </button>
              <button className="btn-ghost" onClick={() => setModal("rename")} title="Rename">
                <Pencil size={14} />
              </button>
              <button className="btn-ghost hover:text-danger" onClick={() => remove(current.id, current.name)} title="Remove feed">
                <Trash2 size={14} />
              </button>
            </div>
            <div className="min-h-0 flex-1 overflow-auto">
              <table className="w-full text-[13px]">
                <thead className="sticky top-0 bg-panel text-left text-xs text-muted">
                  <tr>
                    <th className="px-3 py-1.5 font-medium">Title</th>
                    <th className="w-24 px-2 py-1.5 text-right font-medium">Size</th>
                    <th className="w-40 px-2 py-1.5 font-medium">Published</th>
                  </tr>
                </thead>
                <tbody>
                  {articles.map((a) => (
                    <tr
                      key={a.id}
                      onClick={() => {
                        setSelected(a.id);
                        markRead(a);
                      }}
                      onDoubleClick={() => download(a)}
                      className={clsx("cursor-default", selected === a.id ? "bg-accent/15" : "hover:bg-raised")}
                    >
                      <td className={clsx("max-w-0 truncate px-3 py-1", !a.read && "font-semibold")} title={a.title}>
                        {a.downloaded && <Download size={12} className="mr-1 inline text-ok" />}
                        {a.title}
                      </td>
                      <td className="px-2 text-right tabular-nums text-muted">{a.size ? formatBytes(a.size) : "–"}</td>
                      <td className="px-2 text-muted">{formatDate(a.date)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
              {!articles.length && <p className="p-6 text-center text-sm text-muted">No articles.</p>}
            </div>
            {article && (
              <div className="flex items-center gap-2 border-t border-border px-3 py-2">
                <span className="min-w-0 flex-1 truncate text-xs text-muted">{article.title}</span>
                {article.page && (
                  <button className="btn-ghost py-1 text-xs" onClick={() => api.openWebUrl(article.page!)}>
                    <ExternalLink size={13} /> Open Page
                  </button>
                )}
                <button className="btn-primary py-1 text-xs" onClick={() => download(article)}>
                  <Download size={13} /> Download
                </button>
              </div>
            )}
          </>
        ) : (
          <div className="flex h-full flex-col items-center justify-center gap-2 text-sm text-muted">
            <Rss size={28} />
            <p>Follow RSS feeds and let rules add new torrents automatically.</p>
          </div>
        )}
      </main>

      {modal === "add" && <AddFeedDialog onClose={() => setModal(null)} onAdded={(id) => { setFeed(id); load(); }} />}
      {modal === "rename" && current && (
        <RenameDialog
          current={current.name}
          onClose={() => setModal(null)}
          onSave={(name) => {
            setModal(null);
            run(api.rssRenameFeed(current.id, name));
          }}
        />
      )}
      {modal === "rules" && <RulesDialog view={view} categories={categories} onClose={() => setModal(null)} onChanged={load} />}
    </div>
  );
}
