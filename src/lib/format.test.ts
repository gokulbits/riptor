import { describe, expect, it } from "vitest";
import { formatBytes, formatDuration, formatPercent, formatRate, formatRatio } from "./format";
import { matchesStatus, visibleTorrents, useApp } from "../stores/app";
import type { TorrentSummary } from "./types";

describe("format", () => {
  it("bytes", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1536)).toBe("1.5 KiB");
    expect(formatBytes(5 * 1024 ** 3)).toBe("5.0 GiB");
    expect(formatBytes(-1)).toBe("–");
  });
  it("rates, durations, ratios, percents", () => {
    expect(formatRate(0)).toBe("–");
    expect(formatRate(2048)).toBe("2.0 KiB/s");
    expect(formatDuration(null)).toBe("∞");
    expect(formatDuration(59)).toBe("59s");
    expect(formatDuration(3661)).toBe("1h 1m");
    expect(formatRatio(9999)).toBe("∞");
    expect(formatRatio(1.234)).toBe("1.23");
    expect(formatPercent(0.9999)).toBe("99.9%");
    expect(formatPercent(1)).toBe("100%");
  });
});

const t = (over: Partial<TorrentSummary>): TorrentSummary => ({
  id: "a".repeat(40),
  name: "x",
  state: "downloading",
  forced: false,
  progress: 0.5,
  size: 100,
  done: 50,
  total_size: 100,
  dl_rate: 10,
  ul_rate: 0,
  eta: 5,
  ratio: 0,
  uploaded: 0,
  downloaded: 50,
  peers: 1,
  peers_seen: 2,
  category: null,
  tags: [],
  save_path: "/dl",
  tracker: null,
  added_on: 1,
  completed_on: null,
  seeding_time: 0,
  queue_pos: 0,
  limits: { ratio: { mode: "use_global" }, seeding_time: { mode: "use_global" }, inactive_seeding_time: { mode: "use_global" } },
  limit_action: null,
  has_executables: false,
  dl_limit_kib: null,
  ul_limit_kib: null,
  error: null,
  ...over,
});

describe("filters", () => {
  it("status filters", () => {
    expect(matchesStatus(t({}), "downloading")).toBe(true);
    expect(matchesStatus(t({ state: "stalled_up", progress: 1 }), "seeding")).toBe(true);
    expect(matchesStatus(t({ state: "stopped" }), "downloading")).toBe(false);
    expect(matchesStatus(t({ state: "error" }), "errored")).toBe(true);
    expect(matchesStatus(t({ state: "queued" }), "inactive")).toBe(true);
  });

  it("search, category and sort", () => {
    const list = [
      t({ id: "1".repeat(40), name: "Ubuntu ISO", size: 3, category: "linux" }),
      t({ id: "2".repeat(40), name: "Debian ISO", size: 9, category: "linux" }),
      t({ id: "3".repeat(40), name: "Podcast", size: 1 }),
    ];
    useApp.setState({ torrents: new Map(list.map((x) => [x.id, x])), order: list.map((x) => x.id), sort: { key: "size", desc: true } });
    expect(visibleTorrents(useApp.getState()).map((x) => x.name)).toEqual(["Debian ISO", "Ubuntu ISO", "Podcast"]);
    useApp.setState({ search: "iso", category: "linux" });
    expect(visibleTorrents(useApp.getState()).length).toBe(2);
    useApp.setState({ search: "", category: null });
    expect(visibleTorrents(useApp.getState()).map((x) => x.name)).toEqual(["Podcast"]);
  });
});
