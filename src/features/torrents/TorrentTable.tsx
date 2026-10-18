import { useVirtualizer } from "@tanstack/react-virtual";
import clsx from "clsx";
import { AlertTriangle, ArrowDown, ArrowUp } from "lucide-react";
import { useMemo, useRef, useState, type ReactNode } from "react";
import { formatBytes, formatDate, formatDuration, formatPercent, formatRate, formatRatio } from "../../lib/format";
import type { TorrentSummary } from "../../lib/types";
import { ProgressBar, StateLabel } from "../../components/ui";
import { useApp, visibleTorrents, type SortKey } from "../../stores/app";
import { useShallow } from "zustand/react/shallow";

interface Column {
  key: SortKey;
  label: string;
  width: number;
  align?: "right";
  render: (t: TorrentSummary) => ReactNode;
}

const COLUMNS: Column[] = [
  { key: "queue_pos", label: "#", width: 44, align: "right", render: (t) => (t.queue_pos == null ? "*" : t.queue_pos + 1) },
  {
    key: "name",
    label: "Name",
    width: 380,
    render: (t) => (
      <span className="flex min-w-0 items-center gap-1.5" title={t.name}>
        {t.has_executables && (
          <AlertTriangle size={13} className="shrink-0 text-warn" aria-label="Contains executable files" />
        )}
        <span className="truncate">{t.name}</span>
      </span>
    ),
  },
  { key: "size", label: "Size", width: 86, align: "right", render: (t) => formatBytes(t.size) },
  {
    key: "progress",
    label: "Progress",
    width: 140,
    render: (t) => (
      <span className="flex items-center gap-2">
        <ProgressBar value={t.progress} state={t.state} />
        <span className="w-11 shrink-0 text-right tabular-nums text-muted">{formatPercent(t.progress)}</span>
      </span>
    ),
  },
  { key: "state", label: "Status", width: 104, render: (t) => <StateLabel state={t.state} forced={t.forced} /> },
  { key: "peers", label: "Peers", width: 72, align: "right", render: (t) => `${t.peers} (${t.peers_seen})` },
  { key: "dl_rate", label: "Down", width: 92, align: "right", render: (t) => formatRate(t.dl_rate) },
  { key: "ul_rate", label: "Up", width: 92, align: "right", render: (t) => formatRate(t.ul_rate) },
  { key: "eta", label: "ETA", width: 80, align: "right", render: (t) => (t.progress >= 1 ? "–" : formatDuration(t.eta)) },
  { key: "ratio", label: "Ratio", width: 64, align: "right", render: (t) => formatRatio(t.ratio) },
  { key: "category", label: "Category", width: 100, render: (t) => t.category ?? "" },
  { key: "added_on", label: "Added", width: 150, render: (t) => formatDate(t.added_on) },
];

const ROW_HEIGHT = 30;

export function TorrentTable({ onContextMenu }: { onContextMenu: (e: React.MouseEvent, id: string) => void }) {
  const rows = useApp(useShallow(visibleTorrents));
  const { selection, sort, setSort, select, selectAll, anchor } = useApp(
    useShallow((s) => ({
      selection: s.selection,
      sort: s.sort,
      setSort: s.setSort,
      select: s.select,
      selectAll: s.selectAll,
      anchor: s.anchor,
    })),
  );
  const [widths, setWidths] = useState<Record<string, number>>(() => {
    try {
      return JSON.parse(localStorage.getItem("riptor.columns") ?? "{}");
    } catch {
      return {};
    }
  });
  const parent = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => parent.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 12,
  });
  const ids = useMemo(() => rows.map((r) => r.id), [rows]);
  const template = COLUMNS.map((c) => `${widths[c.key] ?? c.width}px`).join(" ");
  const totalWidth = COLUMNS.reduce((n, c) => n + (widths[c.key] ?? c.width), 0);

  const onRowClick = (e: React.MouseEvent, id: string) => {
    if (e.shiftKey && anchor) {
      const a = ids.indexOf(anchor);
      const b = ids.indexOf(id);
      if (a >= 0 && b >= 0) {
        const [lo, hi] = a < b ? [a, b] : [b, a];
        selectAll(ids.slice(lo, hi + 1));
        select(id, "range");
        return;
      }
    }
    select(id, e.ctrlKey || e.metaKey ? "toggle" : "single");
  };

  const startResize = (e: React.MouseEvent, key: string, start: number) => {
    e.preventDefault();
    e.stopPropagation();
    const x0 = e.clientX;
    const move = (ev: MouseEvent) => setWidths((w) => ({ ...w, [key]: Math.max(40, start + ev.clientX - x0) }));
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      setWidths((w) => {
        try {
          localStorage.setItem("riptor.columns", JSON.stringify(w));
        } catch {
          /* ignore */
        }
        return w;
      });
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  return (
    <div ref={parent} className="relative h-full overflow-auto" tabIndex={0} aria-label="Torrents" role="grid">
      <div style={{ minWidth: totalWidth }}>
        <div
          role="row"
          className="sticky top-0 z-10 grid border-b border-border bg-panel text-xs font-medium text-muted"
          style={{ gridTemplateColumns: template }}
        >
          {COLUMNS.map((c) => (
            <div
              key={c.key}
              role="columnheader"
              aria-sort={sort.key === c.key ? (sort.desc ? "descending" : "ascending") : "none"}
              onClick={() => setSort(c.key)}
              className={clsx(
                "relative flex cursor-pointer select-none items-center gap-1 px-2 py-2 hover:text-fg",
                c.align === "right" && "justify-end",
              )}
            >
              <span className="truncate">{c.label}</span>
              {sort.key === c.key && (sort.desc ? <ArrowDown size={12} /> : <ArrowUp size={12} />)}
              <span
                className="absolute right-0 top-1 h-[calc(100%-8px)] w-1.5 cursor-col-resize border-r border-border hover:border-accent"
                onMouseDown={(e) => startResize(e, c.key, widths[c.key] ?? c.width)}
                onClick={(e) => e.stopPropagation()}
              />
            </div>
          ))}
        </div>

        {rows.length === 0 ? (
          <div className="flex h-64 flex-col items-center justify-center gap-1 text-muted">
            <p className="text-sm">No torrents here</p>
            <p className="text-xs">Drop a .torrent file, paste a magnet link (Ctrl+V), or use Add.</p>
          </div>
        ) : (
          <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
            {virtualizer.getVirtualItems().map((v) => {
              const t = rows[v.index];
              const selected = selection.has(t.id);
              return (
                <div
                  key={t.id}
                  role="row"
                  aria-selected={selected}
                  onMouseDown={(e) => e.button === 0 && onRowClick(e, t.id)}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    if (!selected) select(t.id, "single");
                    onContextMenu(e, t.id);
                  }}
                  className={clsx(
                    "absolute left-0 grid w-full items-center text-[13px]",
                    selected ? "bg-accent/15" : v.index % 2 ? "bg-raised/40" : "",
                    !selected && "hover:bg-raised",
                  )}
                  style={{ top: v.start, height: ROW_HEIGHT, gridTemplateColumns: template }}
                >
                  {COLUMNS.map((c) => (
                    <div
                      key={c.key}
                      role="gridcell"
                      className={clsx("truncate px-2 tabular-nums", c.align === "right" && "text-right")}
                    >
                      {c.render(t)}
                    </div>
                  ))}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
