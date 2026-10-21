import { open } from "@tauri-apps/plugin-dialog";
import clsx from "clsx";
import { FolderOpen, Loader2, Plus, Trash2 } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { Dialog, Field, OptionalNumber, Toggle } from "../../components/ui";
import { api, errorText } from "../../lib/ipc";
import type { Settings, WatchedFolder } from "../../lib/types";
import { useApp } from "../../stores/app";
import { AssociationSection } from "./AssociationSection";
import { BackupSection } from "./BackupSection";

type Section = "downloads" | "connection" | "speed" | "bittorrent" | "storage" | "privacy" | "apps" | "backup";

const SECTIONS: { id: Section; label: string }[] = [
  { id: "downloads", label: "Downloads" },
  { id: "connection", label: "Connection" },
  { id: "speed", label: "Speed" },
  { id: "bittorrent", label: "Seeding & Queue" },
  { id: "storage", label: "Disk Cache" },
  { id: "privacy", label: "Privacy & Security" },
  { id: "apps", label: "Default App" },
  { id: "backup", label: "Backup & Restore" },
];

const DAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

const toTime = (m: number) => `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
const fromTime = (v: string) => {
  const [h, m] = v.split(":").map(Number);
  return Number.isFinite(h) && Number.isFinite(m) ? Math.min(1439, Math.max(0, h * 60 + m)) : 0;
};

function FolderInput({ value, onChange, placeholder }: { value: string; onChange: (v: string) => void; placeholder?: string }) {
  return (
    <div className="flex gap-1">
      <input className="input" value={value} placeholder={placeholder} onChange={(e) => onChange(e.target.value)} />
      <button
        className="btn-ghost border border-border"
        onClick={async () => {
          const d = await open({ directory: true, defaultPath: value || undefined });
          if (typeof d === "string") onChange(d);
        }}
      >
        <FolderOpen size={15} />
      </button>
    </div>
  );
}

function WatchedFolders({ value, onChange }: { value: WatchedFolder[]; onChange: (v: WatchedFolder[]) => void }) {
  const [categories, setCategories] = useState<string[]>([]);
  useEffect(() => {
    api.categories().then((c) => setCategories(Object.keys(c)), () => {});
  }, []);
  const edit = (i: number, p: Partial<WatchedFolder>) => onChange(value.map((w, j) => (j === i ? { ...w, ...p } : w)));
  const add = async () => {
    const d = await open({ directory: true, title: "Watch this folder" });
    if (typeof d === "string" && !value.some((w) => w.path === d)) onChange([...value, { path: d, category: null, save_path: null }]);
  };
  return (
    <div className="space-y-2">
      {value.map((w, i) => (
        <div key={w.path} className="space-y-1.5 rounded-md border border-border p-2">
          <div className="flex items-center gap-2">
            <span className="min-w-0 flex-1 truncate font-mono text-xs" title={w.path}>
              {w.path}
            </span>
            <button className="btn-ghost p-1 hover:text-danger" onClick={() => onChange(value.filter((_, j) => j !== i))} aria-label={`Stop watching ${w.path}`}>
              <Trash2 size={14} />
            </button>
          </div>
          <div className="grid grid-cols-2 gap-2">
            <select className="input text-xs" value={w.category ?? ""} onChange={(e) => edit(i, { category: e.target.value || null })} aria-label="Category">
              <option value="">No category</option>
              {categories.map((c) => (
                <option key={c} value={c}>
                  {c}
                </option>
              ))}
            </select>
            <FolderInput value={w.save_path ?? ""} placeholder="Default save folder" onChange={(v) => edit(i, { save_path: v.trim() || null })} />
          </div>
        </div>
      ))}
      <button className="btn-ghost border border-border text-xs" onClick={add}>
        <Plus size={13} /> Add Folder
      </button>
    </div>
  );
}

const MiB = 1024 * 1024;

function AutostartToggle() {
  const toast = useApp((s) => s.toast);
  const [on, setOn] = useState<boolean | null>(null);
  useEffect(() => {
    api.autostartGet().then(setOn, () => setOn(false));
  }, []);
  return (
    <Toggle
      checked={!!on}
      disabled={on === null}
      onChange={(v) =>
        api.autostartSet(v).then(
          () => setOn(v),
          (e) => toast("error", errorText(e)),
        )
      }
      label="Start Riptor When I Sign In"
      hint="Starts in the tray. Takes effect right away."
    />
  );
}

/** One URL per line; saved when the field loses focus so typing a new line works. */
function LinesInput({ value, onChange, placeholder }: { value: string[]; onChange: (v: string[]) => void; placeholder?: string }) {
  const [text, setText] = useState(value.join("\n"));
  useEffect(() => setText(value.join("\n")), [value]);
  return (
    <textarea
      className="input h-24 font-mono text-xs"
      value={text}
      placeholder={placeholder}
      onChange={(e) => setText(e.target.value)}
      onBlur={() => onChange(text.split(/\s+/).filter(Boolean))}
      spellCheck={false}
    />
  );
}

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
  const schedule = (patch: Partial<Settings["alt_schedule"]>) => setS({ ...s, alt_schedule: { ...s.alt_schedule, ...patch } });

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
              <Field label="Default Download Folder">
                <FolderInput value={s.download_dir} onChange={(v) => set({ download_dir: v })} />
              </Field>
              <Field
                label="Keep Incomplete Downloads In"
                hint="Torrents download here and move to their save folder once complete. Leave empty to download in place."
              >
                <FolderInput
                  value={s.incomplete_dir ?? ""}
                  placeholder="Off"
                  onChange={(v) => set({ incomplete_dir: v.trim() || null })}
                />
              </Field>
              <Toggle
                checked={s.create_subfolder}
                onChange={(v) => set({ create_subfolder: v })}
                label="Put Multi-File Torrents In Their Own Folder"
              />
              <Toggle
                checked={s.add_stopped}
                onChange={(v) => set({ add_stopped: v })}
                label="Add New Torrents Stopped"
                hint="The add dialog starts with this ticked; watched folders and RSS rules follow it too."
              />
              <Field
                label="Pause Downloads Below This Much Free Space (MiB)"
                hint="Checked on the drive each download saves to. They resume once there's room again. Empty turns it off."
              >
                <OptionalNumber value={s.min_free_mib} onChange={(v) => set({ min_free_mib: v })} min={1} placeholder="Never" />
              </Field>
              <Toggle
                checked={s.move_on_category_change}
                onChange={(v) => set({ move_on_category_change: v })}
                label="Move Files When A Torrent's Category Changes"
                hint="Files go to the new category's folder, or the default folder if it has none."
              />
              <Toggle
                checked={s.mark_of_the_web}
                onChange={(v) => set({ mark_of_the_web: v })}
                label="Mark Downloaded Files As Coming From The Internet"
                hint="Windows: lets SmartScreen and Office warn before opening downloaded programs and documents."
              />
            </Group>
          )}
          {section === "downloads" && (
            <Group title="Watched Folders">
              <p className="text-xs text-muted">
                .torrent files saved to these folders are added automatically, with the folder's category and save folder. Checked every few
                seconds; added files are renamed to .added, unreadable ones to .invalid.
              </p>
              <WatchedFolders value={s.watched_folders} onChange={(v) => set({ watched_folders: v, watched_folder: null })} />
            </Group>
          )}
          {section === "downloads" && (
            <Group title="Window">
              <Toggle
                checked={s.close_to_tray}
                onChange={(v) => set({ close_to_tray: v })}
                label="Keep Running In The Tray When The Window Is Closed"
                hint="Downloads and seeding carry on. Use Quit in the tray menu to exit."
              />
              <AutostartToggle />
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
                      <option value="tcp_and_utp">TCP And µTP (Experimental)</option>
                    </select>
                  </Field>
                </div>
                <Field label="Listen On Address" hint="Leave empty for all network adapters. Set a VPN adapter's IP to accept peers only through the VPN.">
                  <input
                    className="input font-mono text-xs"
                    value={s.listen_address ?? ""}
                    onChange={(e) => set({ listen_address: e.target.value.trim() || null })}
                    placeholder="All Adapters"
                  />
                </Field>
                <Toggle checked={s.upnp} onChange={(v) => set({ upnp: v })} label="Forward The Port With UPnP" />
              </Group>
              <Group title="Peers">
                <Toggle checked={s.dht} onChange={(v) => set({ dht: v })} label="DHT (Find Peers Without Trackers)" />
                <Toggle checked={s.lsd} onChange={(v) => set({ lsd: v })} label="Local Peer Discovery" />
                <Toggle
                  checked={s.web_seeds}
                  onChange={(v) => set({ web_seeds: v })}
                  label="Web Seeds"
                  hint="Also download from HTTP servers a torrent lists. Off while a proxy or anonymous mode is in use. Local and private addresses are never contacted."
                />
                <Field label="Max Peers Per Torrent">
                  <OptionalNumber value={s.peer_limit} onChange={(v) => set({ peer_limit: v })} min={1} />
                </Field>
                <Field label="SOCKS5 Proxy For Peer Connections" hint="socks5://user:pass@host:port — leave empty for none">
                  <input
                    className="input font-mono text-xs"
                    value={s.proxy_url ?? ""}
                    onChange={(e) => set({ proxy_url: e.target.value.trim() || null })}
                    placeholder="socks5://127.0.0.1:1080"
                  />
                </Field>
              </Group>
              <Group title="Trackers">
                <Field
                  label="Add These Trackers To New Torrents"
                  hint="One per line (http, https or udp). Never added to private torrents."
                >
                  <LinesInput
                    value={s.extra_trackers}
                    onChange={(v) => set({ extra_trackers: v })}
                    placeholder="udp://tracker.example.org:1337/announce"
                  />
                </Field>
              </Group>
            </>
          )}

          {section === "speed" && (
            <>
              <Group title="Global Limits (KiB/s)">
                <div className="grid grid-cols-2 gap-3">
                  <Field label="Download">
                    <OptionalNumber value={s.dl_limit_kib} onChange={(v) => set({ dl_limit_kib: v })} min={1} />
                  </Field>
                  <Field label="Upload">
                    <OptionalNumber value={s.ul_limit_kib} onChange={(v) => set({ ul_limit_kib: v })} min={1} />
                  </Field>
                </div>
              </Group>
              <Group title="Alternative Limits (KiB/s)">
                <div className="grid grid-cols-2 gap-3">
                  <Field label="Download">
                    <OptionalNumber value={s.alt_dl_limit_kib} onChange={(v) => set({ alt_dl_limit_kib: v })} min={1} />
                  </Field>
                  <Field label="Upload">
                    <OptionalNumber value={s.alt_ul_limit_kib} onChange={(v) => set({ alt_ul_limit_kib: v })} min={1} />
                  </Field>
                </div>
                <Toggle checked={s.alt_speed_enabled} onChange={(v) => set({ alt_speed_enabled: v })} label="Use Alternative Limits Now" />
              </Group>
              <Group title="Schedule">
                <Toggle
                  checked={s.alt_schedule.enabled}
                  onChange={(v) => schedule({ enabled: v })}
                  label="Use Alternative Limits On A Schedule"
                  hint="They switch on when the window starts and off when it ends. You can still change them by hand in between."
                />
                <div className="grid grid-cols-2 gap-3">
                  <Field label="From">
                    <input
                      className="input"
                      type="time"
                      disabled={!s.alt_schedule.enabled}
                      value={toTime(s.alt_schedule.from)}
                      onChange={(e) => schedule({ from: fromTime(e.target.value) })}
                    />
                  </Field>
                  <Field label="To" hint={s.alt_schedule.from === s.alt_schedule.to ? "Same time: all day" : undefined}>
                    <input
                      className="input"
                      type="time"
                      disabled={!s.alt_schedule.enabled}
                      value={toTime(s.alt_schedule.to)}
                      onChange={(e) => schedule({ to: fromTime(e.target.value) })}
                    />
                  </Field>
                </div>
                <div className="flex gap-1" role="group" aria-label="Days">
                  {DAYS.map((d, i) => {
                    const on = (s.alt_schedule.days & (1 << i)) !== 0;
                    return (
                      <button
                        key={d}
                        type="button"
                        aria-pressed={on}
                        disabled={!s.alt_schedule.enabled}
                        onClick={() => schedule({ days: s.alt_schedule.days ^ (1 << i) })}
                        className={clsx(
                          "w-11 rounded-md border px-2 py-1 text-xs disabled:opacity-50",
                          on ? "border-accent bg-accent/15 text-fg" : "border-border text-muted hover:text-fg",
                        )}
                      >
                        {d}
                      </button>
                    );
                  })}
                </div>
              </Group>
            </>
          )}

          {section === "bittorrent" && (
            <>
              <Group title="Seeding Limits">
                <div className="grid grid-cols-3 gap-3">
                  <Field label="Max Ratio">
                    <OptionalNumber value={s.seeding.max_ratio} onChange={(v) => seeding({ max_ratio: v })} step={0.1} />
                  </Field>
                  <Field label="Max Seeding Time (Minutes)">
                    <OptionalNumber value={s.seeding.max_seeding_minutes} onChange={(v) => seeding({ max_seeding_minutes: v })} />
                  </Field>
                  <Field label="Max Inactive Time (Minutes)">
                    <OptionalNumber
                      value={s.seeding.max_inactive_seeding_minutes}
                      onChange={(v) => seeding({ max_inactive_seeding_minutes: v })}
                    />
                  </Field>
                </div>
                <Field label="When A Limit Is Reached">
                  <select
                    className="input"
                    value={s.seeding.limit_action}
                    onChange={(e) => seeding({ limit_action: e.target.value as Settings["seeding"]["limit_action"] })}
                  >
                    <option value="stop">Stop Torrent</option>
                    <option value="remove">Remove Torrent</option>
                    <option value="remove_with_files">Remove Torrent And Its Files</option>
                  </select>
                </Field>
              </Group>
              <Group title="Queueing">
                <Toggle checked={s.seeding.queue.enabled} onChange={(v) => queue({ enabled: v })} label="Limit How Many Torrents Run At Once" />
                <div className="grid grid-cols-3 gap-3">
                  <Field label="Active Downloads">
                    <OptionalNumber value={s.seeding.queue.max_active_downloads} onChange={(v) => queue({ max_active_downloads: v })} />
                  </Field>
                  <Field label="Active Uploads">
                    <OptionalNumber value={s.seeding.queue.max_active_uploads} onChange={(v) => queue({ max_active_uploads: v })} />
                  </Field>
                  <Field label="Active Torrents">
                    <OptionalNumber value={s.seeding.queue.max_active_torrents} onChange={(v) => queue({ max_active_torrents: v })} />
                  </Field>
                </div>
                <Toggle
                  checked={s.seeding.queue.dont_count_slow}
                  onChange={(v) => queue({ dont_count_slow: v })}
                  label="Don't Count Slow Torrents In These Limits"
                />
                <div className="grid grid-cols-3 gap-3">
                  <Field label="Download Threshold (KiB/s)">
                    <input
                      className="input"
                      type="number"
                      min={0}
                      value={Math.round(s.seeding.queue.dl_rate_threshold / 1024)}
                      onChange={(e) => queue({ dl_rate_threshold: Math.max(0, Number(e.target.value) || 0) * 1024 })}
                    />
                  </Field>
                  <Field label="Upload Threshold (KiB/s)">
                    <input
                      className="input"
                      type="number"
                      min={0}
                      value={Math.round(s.seeding.queue.ul_rate_threshold / 1024)}
                      onChange={(e) => queue({ ul_rate_threshold: Math.max(0, Number(e.target.value) || 0) * 1024 })}
                    />
                  </Field>
                  <Field label="Inactivity Timer (Seconds)">
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
            <Group title="Disk Cache">
              <p className="text-xs text-muted">
                Riptor keeps downloaded pieces in memory until they are verified, then writes each one in a few large writes.
                It also keeps recently uploaded data in memory, so peers asking for the same piece don't hit the disk again.
              </p>
              <div className="grid grid-cols-2 gap-3">
                <Field label="Read Cache (MiB)" hint="0 disables it">
                  <input
                    className="input"
                    type="number"
                    min={0}
                    max={4096}
                    value={Math.round(s.storage.read_cache_bytes / MiB)}
                    onChange={(e) => storage({ read_cache_bytes: Math.min(4096, Math.max(0, Number(e.target.value) || 0)) * MiB })}
                  />
                </Field>
                <Field label="Write Buffer (MiB)" hint="0 writes straight to disk">
                  <input
                    className="input"
                    type="number"
                    min={0}
                    max={4096}
                    value={Math.round(s.storage.write_buffer_bytes / MiB)}
                    onChange={(e) => storage({ write_buffer_bytes: Math.min(4096, Math.max(0, Number(e.target.value) || 0)) * MiB })}
                  />
                </Field>
                <Field label="Max Open Files">
                  <input
                    className="input"
                    type="number"
                    min={16}
                    max={65536}
                    value={s.storage.max_open_files}
                    onChange={(e) => storage({ max_open_files: Math.min(65536, Math.max(16, Number(e.target.value) || 16)) })}
                  />
                </Field>
                <Field label="Disk Space Allocation">
                  <select
                    className="input"
                    value={s.storage.preallocation}
                    onChange={(e) => storage({ preallocation: e.target.value as Settings["storage"]["preallocation"] })}
                  >
                    <option value="sparse">Sparse (Allocate As Data Arrives)</option>
                    <option value="full">Pre-Allocate Full Size</option>
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
                  label="Anonymous Mode"
                  hint="Turns off DHT, local discovery and UPnP, and hides the client name."
                />
              </Group>
              <Group title="IP Blocklist">
                <Field label="Blocklist File" hint="A local .p2p, .dat or plain list; used instead of the URL when set.">
                  <div className="flex gap-1">
                    <input className="input font-mono text-xs" value={s.blocklist_file ?? ""} placeholder="None" onChange={(e) => set({ blocklist_file: e.target.value.trim() || null })} />
                    <button
                      className="btn-ghost border border-border"
                      onClick={async () => {
                        const f = await open({ multiple: false });
                        if (typeof f === "string") set({ blocklist_file: f });
                      }}
                    >
                      <FolderOpen size={15} />
                    </button>
                  </div>
                </Field>
                <Field label="Blocklist URL" hint="https only. Peers on the list are never connected.">
                  <input
                    className="input font-mono text-xs"
                    value={s.blocklist_url ?? ""}
                    onChange={(e) => set({ blocklist_url: e.target.value.trim() || null })}
                    placeholder="https://…"
                  />
                </Field>
              </Group>
              <Group title="Banned Peers">
                {s.banned_ips.length === 0 ? (
                  <p className="text-xs text-muted">None. Ban a peer from a torrent's Peers tab.</p>
                ) : (
                  <ul className="space-y-1">
                    {s.banned_ips.map((ip) => (
                      <li key={ip} className="flex items-center gap-2 text-xs">
                        <span className="flex-1 font-mono">{ip}</span>
                        <button className="btn-ghost py-0.5 text-xs" onClick={() => set({ banned_ips: s.banned_ips.filter((x) => x !== ip) })}>
                          Unban
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
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
