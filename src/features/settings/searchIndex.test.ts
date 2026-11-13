import { describe, expect, it } from "vitest";
import { ENTRIES, searchSettings, tokens } from "./searchIndex";

const raw = import.meta.glob<string>(["./*.tsx", "!./*.test.tsx", "../about/Updates.tsx"], { query: "?raw", import: "default", eager: true });
const sources = Object.values(raw).join("\n");

const found = (re: RegExp) => new Set([...sources.matchAll(re)].map((m) => m[1].trim()));
// aria-label is not a visible setting label.
const labels = found(/(?<![-\w])label="([^"]+)"/g);
const titles = found(/(?<![-\w])title="([^"]+)"/g);
const groups = new Set([...found(/<legend[^>]*>([^<{]+)<\/legend>/g), ...found(/<Group title="([^"]+)"/g), "About"]);

const top = (q: string) => searchSettings(q).map((e) => e.label);

describe("settings search", () => {
  it("finds a setting by its label, in any order and case", () => {
    expect(top("max ratio")[0]).toBe("Max ratio");
    expect(top("RATIO max")[0]).toBe("Max ratio");
  });

  it("finds a setting by a word only its keywords have", () => {
    expect(top("firewall")).toContain("Port");
    expect(top("ddns")).toContain("Update my dynamic domain name");
    expect(top("scrub")).toContain("Check finished data in the background");
  });

  it("matches the start of the word being typed, and plurals", () => {
    expect(top("encr")).toContain("Encrypt peer traffic");
    expect(top("limits")).toContain("Download");
    expect(top("proxies")).toContain("Trusted proxies");
  });

  it("needs every word to match", () => {
    expect(top("max ratio zebra")).toEqual([]);
    expect(top("")).toEqual([]);
    expect(top("   ")).toEqual([]);
    expect(top("!!")).toEqual([]);
  });

  it("puts label matches above keyword and section matches", () => {
    const r = searchSettings("upnp");
    expect(r[0].label).toBe("Forward the port with UPnP");
    expect(r.map((e) => e.label)).toContain("Use UPnP to forward the port from my router");
    // A section's name finds its settings.
    expect(searchSettings("backup").every((e) => e.section === "backup" || e.keywords.includes("backup") || e.label.toLowerCase().includes("backup"))).toBe(true);
  });

  it("only treats the last word as unfinished", () => {
    expect(top("ma ratio")).toEqual([]);
    expect(top("max rat")[0]).toBe("Max ratio");
  });

  it("caps the number of results", () => {
    expect(searchSettings("a", 5).length).toBeLessThanOrEqual(5);
  });

  it("ignores accents and symbols", () => {
    expect(tokens("Résumé, µTP!")).toEqual(["resume", "utp"]);
  });
});

describe("the index matches the settings pages", () => {
  it("reads the page sources", () => {
    expect(labels.size).toBeGreaterThan(80);
    expect(groups.size).toBeGreaterThan(20);
  });

  it("only points at labels and headings that exist", () => {
    for (const e of ENTRIES) {
      expect(groups.has(e.group), `heading "${e.group}" (${e.label})`).toBe(true);
      expect(labels.has(e.label) || groups.has(e.label) || titles.has(e.label), `label "${e.label}" (${e.group})`).toBe(true);
    }
  });

  it("has no duplicate entries", () => {
    const keys = ENTRIES.map((e) => `${e.section}|${e.group}|${e.label}`);
    expect(new Set(keys).size).toBe(keys.length);
  });

  it("covers every label on the pages", () => {
    // Status lines and buttons that aren't settings.
    const skip = new Set([
      "Profile",
      "Riptor is listed in “Open with” for torrent files",
      "This backup is password protected",
      "Repeat password",
      "Copy the key",
      "Generate a new key",
      "Remove the key",
    ]);
    const indexed = new Set(ENTRIES.map((e) => e.label));
    const missing = [...labels].filter((l) => !skip.has(l) && !indexed.has(l));
    expect(missing, "add these to searchIndex.ts").toEqual([]);
  });

  it("covers every heading", () => {
    const indexed = new Set(ENTRIES.map((e) => e.group));
    const missing = [...groups].filter((g) => !indexed.has(g));
    expect(missing, "add these to searchIndex.ts").toEqual([]);
  });
});
