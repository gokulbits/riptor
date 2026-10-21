import { create } from "zustand";
import { api, errorText } from "../lib/ipc";
import type { ColumnPref, GlobalStats, PublicIp, Source, TorrentState, TorrentSummary, UpdateInfo, VpnStatus } from "../lib/types";

export type StatusFilter =
  | "all"
  | "downloading"
  | "seeding"
  | "completed"
  | "stopped"
  | "active"
  | "inactive"
  | "queued"
  | "errored";

const SORT_KEYS = [
  "name", "size", "progress", "state", "peers", "dl_rate", "ul_rate", "eta", "ratio", "added_on", "queue_pos", "category",
  "downloaded", "uploaded", "save_path", "tags", "completed_on", "seeding_time", "tracker",
] as const;
export type SortKey = (typeof SORT_KEYS)[number];

function loadSort(): { key: SortKey; desc: boolean } {
  try {
    const v = JSON.parse(loadPref("riptor.sort", ""));
    if (SORT_KEYS.includes(v?.key) && typeof v.desc === "boolean") return { key: v.key, desc: v.desc };
  } catch {
    // nothing saved yet
  }
  return { key: "added_on", desc: true };
}

export interface Toast {
  id: number;
  level: "info" | "warning" | "error" | "success";
  message: string;
}

const ACTIVE: TorrentState[] = ["downloading", "seeding"];

export function matchesStatus(t: TorrentSummary, f: StatusFilter): boolean {
  switch (f) {
    case "all":
      return true;
    case "downloading":
      return t.progress < 1 && !["stopped", "error"].includes(t.state);
    case "seeding":
      return t.state === "seeding" || t.state === "stalled_up";
    case "completed":
      return t.progress >= 1;
    case "stopped":
      return t.state === "stopped";
    case "active":
      return ACTIVE.includes(t.state);
    case "inactive":
      return !ACTIVE.includes(t.state);
    case "queued":
      return t.state === "queued";
    case "errored":
      return t.state === "error";
  }
}

export type Theme = string;

export function loadPref(key: string, fallback: string): string {
  try {
    return localStorage.getItem(key) ?? fallback;
  } catch {
    return fallback; // storage unavailable
  }
}

export function savePref(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* ignore */
  }
}

interface AppState {
  ready: boolean;
  engineError: string | null;
  torrents: Map<string, TorrentSummary>;
  order: string[];
  global: GlobalStats | null;
  /** Newest last. */
  history: { dl: number; ul: number }[];

  selection: Set<string>;
  anchor: string | null;
  focused: string | null;

  status: StatusFilter;
  category: string | null | undefined; // undefined = all, null = uncategorised
  tag: string | null;
  /** Tracker host; "" means torrents without trackers. */
  tracker: string | null;
  /** A newer version found by the update check. */
  update: UpdateInfo | null;
  search: string;
  sort: { key: SortKey; desc: boolean };

  view: "transfers" | "search" | "rss" | "vpn";
  /** From settings; null until loaded. */
  columnPrefs: ColumnPref[] | null;
  setColumnPrefs: (c: ColumnPref[]) => void;
  vpn: VpnStatus | null;
  setVpn: (v: VpnStatus) => void;
  publicIp: { value: PublicIp | null; error: string | null; loading: boolean };
  refreshPublicIp: () => void;
  theme: Theme;
  accent: string;
  pendingAdds: Source[];
  toasts: Toast[];

  setReady: (ready: boolean, error?: string | null) => void;
  applyTick: (torrents: TorrentSummary[], global: GlobalStats) => void;
  removeTorrent: (id: string) => void;
  select: (id: string, mode: "single" | "toggle" | "range") => void;
  selectAll: (ids: string[]) => void;
  clearSelection: () => void;
  setStatus: (s: StatusFilter) => void;
  setCategory: (c: string | null | undefined) => void;
  setTag: (t: string | null) => void;
  setTracker: (t: string | null) => void;
  setUpdate: (u: UpdateInfo | null) => void;
  setSearch: (s: string) => void;
  setSort: (key: SortKey) => void;
  setView: (v: "transfers" | "search" | "rss" | "vpn") => void;
  setTheme: (t: Theme) => void;
  setAccent: (a: string) => void;
  queueAdds: (items: Source[]) => void;
  shiftAdd: () => void;
  toast: (level: Toast["level"], message: string) => void;
  dismissToast: (id: number) => void;
}

let toastSeq = 0;

