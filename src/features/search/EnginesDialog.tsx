import { open } from "@tauri-apps/plugin-dialog";
import { AlertTriangle, ExternalLink, FileCode2, Globe, KeyRound, Loader2, Plus, RefreshCw, ShieldAlert, Trash2 } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Dialog, Field, Toggle } from "../../components/ui";
import { api, errorText } from "../../lib/ipc";
import { formatBytes } from "../../lib/format";
import type { PluginPreview, ProviderInfo, PythonStatus } from "../../lib/types";
import { useApp } from "../../stores/app";

const KIND_LABEL = { builtin: "Built in", torznab: "Torznab", plugin: "Plugin" } as const;

function TorznabForm({ onAdded }: { onAdded: () => void }) {
  const toast = useApp((s) => s.toast);
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const add = async () => {
    setBusy(true);
    try {
      await api.addTorznab(name, url, key);
      setName("");
      setUrl("");
      setKey("");
      onAdded();
      toast("success", "Indexer added");
    } catch (e) {
      toast("error", errorText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="space-y-2">
      <div className="grid grid-cols-[1fr_2fr] gap-2">
        <Field label="Name">
          <input className="input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Jackett" maxLength={64} />
        </Field>
        <Field label="Torznab feed URL" hint="From Jackett/Prowlarr: “Copy Torznab Feed”. https, or http on this computer/your network.">
          <input
            className="input font-mono text-xs"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder="http://127.0.0.1:9117/api/v2.0/indexers/all/results/torznab/"
          />
        </Field>
      </div>
      <Field label="API key" hint="Stored in the system credential store, not in Riptor's files or backups.">
        <input className="input font-mono text-xs" type="password" value={key} onChange={(e) => setKey(e.target.value)} autoComplete="off" />
      </Field>
      <button className="btn-primary" disabled={busy || !name.trim() || !url.trim() || !key.trim()} onClick={add}>
        {busy ? <Loader2 size={14} className="animate-spin" /> : <KeyRound size={14} />} Add indexer
      </button>
    </div>
  );
}

function PluginInstaller({ python, onInstalled }: { python: PythonStatus | null; onInstalled: () => void }) {
  const toast = useApp((s) => s.toast);
  const [url, setUrl] = useState("");
  const [preview, setPreview] = useState<PluginPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirmed, setConfirmed] = useState(false);

  const inspect = async (source: string) => {
    setBusy(true);
    setPreview(null);
    setConfirmed(false);
    try {
      setPreview(await api.previewPlugin(source));
    } catch (e) {
      toast("error", errorText(e));
    } finally {
      setBusy(false);
    }
  };
  const pickFile = async () => {
    const f = await open({ filters: [{ name: "Python search plugin", extensions: ["py"] }] });
    if (typeof f === "string") inspect(f);
  };
  const install = async () => {
    if (!preview) return;
    setBusy(true);
    try {
      await api.installPlugin(preview.sha256);
      toast("success", `Installed ${preview.declared_name ?? preview.id}`);
      setPreview(null);
      setUrl("");
      onInstalled();
    } catch (e) {
      toast("error", errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="space-y-2">
      {python && !python.available && (
        <p className="flex items-start gap-2 rounded-md bg-warn/10 px-3 py-2 text-xs text-warn">
          <AlertTriangle size={14} className="mt-0.5 shrink-0" />
          <span>
            qBittorrent plugins need Python 3.9 or newer, which isn't installed.{" "}
            <button className="underline" onClick={() => api.openWebUrl("https://www.python.org/downloads/").catch(() => {})}>
              Get Python
            </button>
            , then press “Check again”. The Internet Archive and Torznab indexers work without it.
          </span>
        </p>
      )}
      <div className="flex gap-2">
        <input
          className="input font-mono text-xs"
          placeholder="https://raw.githubusercontent.com/…/engine.py"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
        />
        <button className="btn-ghost border border-border" disabled={busy || !url.startsWith("https://")} onClick={() => inspect(url.trim())}>
          <Globe size={14} /> From URL
        </button>
        <button className="btn-ghost border border-border" disabled={busy} onClick={pickFile}>
          <FileCode2 size={14} /> From file
        </button>
      </div>
      {preview && (
        <div className="card space-y-2 border-warn/40 p-3 text-[13px]">
          <p className="flex items-center gap-2 font-medium text-warn">
            <ShieldAlert size={16} /> Plugins are programs. Only install ones you trust.
          </p>
          <dl className="grid grid-cols-[7rem_1fr] gap-x-3 gap-y-1">
            <dt className="text-muted">Name</dt>
            <dd>{preview.declared_name ?? preview.id}</dd>
            <dt className="text-muted">Site</dt>
            <dd className="truncate">{preview.declared_url ?? "–"}</dd>
            <dt className="text-muted">Source</dt>
            <dd className="select-text truncate font-mono text-xs" title={preview.source}>
              {preview.source}
            </dd>
            <dt className="text-muted">SHA-256</dt>
            <dd className="select-text break-all font-mono text-xs">{preview.sha256}</dd>
            <dt className="text-muted">Size</dt>
            <dd>{formatBytes(preview.size)}</dd>
          </dl>
          {preview.replaces_existing && <p className="text-xs text-warn">This replaces an installed plugin with the same name.</p>}
          <Toggle
            checked={confirmed}
            onChange={setConfirmed}
            label="I trust this plugin and its source"
            hint="It will run on your computer whenever you search with it, without administrator rights."
          />
          <div className="flex gap-2">
            <button className="btn-primary" disabled={!confirmed || busy} onClick={install}>
              Install
            </button>
            <button className="btn-ghost" onClick={() => setPreview(null)}>
              Cancel
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

export function EnginesDialog({ onClose }: { onClose: () => void }) {
  const toast = useApp((s) => s.toast);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [python, setPython] = useState<PythonStatus | null>(null);
  const [checking, setChecking] = useState(false);

  const reload = useCallback(() => {
    api.searchProviders().then(setProviders).catch((e) => toast("error", errorText(e)));
  }, [toast]);
  useEffect(() => {
    reload();
    api.pythonStatus(false).then(setPython).catch(() => {});
  }, [reload]);

  const recheck = async () => {
    setChecking(true);
    try {
      setPython(await api.pythonStatus(true));
      reload();
    } finally {
      setChecking(false);
    }
  };
  const toggle = async (p: ProviderInfo, enabled: boolean) => {
    try {
      await api.searchSetEnabled(p.id, enabled);
      reload();
    } catch (e) {
      toast("error", errorText(e));
    }
  };
  const remove = async (p: ProviderInfo) => {
    try {
      if (p.kind === "torznab") await api.removeTorznab(p.id);
      else await api.uninstallPlugin(p.id);
      reload();
    } catch (e) {
      toast("error", errorText(e));
    }
  };

  return (
    <Dialog title="Search engines" onClose={onClose} width="max-w-3xl">
      <div className="space-y-6">
        <section>
          <div className="mb-2 flex items-center justify-between">
            <h3 className="text-xs font-semibold uppercase tracking-wider text-muted">Installed</h3>
            <span className="flex items-center gap-2 text-xs text-muted">
              {python?.available ? `Python ${python.version}${python.sandboxed ? " · sandboxed" : ""}` : "Python not found"}
              <button className="btn-ghost px-1.5 py-0.5 text-xs" onClick={recheck} disabled={checking}>
                <RefreshCw size={12} className={checking ? "animate-spin" : ""} /> Check again
              </button>
            </span>
          </div>
          <ul className="card divide-y divide-border">
            {providers.map((p) => (
              <li key={p.id} className="flex items-center gap-3 px-3 py-2 text-[13px]">
                <div className="min-w-0 flex-1">
                  <p className="flex items-center gap-2">
                    <span className="truncate font-medium">{p.name}</span>
                    <span className="rounded bg-raised px-1.5 py-0.5 text-xxs text-muted">{KIND_LABEL[p.kind]}</span>
                  </p>
                  {p.error ? (
                    <p className="truncate text-xs text-danger" title={p.error}>
                      {p.error}
                    </p>
                  ) : (
                    <p className="truncate text-xs text-muted">{p.url}</p>
                  )}
                  {p.id === "archive" && (
                    <p className="text-xs text-muted">
                      Archive items are mostly served by web seeds, which Riptor does not support yet, so some downloads may be slow.
                    </p>
                  )}
                </div>
                {p.url.startsWith("https://") && (
                  <button className="btn-ghost p-1" title="Open site" onClick={() => api.openWebUrl(p.url).catch(() => {})}>
                    <ExternalLink size={13} />
                  </button>
                )}
                {p.kind !== "builtin" && (
                  <button className="btn-ghost p-1 hover:text-danger" title="Remove" onClick={() => remove(p)}>
                    <Trash2 size={13} />
                  </button>
                )}
                <Toggle checked={p.enabled} onChange={(v) => toggle(p, v)} label="" disabled={!!p.error} />
              </li>
            ))}
          </ul>
        </section>

        <section>
          <h3 className="mb-2 flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-muted">
            <Plus size={12} /> Jackett / Prowlarr indexer
          </h3>
          <TorznabForm onAdded={reload} />
        </section>

        <section>
          <h3 className="mb-2 flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-muted">
            <Plus size={12} /> qBittorrent search plugin
          </h3>
          <PluginInstaller python={python} onInstalled={reload} />
        </section>
      </div>
    </Dialog>
  );
}
