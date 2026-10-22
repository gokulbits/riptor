import clsx from "clsx";
import {
  ArrowDownUp,
  BarChart3,
  ChevronDown,
  ChevronUp,
  FilePlus2,
  Info,
  Link2,
  MoreHorizontal,
  PackagePlus,
  FolderInput,
  Power,
  Check,
  Rss,
  Pause,
  Play,
  Network,
  ScrollText,
  Search,
  Settings,
  ShieldCheck,
  Trash2,
  Zap,
} from "lucide-react";
import { useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { ContextMenu } from "./ContextMenu";
import { ThemePicker } from "../../components/ThemePicker";
import { useApp } from "../../stores/app";
import { api } from "../../lib/ipc";
import type { WhenDone } from "../../lib/types";

export function Toolbar(props: {
  onAddFile: () => void;
  onAddMagnet: () => void;
  onResume: () => void;
  onStop: () => void;
  onForce: () => void;
  onRemove: () => void;
  onQueue: (how: "up" | "down") => void;
  onSettings: () => void;
  onStats: () => void;
  onCreate: () => void;
  onLogs: () => void;
  onConnections: () => void;
  onAbout: () => void;
  onResumeAll: () => void;
  onStopAll: () => void;
  onImport: () => void;
}) {
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [whenDone, setWhenDone] = useState<WhenDone>("nothing");
  const { selection, search, setSearch, view, setView } = useApp(
    useShallow((s) => ({ selection: s.selection, search: s.search, setSearch: s.setSearch, view: s.view, setView: s.setView })),
  );
  const none = selection.size === 0;
  const tab = (id: "transfers" | "search" | "rss" | "vpn", label: string, icon: React.ReactNode) => (
    <button
      role="tab"
      aria-selected={view === id}
      onClick={() => setView(id)}
      className={clsx(
        "flex items-center gap-1.5 rounded-md px-3 py-1.5 text-sm font-medium transition-colors",
        view === id ? "bg-panel text-fg shadow-xs" : "text-muted hover:text-fg",
      )}
    >
      {icon}
      {label}
    </button>
  );
  return (
    <header className="flex h-12 shrink-0 items-center gap-1 border-b border-border bg-panel px-2">
      <div role="tablist" className="mr-2 flex gap-0.5 rounded-lg bg-raised p-0.5">
        {tab("transfers", "Transfers", <ArrowDownUp size={15} />)}
        {tab("search", "Search", <Search size={15} />)}
        {tab("rss", "RSS", <Rss size={15} />)}
        {tab("vpn", "VPN", <ShieldCheck size={15} />)}
      </div>
      {view === "transfers" && (
        <>
      <button className="btn-primary" onClick={props.onAddFile} title="Add torrent file (Ctrl+O)">
        <FilePlus2 size={16} /> Add
      </button>
      <button className="btn-ghost" onClick={props.onAddMagnet} title="Add magnet link (Ctrl+U)">
        <Link2 size={16} /> Magnet
      </button>
      <button className="btn-ghost" onClick={props.onCreate} title="Create a torrent from your files">
        <PackagePlus size={16} /> Create
      </button>
      <div className="mx-1 h-6 w-px bg-border" />
      <button className="btn-ghost" disabled={none} onClick={props.onResume} title="Resume">
        <Play size={16} />
      </button>
      <button className="btn-ghost" disabled={none} onClick={props.onStop} title="Stop">
        <Pause size={16} />
      </button>
      <button className="btn-ghost" disabled={none} onClick={props.onForce} title="Force start (ignore queue)">
        <Zap size={16} />
      </button>
      <button className="btn-ghost hover:text-danger" disabled={none} onClick={props.onRemove} title="Remove (Del)">
        <Trash2 size={16} />
      </button>
      <div className="mx-1 h-6 w-px bg-border" />
      <button className="btn-ghost" disabled={none} onClick={() => props.onQueue("up")} title="Move up in queue">
        <ChevronUp size={16} />
      </button>
      <button className="btn-ghost" disabled={none} onClick={() => props.onQueue("down")} title="Move down in queue">
        <ChevronDown size={16} />
      </button>
        </>
      )}
      <div className="flex-1" />
      {view === "transfers" && (
      <div className="relative w-64">
        <Search size={14} className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-muted" />
        <input
          className="input pl-8"
          placeholder="Filter torrents (Ctrl+F)"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          aria-label="Filter torrents"
        />
      </div>
      )}
      <button className="btn-ghost" onClick={props.onStats} title="Statistics">
        <BarChart3 size={16} />
      </button>
      <ThemePicker />
      <button className="btn-ghost" onClick={props.onSettings} title="Settings">
        <Settings size={16} />
      </button>
      <button
        className="btn-ghost"
        title="More"
        aria-haspopup="menu"
        onClick={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          api.whenDoneGet().then(setWhenDone, () => {});
          setMenu({ x: r.right - 180, y: r.bottom + 4 });
        }}
      >
        <MoreHorizontal size={16} />
      </button>
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
          items={[
            { label: "Resume all", icon: <Play size={14} />, onClick: props.onResumeAll },
            { label: "Stop all", icon: <Pause size={14} />, onClick: props.onStopAll },
            {
              label: "When downloads finish",
              icon: <Power size={14} />,
              children: (
                [
                  ["nothing", "Do nothing"],
                  ["sleep", "Sleep"],
                  ["shutdown", "Shut down"],
                ] as [WhenDone, string][]
              ).map(([a, label]) => ({
                label,
                icon: whenDone === a ? <Check size={14} /> : undefined,
                onClick: () => api.whenDoneSet(a).catch(() => {}),
              })),
            },
            "sep",
            { label: "Import from qBittorrent…", icon: <FolderInput size={14} />, onClick: props.onImport },
            { label: "Connections", icon: <Network size={14} />, onClick: props.onConnections },
            { label: "Logs", icon: <ScrollText size={14} />, onClick: props.onLogs },
            { label: "About Riptor", icon: <Info size={14} />, onClick: props.onAbout },
          ]}
        />
      )}
    </header>
  );
}
