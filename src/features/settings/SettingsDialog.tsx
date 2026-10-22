import { open } from "@tauri-apps/plugin-dialog";
import clsx from "clsx";
import { FolderOpen, Loader2, Plus, Trash2 } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { Dialog, Field, OptionalNumber, Toggle } from "../../components/ui";
import { api, errorText } from "../../lib/ipc";
import type { CountryStatus, DataCap, NetInfo, Settings, WatchedFolder } from "../../lib/types";
import { useApp } from "../../stores/app";
import { AboutSection } from "./AboutSection";
import { AssociationSection } from "./AssociationSection";
import { BackupSection } from "./BackupSection";
import { RemoteControlSection } from "./RemoteControlSection";

export type Section = "downloads" | "connection" | "speed" | "bittorrent" | "storage" | "privacy" | "apps" | "backup" | "remote" | "about";

const SECTIONS: { id: Section; label: string }[] = [
  { id: "downloads", label: "Downloads" },
  { id: "connection", label: "Connection" },
  { id: "speed", label: "Speed" },
  { id: "bittorrent", label: "Seeding & Queue" },
  { id: "storage", label: "Disk Cache" },
  { id: "privacy", label: "Privacy & Security" },
  { id: "apps", label: "Default App" },
  { id: "backup", label: "Backup & Restore" },
  { id: "remote", label: "Remote Control" },
  { id: "about", label: "About" },
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
        <Plus size={13} /> Add folder
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
      label="Start Riptor when I sign in"
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

/** Networks where torrents may run without the VPN. */
function TrustedNetworks({ s, set }: { s: Settings; set: (patch: Partial<Settings>) => void }) {
  const [net, setNet] = useState<NetInfo | null>(null);
  useEffect(() => {
    api.networkInfo().then(setNet, () => {});
  }, []);
  const here = net?.names ?? [];
  const trusted = here.some((n) => s.trusted_networks.includes(n));
  return (
    <Group title="Untrusted networks">
      <Toggle
        checked={s.require_vpn_untrusted}
        onChange={(v) => set({ require_vpn_untrusted: v })}
        label="Require the VPN on networks I haven't trusted"
        hint="On any other network (a café, a hotel, a friend's Wi-Fi) the kill switch holds torrents until the VPN connects."
      />
      <p className="text-xs text-muted">
        {here.length === 0
          ? "Riptor can't tell which network this is, so it counts as untrusted."
          : `You're on ${here.join(", ")}${trusted ? ", which you trust." : ", which you haven't trusted."}`}
      </p>
      {here.length > 0 && !trusted && (
        <button
          className="btn-ghost border border-border text-xs"
          onClick={() => set({ trusted_networks: [...s.trusted_networks, ...here.filter((n) => !s.trusted_networks.includes(n))] })}
        >
          <Plus size={13} /> Trust this network
        </button>
      )}
      {s.trusted_networks.length > 0 && (
        <ul className="space-y-1">
          {s.trusted_networks.map((n) => (
            <li key={n} className="flex items-center gap-2 text-xs">
              <span className="flex-1 truncate">{n}</span>
              <button className="btn-ghost py-0.5 text-xs" onClick={() => set({ trusted_networks: s.trusted_networks.filter((x) => x !== n) })}>
                Remove
              </button>
            </li>
          ))}
        </ul>
      )}
    </Group>
  );
}

/** Metered connections and a monthly cap. */
function DataUse({ s, set }: { s: Settings; set: (patch: Partial<Settings>) => void }) {
  const [net, setNet] = useState<NetInfo | null>(null);
  useEffect(() => {
    api.networkInfo().then(setNet, () => {});
  }, []);
  const cap = s.data_cap;
  const editCap = (patch: Partial<DataCap>) => cap && set({ data_cap: { ...cap, ...patch } });
  const now =
    net?.metered == null ? "Riptor can't tell whether this connection is metered." : net.metered ? "This connection is metered now." : "This connection isn't metered.";
  return (
    <Group title="Data use">
      <Toggle
        checked={s.pause_on_metered}
        onChange={(v) => set({ pause_on_metered: v })}
        label="Pause torrents on metered connections"
        hint={`Uses what ${navigator.userAgent.includes("Windows") ? "Windows" : "NetworkManager"} says about the network. ${now}`}
      />
      <Toggle
        checked={!!cap}
        onChange={(v) => set({ data_cap: v ? { gib: 100, counts: "both", reset_day: 1 } : null })}
        label="Limit how much Riptor transfers each month"
        hint="Torrents wait once the limit is reached and start again when the month resets. Counted from the statistics, in UTC."
      />
      {cap && (
        <div className="grid grid-cols-3 gap-3">
          <Field label="Limit (GiB)">
            <input
              className="input"
              type="number"
              min={1}
              value={cap.gib}
              onChange={(e) => editCap({ gib: Math.min(1_000_000, Math.max(1, Math.round(Number(e.target.value)) || 1)) })}
            />
          </Field>
          <Field label="Counts">
            <select className="input" value={cap.counts} onChange={(e) => editCap({ counts: e.target.value as DataCap["counts"] })}>
              <option value="both">Downloads and uploads</option>
              <option value="download">Downloads only</option>
              <option value="upload">Uploads only</option>
            </select>
          </Field>
          <Field label="Resets on day">
            <input
              className="input"
              type="number"
              min={1}
              max={28}
              value={cap.reset_day}
              onChange={(e) => editCap({ reset_day: Math.min(28, Math.max(1, Math.round(Number(e.target.value)) || 1)) })}
            />
          </Field>
        </div>
      )}
    </Group>
  );
}

/** Country codes next to peers, from DB-IP's free list. */
function PeerCountries({ s, set }: { s: Settings; set: (patch: Partial<Settings>) => void }) {
  const [status, setStatus] = useState<CountryStatus | null>(null);
  useEffect(() => {
    let alive = true;
    const load = () => api.countryStatus().then((v) => alive && setStatus(v), () => {});
    load();
    const t = window.setInterval(load, 3000);
    return () => {
      alive = false;
      window.clearInterval(t);
    };
  }, []);
  const saved = status?.enabled;
  let line: string | null = null;
  if (s.peer_countries && saved) {
    if (status?.ready && status.updated) line = `Country list downloaded ${new Date(status.updated * 1000).toLocaleDateString()}.`;
    else if (status?.error) line = `Couldn't get the country list: ${status.error}`;
    else line = "Getting the country list…";
  } else if (s.peer_countries) line = "The country list is downloaded once you save.";
  else if (saved) line = "Saving turns this off and deletes the downloaded list.";
  return (
    <>
      <Toggle
        checked={s.peer_countries}
        onChange={(v) => set({ peer_countries: v })}
        label="Show peer countries"
        hint="Downloads a free country list (about 4.5 MB) from db-ip.com, through the VPN when it's on, and refreshes it monthly. Lookups happen on this computer."
      />
      {line && <p className={clsx("text-xs", status?.error && s.peer_countries ? "text-danger" : "text-muted")}>{line}</p>}
      {s.peer_countries && <p className="text-xs text-muted">{status?.attribution ?? "IP geolocation by DB-IP"}</p>}
    </>
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

export function SettingsDialog({ onClose, initialSection = "downloads" }: { onClose: () => void; initialSection?: Section }) {
  const toast = useApp((s) => s.toast);
  const [s, setS] = useState<Settings | null>(null);
  const [section, setSection] = useState<Section>(initialSection);
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
      {/* Fixed height so switching sections doesn't resize the dialog; the section scrolls instead. */}
      <div className="flex h-[min(40rem,calc(90vh-9rem))] gap-6">
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
        <div key={section} className="-mr-2 min-w-0 flex-1 space-y-6 overflow-y-auto pr-2">
          {section === "downloads" && (
            <Group title="Saving">
              <Field label="Default download folder">
                <FolderInput value={s.download_dir} onChange={(v) => set({ download_dir: v })} />
              </Field>
              <Field
                label="Keep incomplete downloads in"
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
                label="Put multi-file torrents in their own folder"
              />
              <Toggle
                checked={s.add_stopped}
                onChange={(v) => set({ add_stopped: v })}
                label="Add new torrents stopped"
                hint="The add dialog starts with this ticked; watched folders and RSS rules follow it too."
              />
              <Field
                label="Pause downloads below this much free space (MiB)"
                hint="Checked on the drive each download saves to. They resume once there's room again. Empty turns it off."
              >
                <OptionalNumber value={s.min_free_mib} onChange={(v) => set({ min_free_mib: v })} min={1} placeholder="Never" />
              </Field>
              <Toggle
                checked={s.move_on_category_change}
                onChange={(v) => set({ move_on_category_change: v })}
                label="Move files when a torrent's category changes"
                hint="Files go to the new category's folder, or the default folder if it has none."
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
            <Group title="Watched folders">
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
                label="Keep running in the tray when the window is closed"
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
                <Field
                  label="Encrypt peer traffic"
                  hint="Hides BitTorrent traffic from throttling and simple inspection. It isn't real privacy: the method is weak and anyone in the middle can still read it. Use the VPN for that."
                >
                  <select className="input" value={s.encryption} onChange={(e) => set({ encryption: e.target.value as Settings["encryption"] })}>
                    <option value="prefer">When the peer can (recommended)</option>
                    <option value="require">Always (skip peers that can't)</option>
                    <option value="off">Never</option>
                  </select>
                </Field>
                <Toggle checked={s.dht} onChange={(v) => set({ dht: v })} label="DHT (find peers without trackers)" />
                <Toggle checked={s.lsd} onChange={(v) => set({ lsd: v })} label="Local peer discovery" />
                <Toggle
                  checked={s.web_seeds}
                  onChange={(v) => set({ web_seeds: v })}
                  label="Web seeds"
                  hint="Also download from HTTP servers a torrent lists. Off while a proxy or anonymous mode is in use. Local and private addresses are never contacted."
                />
                <PeerCountries s={s} set={set} />
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
              <Group title="Trackers">
                <Field
                  label="Add these trackers to new torrents"
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
              <Group title="Schedule">
                <Toggle
                  checked={s.alt_schedule.enabled}
                  onChange={(v) => schedule({ enabled: v })}
                  label="Use alternative limits on a schedule"
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
              <DataUse s={s} set={set} />
            </>
          )}

          {section === "bittorrent" && (
            <>
              <Group title="Seeding limits">
                <div className="grid grid-cols-3 gap-3">
                  <Field label="Max ratio">
                    <OptionalNumber value={s.seeding.max_ratio} onChange={(v) => seeding({ max_ratio: v })} step={0.1} />
                  </Field>
                  <Field label="Max seeding time (minutes)">
                    <OptionalNumber value={s.seeding.max_seeding_minutes} onChange={(v) => seeding({ max_seeding_minutes: v })} />
                  </Field>
                  <Field label="Max inactive time (minutes)">
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
                  <Field label="Inactivity timer (seconds)">
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
              <Group title="Looking after what you seed">
                <Toggle
                  checked={s.scrub_days != null}
                  onChange={(v) => set({ scrub_days: v ? 30 : null })}
                  label="Check finished data in the background"
                  hint="Re-reads what you seed now and then and compares it with the torrent, so a damaged disk doesn't send bad data. Damaged parts are downloaded again."
                />
                {s.scrub_days != null && (
                  <div className="grid grid-cols-2 gap-3">
                    <Field label="Every (days)">
                      <input
                        className="input"
                        type="number"
                        min={1}
                        max={365}
                        value={s.scrub_days}
                        onChange={(e) => set({ scrub_days: Math.min(365, Math.max(1, Math.round(Number(e.target.value)) || 1)) })}
                      />
                    </Field>
                    <Field label="Read at most (MiB/s)">
                      <input
                        className="input"
                        type="number"
                        min={1}
                        max={4096}
                        value={s.scrub_mib_per_sec}
                        onChange={(e) => set({ scrub_mib_per_sec: Math.min(4096, Math.max(1, Math.round(Number(e.target.value)) || 1)) })}
                      />
                    </Field>
                  </div>
                )}
                <Toggle
                  checked={s.seeding.queue.prefer_rare}
                  onChange={(v) => queue({ prefer_rare: v })}
                  label="Seed where it matters"
                  hint="Seeding slots go first to torrents whose trackers report few other seeders, so rare files stay available."
                />
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

          {section === "about" && <AboutSection checkUpdates={s.check_updates} onCheckUpdates={(v) => set({ check_updates: v })} />}
          {section === "remote" && <RemoteControlSection s={s} set={set} />}
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
              <TrustedNetworks s={s} set={set} />
              <Group title="IP blocklist">
                <Field label="Blocklist file" hint="A local .p2p, .dat or plain list; used instead of the URL when set.">
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
              <Group title="Banned peers">
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
                Peer encryption (Connection, Peers) only hides BitTorrent traffic from simple inspection; it doesn't hide who you
                connect to or what you share. Use the VPN if you need privacy from your network provider.
              </p>
            </>
          )}
        </div>
      </div>
    </Dialog>
  );
}
