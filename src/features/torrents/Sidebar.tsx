import clsx from "clsx";
import {
  Activity,
  AlertCircle,
  ArrowDownToLine,
  ArrowUpFromLine,
  CheckCircle2,
  Clock,
  Folder,
  Layers,
  Moon,
  PauseCircle,
  Radio,
  Plus,
  Tag,
} from "lucide-react";
import { useMemo, type ReactNode } from "react";
import { useShallow } from "zustand/react/shallow";
import { matchesStatus, useApp, type StatusFilter } from "../../stores/app";

const STATUSES: { id: StatusFilter; label: string; icon: ReactNode }[] = [
  { id: "all", label: "All", icon: <Layers size={15} /> },
  { id: "downloading", label: "Downloading", icon: <ArrowDownToLine size={15} /> },
  { id: "seeding", label: "Seeding", icon: <ArrowUpFromLine size={15} /> },
  { id: "completed", label: "Completed", icon: <CheckCircle2 size={15} /> },
  { id: "active", label: "Active", icon: <Activity size={15} /> },
  { id: "inactive", label: "Inactive", icon: <Moon size={15} /> },
  { id: "queued", label: "Queued", icon: <Clock size={15} /> },
  { id: "stopped", label: "Stopped", icon: <PauseCircle size={15} /> },
  { id: "errored", label: "Errored", icon: <AlertCircle size={15} /> },
];

function Item({
  active,
  onClick,
  icon,
  label,
  count,
}: {
  active: boolean;
  onClick: () => void;
  icon: ReactNode;
  label: string;
  count: number;
}) {
  return (
    <button
      onClick={onClick}
      className={clsx(
        "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] transition-colors",
        active ? "bg-accent/15 text-fg" : "text-muted hover:bg-raised hover:text-fg",
      )}
    >
      <span className={clsx(active && "text-accent")}>{icon}</span>
      <span className="flex-1 truncate">{label}</span>
      <span className="text-xs tabular-nums opacity-70">{count}</span>
    </button>
  );
}

export function Sidebar({ categories, onAddCategory }: { categories: string[]; onAddCategory: () => void }) {
  const { torrents, status, category, tag, tracker, setStatus, setCategory, setTag, setTracker } = useApp(
    useShallow((s) => ({
      torrents: s.torrents,
      status: s.status,
      category: s.category,
      tag: s.tag,
      setStatus: s.setStatus,
      setCategory: s.setCategory,
      setTag: s.setTag,
      tracker: s.tracker,
      setTracker: s.setTracker,
    })),
  );
  const list = useMemo(() => [...torrents.values()], [torrents]);
  const tags = useMemo(() => [...new Set(list.flatMap((t) => t.tags))].sort(), [list]);
  const trackers = useMemo(() => [...new Set(list.map((t) => t.tracker ?? ""))].sort(), [list]);

  return (
    <nav className="flex h-full w-52 shrink-0 flex-col gap-4 overflow-y-auto border-r border-border bg-panel p-2" aria-label="Filters">
      <section>
        <h3 className="px-2 pb-1 text-xxs font-semibold uppercase tracking-wider text-muted">Status</h3>
        {STATUSES.map((s) => (
          <Item
            key={s.id}
            active={status === s.id}
            onClick={() => setStatus(s.id)}
            icon={s.icon}
            label={s.label}
            count={list.filter((t) => matchesStatus(t, s.id)).length}
          />
        ))}
      </section>

      <section>
        <div className="flex items-center justify-between px-2 pb-1">
          <h3 className="text-xxs font-semibold uppercase tracking-wider text-muted">Categories</h3>
          <button className="text-muted hover:text-fg" onClick={onAddCategory} aria-label="Add category" title="Add category">
            <Plus size={14} />
          </button>
        </div>
        <Item active={category === undefined} onClick={() => setCategory(undefined)} icon={<Folder size={15} />} label="All" count={list.length} />
        <Item
          active={category === null}
          onClick={() => setCategory(null)}
          icon={<Folder size={15} />}
          label="Uncategorized"
          count={list.filter((t) => !t.category).length}
        />
        {categories.map((c) => (
          <Item
            key={c}
            active={category === c}
            onClick={() => setCategory(c)}
            icon={<Folder size={15} />}
            label={c}
            count={list.filter((t) => t.category === c).length}
          />
        ))}
      </section>

      {tags.length > 0 && (
        <section>
          <h3 className="px-2 pb-1 text-xxs font-semibold uppercase tracking-wider text-muted">Tags</h3>
          <Item active={tag === null} onClick={() => setTag(null)} icon={<Tag size={15} />} label="All" count={list.length} />
          {tags.map((t) => (
            <Item
              key={t}
              active={tag === t}
              onClick={() => setTag(t)}
              icon={<Tag size={15} />}
              label={t}
              count={list.filter((x) => x.tags.includes(t)).length}
            />
          ))}
        </section>
      )}

      {trackers.some(Boolean) && (
        <section>
          <h3 className="px-2 pb-1 text-xxs font-semibold uppercase tracking-wider text-muted">Trackers</h3>
          <Item active={tracker === null} onClick={() => setTracker(null)} icon={<Radio size={15} />} label="All" count={list.length} />
          {trackers.map((h) => (
            <Item
              key={h || "-"}
              active={tracker === h}
              onClick={() => setTracker(h)}
              icon={<Radio size={15} />}
              label={h || "Trackerless"}
              count={list.filter((x) => (x.tracker ?? "") === h).length}
            />
          ))}
        </section>
      )}
    </nav>
  );
}
