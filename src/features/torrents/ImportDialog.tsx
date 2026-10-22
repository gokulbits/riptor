import { open } from "@tauri-apps/plugin-dialog";
import { CheckCircle2, FolderOpen, Loader2 } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { Dialog } from "../../components/ui";
import { formatBytes } from "../../lib/format";
import { api, errorText } from "../../lib/ipc";
import type { ImportReport, ImportScan } from "../../lib/types";

const WHERE = navigator.userAgent.includes("Windows")
  ? "%LOCALAPPDATA%\\qBittorrent\\BT_backup"
  : "~/.local/share/qBittorrent/BT_backup";

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

function Step({ n, title, children }: { n: number; title: string; children: ReactNode }) {
  return (
    <li className="flex gap-3">
      <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-accent/15 text-xs font-semibold text-accent">{n}</span>
      <div className="min-w-0 flex-1 space-y-1.5 pt-0.5">
        <p className="text-sm font-medium">{title}</p>
        {children}
      </div>
    </li>
  );
}

export function ImportDialog({ onClose }: { onClose: () => void }) {
  const [scan, setScan] = useState<ImportScan | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [report, setReport] = useState<ImportReport | null>(null);

  const look = (path: string | null) => {
    setError(null);
    setScan(null);
    api.importScan(path).then(setScan, (e) => {
      setError(errorText(e));
      setScan({ dir: path, found: 0, skipped: 0, all_time: null });
    });
  };
  useEffect(() => look(null), []);

  const choose = async () => {
    const dir = await open({ directory: true, title: "Choose qBittorrent's BT_backup folder" });
    if (typeof dir === "string") look(dir);
  };

  const start = async () => {
    if (!scan?.dir) return;
    setBusy(true);
    setError(null);
    try {
      setReport(await api.importTorrents(scan.dir));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const found = scan?.found ?? 0;
  const totals = scan?.all_time ?? null;
  const ready = found > 0 || totals !== null;
  return (
    <Dialog
      title="Import from qBittorrent"
      onClose={onClose}
      width="max-w-xl"
      footer={
        report ? (
          <button className="btn-primary" onClick={onClose}>
            Done
          </button>
        ) : (
          <>
            <button className="btn-ghost" onClick={onClose}>
              Cancel
            </button>
            <button className="btn-primary" disabled={!ready || busy} onClick={start}>
              {busy && <Loader2 size={14} className="animate-spin" />}
              {found ? `Import ${plural(found, "torrent")}` : "Import"}
            </button>
          </>
        )
      }
    >
      {report ? (
        <div className="space-y-3">
          <p className="flex items-center gap-2 text-sm font-medium">
            <CheckCircle2 size={18} className="text-ok" /> {plural(report.added, "torrent")} imported
          </p>
          <ul className="space-y-1 text-xs text-muted">
            {report.already_present > 0 && <li>{plural(report.already_present, "was", "were")} already in Riptor and left as they are.</li>}
            {report.skipped > 0 && <li>{plural(report.skipped, "entry", "entries")} had no usable torrent file or save folder.</li>}
            {report.all_time && (
              <li>
                qBittorrent's all-time totals ({formatBytes(report.all_time.downloaded)} downloaded, {formatBytes(report.all_time.uploaded)} uploaded)
                were added to Riptor's statistics.
              </li>
            )}
            {report.failed.map((f) => (
              <li key={f} className="text-danger">
                {f}
              </li>
            ))}
          </ul>
          <p className="text-xs text-muted">
            Riptor is now checking the files where they are; nothing is downloaded again. When you're happy, remove the torrents in
            qBittorrent and choose to keep the files.
          </p>
        </div>
      ) : (
        <ol className="space-y-5">
          <Step n={1} title="Stop the downloads in qBittorrent">
            <p className="text-xs text-muted">
              Pause any torrent that's still downloading, or close qBittorrent. Two apps writing the same files can damage them. Torrents
              that are only seeding can keep going.
            </p>
          </Step>
          <Step n={2} title="Find qBittorrent's torrents">
            {!scan ? (
              <p className="flex items-center gap-2 text-xs text-muted">
                <Loader2 size={13} className="animate-spin" /> Looking…
              </p>
            ) : scan.dir && ready ? (
              <div className="space-y-1">
                <p className="text-xs">
                  Found <strong>{plural(found, "torrent")}</strong> to import
                  {totals && (
                    <>
                      , and qBittorrent's all-time totals: <strong>{formatBytes(totals.downloaded)}</strong> downloaded,{" "}
                      <strong>{formatBytes(totals.uploaded)}</strong> uploaded
                    </>
                  )}
                  .
                </p>
                <p className="truncate font-mono text-xxs text-muted" title={scan.dir}>
                  {scan.dir}
                </p>
              </div>
            ) : (
              <p className="text-xs text-muted">
                {scan.dir ? "That folder has no torrents to import." : "Riptor couldn't find qBittorrent's torrents on this computer."} They're
                in the <span className="font-mono">BT_backup</span> folder, usually <span className="font-mono">{WHERE}</span>.
              </p>
            )}
            {error && <p className="text-xs text-danger">{error}</p>}
            <button className="btn-ghost border border-border text-xs" onClick={choose}>
              <FolderOpen size={13} /> {scan?.dir ? "Use a different folder…" : "Choose folder…"}
            </button>
          </Step>
          <Step n={3} title="Import">
            <p className="text-xs text-muted">
              Each torrent keeps its save folder, category, tags, downloaded and uploaded totals, seeding time and share limits. Files are
              checked where they are, not downloaded again. qBittorrent's all-time totals are added to Riptor's statistics once; importing again won't count them twice. qBittorrent itself isn't changed.
            </p>
          </Step>
        </ol>
      )}
    </Dialog>
  );
}
