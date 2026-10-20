import { AlertTriangle, Download, ExternalLink, Loader2, Lock } from "lucide-react";
import { useEffect, useState } from "react";
import { Dialog } from "../../components/ui";
import { formatBytes } from "../../lib/format";
import { api, errorText } from "../../lib/ipc";
import { rememberPreview } from "../../lib/previewCache";
import type { Preview, SearchResult, Source } from "../../lib/types";

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[120px_1fr] gap-2 py-1 text-sm">
      <span className="text-muted">{label}</span>
      <span className="min-w-0 select-text break-words">{children}</span>
    </div>
  );
}

export function ResultDetails({
  result,
  engineName,
  resolve,
  onDownload,
  onClose,
}: {
  result: SearchResult;
  engineName: string;
  resolve: (r: SearchResult) => Promise<Source>;
  onDownload: (src: Source) => void;
  onClose: () => void;
}) {
  const [source, setSource] = useState<Source | null>(null);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    resolve(result)
      .then(async (src) => {
        if (!alive) return;
        setSource(src);
        const p = await api.preview(src);
        if (alive) setPreview(p);
      })
      .catch((e) => alive && setError(errorText(e)));
    return () => {
      alive = false;
    };
  }, [result, resolve]);

  const download = () => {
    if (!source) return;
    if (preview) rememberPreview(source, preview);
    onDownload(source);
    onClose();
  };

  const published = result.pub_date
    ? new Date(result.pub_date * 1000).toLocaleDateString(undefined, { year: "numeric", month: "long", day: "numeric" })
    : null;

  return (
    <Dialog
      title={result.name}
      onClose={onClose}
      width="max-w-3xl"
      footer={
        <>
          {result.desc_link && (
            <button className="btn-ghost mr-auto" onClick={() => api.openWebUrl(result.desc_link!).catch(() => {})} title={result.desc_link}>
              <ExternalLink size={14} /> Open web page
            </button>
          )}
          <button className="btn-ghost" onClick={onClose}>
            Close
          </button>
          <button className="btn-primary" disabled={!source || preview?.already_added} onClick={download}>
            <Download size={14} /> Download…
          </button>
        </>
      }
    >
      <div className="space-y-4">
        <section>
          <Row label="Engine">{engineName}</Row>
          <Row label="Size">{result.size != null ? formatBytes(result.size) : "Unknown"}</Row>
          <Row label="Seeds / peers">
            <span className="text-ok">{result.seeders ?? "?"}</span> / {result.leechers ?? "?"}
            {result.scraped && <span className="ml-2 text-xs text-muted">(from the trackers)</span>}
          </Row>
          {published && <Row label="Published">{published}</Row>}
        </section>

        {error ? (
          <p className="rounded-md bg-danger/10 p-3 text-sm text-danger">Couldn't load the torrent's details: {error}</p>
        ) : !preview ? (
          <div className="flex items-center gap-2 py-6 text-sm text-muted">
            <Loader2 size={16} className="animate-spin" />
            {result.link.startsWith("magnet:") ? "Fetching details from peers… this can take a minute." : "Fetching torrent details…"}
          </div>
        ) : (
          <>
            {preview.already_added && (
              <p className="rounded-md bg-info/10 px-3 py-2 text-sm text-info">This torrent is already in your list.</p>
            )}
            {preview.has_executables && (
              <p className="flex items-start gap-2 rounded-md bg-warn/10 px-3 py-2 text-sm text-warn">
                <AlertTriangle size={16} className="mt-0.5 shrink-0" />
                Contains programs or scripts (marked below). Only run them if you trust where this torrent came from.
              </p>
            )}
            <section className="border-t border-border pt-3">
              <Row label="Total size">{formatBytes(preview.total_size)}</Row>
              <Row label="Files">{preview.files.length}</Row>
              <Row label="Piece size">{formatBytes(preview.piece_length)}</Row>
              <Row label="Info hash">
                <span className="font-mono text-xs">{preview.info_hash}</span>
              </Row>
              {preview.private && (
                <Row label="Private">
                  <span className="flex items-center gap-1">
                    <Lock size={12} /> Trackers only (no DHT or peer exchange)
                  </span>
                </Row>
              )}
              {preview.comment && <Row label="Comment">{preview.comment}</Row>}
              {preview.trackers.length > 0 && (
                <Row label="Trackers">
                  <span className="block max-h-24 overflow-auto font-mono text-xs text-muted">
                    {preview.trackers.map((t) => (
                      <span key={t} className="block truncate" title={t}>
                        {t}
                      </span>
                    ))}
                  </span>
                </Row>
              )}
            </section>
            <section className="rounded-md border border-border">
              <div className="max-h-64 overflow-auto">
                {preview.files.map((f) => (
                  <div key={f.index} className="flex items-center gap-2 border-b border-border/50 px-3 py-1.5 text-[13px] last:border-0">
                    {f.executable && <AlertTriangle size={13} className="shrink-0 text-warn" aria-label="Program or script" />}
                    <span className="min-w-0 flex-1 truncate" title={f.path}>
                      {f.path}
                    </span>
                    <span className="shrink-0 tabular-nums text-muted">{formatBytes(f.size)}</span>
                  </div>
                ))}
              </div>
            </section>
          </>
        )}
      </div>
    </Dialog>
  );
}
