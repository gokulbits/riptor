import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import clsx from "clsx";
import {
  ArrowDownToLine,
  ArrowUpToLine,
  ChevronDown,
  ChevronUp,
  Copy,
  Folder,
  FolderOpen,
  Gauge,
  Pause,
  Play,
  Tag,
  Trash2,
  X,
  Zap,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { api, errorText, events } from "../lib/ipc";
import { applyTheme, resolveTheme } from "../lib/themes";
import type { Source } from "../lib/types";
import { useApp, visibleTorrents } from "../stores/app";
import { AddDialog } from "../features/torrents/AddDialog";
import { ContextMenu, type MenuItem } from "../features/torrents/ContextMenu";
import { DetailPanel } from "../features/torrents/DetailPanel";
import { Sidebar } from "../features/torrents/Sidebar";
import { CategoryDialog, MagnetDialog, RemoveDialog, ShareLimitsDialog } from "../features/torrents/SmallDialogs";
import { StatusBar } from "../features/torrents/StatusBar";
import { Toolbar } from "../features/torrents/Toolbar";
import { TorrentTable } from "../features/torrents/TorrentTable";
import { SettingsDialog } from "../features/settings/SettingsDialog";
import { StatsDialog } from "../features/stats/StatsDialog";
import { SearchView, useSearchEvents } from "../features/search/SearchView";
import { VpnView } from "../features/vpn/VpnView";

type Modal =
  | { kind: "magnet" }
  | { kind: "remove"; ids: string[] }
  | { kind: "settings" }
  | { kind: "stats" }
  | { kind: "category" }
  | { kind: "limits"; ids: string[] }
  | null;

function useTheme() {
  const theme = useApp((s) => s.theme);
  const accent = useApp((s) => s.accent);
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => applyTheme(resolveTheme(theme, mq.matches), accent);
    apply();
    mq.addEventListener("change", apply);
    return () => mq.removeEventListener("change", apply);
  }, [theme, accent]);
}

function Toasts() {
  const { toasts, dismiss } = useApp(useShallow((s) => ({ toasts: s.toasts, dismiss: s.dismissToast })));
  return (
    <div className="pointer-events-none fixed bottom-10 right-4 z-50 flex w-80 flex-col gap-2" aria-live="polite">
      {toasts.map((t) => (
        <div
          key={t.id}
          className={clsx(
            "card pointer-events-auto flex items-start gap-2 px-3 py-2 text-sm shadow-lg",
            t.level === "error" && "border-danger/50",
            t.level === "warning" && "border-warn/50",
            t.level === "success" && "border-ok/50",
          )}
        >
          <span className="flex-1 break-words">{t.message}</span>
          <button className="text-muted hover:text-fg" onClick={() => dismiss(t.id)} aria-label="Dismiss">
            <X size={14} />
          </button>
        </div>
      ))}
    </div>
  );
}

