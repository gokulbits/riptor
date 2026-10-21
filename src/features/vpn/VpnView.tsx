import { ask } from "@tauri-apps/plugin-dialog";
import clsx from "clsx";
import {
  AlertTriangle,
  ExternalLink,
  FileUp,
  KeyRound,
  Loader2,
  Plug,
  PlugZap,
  ShieldAlert,
  ShieldCheck,
  ShieldOff,
  Trash2,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { PublicIpView } from "../../components/PublicIp";
import { Field, Toggle } from "../../components/ui";
import { api, errorText, events } from "../../lib/ipc";
import type { LeakCheck, SwarmView, VpnPrefs, VpnProfile, VpnStatus } from "../../lib/types";
import { useApp } from "../../stores/app";

const OPENVPN_URL = "https://openvpn.net/community-downloads/";

const PHASE_LABEL: Record<VpnStatus["phase"], string> = {
  disconnected: "Not connected",
  connecting: "Connecting…",
  connected: "Connected",
  reconnecting: "Reconnecting…",
  error: "Error",
};

/** Compares the address through the tunnel with the normal one. */
function LeakCheckCard() {
  const toast = useApp((s) => s.toast);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<LeakCheck | null>(null);
  const run = async () => {
    setBusy(true);
    try {
      setResult(await api.leakCheck());
    } catch (e) {
      toast("error", errorText(e));
    } finally {
      setBusy(false);
    }
  };
  const row = (k: string, v: React.ReactNode) => (
    <div className="flex gap-2">
      <span className="w-44 shrink-0 text-muted">{k}</span>
      <span className="min-w-0 select-text font-mono text-xs">{v}</span>
    </div>
  );
  return (
    <div className="card space-y-2 p-3 text-sm">
      <div className="flex items-center gap-2">
        <span className="flex-1 font-medium">Leak check</span>
        <button className="btn-ghost border border-border text-xs" onClick={run} disabled={busy}>
          {busy ? <Loader2 size={13} className="animate-spin" /> : <ShieldCheck size={13} />} Run check
        </button>
      </div>
      <p className="text-xs text-muted">Asks an IP lookup service for your address twice: through the tunnel and over your normal connection.</p>
      {result && (
        <div className="space-y-1">
          <p className={clsx("font-medium", result.passed ? "text-ok" : "text-danger")}>{result.passed ? "No leaks found" : "Problems found"}</p>
          {row("Trackers and peers see", result.vpn_ip ?? "unknown")}
          {row("Your normal address", result.normal_ip ?? "unknown")}
          {row("Name lookups via tunnel", result.tunnel_dns ? "yes" : "no")}
          {row("Torrent traffic bound to", result.adapter)}
          {row("Kill switch", result.kill_switch ? "on" : "off")}
          {result.notes.map((n) => (
            <p key={n} className="flex items-start gap-1.5 text-xs text-warn">
              <AlertTriangle size={13} className="mt-0.5 shrink-0" /> {n}
            </p>
          ))}
        </div>
      )}
    </div>
  );
}

/** What peers and trackers report seeing, checked all the time without extra requests. */
function SwarmCard() {
  const toast = useApp((s) => s.toast);
  const [view, setView] = useState<SwarmView | null>(null);
  const load = useCallback(() => {
    api.swarmView().then(setView, () => {});
  }, []);
  useEffect(() => {
    load();
    const t = window.setInterval(load, 5000);
    return () => window.clearInterval(t);
  }, [load]);
  if (!view) return null;
  const v = view.verdict;
  const release = () =>
    api.releaseLeakHold().then(
      () => {
        toast("info", "Torrents can run again. Riptor keeps watching what the swarm sees.");
        load();
      },
      (e) => toast("error", errorText(e)),
    );
  return (
    <div className={clsx("card space-y-2 p-3 text-sm", view.held && "border-danger/60")}>
      <span className="font-medium">What the swarm sees</span>
      <p className="text-xs text-muted">
        Peers and some trackers tell Riptor which address they see it on. Riptor compares that with the VPN's address all the time,
        without sending anything extra. Peers can lie, so it takes several unrelated ones to count.
      </p>
      {view.held && (
        <div className="space-y-2 rounded-md bg-danger/10 p-2 text-danger">
          <p>
            Torrents are held: several peers and trackers saw your normal address ({view.held}). Reconnect the VPN or run the leak check,
            then let them run again.
          </p>
          <button className="btn-ghost border border-danger/40 text-xs" onClick={release}>
            Let torrents run again
          </button>
        </div>
      )}
      <p
        className={clsx(
          "font-medium",
          v.kind === "expected" ? "text-ok" : v.kind === "leak" ? "text-danger" : v.kind === "unexpected" ? "text-warn" : "text-muted",
        )}
      >
        {v.kind === "expected"
          ? "The swarm sees the VPN's address."
          : v.kind === "leak"
            ? `The swarm sees your normal address (${v.ip}).`
            : v.kind === "unexpected"
              ? `Most of the swarm sees ${v.ip}, not the VPN's address.`
              : "Not enough reports yet."}
      </p>
      {view.seen.length > 0 && (
        <ul className="space-y-0.5 text-xs">
          {view.seen.map((s) => (
            <li key={s.ip} className="flex gap-2">
              <span className="w-40 select-text font-mono">{s.ip}</span>
              <span className="text-muted">
                {s.peers} peer network{s.peers === 1 ? "" : "s"}, {s.trackers} tracker{s.trackers === 1 ? "" : "s"}
                {s.ip === view.vpn_ip && " · the VPN"}
                {s.ip === view.normal_ip && " · your normal address"}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function StatusCard({ status, killSwitch, onDisconnect }: { status: VpnStatus; killSwitch: boolean; onDisconnect: () => void }) {
  const network = useApp((s) => s.global?.network);
  const up = status.phase === "connected";
  const busy = status.phase === "connecting" || status.phase === "reconnecting";
  const Icon = up ? ShieldCheck : status.phase === "error" ? ShieldAlert : ShieldOff;
  return (
    <div className="card flex items-start gap-4 p-4">
      <Icon size={32} className={clsx("shrink-0", up ? "text-ok" : status.phase === "error" ? "text-danger" : "text-muted")} />
      <div className="min-w-0 flex-1 space-y-1">
        <div className="flex items-center gap-2">
          <span className="text-base font-semibold">{PHASE_LABEL[status.phase]}</span>
          {busy && <Loader2 size={14} className="animate-spin text-muted" />}
          {status.profile_name && <span className="truncate text-sm text-muted">· {status.profile_name}</span>}
        </div>
        {up && (
          <p className="text-sm text-muted">
            Tunnel address <span className="font-mono text-fg">{status.local_ip}</span>
            {status.forwarded_port != null && (
              <>
                {" "}· forwarded port <span className="font-mono text-fg">{status.forwarded_port}</span>
              </>
            )}
          </p>
        )}
        {status.message && <p className={clsx("select-text text-sm", status.phase === "error" ? "text-danger" : "text-muted")}>{status.message}</p>}
        <p className="text-xs text-muted">
          {network === "vpn"
            ? "All torrent traffic is going through the tunnel. Other apps use your normal connection."
            : network === "blocked"
              ? "Kill switch engaged: torrents are held and nothing is sent until the VPN connects."
              : killSwitch
                ? "Kill switch is on."
                : "Torrents use your normal connection."}
        </p>
        <div className="flex items-center gap-1 pt-1 text-xs text-muted">
          Public IP: <PublicIpView />
        </div>
      </div>
      {status.phase !== "disconnected" && (
        <button className="btn-ghost" onClick={onDisconnect}>
          <Plug size={14} /> Disconnect
        </button>
      )}
    </div>
  );
}

function CredentialsForm({ profile, onSaved }: { profile: VpnProfile; onSaved: () => void }) {
  const toast = useApp((s) => s.toast);
  const [user, setUser] = useState("");
  const [pass, setPass] = useState("");
  const [busy, setBusy] = useState(false);
  const save = async () => {
    setBusy(true);
    try {
      await api.vpnSetCredentials(profile.id, user, pass);
      setUser("");
      setPass("");
      toast("success", "Credentials saved to the system credential store");
      onSaved();
    } catch (e) {
      toast("error", errorText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="mt-3 space-y-2 rounded-md border border-border p-3">
      <div className="grid grid-cols-2 gap-2">
        <Field label="OpenVPN username">
          <input className="input" value={user} onChange={(e) => setUser(e.target.value)} autoComplete="off" maxLength={256} />
        </Field>
        <Field label="OpenVPN password">
          <input className="input" type="password" value={pass} onChange={(e) => setPass(e.target.value)} autoComplete="off" maxLength={1024} />
        </Field>
      </div>
      <p className="text-xs text-muted">
        Many providers use separate OpenVPN credentials, not your account login. Stored in the system credential store,
        never in Riptor's files or backups.
      </p>
      <button className="btn-primary" disabled={busy || !user || !pass} onClick={save}>
        {busy ? <Loader2 size={14} className="animate-spin" /> : <KeyRound size={14} />} Save credentials
      </button>
    </div>
  );
}

function ProfileRow({
  profile,
  status,
  onChanged,
}: {
  profile: VpnProfile;
  status: VpnStatus;
  onChanged: () => void;
}) {
  const toast = useApp((s) => s.toast);
  const [showCreds, setShowCreds] = useState(false);
  const active = status.profile_id === profile.id && status.phase !== "disconnected";
  const missingCreds = profile.needs_credentials && !profile.has_credentials;

  const connect = async () => {
    try {
      await api.vpnConnect(profile.id);
    } catch (e) {
      toast("error", errorText(e));
    }
  };
  const remove = async () => {
    const ok = await ask(`Delete the VPN profile “${profile.name}” and its saved credentials?`, { title: "Delete profile", kind: "warning" });
    if (!ok) return;
    try {
      await api.vpnDelete(profile.id);
      onChanged();
    } catch (e) {
      toast("error", errorText(e));
    }
  };

  return (
    <li className="card p-3">
      <div className="flex items-center gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="truncate font-medium">{profile.name}</span>
            <span className="rounded-sm bg-raised px-1.5 py-0.5 text-[11px] text-muted">{profile.kind === "open_vpn" ? "OpenVPN" : "WireGuard"}</span>
          </div>
          <div className="truncate text-xs text-muted">{profile.servers.join(", ") || "No server listed"}</div>
        </div>
        {profile.needs_credentials && (
          <button className="btn-ghost" onClick={() => setShowCreds((v) => !v)} title="Username and password">
            <KeyRound size={14} /> {profile.has_credentials ? "Change login" : "Add login"}
          </button>
        )}
        {active ? (
          <span className="px-2 text-sm text-ok">In use</span>
        ) : (
          <button
            className="btn-primary"
            onClick={connect}
            disabled={!profile.app_only_supported || missingCreds || status.phase === "connecting"}
            title={missingCreds ? "Add the OpenVPN username and password first" : undefined}
          >
            <PlugZap size={14} /> Connect
          </button>
        )}
        <button className="btn-ghost p-1.5 text-muted hover:text-danger" onClick={remove} aria-label={`Delete ${profile.name}`}>
          <Trash2 size={14} />
        </button>
      </div>
      {!profile.app_only_supported && (
        <p className="mt-2 flex items-start gap-1.5 text-xs text-warn">
          <AlertTriangle size={13} className="mt-px shrink-0" />
          WireGuard can't be limited to Riptor on Windows yet, and Riptor won't tunnel your whole computer. Import the provider's OpenVPN config for
          this server instead.
        </p>
      )}
      {profile.removed.length > 0 && (
        <p className="mt-2 text-xs text-muted">
          Removed on import for safety: <span className="font-mono">{profile.removed.join(", ")}</span>
        </p>
      )}
      {(showCreds || (missingCreds && profile.app_only_supported)) && (
        <CredentialsForm
          profile={profile}
          onSaved={() => {
            setShowCreds(false);
            onChanged();
          }}
        />
      )}
    </li>
  );
}

export function VpnView() {
  const toast = useApp((s) => s.toast);
  const [status, setStatus] = useState<VpnStatus | null>(null);
  const [profiles, setProfiles] = useState<VpnProfile[]>([]);
  const [prefs, setPrefs] = useState<VpnPrefs | null>(null);
  const [killSwitch, setKillSwitch] = useState(false);
  const [openvpn, setOpenvpn] = useState<boolean | null>(null);
  const [importing, setImporting] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const [st, list, p, settings, ovpn] = await Promise.all([
        api.vpnStatus(),
        api.vpnProfiles(),
        api.vpnPrefs(),
        api.settings(),
        api.openVpnInstalled(),
      ]);
      setStatus(st);
      setProfiles(list);
      setPrefs(p);
      setKillSwitch(settings.vpn_required);
      setOpenvpn(ovpn);
    } catch {
    }
  }, []);

  useEffect(() => {
    refresh();
    const un = events.vpn(setStatus);
    return () => {
      un.then((f) => f());
    };
  }, [refresh]);

  const importProfile = async () => {
    setImporting(true);
    try {
      const p = await api.vpnImport();
      if (p) {
        toast("success", `Imported ${p.name}`);
        await refresh();
      }
    } catch (e) {
      toast("error", errorText(e));
    } finally {
      setImporting(false);
    }
  };

  const setKill = async (on: boolean) => {
    try {
      const s = await api.settings();
      await api.updateSettings({ ...s, vpn_required: on });
      setKillSwitch(on);
    } catch (e) {
      toast("error", errorText(e));
    }
  };

  const savePrefs = async (p: Partial<VpnPrefs>) => {
    if (!prefs) return;
    try {
      await api.vpnSetPrefs({ ...prefs, ...p });
      setPrefs({ ...prefs, ...p });
    } catch (e) {
      toast("error", errorText(e));
    }
  };

  const disconnect = () => api.vpnDisconnect().catch((e) => toast("error", errorText(e)));

  if (!status || !prefs) {
    return <div className="flex h-full items-center justify-center text-sm text-muted">Loading VPN…</div>;
  }

  return (
    <div className="h-full overflow-auto">
      <div className="mx-auto max-w-3xl space-y-5 p-6">
        <StatusCard status={status} killSwitch={killSwitch} onDisconnect={disconnect} />
        {status.phase === "connected" && <LeakCheckCard />}
        <SwarmCard />

        {openvpn === false && (
          <div className="card flex items-start gap-3 border-warn/50 p-3 text-sm">
            <AlertTriangle size={16} className="mt-0.5 shrink-0 text-warn" />
            <div className="flex-1">
              OpenVPN isn't installed. Riptor's setup normally installs it on Windows, and the Linux packages depend on it; run the setup
              again, install your distribution's <span className="font-mono">openvpn</span> package, or get it directly.
              <button className="ml-1 inline-flex items-center gap-1 text-accent hover:underline" onClick={() => api.openWebUrl(OPENVPN_URL)}>
                Download OpenVPN <ExternalLink size={12} />
              </button>
              <button className="ml-3 text-muted hover:text-fg" onClick={refresh}>
                Check again
              </button>
            </div>
          </div>
        )}

        <section className="space-y-2">
          <h2 className="text-sm font-semibold">Protection</h2>
          <div className="card space-y-1 p-3">
            <Toggle
              checked={killSwitch}
              onChange={setKill}
              label="Kill switch: only run torrents through the VPN"
              hint="When the VPN isn't connected, torrents are held: no peers, trackers or DHT. Also applies at startup."
            />
            <Toggle
              checked={prefs.port_forwarding}
              onChange={(v) => savePrefs({ port_forwarding: v })}
              label="Ask the VPN to forward a port (NAT-PMP)"
              hint="Lets other peers connect to you. Only works if your VPN provider and server support NAT-PMP port forwarding. Applies on the next connect."
            />
            <Toggle
              checked={prefs.auto_connect}
              onChange={(v) => savePrefs({ auto_connect: v })}
              label="Connect when Riptor starts"
              hint={
                prefs.last_profile
                  ? "Reconnects the profile you used last. Windows asks for administrator rights each time."
                  : "Connects your profile if you have just one ready, otherwise the one you connect next. Windows asks for administrator rights each time."
              }
            />
          </div>
        </section>

        <section className="space-y-2">
          <div className="flex items-center justify-between">
            <h2 className="text-sm font-semibold">Profiles</h2>
            <button className="btn-ghost" onClick={importProfile} disabled={importing}>
              {importing ? <Loader2 size={14} className="animate-spin" /> : <FileUp size={14} />} Import .ovpn / .conf
            </button>
          </div>
          {profiles.length === 0 ? (
            <div className="card p-4 text-sm text-muted">
              No profiles yet. Download an OpenVPN config (.ovpn) from your VPN provider's account page (pick a server that allows P2P),
              then import it here.
            </div>
          ) : (
            <ul className="space-y-2">
              {profiles.map((p) => (
                <ProfileRow key={p.id} profile={p} status={status} onChanged={refresh} />
              ))}
            </ul>
          )}
        </section>

        <section className="space-y-1 text-xs text-muted">
          <h2 className="text-sm font-semibold text-fg">How this works</h2>
          <p>
            The VPN only carries Riptor's torrent traffic: Riptor's sockets are pinned to the tunnel adapter, and the tunnel is not made your
            computer's default route. Your browser and other apps keep using your normal connection.
          </p>
          <p>
            Connecting asks for administrator permission once, because creating a tunnel adapter needs it. Scripts and other risky options in
            imported profiles are removed. If the tunnel drops, torrents are held within about a second.
          </p>
          <p>
            Known limit: tracker host names are still looked up with your normal DNS server, so your DNS provider can see which trackers you
            contact (not what you download). Peer and tracker traffic itself always goes through the tunnel.
          </p>
        </section>
      </div>
    </div>
  );
}
