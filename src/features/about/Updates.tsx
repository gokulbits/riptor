import { Download, Loader2, RefreshCw } from "lucide-react";
import { useEffect, useState } from "react";
import { Toggle } from "../../components/ui";
import { api, errorText, events } from "../../lib/ipc";
import { formatBytes } from "../../lib/format";
import { useApp } from "../../stores/app";

export function Updates() {
  const toast = useApp((s) => s.toast);
  const update = useApp((s) => s.update);
  const setUpdate = useApp((s) => s.setUpdate);
  const [supported, setSupported] = useState<boolean | null>(null);
  const [auto, setAuto] = useState<boolean | null>(null);
  const [state, setState] = useState<"idle" | "checking" | "current" | "installing">("idle");
  const [progress, setProgress] = useState<{ done: number; total: number | null } | null>(null);

  useEffect(() => {
    api.updateSupported().then(setSupported, () => setSupported(false));
    api.settings().then((s) => setAuto(s.check_updates), () => {});
    const un = events.updateProgress(setProgress);
    return () => {
      un.then((f) => f());
    };
  }, []);

  if (supported === false) {
    return <p className="text-xs text-muted">Updates come from your package manager on Linux.</p>;
  }

  const check = async () => {
    setState("checking");
    try {
      const u = await api.updateCheck();
      setUpdate(u);
      setState(u ? "idle" : "current");
    } catch (e) {
      setState("idle");
      toast("error", errorText(e));
    }
  };
  const install = async () => {
    setState("installing");
    try {
      // Riptor closes once the installer starts.
      await api.updateInstall();
    } catch (e) {
      setState("idle");
      setProgress(null);
      toast("error", errorText(e));
    }
  };
  const setAutoCheck = async (v: boolean) => {
    try {
      const s = await api.settings();
      await api.updateSettings({ ...s, check_updates: v });
      setAuto(v);
    } catch (e) {
      toast("error", errorText(e));
    }
  };

  return (
    <div className="w-full space-y-2 rounded-md bg-raised p-3 text-left">
      {update ? (
        <>
          <p className="text-sm font-medium">Riptor {update.version} is available</p>
          {update.notes && <p className="max-h-32 overflow-auto whitespace-pre-wrap text-xs text-muted">{update.notes}</p>}
          {state === "installing" ? (
            <p className="flex items-center gap-2 text-xs text-muted">
              <Loader2 size={13} className="animate-spin" />
              {progress ? `Downloading ${formatBytes(progress.done)}${progress.total ? ` of ${formatBytes(progress.total)}` : ""}…` : "Starting…"}
            </p>
          ) : (
            <button className="btn-primary text-xs" onClick={install}>
              <Download size={13} /> Install and restart
            </button>
          )}
          <p className="text-xs text-muted">Riptor closes, the installer asks for administrator rights, then Riptor starts again.</p>
        </>
      ) : (
        <div className="flex items-center gap-2">
          <button className="btn-ghost border border-border text-xs" onClick={check} disabled={state === "checking"}>
            {state === "checking" ? <Loader2 size={13} className="animate-spin" /> : <RefreshCw size={13} />} Check for updates
          </button>
          {state === "current" && <span className="text-xs text-muted">Riptor is up to date.</span>}
        </div>
      )}
      <Toggle checked={!!auto} disabled={auto === null} onChange={setAutoCheck} label="Check automatically when Riptor starts" />
    </div>
  );
}
