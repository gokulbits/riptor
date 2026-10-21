import { open, save } from "@tauri-apps/plugin-dialog";
import { File, Folder, Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { Dialog, Field, Toggle } from "../../components/ui";
import { formatBytes } from "../../lib/format";
import { api, errorText } from "../../lib/ipc";
import { useApp } from "../../stores/app";

const PIECE_SIZES = [16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536];

const lines = (s: string) =>
  s
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean);

const baseName = (p: string) => p.replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? "";
const parentOf = (p: string) => p.replace(/[\\/]+$/, "").replace(/[\\/][^\\/]*$/, "");

export function CreateTorrentDialog({ onClose }: { onClose: () => void }) {
  const toast = useApp((s) => s.toast);
  const [source, setSource] = useState("");
  const [trackers, setTrackers] = useState("");
  const [webSeeds, setWebSeeds] = useState("");
  const [comment, setComment] = useState("");
  const [isPrivate, setPrivate] = useState(false);
  const [piece, setPiece] = useState<number | null>(null);
  const [seed, setSeed] = useState(true);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<[number, number] | null>(null);

  useEffect(() => {
    if (!busy) return;
    const t = window.setInterval(() => {
      api.createProgress().then(setProgress).catch(() => {});
    }, 300);
    return () => window.clearInterval(t);
  }, [busy]);

  const pick = async (directory: boolean) => {
    const p = await open({ directory, multiple: false });
    if (typeof p === "string") setSource(p);
  };

  const create = async () => {
    const name = baseName(source);
    const output = await save({
      defaultPath: `${parentOf(source)}/${name}.torrent`,
      filters: [{ name: "Torrent files", extensions: ["torrent"] }],
    });
    if (!output) return;
    setBusy(true);
    setProgress(null);
    try {
      const id = await api.createTorrent(
        source,
        output,
        {
          trackers: lines(trackers),
          web_seeds: lines(webSeeds),
          private: isPrivate,
          comment: comment.trim() || null,
          piece_length: piece,
        },
        seed,
      );
      toast("success", id ? `Created ${name}.torrent and started seeding` : `Created ${name}.torrent`);
      onClose();
    } catch (e) {
      toast("error", errorText(e));
      setBusy(false);
    }
  };

  const pct = progress && progress[1] > 0 ? Math.floor((progress[0] / progress[1]) * 100) : 0;

  return (
    <Dialog
      title="Create torrent"
      onClose={busy ? () => {} : onClose}
      footer={
        busy ? (
          <button className="btn-ghost" onClick={() => api.cancelCreate()}>
            Cancel
          </button>
        ) : (
          <>
            <button className="btn-ghost" onClick={onClose}>
              Close
            </button>
            <button className="btn-primary" disabled={!source} onClick={create}>
              Create…
            </button>
          </>
        )
      }
    >
      <div className="space-y-4">
        <Field label="Files to share">
          <div className="flex gap-1">
            <input className="input font-mono text-xs" readOnly value={source} placeholder="Choose a file or a folder" />
            <button className="btn-ghost border border-border" disabled={busy} onClick={() => pick(false)} title="Choose a file">
              <File size={15} />
            </button>
            <button className="btn-ghost border border-border" disabled={busy} onClick={() => pick(true)} title="Choose a folder">
              <Folder size={15} />
            </button>
          </div>
        </Field>
        <Field label="Trackers" hint="One per line: http, https or udp. Leave empty to rely on DHT.">
          <textarea
            className="input h-20 resize-y font-mono text-xs"
            value={trackers}
            disabled={busy}
            onChange={(e) => setTrackers(e.target.value)}
            spellCheck={false}
          />
        </Field>
        <Field label="Web seeds" hint="Optional. HTTP servers that host the same files, one per line.">
          <textarea
            className="input h-14 resize-y font-mono text-xs"
            value={webSeeds}
            disabled={busy}
            onChange={(e) => setWebSeeds(e.target.value)}
            spellCheck={false}
          />
        </Field>
        <div className="grid grid-cols-2 gap-3">
          <Field label="Comment">
            <input className="input" value={comment} disabled={busy} onChange={(e) => setComment(e.target.value)} />
          </Field>
          <Field label="Piece size">
            <select
              className="input"
              value={piece ?? ""}
              disabled={busy}
              onChange={(e) => setPiece(e.target.value ? Number(e.target.value) : null)}
            >
              <option value="">Auto</option>
              {PIECE_SIZES.map((k) => (
                <option key={k} value={k * 1024}>
                  {formatBytes(k * 1024)}
                </option>
              ))}
            </select>
          </Field>
        </div>
        <Toggle
          checked={isPrivate}
          onChange={setPrivate}
          disabled={busy}
          label="Private torrent"
          hint="Peers only come from the trackers: no DHT, peer exchange or local discovery."
        />
        <Toggle checked={seed} onChange={setSeed} disabled={busy} label="Start seeding" hint="Adds the torrent and seeds it from where the files are." />
        {busy && (
          <div className="space-y-1">
            <div className="h-2 overflow-hidden rounded-sm bg-raised">
              <div className="h-full bg-accent transition-[width]" style={{ width: `${pct}%` }} />
            </div>
            <p className="flex items-center gap-2 text-xs text-muted">
              <Loader2 size={12} className="animate-spin" />
              {progress ? `Hashing… ${formatBytes(progress[0])} of ${formatBytes(progress[1])}` : "Reading files…"}
            </p>
          </div>
        )}
      </div>
    </Dialog>
  );
}
