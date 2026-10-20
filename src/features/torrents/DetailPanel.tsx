import { AlertTriangle, ExternalLink, FolderOpen } from "lucide-react";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { api, errorText } from "../../lib/ipc";
import { formatBytes, formatDate, formatDuration, formatPercent, formatRatio } from "../../lib/format";
import type { FileEntry, Limit, TorrentSummary } from "../../lib/types";
import { ProgressBar, StateLabel, Tabs } from "../../components/ui";
import { loadPref, savePref, useApp } from "../../stores/app";
import { SpeedGraph } from "./SpeedGraph";

const TABS = ["general", "files", "trackers", "peers", "speed"] as const;
type Tab = (typeof TABS)[number];

function limitText(l: Limit, unit = ""): string {
  if (l.mode === "use_global") return "Global";
  if (l.mode === "unlimited") return "Unlimited";
  return `${l.value}${unit}`;
}

function Row({ k, children }: { k: string; children: ReactNode }) {
  return (
    <>
      <dt className="text-muted">{k}</dt>
      <dd className="truncate tabular-nums">{children}</dd>
    </>
  );
}

function General({ t }: { t: TorrentSummary }) {
  return (
    <div className="space-y-3 p-3 text-[13px]">
      <div className="flex items-center gap-3">
        <ProgressBar value={t.progress} state={t.state} />
        <span className="w-14 text-right tabular-nums">{formatPercent(t.progress)}</span>
      </div>
      {t.error && <p className="rounded-md bg-danger/10 px-2 py-1.5 text-danger">{t.error}</p>}
      {t.has_executables && (
        <p className="flex items-center gap-2 rounded-md bg-warn/10 px-2 py-1.5 text-warn">
          <AlertTriangle size={14} /> This torrent contains programs or scripts. Only run them if you trust the source.
        </p>
      )}
      <dl className="grid grid-cols-[9rem_1fr_9rem_1fr] gap-x-4 gap-y-1.5">
        <Row k="Status">
          <StateLabel state={t.state} forced={t.forced} />
        </Row>
        <Row k="Save path">
          <span title={t.save_path}>{t.save_path}</span>
        </Row>
        <Row k="Size">{formatBytes(t.size)}</Row>
        <Row k="Info hash">
          <span className="select-text font-mono text-xs">{t.id}</span>
        </Row>
        <Row k="Downloaded">{formatBytes(t.downloaded)}</Row>
        <Row k="Uploaded">{formatBytes(t.uploaded)}</Row>
        <Row k="Ratio">{formatRatio(t.ratio)}</Row>
        <Row k="Seeding time">{formatDuration(t.seeding_time)}</Row>
        <Row k="Added">{formatDate(t.added_on)}</Row>
        <Row k="Completed">{formatDate(t.completed_on)}</Row>
        <Row k="Ratio limit">{limitText(t.limits.ratio)}</Row>
        <Row k="Seeding limit">{limitText(t.limits.seeding_time, " min")}</Row>
        <Row k="Inactive limit">{limitText(t.limits.inactive_seeding_time, " min")}</Row>
        <Row k="Peers">
          {t.peers} connected, {t.peers_seen} known
        </Row>
      </dl>
    </div>
  );
}