export default function App() {
  useTheme();
  useSearchEvents();
  const view = useApp((s) => s.view);
  const s = useApp(
    useShallow((s) => ({
      ready: s.ready,
      engineError: s.engineError,
      selection: s.selection,
      pendingAdds: s.pendingAdds,
      torrents: s.torrents,
    })),
  );
  const actions = useApp.getState();
  const [modal, setModal] = useState<Modal>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [categories, setCategories] = useState<Record<string, string | null>>({});
  const [detailHeight, setDetailHeight] = useState(260);

  const refreshCategories = useCallback(() => {
    api.categories().then(setCategories).catch(() => {});
  }, []);

  // Engine events, readiness, and external add requests. No "run once" guard:
  // StrictMode mounts effects twice and the cleanup below must be allowed to undo the first.
  useEffect(() => {
    const { applyTick, removeTorrent, toast, queueAdds, setReady } = useApp.getState();
    const subs = [
      events.engine((e) => {
        if (e.type === "tick") applyTick(e.torrents, e.global);
        else if (e.type === "removed") removeTorrent(e.id);
        else if (e.type === "finished") toast("success", `Finished: ${e.name}`);
        else if (e.type === "notice") toast(e.level, e.message);
      }),
      events.externalAdd((items) => queueAdds(items)),
      getCurrentWebview().onDragDropEvent((ev) => {
        if (ev.payload.type !== "drop") return;
        const files = ev.payload.paths.filter((p) => p.toLowerCase().endsWith(".torrent"));
        if (files.length) queueAdds(files.map((value) => ({ kind: "file", value }) as Source));
        else if (ev.payload.paths.length) toast("warning", "Only .torrent files can be dropped here");
      }),
    ];
    // Poll until the engine is up (its ready event may fire before we listen).
    let stop = false;
    const poll = async () => {
      while (!stop) {
        try {
          await api.engineStatus();
          setReady(true);
          refreshCategories();
          const startup = await api.takeStartupItems();
          if (startup.length) queueAdds(startup);
          return;
        } catch (e) {
          const msg = errorText(e);
          if (msg.includes("failed to start")) {
            setReady(false, msg);
            return;
          }
        }
        await new Promise((r) => setTimeout(r, 300));
      }
    };
    poll();
    return () => {
      stop = true;
      subs.forEach((p) => p.then((un) => un()));
    };
  }, [refreshCategories]);

  const selected = [...s.selection];
  const run = (p: Promise<unknown>) => p.catch((e) => actions.toast("error", errorText(e)));

  const addFile = async () => {
    const picked = await open({ multiple: true, filters: [{ name: "Torrent files", extensions: ["torrent"] }] });
    const list = Array.isArray(picked) ? picked : picked ? [picked] : [];
    if (list.length) actions.queueAdds(list.map((value) => ({ kind: "file", value }) as Source));
  };

  // Tray menu items that need a dialog (Rust has already shown the window).
  useEffect(() => {
    const un = events.trayAction((action) => {
      if (action === "add_file") addFile();
      else if (action === "add_magnet") setModal({ kind: "magnet" });
      else if (action === "stats") setModal({ kind: "stats" });
      else if (action === "settings") setModal({ kind: "settings" });
      else if (action === "vpn") useApp.getState().setView("vpn");
    });
    return () => {
      un.then((f) => f());
    };
    // addFile only touches stable store actions, so subscribing once is fine.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Keyboard shortcuts and paste-to-add.
  useEffect(() => {
    const inField = (t: EventTarget | null) => t instanceof HTMLElement && /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName);
    const onKey = (e: KeyboardEvent) => {
      if (inField(e.target) || modal) return;
      const ctrl = e.ctrlKey || e.metaKey;
      if (e.key === "Delete" && useApp.getState().selection.size) {
        setModal({ kind: "remove", ids: [...useApp.getState().selection] });
      } else if (ctrl && e.key.toLowerCase() === "a") {
        e.preventDefault();
        actions.selectAll(visibleTorrents(useApp.getState()).map((t) => t.id));
      } else if (ctrl && e.key.toLowerCase() === "o") {
        e.preventDefault();
        addFile();
      } else if (ctrl && e.key.toLowerCase() === "f") {
        e.preventDefault();
        actions.setView("search");
      } else if (ctrl && e.key.toLowerCase() === "u") {
        e.preventDefault();
        setModal({ kind: "magnet" });
      }
    };
    const onPaste = (e: ClipboardEvent) => {
      if (inField(e.target) || modal) return;
      const text = e.clipboardData?.getData("text") ?? "";
      const magnets = text.split(/\s+/).filter((t) => t.startsWith("magnet:?"));
      if (magnets.length) actions.queueAdds(magnets.map((value) => ({ kind: "magnet", value }) as Source));
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("paste", onPaste);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("paste", onPaste);
    };
  });

  const startResize = (e: React.MouseEvent) => {
    e.preventDefault();
    const y0 = e.clientY;
    const h0 = detailHeight;
    const move = (ev: MouseEvent) => setDetailHeight(Math.min(window.innerHeight - 200, Math.max(120, h0 - (ev.clientY - y0))));
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  const menuItems = (): (MenuItem | "sep")[] => {
    const ts = selected.map((id) => s.torrents.get(id)).filter(Boolean);
    return [
      { label: "Resume", icon: <Play size={14} />, onClick: () => run(api.resume(selected)) },
      { label: "Stop", icon: <Pause size={14} />, onClick: () => run(api.stop(selected)) },
      { label: "Force start", icon: <Zap size={14} />, onClick: () => run(api.forceStart(selected)) },
      "sep",
      {
        label: "Category",
        icon: <Folder size={14} />,
        children: [
          { label: "None", onClick: () => run(api.setCategory(selected, null)) },
          ...Object.keys(categories).map((c) => ({ label: c, onClick: () => run(api.setCategory(selected, c)) })),
          { label: "New category…", onClick: () => setModal({ kind: "category" }) },
        ],
      },
      {
        label: "Edit tags…",
        icon: <Tag size={14} />,
        onClick: () => {
          const current = ts[0]?.tags.join(", ") ?? "";
          const next = window.prompt("Tags (comma separated)", current);
          if (next != null) run(api.setTags(selected, next.split(",").map((t) => t.trim()).filter(Boolean)));
        },
      },
      { label: "Share limits…", icon: <Gauge size={14} />, onClick: () => setModal({ kind: "limits", ids: selected }) },
      {
        label: "Queue",
        icon: <ChevronUp size={14} />,
        children: [
          { label: "Move to top", icon: <ArrowUpToLine size={14} />, onClick: () => run(api.queueMove(selected, "top")) },
          { label: "Move up", icon: <ChevronUp size={14} />, onClick: () => run(api.queueMove(selected, "up")) },
          { label: "Move down", icon: <ChevronDown size={14} />, onClick: () => run(api.queueMove(selected, "down")) },
          { label: "Move to bottom", icon: <ArrowDownToLine size={14} />, onClick: () => run(api.queueMove(selected, "bottom")) },
        ],
      },
      "sep",
      { label: "Open folder", icon: <FolderOpen size={14} />, disabled: selected.length !== 1, onClick: () => run(api.reveal(selected[0])) },
      {
        label: "Copy info hash",
        icon: <Copy size={14} />,
        onClick: () => navigator.clipboard.writeText(selected.join("\n")).catch(() => {}),
      },
      "sep",
      { label: "Remove…", icon: <Trash2 size={14} />, danger: true, onClick: () => setModal({ kind: "remove", ids: selected }) },
    ];
  };

  if (s.engineError) {
    return (
      <div className="flex h-full items-center justify-center p-8">
        <div className="card max-w-lg space-y-2 p-6">
          <h1 className="text-lg font-semibold text-danger">Riptor could not start</h1>
          <p className="select-text text-sm text-muted">{s.engineError}</p>
          <p className="text-xs text-muted">Another program may be using the listening port, or the data folder is not writable.</p>
        </div>
      </div>
    );
  }

  const limitTargets = modal?.kind === "limits" ? modal.ids.map((id) => s.torrents.get(id)!).filter(Boolean) : [];

  return (
    <div className="flex h-full flex-col">
      <Toolbar
        onAddFile={addFile}
        onAddMagnet={() => setModal({ kind: "magnet" })}
        onResume={() => run(api.resume(selected))}
        onStop={() => run(api.stop(selected))}
        onForce={() => run(api.forceStart(selected))}
        onRemove={() => setModal({ kind: "remove", ids: selected })}
        onQueue={(how) => run(api.queueMove(selected, how))}
        onSettings={() => setModal({ kind: "settings" })}
        onStats={() => setModal({ kind: "stats" })}
      />
      {view === "search" ? (
        <div className="min-h-0 flex-1">
          <SearchView />
        </div>
      ) : view === "vpn" ? (
        <div className="min-h-0 flex-1">
          <VpnView />
        </div>
      ) : (
      <div className="flex min-h-0 flex-1">
        <Sidebar categories={Object.keys(categories)} onAddCategory={() => setModal({ kind: "category" })} />
        <main className="flex min-w-0 flex-1 flex-col">
          <div className="min-h-0 flex-1">
            {s.ready ? (
              <TorrentTable onContextMenu={(e) => setMenu({ x: e.clientX, y: e.clientY })} />
            ) : (
              <div className="flex h-full items-center justify-center text-sm text-muted">Starting engine…</div>
            )}
          </div>
          <div className="h-1 shrink-0 cursor-row-resize border-t border-border hover:bg-accent/40" onMouseDown={startResize} />
          <div style={{ height: detailHeight }} className="shrink-0">
            <DetailPanel />
          </div>
        </main>
      </div>
      )}
      <StatusBar
        onToggleAlt={() => run(api.setAltSpeed(!useApp.getState().global?.alt_speed))}
      />

      {menu && selected.length > 0 && <ContextMenu x={menu.x} y={menu.y} items={menuItems()} onClose={() => setMenu(null)} />}
      {s.pendingAdds[0] && (
        <AddDialog key={JSON.stringify(s.pendingAdds[0])} source={s.pendingAdds[0]} categories={categories} onClose={actions.shiftAdd} />
      )}
      {modal?.kind === "magnet" && <MagnetDialog onSubmit={actions.queueAdds} onClose={() => setModal(null)} />}
      {modal?.kind === "remove" && <RemoveDialog ids={modal.ids} onClose={() => setModal(null)} />}
      {modal?.kind === "settings" && <SettingsDialog onClose={() => setModal(null)} />}
      {modal?.kind === "stats" && <StatsDialog onClose={() => setModal(null)} />}
      {modal?.kind === "category" && <CategoryDialog onClose={() => setModal(null)} onSaved={refreshCategories} />}
      {modal?.kind === "limits" && limitTargets.length > 0 && <ShareLimitsDialog torrents={limitTargets} onClose={() => setModal(null)} />}
      <Toasts />
    </div>
  );
}
