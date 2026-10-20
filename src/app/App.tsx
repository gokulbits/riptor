import { getCurrentWebview } from "@tauri-apps/api/webview";
import { ask, open, save } from "@tauri-apps/plugin-dialog";
import clsx from "clsx";
import {
  ArrowDownToLine,
  ArrowUpToLine,
  Check,
  ChevronDown,
  ChevronUp,
  Copy,
  Type,
  FileDown,
  Link2,
  Folder,
  FolderInput,
  FolderOpen,
  RefreshCw,
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
import { parseLinks } from "../lib/links";
import { applyTheme, resolveTheme } from "../lib/themes";
import type { Source } from "../lib/types";
import { loadPref, savePref, useApp, visibleTorrents } from "../stores/app";
import { AddDialog } from "../features/torrents/AddDialog";
import { ContextMenu, type MenuItem } from "../features/torrents/ContextMenu";
import { DetailPanel } from "../features/torrents/DetailPanel";
import { Sidebar } from "../features/torrents/Sidebar";
import { CategoryDialog, MagnetDialog, RemoveDialog, ShareLimitsDialog, TagsDialog } from "../features/torrents/SmallDialogs";
import { StatusBar } from "../features/torrents/StatusBar";
import { Toolbar } from "../features/torrents/Toolbar";
import { loadColumnPrefs, TorrentTable } from "../features/torrents/TorrentTable";
import { SettingsDialog } from "../features/settings/SettingsDialog";
import { StatsDialog } from "../features/stats/StatsDialog";
import { AboutDialog } from "../features/about/AboutDialog";
import { LogsDialog } from "../features/about/LogsDialog";
import { CreateTorrentDialog } from "../features/create/CreateTorrentDialog";
import { SearchView, useSearchEvents } from "../features/search/SearchView";
import { VpnView } from "../features/vpn/VpnView";
import { RssView } from "../features/rss/RssView";

type Modal =
  | { kind: "magnet" }
  | { kind: "remove"; ids: string[] }
  | { kind: "settings" }
  | { kind: "stats" }
  | { kind: "create" }
  | { kind: "logs" }
  | { kind: "about" }
  | { kind: "category" }
  | { kind: "limits"; ids: string[] }
  | { kind: "tags"; ids: string[] }
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
  const [detailHeight, setDetailHeight] = useState(() => {
    const h = Number(loadPref("riptor.detailHeight", "260"));
    return Number.isFinite(h) ? Math.min(Math.max(h, 120), 2000) : 260;
  });

  const refreshCategories = useCallback(() => {
    api.categories().then(setCategories).catch(() => {});
  }, []);

  // No "run once" guard: StrictMode mounts twice and the cleanup must undo the first.
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
      events.vpn((v) => useApp.getState().setVpn(v)),
      getCurrentWebview().onDragDropEvent((ev) => {
        if (ev.payload.type !== "drop") return;
        const files = ev.payload.paths.filter((p) => p.toLowerCase().endsWith(".torrent"));
        if (files.length) queueAdds(files.map((value) => ({ kind: "file", value }) as Source));
        else if (ev.payload.paths.length) toast("warning", "Only .torrent files and magnet links can be dropped here");
      }),
    ];
    // The ready event can fire before we listen.
    let stop = false;
    const poll = async () => {
      while (!stop) {
        try {
          await api.engineStatus();
          setReady(true);
          loadColumnPrefs().catch(() => {});
          api.vpnStatus().then(useApp.getState().setVpn, () => {});
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const inField = (t: EventTarget | null) => t instanceof HTMLElement && /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName);
    const onButton = (t: EventTarget | null) => t instanceof HTMLElement && /^(BUTTON|A)$/.test(t.tagName);
    const focusLater = (selector: string) =>
      setTimeout(() => document.querySelector<HTMLInputElement>(selector)?.focus(), 0);
    // Arrows, Space and Enter act on the torrent list.
    const listKey = (e: KeyboardEvent): boolean => {
      const st = useApp.getState();
      if (st.view !== "transfers" || e.ctrlKey || e.metaKey || e.altKey) return false;
      const ids = visibleTorrents(st).map((t) => t.id);
      if (["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) {
        if (!ids.length) return true;
        const at = st.focused ? ids.indexOf(st.focused) : -1;
        const next =
          e.key === "Home" ? 0 : e.key === "End" ? ids.length - 1 : Math.min(ids.length - 1, Math.max(0, at + (e.key === "ArrowDown" ? 1 : -1)));
        const anchor = st.anchor ? ids.indexOf(st.anchor) : -1;
        if (e.shiftKey && anchor >= 0) {
          const [lo, hi] = anchor < next ? [anchor, next] : [next, anchor];
          actions.selectAll(ids.slice(lo, hi + 1));
          actions.select(ids[next], "range");
        } else {
          actions.select(ids[next], "single");
        }
        return true;
      }
      if (onButton(e.target)) return false;
      if (e.key === " " && st.selection.size) {
        const sel = [...st.selection];
        const allStopped = sel.every((id) => st.torrents.get(id)?.state === "stopped");
        run(allStopped ? api.resume(sel) : api.stop(sel));
        return true;
      }
      if (e.key === "Enter" && st.focused) {
        run(api.reveal(st.focused));
        return true;
      }
      return false;
    };
    const onKey = (e: KeyboardEvent) => {
      if (inField(e.target) || modal) return;
      const ctrl = e.ctrlKey || e.metaKey;
      if (listKey(e)) {
        e.preventDefault();
      } else if (ctrl && e.key.toLowerCase() === "k") {
        e.preventDefault();
        actions.setView("search");
        focusLater('input[placeholder="Search torrents…"]');
      } else if (e.key === "Delete" && useApp.getState().selection.size) {
        setModal({ kind: "remove", ids: [...useApp.getState().selection] });
      } else if (ctrl && e.key.toLowerCase() === "a") {
        e.preventDefault();
        actions.selectAll(visibleTorrents(useApp.getState()).map((t) => t.id));
      } else if (ctrl && e.key.toLowerCase() === "o") {
        e.preventDefault();
        addFile();
      } else if (ctrl && e.key.toLowerCase() === "f") {
        e.preventDefault();
        actions.setView("transfers");
        focusLater('input[aria-label="Filter torrents"]');
      } else if (ctrl && e.key.toLowerCase() === "u") {
        e.preventDefault();
        setModal({ kind: "magnet" });
      }
    };
    const onPaste = (e: ClipboardEvent) => {
      if (inField(e.target) || modal) return;
      const links = parseLinks(e.clipboardData?.getData("text") ?? "");
      if (links.length) actions.queueAdds(links);
    };
    // Links dragged in from a browser arrive as text, not files.
    const textDrag = (e: DragEvent) => !!e.dataTransfer && [...e.dataTransfer.types].some((t) => t === "text/uri-list" || t === "text/plain");
    const onDragOver = (e: DragEvent) => {
      if (textDrag(e)) e.preventDefault();
    };
    const onDrop = (e: DragEvent) => {
      if (!textDrag(e) || inField(e.target) || modal) return;
      e.preventDefault();
      const links = parseLinks(e.dataTransfer?.getData("text/uri-list") || e.dataTransfer?.getData("text/plain") || "");
      if (links.length) actions.queueAdds(links);
      else actions.toast("warning", "Only .torrent files and magnet links can be dropped here");
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("paste", onPaste);
    window.addEventListener("dragover", onDragOver);
    window.addEventListener("drop", onDrop);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("paste", onPaste);
      window.removeEventListener("dragover", onDragOver);
      window.removeEventListener("drop", onDrop);
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
      setDetailHeight((h) => {
        savePref("riptor.detailHeight", String(h));
        return h;
      });
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  const moveSelected = async (ids: string[]) => {
    const first = s.torrents.get(ids[0]);
    const dir = await open({
      directory: true,
      defaultPath: first?.save_path,
      title: ids.length === 1 ? `Move "${first?.name}" to` : `Move ${ids.length} torrents to`,
    });
    if (typeof dir !== "string") return;
    actions.toast("info", ids.length === 1 ? `Moving ${first?.name}…` : `Moving ${ids.length} torrents…`);
    try {
      await api.move(ids, dir);
      actions.toast("success", "Moved. Riptor is checking the files at their new place.");
    } catch (e) {
      actions.toast("error", errorText(e));
    }
  };

  const exportSelected = async (ids: string[]) => {
    const fileName = (id: string) => `${(s.torrents.get(id)?.name ?? id).replace(/[<>:"/\\|?*\u0000-\u001f]/g, "_")}.torrent`;
    if (ids.length === 1) {
      const path = await save({ defaultPath: fileName(ids[0]), filters: [{ name: "Torrent file", extensions: ["torrent"] }] });
      if (path) run(api.exportTorrent(ids[0], path).then(() => actions.toast("success", "Torrent file saved")));
      return;
    }
    const dir = await open({ directory: true, title: `Save ${ids.length} torrent files to` });
    if (typeof dir !== "string") return;
    const sep = dir.includes("\\") ? "\\" : "/";
    run(
      Promise.all(ids.map((id) => api.exportTorrent(id, `${dir}${sep}${fileName(id)}`))).then(() =>
        actions.toast("success", `${ids.length} torrent files saved`),
      ),
    );
  };

  const recheckSelected = async (ids: string[]) => {
    const ok = await ask(
      ids.length === 1
        ? "Check every piece of this torrent again? It pauses while checking."
        : `Check every piece of ${ids.length} torrents again? They pause while checking.`,
      { title: "Force recheck", kind: "info" },
    );
    if (ok) run(api.recheck(ids));
  };

  const menuItems = (): (MenuItem | "sep")[] => {
    const allHave = (c: string | null) => selected.every((id) => (s.torrents.get(id)?.category ?? null) === c);
    return [
      { label: "Resume", icon: <Play size={14} />, onClick: () => run(api.resume(selected)) },
      { label: "Stop", icon: <Pause size={14} />, onClick: () => run(api.stop(selected)) },
      { label: "Force start", icon: <Zap size={14} />, onClick: () => run(api.forceStart(selected)) },
      "sep",
      {
        label: "Category",
        icon: <Folder size={14} />,
        children: [
          { label: "None", icon: allHave(null) ? <Check size={14} /> : undefined, onClick: () => run(api.setCategory(selected, null)) },
          ...Object.keys(categories).map((c) => ({
            label: c,
            icon: allHave(c) ? <Check size={14} /> : undefined,
            onClick: () => run(api.setCategory(selected, c)),
          })),
          { label: "New category…", onClick: () => setModal({ kind: "category" }) },
        ],
      },
      {
        label: "Edit tags…",
        icon: <Tag size={14} />,
        onClick: () => setModal({ kind: "tags", ids: selected }),
      },
      { label: "Limits…", icon: <Gauge size={14} />, onClick: () => setModal({ kind: "limits", ids: selected }) },
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
      { label: "Move…", icon: <FolderInput size={14} />, onClick: () => moveSelected(selected) },
      { label: "Force recheck", icon: <RefreshCw size={14} />, onClick: () => recheckSelected(selected) },
      {
        label: "Copy magnet link",
        icon: <Link2 size={14} />,
        onClick: () =>
          run(
            Promise.all(selected.map((id) => api.magnet(id))).then((links) =>
              navigator.clipboard.writeText(links.join("\n")),
            ),
          ),
      },
      { label: "Export .torrent…", icon: <FileDown size={14} />, onClick: () => exportSelected(selected) },
      {
        label: "Copy name",
        icon: <Type size={14} />,
        onClick: () =>
          navigator.clipboard.writeText(selected.map((id) => s.torrents.get(id)?.name ?? id).join("\n")).catch(() => {}),
      },
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
  const tagTargets = modal?.kind === "tags" ? modal.ids.map((id) => s.torrents.get(id)!).filter(Boolean) : [];

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
        onCreate={() => setModal({ kind: "create" })}
        onLogs={() => setModal({ kind: "logs" })}
        onResumeAll={() => run(api.resume([...s.torrents.keys()]))}
        onStopAll={() => run(api.stop([...s.torrents.keys()]))}
        onAbout={() => setModal({ kind: "about" })}
      />
      {view === "search" ? (
        <div className="min-h-0 flex-1">
          <SearchView />
        </div>
      ) : view === "rss" ? (
        <div className="min-h-0 flex-1">
          <RssView categories={Object.keys(categories)} />
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
              <TorrentTable onContextMenu={(e) => setMenu({ x: e.clientX, y: e.clientY })} onOpen={(id) => run(api.reveal(id))} />
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
      {modal?.kind === "settings" && (
        <SettingsDialog
          onClose={() => {
            setModal(null);
            // A restore may have brought back a different layout.
            loadColumnPrefs().catch(() => {});
          }}
        />
      )}
      {modal?.kind === "stats" && <StatsDialog onClose={() => setModal(null)} />}
      {modal?.kind === "create" && <CreateTorrentDialog onClose={() => setModal(null)} />}
      {modal?.kind === "logs" && <LogsDialog onClose={() => setModal(null)} />}
      {modal?.kind === "about" && <AboutDialog onClose={() => setModal(null)} onLogs={() => setModal({ kind: "logs" })} />}
      {modal?.kind === "category" && <CategoryDialog onClose={() => setModal(null)} onSaved={refreshCategories} />}
      {modal?.kind === "limits" && limitTargets.length > 0 && <ShareLimitsDialog torrents={limitTargets} onClose={() => setModal(null)} />}
      {modal?.kind === "tags" && tagTargets.length > 0 && <TagsDialog torrents={tagTargets} onClose={() => setModal(null)} />}
      <Toasts />
    </div>
  );
}
