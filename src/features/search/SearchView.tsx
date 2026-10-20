import { useVirtualizer } from "@tanstack/react-virtual";
import clsx from "clsx";
import { AlertTriangle, ArrowDown, ArrowUp, Check, Download, ExternalLink, Loader2, Search, SlidersHorizontal, Square } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { api, errorText, events } from "../../lib/ipc";
import { formatBytes } from "../../lib/format";
import type { SearchResult, Source } from "../../lib/types";
import { useApp } from "../../stores/app";
import { useSearch, visibleResults, type ResultSort } from "../../stores/search";
import { EnginesDialog } from "./EnginesDialog";

const CATEGORY_LABELS: Record<string, string> = {
  all: "All categories",
  movies: "Movies",
  tv: "TV",
  anime: "Anime",
  music: "Music",
  books: "Books",
  games: "Games",
  software: "Software",
  pictures: "Pictures",
};

const COLUMNS: { key: ResultSort; label: string; width: string; align?: "right" }[] = [
  { key: "name", label: "Name", width: "minmax(240px,1fr)" },
  { key: "size", label: "Size", width: "90px", align: "right" },
  { key: "seeders", label: "Seeds", width: "70px", align: "right" },
  { key: "leechers", label: "Leech", width: "70px", align: "right" },
  { key: "engine", label: "Engine", width: "140px" },
  { key: "pub_date", label: "Published", width: "120px" },
];
const ROW = 32;

export function useSearchEvents() {
  useEffect(() => {
    const un = events.search((e) => useSearch.getState().onEvent(e));
    return () => {
      un.then((f) => f());
    };
  }, []);
}

function EnginePicker() {
  const { providers, selected, setSelected, category } = useSearch(
    useShallow((s) => ({ providers: s.providers, selected: s.selected, setSelected: s.setSelected, category: s.category })),
  );
  const [open, setOpen] = useState(false);
  const usable = providers.filter((p) => !p.error && (category === "all" || p.categories.includes(category)));
  const active = selected ?? usable.filter((p) => p.enabled).map((p) => p.id);
  const label = selected == null ? "Enabled engines" : `${active.length} engine${active.length === 1 ? "" : "s"}`;
  return (
    <div className="relative">
      <button className="btn-ghost h-full border border-border" onClick={() => setOpen(!open)} aria-haspopup="listbox">
        <SlidersHorizontal size={14} /> {label}
      </button>
      {open && (
        <>
          <div className="fixed inset-0 z-30" onClick={() => setOpen(false)} />
          <div className="card absolute right-0 top-full z-40 mt-1 w-64 p-1 shadow-xl" role="listbox">
            <button className="w-full rounded px-2 py-1.5 text-left text-[13px] hover:bg-raised" onClick={() => setSelected(null)}>
              {selected == null && <Check size={13} className="mr-1 inline text-accent" />}All enabled engines
            </button>
            <div className="my-1 h-px bg-border" />
            {usable.map((p) => {
              const on = active.includes(p.id);
              return (
                <button
                  key={p.id}
                  role="option"
                  aria-selected={on}
                  className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-[13px] hover:bg-raised"
                  onClick={() => setSelected(on ? active.filter((x) => x !== p.id) : [...active, p.id])}
                >
                  <span className={clsx("flex h-4 w-4 items-center justify-center rounded border", on ? "border-accent bg-accent text-accent-fg" : "border-border")}>
                    {on && <Check size={11} />}
                  </span>
                  <span className="flex-1 truncate">{p.name}</span>
                </button>
              );
            })}
            {usable.length === 0 && <p className="px-2 py-1.5 text-xs text-muted">No engine supports this category.</p>}
          </div>
        </>
      )}
    </div>
  );
}

