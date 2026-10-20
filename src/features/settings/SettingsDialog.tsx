import { open } from "@tauri-apps/plugin-dialog";
import clsx from "clsx";
import { FolderOpen, Loader2 } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { Dialog, Field, OptionalNumber, Toggle } from "../../components/ui";
import { api, errorText } from "../../lib/ipc";
import type { Settings } from "../../lib/types";
import { useApp } from "../../stores/app";
import { AssociationSection } from "./AssociationSection";
import { BackupSection } from "./BackupSection";

type Section = "downloads" | "connection" | "speed" | "bittorrent" | "storage" | "privacy" | "apps" | "backup";

const SECTIONS: { id: Section; label: string }[] = [
  { id: "downloads", label: "Downloads" },
  { id: "connection", label: "Connection" },
  { id: "speed", label: "Speed" },
  { id: "bittorrent", label: "Seeding & queue" },
  { id: "storage", label: "Disk cache" },
  { id: "privacy", label: "Privacy & security" },
  { id: "apps", label: "Default app" },
  { id: "backup", label: "Backup & restore" },
];

const MiB = 1024 * 1024;

function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <fieldset className="space-y-3">
      <legend className="mb-5 text-xs font-semibold uppercase tracking-wider text-muted">{title}</legend>
      {children}
    </fieldset>
  );
}

