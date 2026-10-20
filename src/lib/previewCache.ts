import type { Preview, Source } from "./types";

// Search details and the add dialog share previews.
const TTL_MS = 10 * 60 * 1000;
const MAX = 20;
const cache = new Map<string, { preview: Preview; at: number }>();

const key = (s: Source) => `${s.kind}:${s.value}`;

export function rememberPreview(source: Source, preview: Preview) {
  cache.delete(key(source));
  cache.set(key(source), { preview, at: Date.now() });
  while (cache.size > MAX) cache.delete(cache.keys().next().value!);
}

// Not removed on read: StrictMode runs the add dialog's effect twice.
export function takePreview(source: Source): Preview | null {
  const hit = cache.get(key(source));
  if (hit && Date.now() - hit.at < TTL_MS) return hit.preview;
  cache.delete(key(source));
  return null;
}
