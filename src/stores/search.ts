import { create } from "zustand";
import type { ProviderInfo, ResultCounts, SearchEvent, SearchResult } from "../lib/types";

export type ResultSort = "name" | "size" | "seeders" | "leechers" | "engine" | "pub_date";

export interface ProviderProgress {
  done: boolean;
  count: number;
  error: string | null;
}

interface SearchState {
  query: string;
  category: string;
  /** null = every enabled engine. */
  selected: string[] | null;
  searchId: number | null;
  running: boolean;
  lookups: boolean;
  results: SearchResult[];
  progress: Record<string, ProviderProgress>;
  providers: ProviderInfo[];
  sort: { key: ResultSort; desc: boolean };
  filter: { text: string; minSeeds: number; minSizeGb: number | null; maxSizeGb: number | null };

  setQuery: (q: string) => void;
  setCategory: (c: string) => void;
  setSelected: (s: string[] | null) => void;
  setProviders: (p: ProviderInfo[]) => void;
  begin: (id: number, providers: string[]) => void;
  stop: () => void;
  onEvent: (e: SearchEvent) => void;
  setSort: (k: ResultSort) => void;
  setFilter: (f: Partial<SearchState["filter"]>) => void;
}

const MAX_RESULTS = 5000;

export const useSearch = create<SearchState>()((set) => ({
  query: "",
  category: "all",
  selected: null,
  searchId: null,
  running: false,
  lookups: false,
  results: [],
  progress: {},
  providers: [],
  sort: { key: "seeders", desc: true },
  filter: { text: "", minSeeds: 0, minSizeGb: null, maxSizeGb: null },

  setQuery: (query) => set({ query }),
  setCategory: (category) => set({ category }),
  setSelected: (selected) => set({ selected }),
  setProviders: (providers) => set({ providers }),
  begin: (searchId, ids) =>
    set({
      searchId,
      running: true,
      lookups: true,
      results: [],
      progress: Object.fromEntries(ids.map((id) => [id, { done: false, count: 0, error: null }])),
    }),
  stop: () => set({ running: false, lookups: false }),
  onEvent: (e) =>
    set((s) => {
      if (e.search_id !== s.searchId) return {};
      if (e.type === "results") {
        return { results: [...s.results, ...e.results].slice(0, MAX_RESULTS) };
      }
      if (e.type === "provider_done") {
        return { progress: { ...s.progress, [e.provider]: { done: true, count: e.count, error: e.error } } };
      }
      if (e.type === "counts") return { results: applyCounts(s.results, e.counts) };
      if (e.type === "lookups_done") return { lookups: false };
      return { running: false };
    }),
  setSort: (key) => set((s) => ({ sort: { key, desc: s.sort.key === key ? !s.sort.desc : key !== "name" && key !== "engine" } })),
  setFilter: (f) => set((s) => ({ filter: { ...s.filter, ...f } })),
}));

/** Never overwrites a provider's own counts. */
export function applyCounts(results: SearchResult[], counts: ResultCounts[]): SearchResult[] {
  const byLink = new Map(counts.map((c) => [c.link, c]));
  return results.map((r) => {
    const c = byLink.get(r.link);
    if (!c || (!r.scraped && (r.seeders != null || r.leechers != null))) return r;
    return { ...r, seeders: c.seeders, leechers: c.leechers, scraped: true };
  });
}

export function visibleResults(s: SearchState): SearchResult[] {
  const t = s.filter.text.trim().toLowerCase();
  const min = s.filter.minSizeGb != null ? s.filter.minSizeGb * 1024 ** 3 : null;
  const max = s.filter.maxSizeGb != null ? s.filter.maxSizeGb * 1024 ** 3 : null;
  const list = s.results.filter(
    (r) =>
      (!t || r.name.toLowerCase().includes(t)) &&
      (s.filter.minSeeds <= 0 || (r.seeders ?? 0) >= s.filter.minSeeds) &&
      (min == null || (r.size ?? 0) >= min) &&
      (max == null || (r.size ?? Infinity) <= max),
  );
  const { key, desc } = s.sort;
  const dir = desc ? -1 : 1;
  return list.sort((a, b) => {
    const av = a[key];
    const bv = b[key];
    if (av == null && bv == null) return 0;
    if (av == null) return 1;
    if (bv == null) return -1;
    if (typeof av === "string") return av.localeCompare(bv as string) * dir;
    return ((av as number) - (bv as number)) * dir;
  });
}
