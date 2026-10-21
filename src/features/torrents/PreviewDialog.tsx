import { Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { Dialog } from "../../components/ui";
import { api, errorText } from "../../lib/ipc";
import type { FileEntry, FilePreview } from "../../lib/types";

/** Shows an image or text file without handing it to another program. */
export function PreviewDialog({ id, file, onClose }: { id: string; file: FileEntry; onClose: () => void }) {
  const [preview, setPreview] = useState<FilePreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    api.previewFile(id, file.index).then(
      (p) => alive && setPreview(p),
      (e) => alive && setError(errorText(e)),
    );
    return () => {
      alive = false;
    };
  }, [id, file.index]);
  const name = file.path.split(/[\\/]/).pop() ?? file.path;
  return (
    <Dialog
      title={name}
      onClose={onClose}
      width="max-w-4xl"
      footer={
        <button className="btn-primary" onClick={onClose}>
          Close
        </button>
      }
    >
      {error ? (
        <p className="rounded-md bg-danger/10 p-3 text-sm text-danger">{error}</p>
      ) : !preview ? (
        <div className="flex justify-center py-10 text-muted">
          <Loader2 className="animate-spin" />
        </div>
      ) : preview.kind === "image" ? (
        <img src={`data:${preview.mime};base64,${preview.data}`} alt={name} className="mx-auto max-h-[70vh] max-w-full object-contain" />
      ) : preview.kind === "text" ? (
        <div className="space-y-2">
          <pre className="max-h-[65vh] select-text overflow-auto whitespace-pre-wrap rounded-md bg-raised p-3 font-mono text-xs leading-snug">
            {preview.text}
          </pre>
          {preview.truncated && <p className="text-xs text-muted">Only the start of the file is shown.</p>}
        </div>
      ) : (
        <p className="text-sm text-muted">{preview.reason}</p>
      )}
    </Dialog>
  );
}
