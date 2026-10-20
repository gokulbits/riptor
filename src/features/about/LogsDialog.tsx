import clsx from "clsx";
import { Copy, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Dialog } from "../../components/ui";
import { api, errorText } from "../../lib/ipc";
import type { LogLine } from "../../lib/types";
import { useApp } from "../../stores/app";

type Level = "all" | "info" | "warn" | "error";

const RANK: Record<LogLine["level"], number> = { trace: 0, debug: 1, info: 2, warn: 3, error: 4 };
const MIN: Record<Level, number> = { all: 0, info: 2, warn: 3, error: 4 };

const time = (ms: number) => new Date(ms).toLocaleTimeString(undefined, { hour12: false });
const asText = (l: LogLine) => `${new Date(l.time).toISOString()} ${l.level.toUpperCase().padEnd(5)} ${l.target}: ${l.message}`;

export function LogsDialog({ onClose }: { onClose: () => void }) {
  const toast = useApp((s) => s.toast);
  const [lines, setLines] = useState<LogLine[]>([]);
  const [level, setLevel] = useState<Level>("all");
  const [follow, setFollow] = useState(true);
  const bottom = useRef<HTMLDivElement>(null);

  const load = useCallback(() => {
    api.logs().then(setLines).catch((e) => toast("error", errorText(e)));
  }, [toast]);

  useEffect(() => {
    load();
    const t = window.setInterval(load, 2000);
    return () => window.clearInterval(t);
  }, [load]);

  const shown = useMemo(() => lines.filter((l) => RANK[l.level] >= MIN[level]), [lines, level]);

  useEffect(() => {
    if (follow) bottom.current?.scrollIntoView({ block: "end" });
  }, [shown, follow]);

  const copy = () =>
    navigator.clipboard
      .writeText(shown.map(asText).join("\n"))
      .then(() => toast("success", `Copied ${shown.length} lines`))
      .catch(() => toast("error", "Couldn't copy to the clipboard"));

  return (
    <Dialog
      title="Logs"
      onClose={onClose}
      width="max-w-5xl"
      footer={
        <>
          <span className="mr-auto self-center text-xs text-muted">
            Kept in memory only, cleared when Riptor quits. Tracker passkeys and URL parameters are masked.
          </span>
          <button className="btn-ghost" onClick={copy} disabled={!shown.length}>
            <Copy size={14} /> Copy
          </button>
          <button className="btn-primary" onClick={onClose}>
            Close
          </button>
        </>
      }
    >
      <div className="mb-2 flex items-center gap-2">
        <select className="input w-40" value={level} onChange={(e) => setLevel(e.target.value as Level)} aria-label="Level">
          <option value="all">All Levels</option>
          <option value="info">Info And Above</option>
          <option value="warn">Warnings And Errors</option>
          <option value="error">Errors Only</option>
        </select>
        <label className="flex items-center gap-1.5 text-xs text-muted">
          <input type="checkbox" checked={follow} onChange={(e) => setFollow(e.target.checked)} /> Follow
        </label>
        <div className="flex-1" />
        <button className="btn-ghost" onClick={load} title="Refresh">
          <RefreshCw size={14} />
        </button>
      </div>
      <div className="h-[55vh] overflow-auto rounded-md bg-raised p-2 font-mono text-[11.5px] leading-5">
        {shown.length === 0 && <p className="text-muted">Nothing logged yet.</p>}
        {shown.map((l, i) => (
          <div key={`${l.time}-${i}`} className="whitespace-pre-wrap break-all">
            <span className="text-muted">{time(l.time)}</span>{" "}
            <span
              className={clsx(
                "inline-block w-12 uppercase",
                l.level === "error" ? "text-danger" : l.level === "warn" ? "text-warn" : "text-muted",
              )}
            >
              {l.level}
            </span>{" "}
            <span className="text-muted">{l.target}</span> {l.message}
          </div>
        ))}
        <div ref={bottom} />
      </div>
    </Dialog>
  );
}
