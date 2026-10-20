import { useVirtualizer } from "@tanstack/react-virtual";
import clsx from "clsx";
import { AlertTriangle, ArrowDown, ArrowUp, Check, RotateCcw } from "lucide-react";
import { useMemo, useRef, useState, type ReactNode } from "react";
import { formatBytes, formatDate, formatDuration, formatPercent, formatRate, formatRatio } from "../../lib/format";
import { api, errorText } from "../../lib/ipc";
import type { ColumnPref, TorrentSummary } from "../../lib/types";
import { ProgressBar, StateLabel } from "../../components/ui";
import { useApp, visibleTorrents, type SortKey } from "../../stores/app";
import { useShallow } from "zustand/react/shallow";
import { ContextMenu, type MenuItem } from "./ContextMenu";

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
const BY_KEY = new Map<string, Column>(COLUMNS.map((c) => [c.key, c]));
const clampWidth = (w: number) => Math.min(2000, Math.max(40, Math.round(w)));

interface Slot {
  col: Column;
  visible: boolean;
  width: number;
}

/** Saved order first, then any columns added since. Name can't be hidden. */
function layout(prefs: ColumnPref[]): Slot[] {
  const out: Slot[] = [];
  const seen = new Set<string>();
  for (const p of prefs) {
    const col = BY_KEY.get(p.key);
    if (!col || seen.has(p.key)) continue;
    seen.add(p.key);
    out.push({ col, visible: p.visible || col.key === "name", width: p.width ?? col.width });
  }
  for (const col of COLUMNS) if (!seen.has(col.key)) out.push({ col, visible: true, width: col.width });
  return out;
}

const toPrefs = (slots: Slot[]): ColumnPref[] =>
  slots.map(({ col, visible, width }) => ({ key: col.key, visible, width: width === col.width ? null : clampWidth(width) }));

export async function loadColumnPrefs() {
  const s = await api.settings();
  let cols = s.columns;
  if (!cols.length) {
    // Widths used to live in localStorage.
    try {
      const old: Record<string, number> = JSON.parse(localStorage.getItem("riptor.columns") ?? "{}");
      if (Object.keys(old).length) cols = toPrefs(COLUMNS.map((col) => ({ col, visible: true, width: old[col.key] ?? col.width })));
    } catch {
      // nothing to migrate
    }
  }
  useApp.getState().setColumnPrefs(cols);
}

function saveColumnPrefs(columns: ColumnPref[]) {
  const { setColumnPrefs, toast } = useApp.getState();
  setColumnPrefs(columns);
  api
    .settings()
    .then((s) => api.updateSettings({ ...s, columns }))
    .catch((e) => toast("error", errorText(e)));
}

