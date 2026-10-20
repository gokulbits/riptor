import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen, X } from "lucide-react";
import { useState } from "react";
import { Dialog, Field, OptionalNumber, Toggle } from "../../components/ui";
import { api, errorText } from "../../lib/ipc";
import type { Limit, ShareLimitAction, ShareLimits, Source, TorrentSummary } from "../../lib/types";
import { useShallow } from "zustand/react/shallow";
import { useApp } from "../../stores/app";

export function MagnetDialog({ onSubmit, onClose }: { onSubmit: (items: Source[]) => void; onClose: () => void }) {
  const [text, setText] = useState("");
  const items: Source[] = text
    .split(/\s+/)
    .map((s) => s.trim())
    .filter(Boolean)
    .map((s) => (s.startsWith("magnet:") ? { kind: "magnet", value: s } : { kind: "url", value: s }));
  const valid = items.length > 0 && items.every((i) => (i.kind === "magnet" ? /^magnet:\?/.test(i.value) : /^https?:\/\//.test(i.value)));
  return (
    <Dialog
      title="Add magnet links or URLs"
      onClose={onClose}
      width="max-w-xl"
      footer={
        <>
          <button className="btn-ghost" onClick={onClose}>
            Cancel
          </button>
          <button className="btn-primary" disabled={!valid} onClick={() => (onSubmit(items), onClose())}>
            Continue
          </button>
        </>
      }
    >
      <Field label="One per line" hint="magnet: links or http(s) links to .torrent files">
        <textarea
          className="input h-32 resize-none font-mono text-xs"
          value={text}
          onChange={(e) => setText(e.target.value)}
          spellCheck={false}
          placeholder="magnet:?xt=urn:btih:…"
        />
      </Field>
    </Dialog>
  );
}

export function RemoveDialog({ ids, onClose }: { ids: string[]; onClose: () => void }) {
  const toast = useApp((s) => s.toast);
  // The selector returns a new array each time.
  const names = useApp(useShallow((s) => ids.map((id) => s.torrents.get(id)?.name).filter(Boolean) as string[]));
  const [deleteFiles, setDeleteFiles] = useState(false);
  const [busy, setBusy] = useState(false);
  const go = async () => {
    setBusy(true);
    try {
      await api.remove(ids, deleteFiles);
      onClose();
    } catch (e) {
      toast("error", errorText(e));
      setBusy(false);
    }
  };
  return (
    <Dialog
      title={ids.length === 1 ? "Remove torrent" : `Remove ${ids.length} torrents`}
      onClose={onClose}
      width="max-w-md"
      footer={
        <>
          <button className="btn-ghost" onClick={onClose}>
            Cancel
          </button>
          <button className="btn-danger" disabled={busy} onClick={go}>
            {deleteFiles ? "Remove and delete files" : "Remove"}
          </button>
        </>
      }
    >
      <ul className="mb-3 max-h-32 list-inside list-disc overflow-auto text-sm">
        {names.slice(0, 8).map((n) => (
          <li key={n} className="truncate">
            {n}
          </li>
        ))}
        {names.length > 8 && <li className="text-muted">and {names.length - 8} more</li>}
      </ul>
      <Toggle
        checked={deleteFiles}
        onChange={setDeleteFiles}
        label="Also delete downloaded files"
        hint="Files are deleted permanently, not moved to the recycle bin."
      />
    </Dialog>
  );
}

export function CategoryDialog({ onClose, onSaved }: { onClose: () => void; onSaved: () => void }) {
  const toast = useApp((s) => s.toast);
  const [name, setName] = useState("");
  const [path, setPath] = useState("");
  const save = async () => {
    try {
      await api.saveCategory(name.trim(), path.trim() || null);
      onSaved();
      onClose();
    } catch (e) {
      toast("error", errorText(e));
    }
  };
  return (
    <Dialog
      title="New category"
      onClose={onClose}
      width="max-w-md"
      footer={
        <>
          <button className="btn-ghost" onClick={onClose}>
            Cancel
          </button>
          <button className="btn-primary" disabled={!name.trim()} onClick={save}>
            Save
          </button>
        </>
      }
    >
      <div className="space-y-3">
        <Field label="Name">
          <input className="input" value={name} maxLength={64} onChange={(e) => setName(e.target.value)} />
        </Field>
        <Field label="Save folder" hint="Optional. Torrents in this category go here by default.">
          <div className="flex gap-1">
            <input className="input" value={path} onChange={(e) => setPath(e.target.value)} />
            <button
              className="btn-ghost border border-border"
              onClick={async () => {
                const d = await open({ directory: true });
                if (typeof d === "string") setPath(d);
              }}
            >
              <FolderOpen size={15} />
            </button>
          </div>
        </Field>
      </div>
    </Dialog>
  );
}

function LimitEditor({ label, unit, value, onChange }: { label: string; unit: string; value: Limit; onChange: (l: Limit) => void }) {
  return (
    <div className="grid grid-cols-[10rem_1fr_7rem] items-center gap-2">
      <span className="text-sm">{label}</span>
      <select
        className="input"
        value={value.mode}
        onChange={(e) => {
          const mode = e.target.value as Limit["mode"];
          onChange(mode === "value" ? { mode, value: value.mode === "value" ? value.value : 1 } : { mode });
        }}
      >
        <option value="use_global">Use global limit</option>
        <option value="unlimited">No limit</option>
        <option value="value">Custom</option>
      </select>
      {value.mode === "value" ? (
        <div className="flex items-center gap-1">
          <OptionalNumber value={value.value} onChange={(v) => onChange({ mode: "value", value: v ?? 0 })} step={unit ? 1 : 0.1} />
          <span className="text-xs text-muted">{unit}</span>
        </div>
      ) : (
        <span />
      )}
    </div>
  );
}

export function ShareLimitsDialog({ torrents, onClose }: { torrents: TorrentSummary[]; onClose: () => void }) {
  const toast = useApp((s) => s.toast);
  const first = torrents[0];
  const [limits, setLimits] = useState<ShareLimits>(first.limits);
  const [action, setAction] = useState<ShareLimitAction | "">(first.limit_action ?? "");
  const save = async () => {
    try {
      await api.setShareLimits(torrents.map((t) => t.id), limits, action || null);
      onClose();
    } catch (e) {
      toast("error", errorText(e));
    }
  };
  return (
    <Dialog
      title={torrents.length === 1 ? `Share limits · ${first.name}` : `Share limits · ${torrents.length} torrents`}
      onClose={onClose}
      width="max-w-xl"
      footer={
        <>
          <button className="btn-ghost" onClick={onClose}>
            Cancel
          </button>
          <button className="btn-primary" onClick={save}>
            Save
          </button>
        </>
      }
    >
      <div className="space-y-3">
        <LimitEditor label="Ratio" unit="" value={limits.ratio} onChange={(ratio) => setLimits({ ...limits, ratio })} />
        <LimitEditor label="Seeding time" unit="min" value={limits.seeding_time} onChange={(seeding_time) => setLimits({ ...limits, seeding_time })} />
        <LimitEditor
          label="Inactive seeding"
          unit="min"
          value={limits.inactive_seeding_time}
          onChange={(inactive_seeding_time) => setLimits({ ...limits, inactive_seeding_time })}
        />
        <div className="grid grid-cols-[10rem_1fr] items-center gap-2">
          <span className="text-sm">When reached</span>
          <select className="input" value={action} onChange={(e) => setAction(e.target.value as ShareLimitAction | "")}>
            <option value="">Use global action</option>
            <option value="stop">Stop torrent</option>
            <option value="remove">Remove torrent</option>
            <option value="remove_with_files">Remove torrent and its files</option>
          </select>
        </div>
      </div>
    </Dialog>
  );
}

export function TagsDialog({ torrents, onClose }: { torrents: TorrentSummary[]; onClose: () => void }) {
  const toast = useApp((s) => s.toast);
  const known = useApp(useShallow((s) => [...new Set([...s.torrents.values()].flatMap((t) => t.tags))].sort()));
  const [tags, setTags] = useState<string[]>(() => [...new Set(torrents.flatMap((t) => t.tags))]);
  const [draft, setDraft] = useState("");
  const add = (raw: string) => {
    const t = raw.trim();
    if (t && !tags.includes(t)) setTags([...tags, t]);
    setDraft("");
  };
  const save = async () => {
    const all = draft.trim() && !tags.includes(draft.trim()) ? [...tags, draft.trim()] : tags;
    try {
      await api.setTags(
        torrents.map((t) => t.id),
        all,
      );
      onClose();
    } catch (e) {
      toast("error", errorText(e));
    }
  };
  const suggestions = known.filter((k) => !tags.includes(k));
  return (
    <Dialog
      title={torrents.length === 1 ? `Tags for ${torrents[0].name}` : `Tags for ${torrents.length} torrents`}
      onClose={onClose}
      width="max-w-md"
      footer={
        <>
          <button className="btn-ghost" onClick={onClose}>
            Cancel
          </button>
          <button className="btn-primary" onClick={save}>
            Save
          </button>
        </>
      }
    >
      <div className="space-y-3">
        <div className="flex min-h-9 flex-wrap gap-1.5 rounded-md border border-border p-1.5">
          {tags.map((t) => (
            <span key={t} className="flex items-center gap-1 rounded-sm bg-accent/15 px-2 py-0.5 text-xs">
              {t}
              <button className="text-muted hover:text-fg" onClick={() => setTags(tags.filter((x) => x !== t))} aria-label={`Remove ${t}`}>
                <X size={12} />
              </button>
            </span>
          ))}
          <input
            className="min-w-24 flex-1 bg-transparent px-1 text-sm outline-hidden"
            value={draft}
            maxLength={64}
            placeholder={tags.length ? "" : "Type a tag and press Enter"}
            onChange={(e) => setDraft(e.target.value.replace(",", ""))}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === ",") {
                e.preventDefault();
                add(draft);
              } else if (e.key === "Backspace" && !draft && tags.length) {
                setTags(tags.slice(0, -1));
              }
            }}
          />
        </div>
        {suggestions.length > 0 && (
          <div className="flex flex-wrap gap-1.5">
            {suggestions.map((s) => (
              <button key={s} className="rounded-sm border border-border px-2 py-0.5 text-xs text-muted hover:text-fg" onClick={() => add(s)}>
                + {s}
              </button>
            ))}
          </div>
        )}
      </div>
    </Dialog>
  );
}
