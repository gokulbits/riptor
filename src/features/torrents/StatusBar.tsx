import clsx from "clsx";
import { ArrowDown, ArrowUp, Check, Database, Download, Gauge, HardDrive, Loader2, Network, PauseCircle, PieChart, ShieldAlert, ShieldCheck, ShieldOff } from "lucide-react";
import { useEffect, useState } from "react";
import { PublicIpView } from "../../components/PublicIp";
import { api, errorText } from "../../lib/ipc";
import { formatBytes, formatRate } from "../../lib/format";
import type { Settings } from "../../lib/types";
import { useApp } from "../../stores/app";
import { ContextMenu } from "./ContextMenu";

const PRESETS_KIB = [null, 128, 512, 1024, 2048, 5120, 10240, 25600];

/** Free space where new downloads go, checked every 30 s. */
function FreeSpace() {
  const [info, setInfo] = useState<{ dir: string; free: number } | null>(null);
  useEffect(() => {
    let alive = true;
    const load = async () => {
      try {
        const s = await api.settings();
        const dir = s.incomplete_dir ?? s.download_dir;
        const free = await api.freeSpace(dir);
        if (alive) setInfo({ dir, free });
      } catch {
        if (alive) setInfo(null);
      }
    };
    load();
    const iv = setInterval(load, 30_000);
    return () => {
      alive = false;
      clearInterval(iv);
    };
  }, []);
  if (!info) return null;
  return (
    <span className={clsx("flex items-center gap-1", info.free < 5 * 1024 ** 3 && "text-warn")} title={`Free space in ${info.dir}`}>
      <HardDrive size={12} /> {formatBytes(info.free)} free
    </span>
  );
}

/** Click a rate to pick a global limit. */
function RateButton({ dir }: { dir: "down" | "up" }) {
  const g = useApp((s) => s.global);
  const toast = useApp((s) => s.toast);
  const [menu, setMenu] = useState<{ x: number; y: number; s: Settings } | null>(null);
  const alt = !!g?.alt_speed;
  const key: keyof Settings = dir === "down" ? (alt ? "alt_dl_limit_kib" : "dl_limit_kib") : alt ? "alt_ul_limit_kib" : "ul_limit_kib";
  const open = async (e: React.MouseEvent) => {
    const r = e.currentTarget.getBoundingClientRect();
    try {
      setMenu({ x: r.left, y: r.top - 8 - 32 * (PRESETS_KIB.length + 1), s: await api.settings() });
    } catch (err) {
      toast("error", errorText(err));
    }
  };
  const apply = (s: Settings, kib: number | null) =>
    api.updateSettings({ ...s, [key]: kib }).catch((err) => toast("error", errorText(err)));
  const Icon = dir === "down" ? ArrowDown : ArrowUp;
  return (
    <>
      <button
        onClick={open}
        className="flex w-28 items-center justify-end gap-1 rounded-sm px-1 tabular-nums text-fg hover:bg-raised"
        title={`${dir === "down" ? "Download" : "Upload"} limit${alt ? " (alternative)" : ""}: click to change`}
      >
        <Icon size={12} className={dir === "down" ? "text-accent" : "text-ok"} /> {formatRate((dir === "down" ? g?.dl_rate : g?.ul_rate) ?? 0)}
      </button>
      {menu && (
        <ContextMenu
          x={menu.x}
          y={Math.max(8, menu.y)}
          onClose={() => setMenu(null)}
          items={PRESETS_KIB.map((kib) => ({
            label: kib == null ? "Unlimited" : formatRate(kib * 1024),
            icon: menu.s[key] === kib ? <Check size={14} /> : undefined,
            onClick: () => apply(menu.s, kib),
          }))}
        />
      )}
    </>
  );
}

function VpnIndicator() {
  const network = useApp((s) => s.global?.network);
  const untrusted = useApp((s) => !!s.global?.untrusted_network);
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
        : network === "blocked" && untrusted
          ? [ShieldAlert, "Untrusted network", "text-danger", "On networks you haven't trusted, torrents wait for the VPN"]
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

export function StatusBar({ onToggleAlt, onUpdate }: { onToggleAlt: () => void; onUpdate: () => void }) {
  const g = useApp((s) => s.global);
  const update = useApp((s) => s.update);
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
      {update && (
        <button className="flex items-center gap-1 rounded-sm px-1.5 py-0.5 text-accent hover:bg-raised" onClick={onUpdate} title="Install the new version">
          <Download size={12} /> Update to {update.version}
        </button>
      )}
      {g?.hold && (
        <span className="flex items-center gap-1 text-warn" title="Change this in Settings, Speed">
          <PauseCircle size={12} /> {g.hold}
        </span>
      )}
      {g?.data_cap && (
        <span
          className={clsx("flex items-center gap-1", g.data_cap.used >= g.data_cap.limit * 0.9 && "text-warn")}
          title="Data used this month, of your cap"
        >
          <PieChart size={12} /> {formatBytes(g.data_cap.used)} / {formatBytes(g.data_cap.limit)}
        </span>
      )}
      <FreeSpace />
      <button
        onClick={onToggleAlt}
        className={clsx("flex items-center gap-1 rounded-sm px-1.5 py-0.5 hover:bg-raised", g?.alt_speed && "text-warn")}
        title="Alternative speed limits"
      >
        <Gauge size={12} /> {g?.alt_speed ? "Alt speed on" : "Alt speed"}
      </button>
      <RateButton dir="down" />
      <RateButton dir="up" />
    </footer>
  );
}
