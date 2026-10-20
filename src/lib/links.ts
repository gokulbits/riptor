import type { Source } from "./types";

const HEX_HASH = /^[0-9a-f]{40}$/i;
const MAX_LINKS = 50;

/** Magnets, bare info hashes and .torrent links from pasted or dropped text. */
export function parseLinks(text: string): Source[] {
  const words = text.split(/\s+/).map((w) => w.trim().replace(/^<|>$/g, "")).filter(Boolean);
  const out: Source[] = [];
  const seen = new Set<string>();
  for (const w of words) {
    let s: Source | null = null;
    if (w.startsWith("magnet:?")) s = { kind: "magnet", value: w };
    else if (HEX_HASH.test(w)) s = { kind: "magnet", value: `magnet:?xt=urn:btih:${w.toLowerCase()}` };
    else if (/^https?:\/\//i.test(w)) {
      try {
        const u = new URL(w);
        // A lone link is taken as meant; inside other text only .torrent links count.
        if (words.length === 1 || u.pathname.toLowerCase().endsWith(".torrent")) s = { kind: "url", value: w };
      } catch {
        // not a URL
      }
    }
    if (s && !seen.has(s.value)) {
      seen.add(s.value);
      out.push(s);
    }
    if (out.length >= MAX_LINKS) break;
  }
  return out;
}