export const useApp = create<AppState>()((set, get) => ({
  ready: false,
  engineError: null,
  torrents: new Map(),
  order: [],
  global: null,
  history: [],
  selection: new Set(),
  anchor: null,
  focused: null,
  status: "all",
  category: undefined,
  tag: null,
  tracker: null,
  update: null,
  search: "",
  sort: loadSort(),
  view: "transfers",
  columnPrefs: null,
  setColumnPrefs: (columnPrefs) => set({ columnPrefs }),
  vpn: null,
  setVpn: (vpn) => set({ vpn }),
  publicIp: { value: null, error: null, loading: false },
  refreshPublicIp: () => {
    if (get().publicIp.loading) return;
    set((s) => ({ publicIp: { ...s.publicIp, loading: true } }));
    api
      .publicIp()
      .then((value) => set({ publicIp: { value, error: null, loading: false } }))
      .catch((e) => set({ publicIp: { value: null, error: errorText(e), loading: false } }));
  },
  theme: loadPref("riptor.theme", "system"),
  accent: loadPref("riptor.accent", "theme"),
  pendingAdds: [],
  toasts: [],

  setReady: (ready, error = null) => set({ ready, engineError: error }),

  applyTick: (list, global) =>
    set((s) => {
      const torrents = new Map(list.map((t) => [t.id, t]));
      const order = list.map((t) => t.id);
      const selection = new Set([...s.selection].filter((id) => torrents.has(id)));
      const history = [...s.history, { dl: global.dl_rate, ul: global.ul_rate }].slice(-300);
      return {
        torrents,
        order,
        global,
        history,
        selection,
        focused: s.focused && torrents.has(s.focused) ? s.focused : null,
      };
    }),

  removeTorrent: (id) =>
    set((s) => {
      const torrents = new Map(s.torrents);
      torrents.delete(id);
      const selection = new Set(s.selection);
      selection.delete(id);
      return { torrents, order: s.order.filter((x) => x !== id), selection, focused: s.focused === id ? null : s.focused };
    }),

  select: (id, mode) =>
    set((s) => {
      if (mode === "single") return { selection: new Set([id]), anchor: id, focused: id };
      if (mode === "toggle") {
        const selection = new Set(s.selection);
        if (selection.has(id)) selection.delete(id);
        else selection.add(id);
        return { selection, anchor: id, focused: id };
      }
      // range: the table knows the visible order
      return { focused: id };
    }),

  selectAll: (ids) => set({ selection: new Set(ids) }),
  clearSelection: () => set({ selection: new Set(), focused: null }),
  setStatus: (status) => set({ status }),
  setCategory: (category) => set({ category }),
  setTag: (tag) => set({ tag }),
  setTracker: (tracker) => set({ tracker }),
  setUpdate: (update) => set({ update }),
  setSearch: (search) => set({ search }),
  setSort: (key) =>
    set((s) => {
      const sort = { key, desc: s.sort.key === key ? !s.sort.desc : key !== "name" && key !== "queue_pos" };
      savePref("riptor.sort", JSON.stringify(sort));
      return { sort };
    }),
  setView: (view) => set({ view }),
  setTheme: (theme) => {
    savePref("riptor.theme", theme);
    set({ theme });
  },
  setAccent: (accent) => {
    savePref("riptor.accent", accent);
    set({ accent });
  },
  queueAdds: (items) => set((s) => ({ pendingAdds: [...s.pendingAdds, ...items] })),
  shiftAdd: () => set((s) => ({ pendingAdds: s.pendingAdds.slice(1) })),
  toast: (level, message) => {
    const id = ++toastSeq;
    set((s) => ({ toasts: [...s.toasts.slice(-4), { id, level, message }] }));
    setTimeout(() => get().dismissToast(id), level === "error" ? 8000 : 4000);
  },
  dismissToast: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
}));

export function visibleTorrents(s: AppState): TorrentSummary[] {
  const q = s.search.trim().toLowerCase();
  const list: TorrentSummary[] = [];
  for (const id of s.order) {
    const t = s.torrents.get(id);
    if (!t) continue;
    if (!matchesStatus(t, s.status)) continue;
    if (s.category !== undefined && t.category !== s.category) continue;
    if (s.tag && !t.tags.includes(s.tag)) continue;
    if (s.tracker !== null && (t.tracker ?? "") !== s.tracker) continue;
    if (q && !t.name.toLowerCase().includes(q)) continue;
    list.push(t);
  }
  const { key, desc } = s.sort;
  const dir = desc ? -1 : 1;
  list.sort((a, b) => {
    const pick = (t: TorrentSummary) => (Array.isArray(t[key]) ? (t[key] as string[]).join(", ") || null : t[key]) as unknown;
    const av = pick(a);
    const bv = pick(b);
    if (av == null && bv == null) return 0;
    if (av == null) return 1;
    if (bv == null) return -1;
    if (typeof av === "string" && typeof bv === "string") return av.localeCompare(bv) * dir;
    return ((av as number) - (bv as number)) * dir;
  });
  return list;
}
