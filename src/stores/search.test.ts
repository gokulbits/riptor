import { describe, expect, it } from "vitest";
import libRs from "../../src-tauri/src/lib.rs?raw";
import isolation from "../../isolation/index.js?raw";
import type { SearchResult } from "../lib/types";
import { applyCounts, useSearch } from "./search";

const result = (link: string, seeders: number | null = null, leechers: number | null = null): SearchResult => ({
  name: link,
  link,
  size: null,
  seeders,
  leechers,
  desc_link: null,
  pub_date: null,
  engine: "archive",
  via_plugin: false,
});

describe("search results", () => {
  it("fills in scraped counts without overwriting a provider's own", () => {
    const out = applyCounts(
      [result("a"), result("b", 7, 1), result("c")],
      [
        { link: "a", seeders: 3, leechers: 2 },
        { link: "b", seeders: 99, leechers: 99 },
      ],
    );
    expect(out[0]).toMatchObject({ seeders: 3, leechers: 2, scraped: true });
    expect(out[1]).toMatchObject({ seeders: 7, leechers: 1 });
    expect(out[1].scraped).toBeUndefined();
    expect(out[2].seeders).toBeNull();
  });

  it("ignores counts from a replaced search and tracks lookups", () => {
    const s = useSearch.getState();
    s.begin(5, ["archive"]);
    s.onEvent({ type: "results", search_id: 5, provider: "archive", results: [result("x")] });
    s.onEvent({ type: "counts", search_id: 4, counts: [{ link: "x", seeders: 1, leechers: 1 }] });
    expect(useSearch.getState().results[0].seeders).toBeNull();
    s.onEvent({ type: "counts", search_id: 5, counts: [{ link: "x", seeders: 9, leechers: 2 }] });
    expect(useSearch.getState().results[0].seeders).toBe(9);
    expect(useSearch.getState().lookups).toBe(true);
    s.onEvent({ type: "lookups_done", search_id: 5 });
    expect(useSearch.getState().lookups).toBe(false);
  });
});

describe("IPC allowlist", () => {
  it("lets every registered command through the isolation hook", () => {
    const handler = libRs.slice(libRs.indexOf("generate_handler!["));
    const registered = [...handler.slice(0, handler.indexOf("]")).matchAll(/commands::(\w+)/g)].map((m) => m[1]);
    expect(registered.length).toBeGreaterThan(20);
    const missing = registered.filter((c) => !isolation.includes(`"${c}"`));
    expect(missing).toEqual([]);
  });
});
