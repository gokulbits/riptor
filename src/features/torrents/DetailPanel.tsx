import { AlertTriangle, Ban, ExternalLink, Eye, FileSymlink, FolderOpen, HeartHandshake, Pencil, Play, Plus, RefreshCw, Trash2 } from "lucide-react";
import { Dialog, Toggle } from "../../components/ui";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { api, errorText } from "../../lib/ipc";
import { formatBytes, formatDate, formatDuration, formatPercent, formatRatio } from "../../lib/format";
import type { FileEntry, FilePriority, Limit, PeerInfo, PieceMap, TorrentSummary } from "../../lib/types";
import { countryName, peerIp, sortPeers } from "./peers";
import { ProgressBar, StateLabel, Tabs } from "../../components/ui";
import { loadPref, savePref, useApp } from "../../stores/app";
import { PreviewDialog } from "./PreviewDialog";
import { SafetyPanel } from "./SafetyPanel";
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

/** Downloaded parts along the torrent, refreshed every few seconds. */
function Pieces({ t }: { t: TorrentSummary }) {
  const [map, setMap] = useState<PieceMap | null>(null);
  useEffect(() => {
    let alive = true;
    const load = () =>
      api.pieces(t.id, 300).then(
        (m) => alive && setMap(m),
        () => {},
      );
    load();
    const iv = setInterval(load, 3000);
    return () => {
      alive = false;
      clearInterval(iv);
    };
  }, [t.id]);
  if (!map || !map.total) return null;
  return (
    <div title={`${map.have} of ${map.total} pieces`} className="flex h-3 overflow-hidden rounded-sm bg-raised">
      {map.buckets.map((b, i) => (
        <span key={i} className="h-full flex-1 bg-accent" style={{ opacity: b / 255 }} />
      ))}
    </div>
  );
}

function General({ t }: { t: TorrentSummary }) {
  return (
    <div className="space-y-3 p-3 text-[13px]">
      <div className="flex items-center gap-3">
        <ProgressBar value={t.progress} state={t.state} />
        <span className="w-14 text-right tabular-nums">{formatPercent(t.progress)}</span>
      </div>
      <Pieces t={t} />
      {t.error && <p className="rounded-md bg-danger/10 px-2 py-1.5 text-danger">{t.error}</p>}
      {t.has_executables && (
        <p className="flex items-center gap-2 rounded-md bg-warn/10 px-2 py-1.5 text-warn">
          <AlertTriangle size={14} /> This torrent contains programs or scripts. Only run them if you trust the source.
        </p>
      )}
      {t.progress >= 1 && t.swarm_seeds != null && t.swarm_seeds <= 1 && (
        <p className="flex items-center gap-2 rounded-md bg-info/10 px-2 py-1.5 text-info">
          <HeartHandshake size={14} className="shrink-0" /> The trackers know of no other seeder: you may be the last copy. Seeding it keeps it alive.
        </p>
      )}
      <SafetyPanel t={t} />
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
        <Row k="Data checked">
          {t.scrub != null ? `Checking now… ${formatPercent(t.scrub)}` : t.last_scrub ? formatDate(t.last_scrub) : "Not yet"}
        </Row>
        <Row k="Swarm">
          {t.swarm_seeds == null ? "Not reported yet" : `${t.swarm_seeds} seeders, ${t.swarm_leechers ?? 0} downloading`}
        </Row>
      </dl>
    </div>
  );
}

