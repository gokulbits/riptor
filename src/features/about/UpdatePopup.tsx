import { Download, RotateCw } from "lucide-react";
import { useEffect, useState } from "react";
import { Dialog } from "../../components/ui";
import { api, errorText } from "../../lib/ipc";
import { useApp } from "../../stores/app";

/** Starts the background download; progress and the "ready" popup arrive as events. */
export function downloadUpdate() {
  const s = useApp.getState();
  if (s.updateStage !== "available") return;
  s.setUpdateStage("downloading");
  api.updateDownload().catch((e) => {
    useApp.getState().setUpdateStage("available");
    useApp.getState().setUpdateProgress(null);
    useApp.getState().toast("error", `The update couldn't be downloaded: ${errorText(e)}`);
  });
}

export async function installUpdate() {
  try {
    // Riptor closes once the installer starts.
    await api.updateInstall();
  } catch (e) {
    useApp.getState().toast("error", errorText(e));
  }
}

export function UpdatePopup() {
  const popup = useApp((s) => s.updatePopup);
  const update = useApp((s) => s.update);
  const close = () => useApp.getState().setUpdatePopup(null);
  const [current, setCurrent] = useState("");
  useEffect(() => {
    api.version().then(setCurrent, () => {});
  }, []);
  if (!popup || !update) return null;

  if (popup === "ready") {
    return (
      <Dialog
        title="Update ready to install"
        onClose={close}
        width="max-w-md"
        footer={
          <>
            <button className="btn-ghost" onClick={close}>
              Later
            </button>
            <button
              className="btn-primary"
              onClick={() => {
                close();
                installUpdate();
              }}
            >
              <RotateCw size={14} /> Restart and install
            </button>
          </>
        }
      >
        <p className="text-sm">Riptor {update.version} has been downloaded.</p>
        <p className="mt-2 text-xs text-muted">
          Riptor closes, the installer asks for administrator rights, and Riptor starts again with your torrents as they were. You can
          also install it later from the status bar.
        </p>
      </Dialog>
    );
  }

  return (
    <Dialog
      title="Update available"
      onClose={close}
      width="max-w-md"
      footer={
        <>
          <button className="btn-ghost" onClick={close}>
            Not now
          </button>
          <button
            className="btn-primary"
            onClick={() => {
              close();
              downloadUpdate();
            }}
          >
            <Download size={14} /> Download in the background
          </button>
        </>
      }
    >
      <p className="text-sm">
        Riptor {update.version} is available{current && <span className="text-muted"> (you have {current})</span>}.
      </p>
      {update.notes && (
        <pre className="mt-3 max-h-48 overflow-auto rounded-lg border border-border bg-raised p-3 font-sans text-xs whitespace-pre-wrap text-muted">
          {update.notes}
        </pre>
      )}
      <p className="mt-3 text-xs text-muted">
        Riptor keeps working while it downloads, and asks before installing. The download is checked against Riptor's signing key.
      </p>
    </Dialog>
  );
}
