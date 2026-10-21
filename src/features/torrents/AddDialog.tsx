import { open } from "@tauri-apps/plugin-dialog";
import { AlertTriangle, FolderOpen, Loader2, Lock, Recycle } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Dialog, Field, Toggle } from "../../components/ui";
import { api, errorText } from "../../lib/ipc";
import { formatBytes } from "../../lib/format";
import { takePreview } from "../../lib/previewCache";
import type { Preview, Source } from "../../lib/types";
import { useApp } from "../../stores/app";
import { FindingsList } from "./SafetyPanel";

function describe(src: Source): string {
  if (src.kind === "file") return src.value.split(/[\\/]/).pop() ?? src.value;
  if (src.kind === "magnet") {
    const dn = new URLSearchParams(src.value.slice(src.value.indexOf("?") + 1)).get("dn");
    return dn ?? "Magnet link";
  }
  return src.value;
}

export function AddDialog({
  source,
  categories,
  onClose,
}: {
  source: Source;
  categories: Record<string, string | null>;
  onClose: () => void;
}) {
  const toast = useApp((s) => s.toast);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [savePath, setSavePath] = useState<string>("");
  const [defaultPath, setDefaultPath] = useState<string>("");
  const [category, setCategory] = useState<string>("");
  const [tags, setTags] = useState("");
  const [startPaused, setStartPaused] = useState(false);
  const [subfolder, setSubfolder] = useState(true);
  const [reuse, setReuse] = useState(true);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let alive = true;
    api.settings().then((s) => {
      if (!alive) return;
      setDefaultPath(s.download_dir);
      setSubfolder(s.create_subfolder);
      setStartPaused(s.add_stopped);
    });
    const cached = takePreview(source);
    (cached ? Promise.resolve(cached) : api.preview(source))
      .then((p) => {
        if (!alive) return;
        setPreview(p);
        setSelected(new Set(p.files.filter((f) => f.selected).map((f) => f.index)));
        if (p.findings.some((f) => f.level === "danger")) setStartPaused(true);
      })
      .catch((e) => alive && setError(errorText(e)));
    return () => {
      alive = false;
    };
  }, [source]);

  const effectivePath = savePath || (category && categories[category]) || defaultPath;
  const selectedSize = useMemo(
    () => (preview ? preview.files.filter((f) => selected.has(f.index)).reduce((n, f) => n + f.size, 0) : 0),
    [preview, selected],
  );
  const exeSelected = preview?.files.some((f) => f.executable && selected.has(f.index));
  const dupes = useMemo(() => new Set(preview?.duplicates.map((d) => d.index)), [preview]);

  const [free, setFree] = useState<number | null>(null);
  useEffect(() => {
    setFree(null);
    if (!effectivePath) return;
    let alive = true;
    const t = setTimeout(() => {
      api
        .freeSpace(effectivePath)
        .then((n) => alive && setFree(n))
        .catch(() => {});
    }, 300);
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [effectivePath]);
  const tooBig = free != null && selectedSize > free;

  const pickFolder = async () => {
    const dir = await open({ directory: true, defaultPath: effectivePath || undefined, title: "Save to" });
    if (typeof dir === "string") setSavePath(dir);
  };

  const submit = async () => {
    if (!preview) return;
    setBusy(true);
    try {
      const all = selected.size === preview.files.length;
      await api.add({
        info_hash: preview.info_hash,
        save_path: savePath || null,
        category: category || null,
        tags: tags.split(",").map((t) => t.trim()).filter(Boolean),
        start_paused: startPaused,
        only_files: all ? null : [...selected],
        create_subfolder: subfolder,
        // The warnings were on screen; "Add stopped" is the user's answer.
        allow_suspicious: true,
        reuse_existing: reuse,
      });
      toast("success", `Added ${preview.name}`);
      onClose();
    } catch (e) {
      toast("error", errorText(e));
      setBusy(false);
    }
  };

  const toggleAll = (on: boolean) => setSelected(on && preview ? new Set(preview.files.map((f) => f.index)) : new Set());

  return (
    <Dialog
      title={preview?.name ?? describe(source)}
      onClose={onClose}
      width="max-w-3xl"
      footer={
        <>
          <button className="btn-ghost" onClick={onClose}>
            Cancel
          </button>
          <button
            className="btn-primary"
            disabled={!preview || busy || selected.size === 0 || preview.already_added}
            onClick={submit}
          >
            {busy && <Loader2 size={14} className="animate-spin" />} Add torrent
          </button>
        </>
      }
    >
      {error ? (
        <p className="rounded-md bg-danger/10 p-3 text-sm text-danger">{error}</p>
      ) : !preview ? (
        <div className="flex flex-col items-center gap-2 py-10 text-muted">
          <Loader2 className="animate-spin" />
          <p className="text-sm">{source.kind === "magnet" ? "Fetching metadata from peers…" : "Reading torrent…"}</p>
        </div>
      ) : (
        <div className="space-y-4">
          {preview.already_added && (
            <p className="rounded-md bg-info/10 px-3 py-2 text-sm text-info">This torrent is already in your list.</p>
          )}
          <FindingsList findings={preview.findings} />
          {preview.duplicates.length > 0 && (
            <div className="rounded-md bg-ok/10 px-3 py-2 text-sm">
              <Toggle
                checked={reuse}
                onChange={setReuse}
                label={`Reuse ${formatBytes(preview.duplicate_bytes)} already on disk`}
                hint={`${preview.duplicates.length} file(s) match files other torrents downloaded, checked against this torrent. They're shared where that's safe and copied otherwise, so they needn't be downloaded again.`}
              />
            </div>
          )}
          {exeSelected && (
            <p className="flex items-start gap-2 rounded-md bg-warn/10 px-3 py-2 text-sm text-warn">
              <AlertTriangle size={16} className="mt-0.5 shrink-0" />
              <span>
                This torrent contains programs or scripts (marked below). They can harm your computer. Only run them if you
                trust where this torrent came from.
              </span>
            </p>
          )}
          <div className="flex flex-wrap gap-x-6 gap-y-1 text-xs text-muted">
            <span>{formatBytes(preview.total_size)} total</span>
            <span>{preview.files.length} files</span>
            <span>{formatBytes(preview.piece_length)} pieces</span>
            {preview.private && (
              <span className="flex items-center gap-1">
                <Lock size={11} /> Private (trackers only)
              </span>
            )}
            <span className="select-text font-mono">{preview.info_hash}</span>
          </div>
          {preview.comment && <p className="select-text whitespace-pre-wrap text-xs text-muted">{preview.comment}</p>}

          <div className="grid grid-cols-2 gap-3">
            <Field
              label="Save to"
              hint={
                free == null ? undefined : (
                  <span className={tooBig ? "text-danger" : undefined}>
                    {formatBytes(free)} free{tooBig && ` · not enough space for ${formatBytes(selectedSize)}`}
                  </span>
                )
              }
            >
              <div className="flex gap-1">
                <input className="input" value={effectivePath} onChange={(e) => setSavePath(e.target.value)} />
                <button className="btn-ghost border border-border" onClick={pickFolder} title="Choose folder">
                  <FolderOpen size={15} />
                </button>
              </div>
            </Field>
            <Field label="Category">
              <select className="input" value={category} onChange={(e) => setCategory(e.target.value)}>
                <option value="">None</option>
                {Object.keys(categories).map((c) => (
                  <option key={c} value={c}>
                    {c}
                  </option>
                ))}
              </select>
            </Field>
            <Field label="Tags" hint="Comma separated">
              <input className="input" value={tags} onChange={(e) => setTags(e.target.value)} placeholder="e.g. linux, iso" />
            </Field>
            <div className="space-y-1 pt-4">
              <Toggle checked={startPaused} onChange={setStartPaused} label="Add stopped" />
              {preview.multi_file && <Toggle checked={subfolder} onChange={setSubfolder} label="Create subfolder" />}
            </div>
          </div>

          <div className="card overflow-hidden">
            <div className="flex items-center gap-3 border-b border-border px-3 py-1.5 text-xs text-muted">
              <input
                type="checkbox"
                checked={selected.size === preview.files.length}
                onChange={(e) => toggleAll(e.target.checked)}
                aria-label="Select all files"
              />
              <span className="flex-1">
                {selected.size} of {preview.files.length} files · {formatBytes(selectedSize)}
              </span>
            </div>
            <ul className="max-h-64 overflow-auto text-[13px]">
              {preview.files.map((f) => (
                <li key={f.index} className="flex items-center gap-3 px-3 py-1 hover:bg-raised">
                  <input
                    type="checkbox"
                    checked={selected.has(f.index)}
                    onChange={() => {
                      const s = new Set(selected);
                      if (s.has(f.index)) s.delete(f.index);
                      else s.add(f.index);
                      setSelected(s);
                    }}
                    aria-label={`Download ${f.path}`}
                  />
                  {f.executable && <AlertTriangle size={13} className="shrink-0 text-warn" aria-label="Executable" />}
                  {reuse && dupes.has(f.index) && <Recycle size={13} className="shrink-0 text-ok" aria-label="Already on disk" />}
                  <span className="flex-1 truncate" title={f.path}>
                    {f.path}
                  </span>
                  <span className="tabular-nums text-muted">{formatBytes(f.size)}</span>
                </li>
              ))}
            </ul>
          </div>
        </div>
      )}
    </Dialog>
  );
}