export function SearchView() {
  const toast = useApp((s) => s.toast);
  const queueAdds = useApp((s) => s.queueAdds);
  const st = useSearch(
    useShallow((s) => ({
      query: s.query,
      category: s.category,
      selected: s.selected,
      running: s.running,
      lookups: s.lookups,
      searchId: s.searchId,
      progress: s.progress,
      providers: s.providers,
      sort: s.sort,
      filter: s.filter,
      total: s.results.length,
    })),
  );
  const rows = useSearch(useShallow(visibleResults));
  const actions = useSearch.getState();
  const [engines, setEngines] = useState(false);
  const [busyLink, setBusyLink] = useState<string | null>(null);
  const parent = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const virtualizer = useVirtualizer({ count: rows.length, getScrollElement: () => parent.current, estimateSize: () => ROW, overscan: 12 });

  const loadProviders = () =>
    api
      .searchProviders()
      .then(actions.setProviders)
      .catch(() => {});
  useEffect(() => {
    loadProviders();
    input.current?.focus();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const names = useMemo(() => Object.fromEntries(st.providers.map((p) => [p.id, p.name])), [st.providers]);

  const start = async () => {
    if (st.searchId != null && st.running) await api.searchCancel(st.searchId).catch(() => {});
    try {
      const id = await api.searchStart(st.query, st.category, st.selected);
      const ids =
        st.selected ??
        st.providers.filter((p) => p.enabled && !p.error && (st.category === "all" || p.categories.includes(st.category))).map((p) => p.id);
      actions.begin(id, ids);
    } catch (e) {
      toast("error", errorText(e));
    }
  };
  const stop = () => {
    if (st.searchId != null) api.searchCancel(st.searchId).catch(() => {});
    actions.stop();
  };

  const download = async (r: SearchResult) => {
    try {
      let src: Source;
      if (r.link.startsWith("magnet:")) src = { kind: "magnet", value: r.link };
      else if (r.via_plugin) {
        setBusyLink(r.link);
        src = { kind: "file", value: await api.pluginDownload(r.engine, r.link) };
      } else src = { kind: "url", value: r.link };
      queueAdds([src]);
    } catch (e) {
      toast("error", errorText(e));
    } finally {
      setBusyLink(null);
    }
  };

  const progress = Object.entries(st.progress);
  const template = COLUMNS.map((c) => c.width).join(" ") + " 72px";

  return (
    <div className="flex h-full flex-col">
      <div className="flex shrink-0 flex-wrap items-stretch gap-2 border-b border-border bg-panel p-3">
        <div className="relative min-w-64 flex-1">
          <Search size={15} className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-muted" />
          <input
            ref={input}
            className="input h-full py-2 pl-9 text-[15px]"
            placeholder="Search torrents…"
            value={st.query}
            maxLength={200}
            onChange={(e) => actions.setQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && st.query.trim() && start()}
            aria-label="Search text"
          />
        </div>
        <select className="input w-40" value={st.category} onChange={(e) => actions.setCategory(e.target.value)} aria-label="Category">
          {Object.entries(CATEGORY_LABELS).map(([k, v]) => (
            <option key={k} value={k}>
              {v}
            </option>
          ))}
        </select>
        <EnginePicker />
        {st.running ? (
          <button className="btn-ghost border border-border" onClick={stop}>
            <Square size={14} /> Stop
          </button>
        ) : (
          <button className="btn-primary px-4" disabled={!st.query.trim()} onClick={start}>
            <Search size={14} /> Search
          </button>
        )}
        <button className="btn-ghost border border-border" onClick={() => setEngines(true)}>
          Search engines…
        </button>
      </div>

      {(progress.length > 0 || st.total > 0) && (
        <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border px-3 py-2 text-xs">
          {progress.map(([id, p]) => (
            <span
              key={id}
              title={p.error ?? undefined}
              className={clsx(
                "flex items-center gap-1 rounded-full border px-2 py-0.5",
                p.error ? "border-danger/40 text-danger" : p.done ? "border-border text-muted" : "border-accent/40 text-fg",
              )}
            >
              {!p.done ? <Loader2 size={11} className="animate-spin" /> : p.error ? <AlertTriangle size={11} /> : <Check size={11} />}
              {names[id] ?? id}
              {p.done && !p.error && <span className="tabular-nums">· {p.count}</span>}
            </span>
          ))}
          {st.lookups && st.total > 0 && (
            <span
              className="flex items-center gap-1 rounded-full border border-border px-2 py-0.5 text-muted"
              title="Asking the torrents' trackers how many people share them. Turn this off in Search engines."
            >
              <Loader2 size={11} className="animate-spin" /> Checking trackers for seeds &amp; peers
            </span>
          )}
          <span className="flex-1" />
          <input
            className="input w-48 py-1 text-xs"
            placeholder="Filter results"
            value={st.filter.text}
            onChange={(e) => actions.setFilter({ text: e.target.value })}
          />
          <label className="flex items-center gap-1 text-muted">
            Min seeds
            <input
              className="input w-16 py-1 text-xs"
              type="number"
              min={0}
              value={st.filter.minSeeds || ""}
              onChange={(e) => actions.setFilter({ minSeeds: Math.max(0, Number(e.target.value) || 0) })}
            />
          </label>
          <label className="flex items-center gap-1 text-muted">
            Size (GiB)
            <input
              className="input w-16 py-1 text-xs"
              type="number"
              min={0}
              placeholder="min"
              value={st.filter.minSizeGb ?? ""}
              onChange={(e) => actions.setFilter({ minSizeGb: e.target.value === "" ? null : Math.max(0, Number(e.target.value)) })}
            />
            –
            <input
              className="input w-16 py-1 text-xs"
              type="number"
              min={0}
              placeholder="max"
              value={st.filter.maxSizeGb ?? ""}
              onChange={(e) => actions.setFilter({ maxSizeGb: e.target.value === "" ? null : Math.max(0, Number(e.target.value)) })}
            />
          </label>
          <span className="tabular-nums text-muted">
            {rows.length} of {st.total}
          </span>
        </div>
      )}

      <div ref={parent} className="min-h-0 flex-1 overflow-auto" role="grid" aria-label="Search results">
        <div className="sticky top-0 z-10 grid border-b border-border bg-panel text-xs font-medium text-muted" style={{ gridTemplateColumns: template }}>
          {COLUMNS.map((c) => (
            <button
              key={c.key}
              onClick={() => actions.setSort(c.key)}
              className={clsx("flex items-center gap-1 px-2 py-2 hover:text-fg", c.align === "right" && "justify-end")}
            >
              {c.label}
              {st.sort.key === c.key && (st.sort.desc ? <ArrowDown size={12} /> : <ArrowUp size={12} />)}
            </button>
          ))}
          <span />
        </div>
        {rows.length === 0 ? (
          <div className="flex h-64 flex-col items-center justify-center gap-1 text-center text-muted">
            {st.running ? (
              <>
                <Loader2 className="animate-spin" />
                <p className="text-sm">Searching…</p>
              </>
            ) : st.total > 0 ? (
              <p className="text-sm">No results match the filters.</p>
            ) : (
              <>
                <p className="text-sm">Search the Internet Archive, your Torznab indexers and installed plugins.</p>
                <p className="text-xs">Double-click a result to download it.</p>
                {st.providers.length > 0 && !st.providers.some((p) => p.kind !== "builtin") && (
                  <button className="btn-ghost mt-2 border border-border text-xs" onClick={() => setEngines(true)}>
                    Add search plugins…
                  </button>
                )}
              </>
            )}
          </div>
        ) : (
          <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
            {virtualizer.getVirtualItems().map((v) => {
              const r = rows[v.index];
              return (
                <div
                  key={`${r.engine}|${r.link}`}
                  role="row"
                  onDoubleClick={() => download(r)}
                  className={clsx("group absolute left-0 grid w-full items-center text-[13px] hover:bg-raised", v.index % 2 && "bg-raised/40")}
                  style={{ top: v.start, height: ROW, gridTemplateColumns: template }}
                >
                  <span className="truncate px-2" title={r.name}>
                    {r.name}
                  </span>
                  <span className="px-2 text-right tabular-nums">{r.size != null ? formatBytes(r.size) : "–"}</span>
                  <span className="px-2 text-right tabular-nums text-ok" title={r.scraped ? "From the torrent's trackers" : undefined}>
                    {r.seeders ?? (st.lookups && !r.via_plugin ? "…" : "–")}
                  </span>
                  <span className="px-2 text-right tabular-nums text-muted" title={r.scraped ? "From the torrent's trackers" : undefined}>
                    {r.leechers ?? (st.lookups && !r.via_plugin ? "…" : "–")}
                  </span>
                  <span className="truncate px-2 text-muted">{names[r.engine] ?? r.engine}</span>
                  <span className="truncate px-2 text-muted">{r.pub_date ? new Date(r.pub_date * 1000).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" }) : "–"}</span>
                  <span className="flex justify-end gap-0.5 px-1 opacity-60 group-hover:opacity-100">
                    {r.desc_link && (
                      <button className="btn-ghost p-1" title="Open details page" onClick={() => api.openWebUrl(r.desc_link!).catch(() => {})}>
                        <ExternalLink size={13} />
                      </button>
                    )}
                    <button className="btn-ghost p-1" title="Download" onClick={() => download(r)} disabled={busyLink === r.link}>
                      {busyLink === r.link ? <Loader2 size={13} className="animate-spin" /> : <Download size={13} />}
                    </button>
                  </span>
                </div>
              );
            })}
          </div>
        )}
      </div>
      {engines && (
        <EnginesDialog
          onClose={() => {
            setEngines(false);
            loadProviders();
          }}
        />
      )}
    </div>
  );
}