export function TorrentTable({
  onContextMenu,
  onOpen,
}: {
  onContextMenu: (e: React.MouseEvent, id: string) => void;
  onOpen: (id: string) => void;
}) {
  const rows = useApp(useShallow(visibleTorrents));
  const { selection, sort, setSort, select, selectAll, anchor, prefs } = useApp(
    useShallow((s) => ({
      selection: s.selection,
      sort: s.sort,
      setSort: s.setSort,
      select: s.select,
      selectAll: s.selectAll,
      anchor: s.anchor,
      prefs: s.columnPrefs,
    })),
  );
  const slots = useMemo(() => layout(prefs ?? []), [prefs]);
  const [resizing, setResizing] = useState<{ key: string; width: number } | null>(null);
  const [drag, setDrag] = useState<{ key: string; target: number } | null>(null);
  const [headerMenu, setHeaderMenu] = useState<{ x: number; y: number } | null>(null);
  const headerRef = useRef<HTMLDivElement>(null);
  const justDragged = useRef(false);

  const shown = slots
    .filter((s) => s.visible)
    .map((s) => (resizing?.key === s.col.key ? { ...s, width: resizing.width } : s));

  const parent = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => parent.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 12,
  });
  const ids = useMemo(() => rows.map((r) => r.id), [rows]);
  const template = shown.map((s) => `${s.width}px`).join(" ");
  const totalWidth = shown.reduce((n, s) => n + s.width, 0);

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
    let width = start;
    const move = (ev: MouseEvent) => {
      width = clampWidth(start + ev.clientX - x0);
      setResizing({ key, width });
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      setResizing(null);
      if (width !== start) saveColumnPrefs(toPrefs(slots.map((s) => (s.col.key === key ? { ...s, width } : s))));
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  // Native HTML5 drag-and-drop is taken over by the window's file drop on
  // Windows, so headers are dragged with plain mouse events.
  const startDrag = (e: React.MouseEvent, key: string) => {
    if (e.button !== 0) return;
    const x0 = e.clientX;
    let target: number | null = null;
    const targetAt = (x: number) => {
      const cells = [...(headerRef.current?.querySelectorAll<HTMLElement>("[role=columnheader]") ?? [])];
      const i = cells.findIndex((c) => {
        const r = c.getBoundingClientRect();
        return x < r.left + r.width / 2;
      });
      return i < 0 ? cells.length : i;
    };
    const move = (ev: MouseEvent) => {
      if (target == null && Math.abs(ev.clientX - x0) < 6) return;
      target = targetAt(ev.clientX);
      setDrag({ key, target });
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      setDrag(null);
      if (target == null) return;
      justDragged.current = true;
      const from = shown.findIndex((s) => s.col.key === key);
      if (target === from || target === from + 1) return;
      // Place before the visible column now at `target`; hidden ones keep their spots.
      const moving = slots.find((s) => s.col.key === key)!;
      const rest = slots.filter((s) => s.col.key !== key);
      const before = shown[target]?.col.key;
      const at = before ? rest.findIndex((s) => s.col.key === before) : rest.length;
      rest.splice(at, 0, moving);
      saveColumnPrefs(toPrefs(rest));
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  const headerItems = (): (MenuItem | "sep")[] => [
    ...slots.map((s) => ({
      label: s.col.label === "#" ? "Queue position" : s.col.label,
      icon: s.visible ? <Check size={14} /> : undefined,
      disabled: s.col.key === "name",
      keepOpen: true,
      onClick: () => saveColumnPrefs(toPrefs(slots.map((x) => (x === s ? { ...x, visible: !x.visible } : x)))),
    })),
    "sep",
    { label: "Reset columns", icon: <RotateCcw size={14} />, onClick: () => saveColumnPrefs([]) },
  ];

  return (
    <div ref={parent} className="relative h-full overflow-auto" tabIndex={0} aria-label="Torrents" role="grid">
      <div style={{ minWidth: totalWidth }}>
        <div
          ref={headerRef}
          role="row"
          className="sticky top-0 z-10 grid border-b border-border bg-panel text-xs font-medium text-muted"
          style={{ gridTemplateColumns: template }}
          onContextMenu={(e) => {
            e.preventDefault();
            setHeaderMenu({ x: e.clientX, y: e.clientY });
          }}
          title="Drag to reorder. Right-click to show or hide columns."
        >
          {shown.map((s, i) => {
            const c = s.col;
            return (
              <div
                key={c.key}
                role="columnheader"
                aria-sort={sort.key === c.key ? (sort.desc ? "descending" : "ascending") : "none"}
                onMouseDown={(e) => startDrag(e, c.key)}
                onClick={() => {
                  if (justDragged.current) {
                    justDragged.current = false;
                    return;
                  }
                  setSort(c.key);
                }}
                className={clsx(
                  "relative flex cursor-pointer select-none items-center gap-1 px-2 py-2 hover:text-fg",
                  c.align === "right" && "justify-end",
                  drag?.key === c.key && "opacity-50",
                  drag && drag.target === i && "shadow-[inset_2px_0_0_rgb(var(--accent))]",
                  drag && drag.target === shown.length && i === shown.length - 1 && "shadow-[inset_-2px_0_0_rgb(var(--accent))]",
                )}
              >
                <span className="truncate">{c.label}</span>
                {sort.key === c.key && (sort.desc ? <ArrowDown size={12} /> : <ArrowUp size={12} />)}
                <span
                  className="absolute right-0 top-1 h-[calc(100%-8px)] w-1.5 cursor-col-resize border-r border-border hover:border-accent"
                  onMouseDown={(e) => startResize(e, c.key, s.width)}
                  onClick={(e) => e.stopPropagation()}
                />
              </div>
            );
          })}
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
                  onDoubleClick={() => onOpen(t.id)}
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
                  {shown.map(({ col: c }) => (
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
      {headerMenu && <ContextMenu x={headerMenu.x} y={headerMenu.y} items={headerItems()} onClose={() => setHeaderMenu(null)} />}
    </div>
  );
}
