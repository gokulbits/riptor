import { ArrowDown, Database, Gauge, HardDrive, Network, Repeat, Timer } from "lucide-react";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { Dialog } from "../../components/ui";
import { api, errorText } from "../../lib/ipc";
import { formatBytes, formatDate, formatDuration, formatRate, formatRatio } from "../../lib/format";
import type { StatsReport } from "../../lib/types";
import { useApp } from "../../stores/app";

function Stat({ label, value, hint }: { label: string; value: ReactNode; hint?: string }) {
  return (
    <div className="flex items-baseline justify-between gap-3 py-1" title={hint}>
      <span className="text-muted">{label}</span>
      <span className="text-right tabular-nums">{value}</span>
    </div>
  );
}

function Card({ title, icon, children }: { title: string; icon: ReactNode; children: ReactNode }) {
  return (
    <section className="card p-3">
      <h3 className="mb-1.5 flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-muted">
        <span className="text-accent">{icon}</span>
        {title}
      </h3>
      <div className="divide-y divide-border/60 text-[13px]">{children}</div>
    </section>
  );
}

function Hero({ label, value, sub, tone }: { label: string; value: string; sub?: string; tone: string }) {
  return (
    <div className="card p-3">
      <p className="text-xs text-muted">{label}</p>
      <p className={`mt-0.5 text-2xl font-semibold tabular-nums ${tone}`}>{value}</p>
      {sub && <p className="text-xs text-muted">{sub}</p>}
    </div>
  );
}

function utcDay(offsetDays: number): string {
  const d = new Date(Date.now() - offsetDays * 86_400_000);
  return d.toISOString().slice(0, 10);
}

/** Last 30 days of upload/download as paired bars. */
function DailyChart({ daily }: { daily: StatsReport["all_time"]["daily"] }) {
  const days = useMemo(() => Array.from({ length: 30 }, (_, i) => utcDay(29 - i)), []);
  const rows = days.map((d) => ({ day: d, ...(daily[d] ?? { uploaded: 0, downloaded: 0 }) }));
  const peak = Math.max(0, ...rows.flatMap((r) => [r.uploaded, r.downloaded]));
  const max = Math.max(1, peak);
  const [hover, setHover] = useState<number | null>(null);
  const h = hover != null ? rows[hover] : null;
  return (
    <section className="card p-3">
      <div className="mb-2 flex items-center gap-4 text-xs">
        <h3 className="font-semibold uppercase tracking-wider text-muted">Last 30 days</h3>
        <span className="flex items-center gap-1.5">
          <span className="h-2 w-2 rounded-sm bg-accent" /> Downloaded
        </span>
        <span className="flex items-center gap-1.5">
          <span className="h-2 w-2 rounded-sm bg-ok" /> Uploaded
        </span>
        <span className="ml-auto tabular-nums text-muted">
          {h ? `${h.day}: ↓ ${formatBytes(h.downloaded)} · ↑ ${formatBytes(h.uploaded)}` : peak ? `peak day ${formatBytes(peak)}` : "No transfers yet"}
        </span>
      </div>
      <div className="flex h-28 items-end gap-[3px]" onMouseLeave={() => setHover(null)} role="img" aria-label="Daily transfer for the last 30 days">
        {rows.map((r, i) => (
          <div
            key={r.day}
            className={`flex h-full flex-1 items-end gap-px rounded-sm ${hover === i ? "bg-raised" : ""}`}
            onMouseEnter={() => setHover(i)}
          >
            <div className="w-1/2 rounded-t-sm bg-accent" style={{ height: `${(r.downloaded / max) * 100}%`, minHeight: r.downloaded ? 2 : 0 }} />
            <div className="w-1/2 rounded-t-sm bg-ok" style={{ height: `${(r.uploaded / max) * 100}%`, minHeight: r.uploaded ? 2 : 0 }} />
          </div>
        ))}
      </div>
      <div className="mt-1 flex justify-between text-xxs text-muted">
        <span>{days[0]}</span>
        <span>today</span>
      </div>
    </section>
  );
}