/** Edits one file's path; optionally moves its whole folder. */
function RenameDialog({
  t,
  file,
  files,
  onClose,
  onDone,
}: {
  t: TorrentSummary;
  file: FileEntry;
  files: FileEntry[];
  onClose: () => void;
  onDone: () => void;
}) {
  const toast = useApp((s) => s.toast);
  const norm = (p: string) => p.replace(/\\/g, "/");
  const old = norm(file.path);
  const [path, setPath] = useState(old);
  const [busy, setBusy] = useState(false);
  const dir = (p: string) => (p.includes("/") ? p.slice(0, p.lastIndexOf("/")) : "");
  const base = (p: string) => p.slice(p.lastIndexOf("/") + 1);
  const folderChanged = dir(old) !== "" && dir(path) !== dir(old) && base(path) === base(old);
  const [wholeFolder, setWholeFolder] = useState(true);
  const save = async () => {
    const from = dir(old);
    const to = dir(path.trim());
    const changes: [number, string][] =
      folderChanged && wholeFolder
        ? files
            .filter((f) => norm(f.path).startsWith(from + "/"))
            .map((f) => [f.index, (to ? to + "/" : "") + norm(f.path).slice(from.length + 1)])
        : [[file.index, path.trim()]];
    setBusy(true);
    try {
      await api.renameFiles(t.id, changes);
      toast("success", "Renamed. Riptor is checking the files at their new place.");
      onDone();
      onClose();
    } catch (e) {
      toast("error", errorText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog
      title="Rename"
      onClose={onClose}
      width="max-w-lg"
      footer={
        <>
          <button className="btn-ghost" onClick={onClose}>
            Cancel
          </button>
          <button className="btn-primary" disabled={busy || !path.trim() || norm(path.trim()) === old} onClick={save}>
            Rename
          </button>
        </>
      }
    >
      <div className="space-y-2">
        <input className="input font-mono text-xs" value={path} onChange={(e) => setPath(e.target.value)} spellCheck={false} />
        <p className="text-xs text-muted">Use / between folders. The files move on disk and the torrent is checked again.</p>
        {folderChanged && (
          <Toggle checked={wholeFolder} onChange={setWholeFolder} label={`Rename the folder for all its files (${dir(old)})`} />
        )}
      </div>
    </Dialog>
  );
}

function Files({ t }: { t: TorrentSummary }) {
  const toast = useApp((s) => s.toast);
  const [files, setFiles] = useState<FileEntry[] | null>(null);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [dirty, setDirty] = useState(false);
  const [renaming, setRenaming] = useState<FileEntry | null>(null);
  const [previewing, setPreviewing] = useState<FileEntry | null>(null);

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
  const openable = (f: FileEntry) =>
    !f.executable && (f.done >= f.size || (f.media && f.ready >= Math.max(1, Math.min(16 * 1024 * 1024, Math.floor(f.size / 20)))));
  // Media that isn't far enough yet gets its start fetched first.
  const open = (f: FileEntry) => {
    if (openable(f)) return api.openFile(t.id, f.index).catch((e) => toast("error", errorText(e)));
    api
      .setFilePriority(t.id, [f.index], "high")
      .then(() => {
        load();
        toast("info", `Getting the start of ${f.path.split(/[\\/]/).pop()} first; play it again in a moment.`);
      })
      .catch((e) => toast("error", errorText(e)));
  };
  const setPriority = (f: FileEntry, p: FilePriority) =>
    api
      .setFilePriority(t.id, [f.index], p)
      .then(load)
      .catch((e) => toast("error", errorText(e)));
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
          <button
            className="btn-ghost py-1 text-xs"
            onClick={() => {
              setDirty(false);
            }}
          >
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
              <th className="w-28 px-2 py-1.5 font-medium">Priority</th>
              <th className="w-28 px-2 py-1.5" />
            </tr>
          </thead>
          <tbody>
            {files.map((f) => (
              <tr
                key={f.index}
                className="hover:bg-raised"
                onDoubleClick={() => api.revealFile(t.id, f.index).catch((e) => toast("error", errorText(e)))}
              >
                <td className="px-3">
                  <input
                    type="checkbox"
                    checked={selected.has(f.index)}
                    onChange={() => toggle(f.index)}
                    aria-label={`Download ${f.path}`}
                  />
                </td>
                <td className="max-w-0 truncate px-2 py-1" title={f.path}>
                  {f.executable && <AlertTriangle size={12} className="mr-1 inline text-warn" />}
                  {f.path}
                </td>
                <td className="px-2 text-right tabular-nums">{formatBytes(f.size)}</td>
                <td className="px-2">
                  <ProgressBar value={f.size ? f.done / f.size : 1} />
                </td>
                <td className="px-2">
                  <select
                    className="input py-0.5 text-xs"
                    value={f.priority}
                    disabled={!f.selected}
                    onChange={(e) => setPriority(f, e.target.value as FilePriority)}
                    aria-label={`Priority of ${f.path}`}
                  >
                    <option value="high">High</option>
                    <option value="normal">Normal</option>
                    <option value="low">Low</option>
                  </select>
                </td>
                <td className="whitespace-nowrap px-2 text-right">
                  <button
                    className="btn-ghost p-1"
                    title={
                      f.executable
                        ? "Programs are never opened from Riptor"
                        : f.done >= f.size
                          ? "Open"
                          : f.media
                            ? openable(f)
                              ? "Play while downloading"
                              : "Play: fetch the start first"
                            : "Not fully downloaded yet"
                    }
                    disabled={f.executable || (f.done < f.size && !f.media) || !f.selected}
                    onClick={() => open(f)}
                  >
                    {f.media && f.done < f.size ? <Play size={13} /> : <FileSymlink size={13} />}
                  </button>
                  {f.previewable && (
                    <button
                      className="btn-ghost p-1"
                      title={f.done >= f.size ? "Preview in Riptor" : "Not fully downloaded yet"}
                      disabled={f.done < f.size || !f.selected}
                      onClick={() => setPreviewing(f)}
                    >
                      <Eye size={13} />
                    </button>
                  )}
                  <button
                    className="btn-ghost p-1"
                    title="Show in folder"
                    onClick={() => api.revealFile(t.id, f.index).catch((e) => toast("error", errorText(e)))}
                  >
                    <FolderOpen size={13} />
                  </button>
                  <button className="btn-ghost p-1" title="Rename" onClick={() => setRenaming(f)}>
                    <Pencil size={13} />
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {renaming && <RenameDialog t={t} file={renaming} files={files} onClose={() => setRenaming(null)} onDone={load} />}
      {previewing && <PreviewDialog id={t.id} file={previewing} onClose={() => setPreviewing(null)} />}
    </div>
  );
}

function Trackers({ t }: { t: TorrentSummary }) {
  const toast = useApp((s) => s.toast);
  const [list, setList] = useState<string[] | null>(null);
  const [draft, setDraft] = useState("");
  const load = useCallback(
    () =>
      api
        .trackers(t.id)
        .then(setList)
        .catch(() => setList([])),
    [t.id],
  );
  useEffect(() => {
    load();
  }, [load]);
  const save = (next: string[]) =>
    api
      .setTrackers(t.id, next)
      .then(load)
      .catch((e) => toast("error", errorText(e)));
  const add = () => {
    const urls = draft.split(/\s+/).filter(Boolean);
    if (!urls.length || !list) return;
    setDraft("");
    save([...list, ...urls.filter((u) => !list.includes(u))]);
  };
  if (!list) return <p className="p-3 text-muted">Loading…</p>;
  return (
    <div className="text-[13px]">
      <div className="flex items-center gap-2 border-b border-border px-3 py-1.5">
        <input
          className="input flex-1 py-1 font-mono text-xs"
          placeholder="Add trackers: udp://… or https://…"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && add()}
        />
        <button className="btn-ghost py-1 text-xs" onClick={add} disabled={!draft.trim()}>
          <Plus size={13} /> Add
        </button>
        <button
          className="btn-ghost py-1 text-xs"
          onClick={() =>
            api.reannounce([t.id]).then(
              () => toast("info", "Announcing again"),
              (e) => toast("error", errorText(e)),
            )
          }
        >
          <RefreshCw size={13} /> Re-announce
        </button>
      </div>
      {list.length === 0 && <p className="p-3 text-muted">No trackers (DHT and peer exchange only).</p>}
      <ul className="divide-y divide-border">
        {list.map((u) => (
          <li key={u} className="flex items-center gap-2 px-3 py-1.5">
            <span className="flex-1 select-text truncate font-mono text-xs">{u}</span>
            {/^https?:/.test(u) && (
              <button className="btn-ghost p-1" title="Open in browser" onClick={() => api.openWebUrl(u)}>
                <ExternalLink size={13} />
              </button>
            )}
            <button className="btn-ghost p-1 hover:text-danger" title="Remove" onClick={() => save(list.filter((x) => x !== u))}>
              <Trash2 size={13} />
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

function Peers({ t }: { t: TorrentSummary }) {
  const [rows, setRows] = useState<PeerInfo[]>([]);
  useEffect(() => {
    let alive = true;
    const load = () =>
      api
        .peers(t.id)
        .then((peers) => alive && setRows(peers))
        .catch(() => alive && setRows([]));
    load();
    const iv = setInterval(load, 2000);
    return () => {
      alive = false;
      clearInterval(iv);
    };
  }, [t.id]);
  const toast = useApp((s) => s.toast);
  const [peer, setPeer] = useState("");
  const sorted = useMemo(() => sortPeers(rows), [rows]);
  const addPeer = () =>
    api.addPeer(t.id, peer).then(
      () => {
        setPeer("");
        toast("info", "Connecting to the peer");
      },
      (e) => toast("error", errorText(e)),
    );
  const countries = rows.some((p) => p.country);
  return (
    <div>
      <div className="flex items-center gap-2 border-b border-border px-3 py-1.5">
        <input
          className="input flex-1 py-1 font-mono text-xs"
          placeholder="Add a peer: 203.0.113.5:6881"
          value={peer}
          onChange={(e) => setPeer(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && peer.trim() && addPeer()}
        />
        <button className="btn-ghost py-1 text-xs" disabled={!peer.trim()} onClick={addPeer}>
          <Plus size={13} /> Add peer
        </button>
      </div>
      {sorted.length === 0 ? (
        <p className="p-3 text-muted">No connected peers.</p>
      ) : (
        <table className="w-full text-[13px]">
          <thead className="sticky top-0 bg-panel text-left text-xs text-muted">
            <tr>
              <th className="px-3 py-1.5 font-medium">Address</th>
              {countries && <th className="w-16 px-2 py-1.5 font-medium">Country</th>}
              <th className="px-2 py-1.5 font-medium">Client</th>
              <th className="px-2 py-1.5 font-medium">Conn</th>
              <th className="px-2 py-1.5 text-right font-medium">Downloaded</th>
              <th className="px-2 py-1.5 text-right font-medium">Uploaded</th>
              <th className="w-10 px-2 py-1.5" />
            </tr>
          </thead>
          <tbody>
            {sorted.map((p) => (
              <tr key={p.addr} className="hover:bg-raised">
                <td className="px-3 py-1 font-mono text-xs">{p.addr}</td>
                {countries && (
                  <td className="px-2" title={p.country ? countryName(p.country) : "Unknown"}>
                    {p.country ? <span className="rounded bg-raised px-1.5 py-0.5 font-mono text-xs">{p.country}</span> : <span className="text-muted">–</span>}
                  </td>
                )}
                <td className="max-w-0 truncate px-2" title={p.client ?? undefined}>{p.client}</td>
                <td className="px-2 text-muted">{p.conn}</td>
                <td className="px-2 text-right tabular-nums">{formatBytes(p.downloaded)}</td>
                <td className="px-2 text-right tabular-nums">{formatBytes(p.uploaded)}</td>
                <td className="px-2 text-right">
                  <button
                    className="btn-ghost p-1 hover:text-danger"
                    title="Ban this peer (all torrents)"
                    onClick={() =>
                      api.banPeer(peerIp(p.addr)).then(
                        () => toast("info", `Banned ${peerIp(p.addr)}. Unban it in Settings, Privacy & Security.`),
                        (e) => toast("error", errorText(e)),
                      )
                    }
                  >
                    <Ban size={13} />
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {countries && sorted.length > 0 && <p className="px-3 py-1.5 text-xxs text-muted">IP geolocation by DB-IP (db-ip.com)</p>}
    </div>
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
