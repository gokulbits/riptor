import { Download, Loader2, RefreshCw, RotateCw } from "lucide-react";
import { useEffect, useState } from "react";
import { Toggle } from "../../components/ui";
import { api, errorText } from "../../lib/ipc";
import { formatBytes } from "../../lib/format";
import { useApp } from "../../stores/app";
import { downloadUpdate, installUpdate } from "./UpdatePopup";

/** Auto-check is part of the Settings form, so it saves with the rest. */
export function Updates({ auto, onAuto }: { auto: boolean; onAuto: (v: boolean) => void }) {
  const toast = useApp((s) => s.toast);
  const update = useApp((s) => s.update);
  const stage = useApp((s) => s.updateStage);
  const progress = useApp((s) => s.updateProgress);
  const setUpdate = useApp((s) => s.setUpdate);
  const [supported, setSupported] = useState<boolean | null>(null);
  const [state, setState] = useState<"idle" | "checking" | "current">("idle");

  useEffect(() => {
    api.updateSupported().then(setSupported, () => setSupported(false));
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

  return (
    <div className="w-full space-y-2 rounded-md bg-raised p-3 text-left">
      {update ? (
        <>
          <p className="text-sm font-medium">
            Riptor {update.version} {stage === "ready" ? "is ready to install" : "is available"}
          </p>
          {update.notes && <p className="max-h-32 overflow-auto whitespace-pre-wrap text-xs text-muted">{update.notes}</p>}
          {stage === "downloading" ? (
            <p className="flex items-center gap-2 text-xs text-muted">
              <Loader2 size={13} className="animate-spin" />
              {progress
                ? `Downloading in the background: ${formatBytes(progress.done)}${progress.total ? ` of ${formatBytes(progress.total)}` : ""}`
                : "Starting the download…"}
            </p>
          ) : stage === "ready" ? (
            <button className="btn-primary text-xs" onClick={installUpdate}>
              <RotateCw size={13} /> Restart and install
            </button>
          ) : (
            <button className="btn-primary text-xs" onClick={downloadUpdate}>
              <Download size={13} /> Download in the background
            </button>
          )}
          <p className="text-xs text-muted">Installing closes Riptor, asks for administrator rights, then starts Riptor again.</p>
        </>
      ) : (
        <div className="flex items-center gap-2">
          <button className="btn-ghost border border-border text-xs" onClick={check} disabled={state === "checking"}>
            {state === "checking" ? <Loader2 size={13} className="animate-spin" /> : <RefreshCw size={13} />} Check for updates
          </button>
          {state === "current" && <span className="text-xs text-muted">Riptor is up to date.</span>}
        </div>
      )}
      <Toggle checked={auto} onChange={onAuto} label="Check automatically when Riptor starts" />
    </div>
  );
}
