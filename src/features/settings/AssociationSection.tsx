import { CheckCircle2, Circle, ExternalLink, Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { api, errorText } from "../../lib/ipc";
import type { AssociationStatus } from "../../lib/types";
import { useApp } from "../../stores/app";

function Line({ ok, label }: { ok: boolean; label: string }) {
  return (
    <p className="flex items-center gap-2 text-sm">
      {ok ? <CheckCircle2 size={15} className="text-ok" /> : <Circle size={15} className="text-muted" />}
      {label}
    </p>
  );
}

export function AssociationSection() {
  const toast = useApp((s) => s.toast);
  const [status, setStatus] = useState<AssociationStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const isWindows = navigator.userAgent.includes("Windows");

  useEffect(() => {
    api.associationStatus().then(setStatus).catch(() => {});
  }, []);

  const makeDefault = async () => {
    setBusy(true);
    try {
      setStatus(await api.makeDefaultApp());
      toast("success", isWindows ? "Riptor now opens magnet links. Choose Riptor for .torrent in the window that opened." : "Riptor is now the default");
    } catch (e) {
      toast("error", errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <fieldset className="space-y-3">
      <legend className="mb-5 text-xs font-semibold uppercase tracking-wider text-muted">Default App</legend>
      {status && (
        <div className="space-y-1">
          <Line ok={status.registered} label="Riptor Is Listed In “Open With” For Torrent Files" />
          <Line ok={status.torrent_default} label="Opens .torrent Files" />
          <Line ok={status.magnet_default} label="Opens Magnet Links From Your Browser" />
        </div>
      )}
      <div className="flex gap-2">
        <button className="btn-primary" disabled={busy} onClick={makeDefault}>
          {busy && <Loader2 size={14} className="animate-spin" />} Make Riptor The Default
        </button>
        {isWindows && (
          <button className="btn-ghost border border-border" onClick={() => api.openDefaultApps()}>
            <ExternalLink size={14} /> Windows Default Apps
          </button>
        )}
      </div>
      <p className="text-xs text-muted">
        {isWindows
          ? "Windows only lets you, not apps, pick the default for file types, so Riptor opens Settings › Default apps for .torrent files. Links opened from a browser or file always show the add dialog first; nothing is added without you seeing it."
          : "Uses xdg-mime for your user account. Links opened from a browser or file always show the add dialog first."}
      </p>
    </fieldset>
  );
}
