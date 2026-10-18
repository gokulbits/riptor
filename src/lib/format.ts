const SIZE_UNITS = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];

export function formatBytes(n: number, digits = 1): string {
  if (!Number.isFinite(n) || n < 0) return "–";
  let v = n;
  let u = 0;
  while (v >= 1024 && u < SIZE_UNITS.length - 1) {
    v /= 1024;
    u++;
  }
  return u === 0 ? `${v} B` : `${v.toFixed(v >= 100 ? 0 : digits)} ${SIZE_UNITS[u]}`;
}

export function formatRate(bps: number): string {
  if (!bps) return "–";
  return `${formatBytes(bps)}/s`;
}

export function formatDuration(secs: number | null | undefined): string {
  if (secs == null || !Number.isFinite(secs)) return "∞";
  if (secs < 60) return `${Math.max(0, Math.round(secs))}s`;
  const m = Math.floor(secs / 60);
  if (m < 60) return `${m}m ${Math.round(secs % 60)}s`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ${m % 60}m`;
  const d = Math.floor(h / 24);
  if (d < 365) return `${d}d ${h % 24}h`;
  return "> 1y";
}

export function formatRatio(r: number): string {
  if (r >= 9999) return "∞";
  return r.toFixed(2);
}

export function formatPercent(p: number): string {
  if (p >= 1) return "100%";
  return `${(Math.floor(p * 1000) / 10).toFixed(1)}%`;
}

export function formatDate(unix: number | null | undefined): string {
  if (!unix) return "–";
  return new Date(unix * 1000).toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}
