import { open, save } from "@tauri-apps/plugin-dialog";
import { AlertTriangle, Download, KeyRound, Loader2, ShieldCheck, Upload } from "lucide-react";
import { useState } from "react";
import { Field, Toggle } from "../../components/ui";
import { api, errorText } from "../../lib/ipc";
import { formatBytes, formatDate } from "../../lib/format";
import type { BackupSummary, RestoreOptions, RestoreReport } from "../../lib/types";
import { useApp } from "../../stores/app";

const EXT = "riptorbackup";

function defaultName() {
  return `riptor-${new Date().toISOString().slice(0, 10)}.${EXT}`;
}

export function BackupSection({ onSettingsRestored }: { onSettingsRestored: () => void }) {
  const toast = useApp((s) => s.toast);
  const [busy, setBusy] = useState<"backup" | "inspect" | "restore" | null>(null);

  const [usePassword, setUsePassword] = useState(false);
  const [pw, setPw] = useState("");
  const [pw2, setPw2] = useState("");
  const pwOk = !usePassword || (pw.length >= 8 && pw === pw2);

  const [file, setFile] = useState<string | null>(null);
  const [needsPw, setNeedsPw] = useState(false);
  const [restorePw, setRestorePw] = useState("");
  const [summary, setSummary] = useState<BackupSummary | null>(null);
  const [opts, setOpts] = useState<RestoreOptions>({ settings: true, torrents: true, statistics: true, resume_torrents: false });
  const [report, setReport] = useState<RestoreReport | null>(null);

  const createBackup = async () => {
    const path = await save({ defaultPath: defaultName(), filters: [{ name: "Riptor backup", extensions: [EXT] }] });
    if (!path) return;
    setBusy("backup");
    try {
      const s = await api.createBackup(path, usePassword ? pw : null);
      toast("success", `Backup saved: ${s.torrents} torrents, settings and statistics`);
      setPw("");
      setPw2("");
    } catch (e) {
      toast("error", errorText(e));
    } finally {
      setBusy(null);
    }
  };

  const inspect = async (path: string, password: string | null) => {
    setBusy("inspect");
    setSummary(null);
    setReport(null);
    try {
      const s = await api.inspectBackup(path, password);
      setSummary(s);
      setNeedsPw(s.password_protected);
      // With a password we know the backup is theirs, so torrents can resume.
      setOpts((o) => ({ ...o, resume_torrents: s.password_protected }));
    } catch (e) {
      const msg = errorText(e);
      if (msg === "password_required") setNeedsPw(true);
      else toast("error", msg);
    } finally {
      setBusy(null);
    }
  };

  const pickFile = async () => {
    const picked = await open({ filters: [{ name: "Riptor backup", extensions: [EXT] }] });
    if (typeof picked !== "string") return;
    setFile(picked);
    setNeedsPw(false);
    setRestorePw("");
    await inspect(picked, null);
  };

  const restore = async () => {
    if (!file) return;
    setBusy("restore");
    try {
      const r = await api.restoreBackup(file, needsPw ? restorePw : null, opts);
      setReport(r);
      if (r.settings_restored) onSettingsRestored();
      toast("success", `Restored ${r.added} torrents`);
    } catch (e) {
      toast("error", errorText(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="space-y-6">
      <fieldset className="space-y-3">
        <legend className="mb-2 text-xs font-semibold uppercase tracking-wider text-muted">Create a backup</legend>
        <p className="text-xs text-muted">
          Saves your torrents (with their .torrent data, save folders and statistics), categories, settings and client
          statistics into one encrypted file. Any change to the file is detected and the restore is refused.
        </p>
        <Toggle
          checked={usePassword}
          onChange={setUsePassword}
          label="Protect with a password"
          hint="Without a password any copy of Riptor can open the backup. With one, only someone who knows it can read or forge it."
        />
        {usePassword && (
          <div className="grid grid-cols-2 gap-3">
            <Field label="Password" hint="At least 8 characters. It cannot be recovered if lost.">
              <input className="input" type="password" value={pw} onChange={(e) => setPw(e.target.value)} autoComplete="new-password" />
            </Field>
            <Field label="Repeat password" hint={pw2 && pw !== pw2 ? "Passwords don't match" : undefined}>
              <input className="input" type="password" value={pw2} onChange={(e) => setPw2(e.target.value)} autoComplete="new-password" />
            </Field>
          </div>
        )}
        <button className="btn-primary" disabled={!pwOk || busy !== null} onClick={createBackup}>
          {busy === "backup" ? <Loader2 size={14} className="animate-spin" /> : <Download size={14} />} Create backup…
        </button>
      </fieldset>

      <fieldset className="space-y-3">
        <legend className="mb-2 text-xs font-semibold uppercase tracking-wider text-muted">Restore</legend>
        <button className="btn-ghost border border-border" disabled={busy !== null} onClick={pickFile}>
          {busy === "inspect" ? <Loader2 size={14} className="animate-spin" /> : <Upload size={14} />} Choose backup file…
        </button>

        {file && needsPw && !summary && (
          <div className="flex items-end gap-2">
            <Field label="This backup is password protected">
              <input
                className="input"
                type="password"
                value={restorePw}
                onChange={(e) => setRestorePw(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && inspect(file, restorePw)}
                autoFocus
              />
            </Field>
            <button className="btn-primary" disabled={!restorePw || busy !== null} onClick={() => inspect(file, restorePw)}>
              <KeyRound size={14} /> Unlock
            </button>
          </div>
        )}

        {summary && (
          <div className="card space-y-3 p-3 text-[13px]">
            <div className="flex items-center gap-2">
              <ShieldCheck size={16} className="text-ok" />
              <span>
                Verified backup from {formatDate(summary.created)} (Riptor {summary.app_version})
                {summary.password_protected ? ", password protected" : ""}
              </span>
            </div>
            <p className="text-muted">
              {summary.torrents} torrents, {summary.categories} categories · all-time ↓ {formatBytes(summary.all_time_download)} ↑{" "}
              {formatBytes(summary.all_time_upload)}
            </p>
            {summary.save_folders.length > 0 && (
              <details>
                <summary className="cursor-pointer text-muted">Torrents will be saved to {summary.save_folders.length} folder(s)</summary>
                <ul className="mt-1 max-h-28 overflow-auto font-mono text-xs">
                  {summary.save_folders.map((f) => (
                    <li key={f}>{f}</li>
                  ))}
                </ul>
              </details>
            )}
            <div className="grid grid-cols-3 gap-x-4">
              <Toggle checked={opts.torrents} onChange={(v) => setOpts({ ...opts, torrents: v })} label="Torrents" />
              <Toggle checked={opts.settings} onChange={(v) => setOpts({ ...opts, settings: v })} label="Settings" />
              <Toggle checked={opts.statistics} onChange={(v) => setOpts({ ...opts, statistics: v })} label="Statistics" />
            </div>
            {opts.torrents && (
              <Toggle
                checked={opts.resume_torrents}
                onChange={(v) => setOpts({ ...opts, resume_torrents: v })}
                label="Resume torrents that were running"
                hint={
                  summary.password_protected
                    ? undefined
                    : "Off by default for backups without a password: check the save folders above first, then resume."
                }
              />
            )}
            {!summary.password_protected && opts.resume_torrents && (
              <p className="flex items-center gap-2 text-xs text-warn">
                <AlertTriangle size={13} /> Only resume if you made this backup yourself.
              </p>
            )}
            <button className="btn-primary" disabled={busy !== null || (!opts.torrents && !opts.settings && !opts.statistics)} onClick={restore}>
              {busy === "restore" && <Loader2 size={14} className="animate-spin" />} Restore
            </button>
          </div>
        )}

        {report && (
          <div className="card space-y-1 p-3 text-[13px]">
            <p>
              Added {report.added} torrents{report.already_present ? `, ${report.already_present} were already in your list` : ""}.
              {report.settings_restored && " Settings restored."}
              {report.statistics_restored && " Statistics restored."}
            </p>
            {report.skipped.length > 0 && (
              <details open>
                <summary className="cursor-pointer text-warn">{report.skipped.length} item(s) skipped</summary>
                <ul className="mt-1 max-h-28 list-inside list-disc overflow-auto text-xs text-muted">
                  {report.skipped.map((s, i) => (
                    <li key={i}>{s}</li>
                  ))}
                </ul>
              </details>
            )}
          </div>
        )}
      </fieldset>
    </div>
  );
}
