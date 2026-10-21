import clsx from "clsx";
import { Trash2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Dialog } from "../../components/ui";
import { api, errorText } from "../../lib/ipc";
import type { Connection } from "../../lib/types";
import { useApp } from "../../stores/app";

const PURPOSE: Record<string, string> = {
  tracker: "Tracker",
  web_seed: "Web seed",
  torrent_link: ".torrent link",
  feed: "RSS feed",
  search: "Search",
  scrape: "Seed count",
  plugin: "Search plugin",
  catalog: "Plugin list",
  ip_check: "IP check",
  dns: "Name lookup",
  blocklist: "Blocklist",
  update: "Update check",
  python: "Python download",
  vpn_server: "VPN server",
};

const ROUTE: Record<Connection["route"], { label: string; cls: string }> = {
  tunnel: { label: "Through the VPN", cls: "bg-ok/15 text-ok" },
  direct: { label: "Normal connection", cls: "bg-raised text-muted" },
  local: { label: "This computer or LAN", cls: "bg-info/15 text-info" },
};

const time = (s: number) => new Date(s * 1000).toLocaleTimeString(undefined, { hour12: false });

/** Every host Riptor contacted this session. */
export function ConnectionsDialog({ onClose }: { onClose: () => void }) {
  const toast = useApp((s) => s.toast);
  const vpn = useApp((s) => s.global?.network === "vpn");
  const [rows, setRows] = useState<Connection[]>([]);
  const [filter, setFilter] = useState("");

  const load = useCallback(() => {
    api.connections().then(setRows).catch((e) => toast("error", errorText(e)));
  }, [toast]);
  useEffect(() => {
    load();
    const t = window.setInterval(load, 3000);
    return () => window.clearInterval(t);
  }, [load]);

  const shown = useMemo(() => {
    const q = filter.trim().toLowerCase();
    return q ? rows.filter((r) => r.host.includes(q) || (PURPOSE[r.purpose] ?? r.purpose).toLowerCase().includes(q)) : rows;
  }, [rows, filter]);
  const directWhileVpn = vpn && rows.some((r) => r.route === "direct" && r.purpose !== "vpn_server" && r.purpose !== "update");

  return (
    <Dialog
      title="Connections"
      onClose={onClose}
      width="max-w-4xl"
      footer={
        <>
          <span className="mr-auto self-center text-xs text-muted">
            Kept in memory only, never saved. Peers and DHT nodes aren't listed one by one.
          </span>
          <button className="btn-ghost" onClick={() => api.connectionsClear().then(load)} disabled={!rows.length}>
            <Trash2 size={14} /> Clear
          </button>
          <button className="btn-primary" onClick={onClose}>
            Close
          </button>
        </>
      }
    >
      <div className="space-y-3">
        <p className="text-sm text-muted">
          Every host Riptor contacted since it started, and whether it went through the VPN. Search plugins run as separate programs, so
          their connections are only listed while the VPN is on.
        </p>
        {directWhileVpn && (
          <p className="rounded-md bg-warn/10 px-3 py-2 text-sm text-warn">
            Some requests used the normal connection while the VPN was on. They were made before the VPN connected, or by a part of
            Riptor that can't use it (listed below).
          </p>
        )}
        <input className="input" placeholder="Filter by host or purpose" value={filter} onChange={(e) => setFilter(e.target.value)} />
        {shown.length === 0 ? (
          <p className="py-6 text-center text-sm text-muted">Nothing yet.</p>
        ) : (
          <table className="w-full table-fixed text-[13px]">
            <thead className="text-left text-xs text-muted">
              <tr>
                <th className="w-32 py-1 font-medium">Purpose</th>
                <th className="py-1 font-medium">Host</th>
                <th className="w-40 py-1 font-medium">Route</th>
                <th className="w-28 py-1 text-right font-medium">Times</th>
                <th className="w-20 py-1 text-right font-medium">Last</th>
              </tr>
            </thead>
            <tbody>
              {shown.map((r) => (
                <tr key={`${r.purpose}|${r.host}|${r.route}`} className="border-t border-border">
                  <td className="py-1 pr-2">{PURPOSE[r.purpose] ?? r.purpose}</td>
                  <td className="truncate py-1 pr-2 font-mono text-xs select-text" title={r.host}>
                    {r.host}
                  </td>
                  <td className="py-1 pr-2">
                    <span className={clsx("rounded px-1.5 py-0.5 text-xs", ROUTE[r.route].cls)}>{ROUTE[r.route].label}</span>
                  </td>
                  <td className="py-1 text-right tabular-nums">
                    {r.count}
                    {r.failures > 0 && <span className="text-muted"> ({r.failures} failed)</span>}
                  </td>
                  <td className="py-1 text-right tabular-nums text-muted">{time(r.last)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </Dialog>
  );
}