function Files({ t }: { t: TorrentSummary }) {
  const toast = useApp((s) => s.toast);
  const [files, setFiles] = useState<FileEntry[] | null>(null);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [dirty, setDirty] = useState(false);

  const load = useCallback(() => {
    api
      .files(t.id)
      .then((f) => {
        setFiles(f);
        if (!dirty) setSelected(new Set(f.filter((x) => x.selected).map((x) => x.index)));
      })
      .catch((e) => toast("error", errorText(e)));
  }, [t.id, dirty, toast]);

  useEffect(() => {
    setDirty(false);
    setFiles(null);
  }, [t.id]);
  useEffect(() => {
    load();
    const iv = setInterval(load, 2000);
    return () => clearInterval(iv);
  }, [load]);

  if (!files) return <p className="p-3 text-muted">Loading…</p>;
  const toggle = (i: number) => {
    const s = new Set(selected);
    if (s.has(i)) s.delete(i);
    else s.add(i);
    setSelected(s);
    setDirty(true);
  };
  const apply = async () => {
    try {
      await api.setFiles(t.id, [...selected]);
      setDirty(false);
      toast("success", "File selection updated");
    } catch (e) {
      toast("error", errorText(e));
    }
  };
  return (
    <div className="flex h-full flex-col">
      {dirty && (
        <div className="flex items-center justify-end gap-2 border-b border-border px-3 py-1.5">
          <span className="text-xs text-muted">{selected.size} files selected</span>
          <button className="btn-ghost py-1 text-xs" onClick={() => { setDirty(false); }}>
            Reset
          </button>
          <button className="btn-primary py-1 text-xs" disabled={selected.size === 0} onClick={apply}>
            Apply
          </button>
        </div>
      )}
      <div className="min-h-0 flex-1 overflow-auto">
        <table className="w-full text-[13px]">
          <thead className="sticky top-0 bg-panel text-left text-xs text-muted">
            <tr>
              <th className="w-8 px-3 py-1.5" />
              <th className="px-2 py-1.5 font-medium">Name</th>
              <th className="w-24 px-2 py-1.5 text-right font-medium">Size</th>
              <th className="w-40 px-2 py-1.5 font-medium">Progress</th>
            </tr>
          </thead>
          <tbody>
            {files.map((f) => (
              <tr key={f.index} className="hover:bg-raised">
                <td className="px-3">
                  <input type="checkbox" checked={selected.has(f.index)} onChange={() => toggle(f.index)} aria-label={`Download ${f.path}`} />
                </td>
                <td className="max-w-0 truncate px-2 py-1" title={f.path}>
                  {f.executable && <AlertTriangle size={12} className="mr-1 inline text-warn" />}
                  {f.path}
                </td>
                <td className="px-2 text-right tabular-nums">{formatBytes(f.size)}</td>
                <td className="px-2">
                  <ProgressBar value={f.size ? f.done / f.size : 1} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function Trackers({ t }: { t: TorrentSummary }) {
  const [list, setList] = useState<string[] | null>(null);
  useEffect(() => {
    api.trackers(t.id).then(setList).catch(() => setList([]));
  }, [t.id]);
  if (!list) return <p className="p-3 text-muted">Loading…</p>;
  if (list.length === 0) return <p className="p-3 text-muted">No trackers (DHT and peer exchange only).</p>;
  return (
    <ul className="divide-y divide-border text-[13px]">
      {list.map((u) => (
        <li key={u} className="flex items-center gap-2 px-3 py-1.5">
          <span className="flex-1 select-text truncate font-mono text-xs">{u}</span>
          {/^https?:/.test(u) && (
            <button className="btn-ghost p-1" title="Open in browser" onClick={() => api.openWebUrl(u)}>
              <ExternalLink size={13} />
            </button>
          )}
        </li>
      ))}
    </ul>
  );
}

interface PeerRow {
  addr: string;
  state: string;
  client: string;
  fetched: number;
  uploaded: number;
  kind: string;
}

function Peers({ t }: { t: TorrentSummary }) {
  const [rows, setRows] = useState<PeerRow[]>([]);
  useEffect(() => {
    let alive = true;
    const load = () =>
      api
        .peers(t.id)
        .then((raw) => {
          if (!alive) return;
          const peers = (raw as { peers?: Record<string, any> } | null)?.peers ?? {};
          setRows(
            Object.entries(peers).map(([addr, p]) => ({
              addr,
              state: String(p?.state ?? ""),
              client: String(p?.client_name ?? ""),
              fetched: Number(p?.counters?.fetched_bytes ?? 0),
              uploaded: Number(p?.counters?.uploaded_bytes ?? 0),
              kind: String(p?.conn_kind ?? ""),
            })),
          );
        })
        .catch(() => alive && setRows([]));
    load();
    const iv = setInterval(load, 2000);
    return () => {
      alive = false;
      clearInterval(iv);
    };
  }, [t.id]);
  const sorted = useMemo(() => [...rows].sort((a, b) => b.fetched + b.uploaded - (a.fetched + a.uploaded)), [rows]);
  if (sorted.length === 0) return <p className="p-3 text-muted">No connected peers.</p>;
  return (
    <table className="w-full text-[13px]">
      <thead className="sticky top-0 bg-panel text-left text-xs text-muted">
        <tr>
          <th className="px-3 py-1.5 font-medium">Address</th>
          <th className="px-2 py-1.5 font-medium">Client</th>
          <th className="px-2 py-1.5 font-medium">Conn</th>
          <th className="px-2 py-1.5 text-right font-medium">Downloaded</th>
          <th className="px-2 py-1.5 text-right font-medium">Uploaded</th>
        </tr>
      </thead>
      <tbody>
        {sorted.map((p) => (
          <tr key={p.addr} className="hover:bg-raised">
            <td className="px-3 py-1 font-mono text-xs">{p.addr}</td>
            <td className="max-w-0 truncate px-2">{p.client}</td>
            <td className="px-2 text-muted">{p.kind}</td>
            <td className="px-2 text-right tabular-nums">{formatBytes(p.fetched)}</td>
            <td className="px-2 text-right tabular-nums">{formatBytes(p.uploaded)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export function DetailPanel() {
  const [tab, setTabState] = useState<Tab>(() => {
    const t = loadPref("riptor.detailTab", "general");
    return TABS.includes(t as Tab) ? (t as Tab) : "general";
  });
  const setTab = (t: Tab) => {
    savePref("riptor.detailTab", t);
    setTabState(t);
  };
  const focused = useApp((s) => (s.focused ? s.torrents.get(s.focused) : undefined));
  return (
    <section className="flex h-full flex-col bg-panel" aria-label="Torrent details">
      <div className="flex items-center">
        <div className="flex-1">
          <Tabs
            value={tab}
            onChange={setTab}
            tabs={[
              { id: "general", label: "General" },
              { id: "files", label: "Files" },
              { id: "trackers", label: "Trackers" },
              { id: "peers", label: "Peers" },
              { id: "speed", label: "Speed" },
            ]}
          />
        </div>
        {focused && (
          <button className="btn-ghost mr-2 py-1 text-xs" onClick={() => api.reveal(focused.id).catch(() => {})}>
            <FolderOpen size={14} /> Open folder
          </button>
        )}
      </div>
      <div className="min-h-0 flex-1 overflow-auto">
        {tab === "speed" ? (
          <SpeedGraph />
        ) : !focused ? (
          <p className="p-3 text-sm text-muted">Select a torrent to see its details.</p>
        ) : tab === "general" ? (
          <General t={focused} />
        ) : tab === "files" ? (
          <Files t={focused} />
        ) : tab === "trackers" ? (
          <Trackers t={focused} />
        ) : (
          <Peers t={focused} />
        )}
      </div>
    </section>
  );
}