export function StatsDialog({ onClose }: { onClose: () => void }) {
  const toast = useApp((s) => s.toast);
  const [s, setS] = useState<StatsReport | null>(null);
  useEffect(() => {
    let alive = true;
    const load = () => api.statistics().then((r) => alive && setS(r)).catch((e) => alive && toast("error", errorText(e)));
    load();
    const iv = setInterval(load, 2000);
    return () => {
      alive = false;
      clearInterval(iv);
    };
  }, [toast]);

  if (!s) return null;
  const a = s.all_time;
  const hits = s.read_cache.hits + s.read_cache.misses;
  const hitRate = hits ? `${((100 * s.read_cache.hits) / hits).toFixed(1)}%` : "–";
  const wasteRate = s.session_download ? `${((100 * s.session_wasted) / s.session_download).toFixed(2)}%` : "0%";

  return (
    <Dialog title="Statistics" onClose={onClose} width="max-w-5xl">
      <div className="space-y-3">
        <div className="grid grid-cols-4 gap-3">
          <Hero label="All-time download" value={formatBytes(a.all_time_download)} sub={`${formatBytes(s.session_download)} this session`} tone="text-accent" />
          <Hero label="All-time upload" value={formatBytes(a.all_time_upload)} sub={`${formatBytes(s.session_upload)} this session`} tone="text-ok" />
          <Hero label="All-time share ratio" value={formatRatio(s.all_time_ratio)} sub={`${formatRatio(s.session_ratio)} this session`} tone="text-fg" />
          <Hero label="Current speed" value={`↓ ${formatRate(s.dl_rate)}`} sub={`↑ ${formatRate(s.ul_rate)}`} tone="text-fg" />
        </div>

        <DailyChart daily={a.daily} />

        <div className="grid grid-cols-3 gap-3">
          <Card title="Transfer" icon={<ArrowDown size={13} />}>
            <Stat label="Today" value={`↓ ${formatBytes(s.today.downloaded)} · ↑ ${formatBytes(s.today.uploaded)}`} />
            <Stat label="Last 7 days" value={`↓ ${formatBytes(s.last_7_days.downloaded)} · ↑ ${formatBytes(s.last_7_days.uploaded)}`} />
            <Stat label="Last 30 days" value={`↓ ${formatBytes(s.last_30_days.downloaded)} · ↑ ${formatBytes(s.last_30_days.uploaded)}`} />
            <Stat label="Peak download" value={formatRate(a.peak_download_rate)} />
            <Stat label="Peak upload" value={formatRate(a.peak_upload_rate)} />
            <Stat
              label="Session waste"
              value={`${formatBytes(s.session_wasted)} (${wasteRate})`}
              hint="Data received that was discarded: duplicate blocks and pieces that failed verification. Measured once a torrent finishes."
            />
            <Stat label="All-time waste" value={formatBytes(a.all_time_wasted)} />
          </Card>

          <Card title="Torrents" icon={<Repeat size={13} />}>
            <Stat label="In list" value={`${s.torrents_total} (${formatBytes(s.total_size)})`} />
            <Stat label="Downloading" value={s.torrents_downloading} />
            <Stat label="Seeding" value={s.torrents_seeding} />
            <Stat label="Completed" value={s.torrents_completed} />
            <Stat label="Queued / stopped" value={`${s.torrents_queued} / ${s.torrents_stopped}`} />
            <Stat label="Errored" value={s.torrents_errored} />
            <Stat label="Ever added / finished / removed" value={`${a.torrents_added} / ${a.torrents_completed} / ${a.torrents_removed}`} />
            <Stat label="Total seeding time" value={formatDuration(s.total_seeding_secs)} />
          </Card>

          <Card title="Network" icon={<Network size={13} />}>
            <Stat label="Connected peers" value={s.peers_connected} />
            <Stat label="Known peers" value={s.peers_known} />
            <Stat label="Listening port" value={s.listen_port ?? "–"} />
            <Stat label="Blocked (in / out)" value={`${s.blocked_incoming} / ${s.blocked_outgoing}`} hint="Connections refused by the IP blocklist" />
          </Card>

          <Card title="Read cache" icon={<Database size={13} />}>
            <Stat label="Hit rate" value={hitRate} />
            <Stat label="Hits / misses" value={`${s.read_cache.hits} / ${s.read_cache.misses}`} />
            <Stat label="Size" value={`${formatBytes(s.read_cache.bytes)} of ${formatBytes(s.read_cache.capacity)}`} />
          </Card>

          <Card title="Write buffer" icon={<HardDrive size={13} />}>
            <Stat label="Awaiting write" value={`${formatBytes(s.write_buffer_used)} of ${formatBytes(s.write_buffer_capacity)}`} />
            <Stat label="Pieces committed" value={s.disk.pieces_flushed.toLocaleString()} />
            <Stat
              label="Early flushes (overload)"
              value={s.disk.spills.toLocaleString()}
              hint="Unfinished pieces written early because the write buffer was full. Raise the buffer if this keeps climbing."
            />
            <Stat label="Average commit time" value={s.disk.avg_flush_us >= 1000 ? `${(s.disk.avg_flush_us / 1000).toFixed(1)} ms` : `${s.disk.avg_flush_us} µs`} />
          </Card>

          <Card title="Disk & uptime" icon={<Timer size={13} />}>
            <Stat label="Written this session" value={`${formatBytes(s.disk.bytes_written)} (${s.disk.disk_writes.toLocaleString()} writes)`} />
            <Stat label="Read this session" value={`${formatBytes(s.disk.bytes_read)} (${s.disk.disk_reads.toLocaleString()} reads)`} />
            <Stat label="Session uptime" value={formatDuration(s.session_uptime_secs)} />
            <Stat label="Total uptime" value={formatDuration(a.total_uptime_secs)} />
            <Stat label="Sessions" value={a.sessions} />
            <Stat label="Using Riptor since" value={formatDate(a.first_started)} />
          </Card>
        </div>
        <p className="flex items-center gap-1.5 text-xs text-muted">
          <Gauge size={12} /> All-time figures are kept when torrents are removed and are included in backups.
        </p>
      </div>
    </Dialog>
  );
}
