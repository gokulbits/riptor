import { FileCheck2, FileWarning, KeyRound, Loader2, ShieldAlert } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { api, errorText } from "../../lib/ipc";
import { formatDate } from "../../lib/format";
import type { Finding, Safety, TorrentSummary } from "../../lib/types";
import { useApp } from "../../stores/app";

function Checksums({ t, safety, reload }: { t: TorrentSummary; safety: Safety; reload: () => Promise<Safety | null> }) {
  const toast = useApp((s) => s.toast);
  const [busy, setBusy] = useState(false);
  const r = safety.checksums;
  if (!safety.has_manifest) return null;
  // Big files take a while; poll until the report's time changes.
  const verify = async () => {
    setBusy(true);
    try {
      await api.verifyChecksums([t.id]);
      for (let i = 0; i < 300; i++) {
        await new Promise((ok) => setTimeout(ok, 1000));
        const s = await reload();
        if (s?.checksums && s.checksums.checked_at !== r?.checked_at) break;
      }
    } catch (e) {
      toast("error", errorText(e));
    } finally {
      setBusy(false);
    }
  };
  const button = (
    <button className="btn-ghost py-0.5 text-xs" disabled={busy || t.progress < 1} onClick={verify}>
      {busy && <Loader2 size={12} className="animate-spin" />} Check again
    </button>
  );
  if (!r) {
    return (
      <p className="flex items-center gap-2 rounded-md bg-raised px-2 py-1.5 text-muted">
        <FileCheck2 size={14} className="shrink-0" />
        <span className="flex-1">Has checksum files. They're checked once the download finishes.</span>
        {button}
      </p>
    );
  }
  const bad = r.mismatched_count > 0;
  const extra = [r.skipped && `${r.skipped} not downloaded`, r.not_found && `${r.not_found} not in this torrent`].filter(Boolean);
  return (
    <div className={`rounded-md px-2 py-1.5 ${bad ? "bg-danger/10 text-danger" : r.verified ? "bg-ok/10 text-ok" : "bg-raised text-muted"}`}>
      <p className="flex items-center gap-2">
        {bad ? <FileWarning size={14} className="shrink-0" /> : <FileCheck2 size={14} className="shrink-0" />}
        <span className="flex-1">
          {bad
            ? `${r.mismatched_count} file(s) don't match ${r.manifests.join(", ")}. Don't trust them.`
            : r.verified
              ? `${r.verified} file(s) match ${r.manifests.join(", ")}.`
              : `Nothing to check yet in ${r.manifests.join(", ")}.`}
          {extra.length > 0 && <span className="text-muted"> ({extra.join(", ")})</span>}
        </span>
        <span className="text-xs text-muted">{formatDate(r.checked_at)}</span>
        {button}
      </p>
      {bad && (
        <ul className="ml-6 mt-1 list-disc text-xs">
          {r.mismatched.map((m) => (
            <li key={m} className="select-text font-mono">
              {m}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** Signs of a fake release, worst first. */
export function FindingsList({ findings }: { findings: Finding[] }) {
  if (!findings.length) return null;
  const danger = findings.some((f) => f.level === "danger");
  const sorted = [...findings].sort((a, b) => (a.level === b.level ? 0 : a.level === "danger" ? -1 : 1));
  return (
    <div className={`rounded-md px-3 py-2 text-sm ${danger ? "bg-danger/10 text-danger" : "bg-warn/10 text-warn"}`}>
      <p className="flex items-center gap-2 font-medium">
        <ShieldAlert size={16} className="shrink-0" />
        {danger ? "This looks like a fake release made to spread malware." : "Some things about this torrent look suspicious."}
      </p>
      <ul className="ml-6 mt-1 list-disc space-y-0.5 text-xs">
        {sorted.map((f, i) => (
          <li key={i} className="select-text">
            {f.message}
          </li>
        ))}
      </ul>
    </div>
  );
}

/** Checks on what the torrent contains. */
export function SafetyPanel({ t }: { t: TorrentSummary }) {
  const [safety, setSafety] = useState<Safety | null>(null);
  const reload = useCallback(
    () =>
      api.safety(t.id).then(
        (s) => {
          setSafety(s);
          return s;
        },
        () => null,
      ),
    [t.id],
  );
  useEffect(() => setSafety(null), [t.id]);
  useEffect(() => {
    reload();
  }, [reload, t.checksum, t.completed_on, t.suspicion]);
  if (!safety) return null;
  return (
    <div className="space-y-2">
      <FindingsList findings={safety.findings} />
      <Checksums t={t} safety={safety} reload={reload} />
      {safety.secret_trackers > 0 && (
        <p className="flex items-center gap-2 rounded-md bg-raised px-2 py-1.5 text-muted">
          <KeyRound size={14} className="shrink-0" />
          {safety.secret_trackers === 1 ? "A tracker carries" : `${safety.secret_trackers} trackers carry`} your personal passkey. Copied magnet
          links leave it out, and exporting the .torrent asks first.
        </p>
      )}
    </div>
  );
}
