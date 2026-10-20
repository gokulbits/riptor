import clsx from "clsx";
import { ArrowDown, ArrowUp, Database, Gauge, Loader2, Network, ShieldAlert, ShieldCheck, ShieldOff } from "lucide-react";
import { useEffect } from "react";
import { PublicIpView } from "../../components/PublicIp";
import { formatBytes, formatRate } from "../../lib/format";
import { useApp } from "../../stores/app";

function VpnIndicator() {
  const network = useApp((s) => s.global?.network);
  const vpn = useApp((s) => s.vpn);
  const setView = useApp((s) => s.setView);
  const refreshPublicIp = useApp((s) => s.refreshPublicIp);

  // The address changes whenever traffic moves on or off the tunnel.
  useEffect(() => {
    if (network) refreshPublicIp();
  }, [network, refreshPublicIp]);

  const busy = vpn?.phase === "connecting" || vpn?.phase === "reconnecting";
  const [Icon, label, tone, title] =
    network === "vpn"
      ? [ShieldCheck, `VPN connected${vpn?.profile_name ? ` · ${vpn.profile_name}` : ""}`, "text-ok", "Torrent traffic goes through the VPN"]
      : busy
        ? [Loader2, vpn?.phase === "reconnecting" ? "VPN reconnecting…" : "VPN connecting…", "text-warn", vpn?.message ?? ""]
        : network === "blocked"
          ? [ShieldAlert, "Kill switch on", "text-danger", "Torrents are held until the VPN connects"]
          : [ShieldOff, "VPN off", "text-muted", "Torrents use your normal connection"];

  return (
    <span className="flex items-center gap-1">
      <button className={clsx("flex items-center gap-1 rounded-sm px-1.5 py-0.5 hover:bg-raised", tone)} onClick={() => setView("vpn")} title={title}>
        <Icon size={12} className={busy ? "animate-spin" : undefined} /> {label}
      </button>
      <PublicIpView compact />
    </span>
  );
}

export function StatusBar({ onToggleAlt }: { onToggleAlt: () => void }) {
  const g = useApp((s) => s.global);
  const count = useApp((s) => s.torrents.size);
  const hitRate =
    g && g.read_cache.hits + g.read_cache.misses > 0
      ? Math.round((100 * g.read_cache.hits) / (g.read_cache.hits + g.read_cache.misses))
      : null;
  return (
    <footer className="flex h-7 shrink-0 items-center gap-4 border-t border-border bg-panel px-3 text-xs text-muted">
      <span>{count} torrents</span>
      <span className="flex items-center gap-1" title="Listening port">
        <Network size={12} /> {g?.listen_port ?? "–"}
      </span>
      {g && (
        <span
          className="flex items-center gap-1"
          title={`Disk cache: ${formatBytes(g.read_cache.bytes)} of ${formatBytes(g.read_cache.capacity)} read cache, ${formatBytes(g.write_buffer_used)} awaiting write`}
        >
          <Database size={12} /> {hitRate == null ? "–" : `${hitRate}% hits`}
        </span>
      )}
      <VpnIndicator />
      <span className="flex-1" />
      <button
        onClick={onToggleAlt}
        className={clsx("flex items-center gap-1 rounded-sm px-1.5 py-0.5 hover:bg-raised", g?.alt_speed && "text-warn")}
        title="Alternative speed limits"
      >
        <Gauge size={12} /> {g?.alt_speed ? "Alt speed on" : "Alt speed"}
      </button>
      <span className="flex w-28 items-center justify-end gap-1 tabular-nums text-fg">
        <ArrowDown size={12} className="text-accent" /> {formatRate(g?.dl_rate ?? 0)}
      </span>
      <span className="flex w-28 items-center justify-end gap-1 tabular-nums text-fg">
        <ArrowUp size={12} className="text-ok" /> {formatRate(g?.ul_rate ?? 0)}
      </span>
    </footer>
  );
}