export function SettingsDialog({ onClose }: { onClose: () => void }) {
  const toast = useApp((s) => s.toast);
  const [s, setS] = useState<Settings | null>(null);
  const [section, setSection] = useState<Section>("downloads");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    api.settings().then(setS).catch((e) => toast("error", errorText(e)));
  }, [toast]);

  if (!s) return null;
  const set = (patch: Partial<Settings>) => setS({ ...s, ...patch });
  const seeding = (patch: Partial<Settings["seeding"]>) => setS({ ...s, seeding: { ...s.seeding, ...patch } });
  const queue = (patch: Partial<Settings["seeding"]["queue"]>) =>
    setS({ ...s, seeding: { ...s.seeding, queue: { ...s.seeding.queue, ...patch } } });
  const storage = (patch: Partial<Settings["storage"]>) => setS({ ...s, storage: { ...s.storage, ...patch } });

  const save = async () => {
    setSaving(true);
    try {
      await api.updateSettings(s);
      toast("success", "Settings saved");
      onClose();
    } catch (e) {
      toast("error", errorText(e));
      setSaving(false);
    }
  };

  return (
    <Dialog
      title="Settings"
      onClose={onClose}
      width="max-w-4xl"
      footer={
        <>
          <button className="btn-ghost" onClick={onClose}>
            Cancel
          </button>
          <button className="btn-primary" disabled={saving} onClick={save}>
            {saving && <Loader2 size={14} className="animate-spin" />} Save
          </button>
        </>
      }
    >
      <div className="flex min-h-[26rem] gap-6">
        <nav className="w-44 shrink-0 space-y-0.5">
          {SECTIONS.map((x) => (
            <button
              key={x.id}
              onClick={() => setSection(x.id)}
              className={clsx(
                "block w-full rounded-md px-3 py-1.5 text-left text-sm",
                section === x.id ? "bg-accent/15 text-fg" : "text-muted hover:bg-raised hover:text-fg",
              )}
            >
              {x.label}
            </button>
          ))}
        </nav>
        <div className="flex-1 space-y-6">
          {section === "downloads" && (
            <Group title="Saving">
              <Field label="Default download folder">
                <div className="flex gap-1">
                  <input className="input" value={s.download_dir} onChange={(e) => set({ download_dir: e.target.value })} />
                  <button
                    className="btn-ghost border border-border"
                    onClick={async () => {
                      const d = await open({ directory: true, defaultPath: s.download_dir });
                      if (typeof d === "string") set({ download_dir: d });
                    }}
                  >
                    <FolderOpen size={15} />
                  </button>
                </div>
              </Field>
              <Toggle
                checked={s.create_subfolder}
                onChange={(v) => set({ create_subfolder: v })}
                label="Put multi-file torrents in their own folder"
              />
              <Toggle
                checked={s.mark_of_the_web}
                onChange={(v) => set({ mark_of_the_web: v })}
                label="Mark downloaded files as coming from the internet"
                hint="Windows: lets SmartScreen and Office warn before opening downloaded programs and documents."
              />
            </Group>
          )}
          {section === "downloads" && (
            <Group title="Window">
              <Toggle
                checked={s.close_to_tray}
                onChange={(v) => set({ close_to_tray: v })}
                label="Keep running in the tray when the window is closed"
                hint="Downloads and seeding carry on. Use Quit in the tray menu to exit."
              />
            </Group>
          )}

          {section === "connection" && (
            <>
              <Group title="Listening">
                <div className="grid grid-cols-2 gap-3">
                  <Field label="Port" hint="0 picks a random port">
                    <input
                      className="input"
                      type="number"
                      min={0}
                      max={65535}
                      value={s.listen_port}
                      onChange={(e) => set({ listen_port: Math.min(65535, Math.max(0, Number(e.target.value) || 0)) })}
                    />
                  </Field>
                  <Field label="Protocol">
                    <select className="input" value={s.transport} onChange={(e) => set({ transport: e.target.value as Settings["transport"] })}>
                      <option value="tcp">TCP</option>
                      <option value="tcp_and_utp">TCP and µTP (experimental)</option>
                    </select>
                  </Field>
                </div>
                <Field label="Listen on address" hint="Leave empty for all network adapters. Set a VPN adapter's IP to accept peers only through the VPN.">
                  <input
                    className="input font-mono text-xs"
                    value={s.listen_address ?? ""}
                    onChange={(e) => set({ listen_address: e.target.value.trim() || null })}
                    placeholder="All adapters"
                  />
                </Field>
                <Toggle checked={s.upnp} onChange={(v) => set({ upnp: v })} label="Forward the port with UPnP" />
              </Group>
              <Group title="Peers">
                <Toggle checked={s.dht} onChange={(v) => set({ dht: v })} label="DHT (find peers without trackers)" />
                <Toggle checked={s.lsd} onChange={(v) => set({ lsd: v })} label="Local peer discovery" />
                <Field label="Max peers per torrent">
                  <OptionalNumber value={s.peer_limit} onChange={(v) => set({ peer_limit: v })} min={1} />
                </Field>
                <Field label="SOCKS5 proxy for peer connections" hint="socks5://user:pass@host:port — leave empty for none">
                  <input
                    className="input font-mono text-xs"
                    value={s.proxy_url ?? ""}
                    onChange={(e) => set({ proxy_url: e.target.value.trim() || null })}
                    placeholder="socks5://127.0.0.1:1080"
                  />
                </Field>
              </Group>
            </>
          )}

          {section === "speed" && (
            <>
              <Group title="Global limits (KiB/s)">
                <div className="grid grid-cols-2 gap-3">
                  <Field label="Download">
                    <OptionalNumber value={s.dl_limit_kib} onChange={(v) => set({ dl_limit_kib: v })} min={1} />
                  </Field>
                  <Field label="Upload">
                    <OptionalNumber value={s.ul_limit_kib} onChange={(v) => set({ ul_limit_kib: v })} min={1} />
                  </Field>
                </div>
              </Group>
              <Group title="Alternative limits (KiB/s)">
                <div className="grid grid-cols-2 gap-3">
                  <Field label="Download">
                    <OptionalNumber value={s.alt_dl_limit_kib} onChange={(v) => set({ alt_dl_limit_kib: v })} min={1} />
                  </Field>
                  <Field label="Upload">
                    <OptionalNumber value={s.alt_ul_limit_kib} onChange={(v) => set({ alt_ul_limit_kib: v })} min={1} />
                  </Field>
                </div>
                <Toggle checked={s.alt_speed_enabled} onChange={(v) => set({ alt_speed_enabled: v })} label="Use alternative limits now" />
              </Group>
            </>
          )}

          {section === "bittorrent" && (
            <>
              <Group title="Seeding limits">
                <div className="grid grid-cols-3 gap-3">
                  <Field label="Max ratio">
                    <OptionalNumber value={s.seeding.max_ratio} onChange={(v) => seeding({ max_ratio: v })} step={0.1} />
                  </Field>
                  <Field label="Max seeding time (min)">
                    <OptionalNumber value={s.seeding.max_seeding_minutes} onChange={(v) => seeding({ max_seeding_minutes: v })} />
                  </Field>
                  <Field label="Max inactive time (min)">
                    <OptionalNumber
                      value={s.seeding.max_inactive_seeding_minutes}
                      onChange={(v) => seeding({ max_inactive_seeding_minutes: v })}
                    />
                  </Field>
                </div>
                <Field label="When a limit is reached">
                  <select
                    className="input"
                    value={s.seeding.limit_action}
                    onChange={(e) => seeding({ limit_action: e.target.value as Settings["seeding"]["limit_action"] })}
                  >
                    <option value="stop">Stop torrent</option>
                    <option value="remove">Remove torrent</option>
                    <option value="remove_with_files">Remove torrent and its files</option>
                  </select>
                </Field>
              </Group>
              <Group title="Queueing">
                <Toggle checked={s.seeding.queue.enabled} onChange={(v) => queue({ enabled: v })} label="Limit how many torrents run at once" />
                <div className="grid grid-cols-3 gap-3">
                  <Field label="Active downloads">
                    <OptionalNumber value={s.seeding.queue.max_active_downloads} onChange={(v) => queue({ max_active_downloads: v })} />
                  </Field>
                  <Field label="Active uploads">
                    <OptionalNumber value={s.seeding.queue.max_active_uploads} onChange={(v) => queue({ max_active_uploads: v })} />
                  </Field>
                  <Field label="Active torrents">
                    <OptionalNumber value={s.seeding.queue.max_active_torrents} onChange={(v) => queue({ max_active_torrents: v })} />
                  </Field>
                </div>
                <Toggle
                  checked={s.seeding.queue.dont_count_slow}
                  onChange={(v) => queue({ dont_count_slow: v })}
                  label="Don't count slow torrents in these limits"
                />
                <div className="grid grid-cols-3 gap-3">
                  <Field label="Download threshold (KiB/s)">
                    <input
                      className="input"
                      type="number"
                      min={0}
                      value={Math.round(s.seeding.queue.dl_rate_threshold / 1024)}
                      onChange={(e) => queue({ dl_rate_threshold: Math.max(0, Number(e.target.value) || 0) * 1024 })}
                    />
                  </Field>
                  <Field label="Upload threshold (KiB/s)">
                    <input
                      className="input"
                      type="number"
                      min={0}
                      value={Math.round(s.seeding.queue.ul_rate_threshold / 1024)}
                      onChange={(e) => queue({ ul_rate_threshold: Math.max(0, Number(e.target.value) || 0) * 1024 })}
                    />
                  </Field>
                  <Field label="Inactivity timer (s)">
                    <input
                      className="input"
                      type="number"
                      min={0}
                      value={s.seeding.queue.inactivity_timer_secs}
                      onChange={(e) => queue({ inactivity_timer_secs: Math.max(0, Number(e.target.value) || 0) })}
                    />
                  </Field>
                </div>
              </Group>
            </>
          )}

          {section === "storage" && (
            <Group title="Disk cache">
              <p className="text-xs text-muted">
                Riptor keeps downloaded pieces in memory until they are verified, then writes each one in a few large writes.
                It also keeps recently uploaded data in memory, so peers asking for the same piece don't hit the disk again.
              </p>
              <div className="grid grid-cols-2 gap-3">
                <Field label="Read cache (MiB)" hint="0 disables it">
                  <input
                    className="input"
                    type="number"
                    min={0}
                    max={4096}
                    value={Math.round(s.storage.read_cache_bytes / MiB)}
                    onChange={(e) => storage({ read_cache_bytes: Math.min(4096, Math.max(0, Number(e.target.value) || 0)) * MiB })}
                  />
                </Field>
                <Field label="Write buffer (MiB)" hint="0 writes straight to disk">
                  <input
                    className="input"
                    type="number"
                    min={0}
                    max={4096}
                    value={Math.round(s.storage.write_buffer_bytes / MiB)}
                    onChange={(e) => storage({ write_buffer_bytes: Math.min(4096, Math.max(0, Number(e.target.value) || 0)) * MiB })}
                  />
                </Field>
                <Field label="Max open files">
                  <input
                    className="input"
                    type="number"
                    min={16}
                    max={65536}
                    value={s.storage.max_open_files}
                    onChange={(e) => storage({ max_open_files: Math.min(65536, Math.max(16, Number(e.target.value) || 16)) })}
                  />
                </Field>
                <Field label="Disk space allocation">
                  <select
                    className="input"
                    value={s.storage.preallocation}
                    onChange={(e) => storage({ preallocation: e.target.value as Settings["storage"]["preallocation"] })}
                  >
                    <option value="sparse">Sparse (allocate as data arrives)</option>
                    <option value="full">Pre-allocate full size</option>
                  </select>
                </Field>
              </div>
            </Group>
          )}

          {section === "apps" && <AssociationSection />}
          {section === "backup" && <BackupSection onSettingsRestored={() => api.settings().then(setS)} />}

          {section === "privacy" && (
            <>
              <Group title="Privacy">
                <Toggle
                  checked={s.anonymous_mode}
                  onChange={(v) => set({ anonymous_mode: v })}
                  label="Anonymous mode"
                  hint="Turns off DHT, local discovery and UPnP, and hides the client name."
                />
              </Group>
              <Group title="IP blocklist">
                <Field label="Blocklist URL" hint="https only. Peers on the list are never connected.">
                  <input
                    className="input font-mono text-xs"
                    value={s.blocklist_url ?? ""}
                    onChange={(e) => set({ blocklist_url: e.target.value.trim() || null })}
                    placeholder="https://…"
                  />
                </Field>
              </Group>
              <p className="rounded-md bg-raised p-3 text-xs text-muted">
                Peer traffic is not encrypted (the engine does not support BitTorrent protocol encryption). Use the VPN feature or a
                SOCKS5 proxy if you need to hide BitTorrent traffic from your network provider.
              </p>
            </>
          )}
        </div>
      </div>
    </Dialog>
  );
}
