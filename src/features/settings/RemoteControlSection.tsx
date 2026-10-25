import { open } from "@tauri-apps/plugin-dialog";
import { Check, Copy, FolderOpen, KeyRound, RefreshCw, Trash2 } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { Field, Toggle } from "../../components/ui";
import { api, errorText } from "../../lib/ipc";
import type { Settings, WebUiSettings, WebUiState } from "../../lib/types";
import { useApp } from "../../stores/app";

function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <fieldset className="space-y-3">
      <legend className="mb-5 text-xs font-semibold uppercase tracking-wider text-muted">{title}</legend>
      {children}
    </fieldset>
  );
}

function Warning({ children }: { children: ReactNode }) {
  return <p className="rounded-md border border-warn/40 bg-warn/10 p-2 text-xs text-fg">{children}</p>;
}

function FileInput({ value, onChange, placeholder }: { value: string | null; onChange: (v: string | null) => void; placeholder: string }) {
  return (
    <div className="flex gap-1">
      <input className="input font-mono text-xs" value={value ?? ""} placeholder={placeholder} onChange={(e) => onChange(e.target.value.trim() || null)} />
      <button
        className="btn-ghost border border-border"
        aria-label="Choose file"
        onClick={async () => {
          const f = await open({ multiple: false });
          if (typeof f === "string") onChange(f);
        }}
      >
        <FolderOpen size={15} />
      </button>
    </div>
  );
}

const minutes = (secs: number) => Math.round(secs / 60);
const clampInt = (v: string, min: number, max: number) => Math.min(max, Math.max(min, Math.round(Number(v)) || min));

export function RemoteControlSection({ s, set }: { s: Settings; set: (patch: Partial<Settings>) => void }) {
  const toast = useApp((st) => st.toast);
  const w = s.web_ui;
  const edit = (patch: Partial<WebUiSettings>) => set({ web_ui: { ...w, ...patch } });
  const [state, setState] = useState<WebUiState | null>(null);
  const [password, setPassword] = useState("");
  const [ddnsPassword, setDdnsPassword] = useState("");
  const [newKey, setNewKey] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const refresh = () => api.webuiState().then(setState, () => {});
  useEffect(() => {
    refresh();
    const t = window.setInterval(refresh, 3000);
    return () => window.clearInterval(t);
  }, []);

  const run = (p: Promise<unknown>, done: string) =>
    p.then(
      () => {
        toast("success", done);
        refresh();
      },
      (e) => toast("error", errorText(e)),
    );

  const st = state?.status;
  const scheme = w.https ? "https" : "http";

  return (
    <div className="space-y-6">
      <Group title="Web user interface (remote control)">
        <Toggle
          checked={w.enabled}
          onChange={(v) => edit({ enabled: v })}
          label="Allow remote control"
          hint="Speaks qBittorrent's Web API, so apps like qBitController can manage Riptor from your phone. Off by default."
        />
        {w.enabled && (
          <p className="text-xs text-muted">
            {st?.running
              ? st.addresses.length > 0
                ? `Running. Address for qBitController: ${st.addresses.map((a) => `${scheme}://${a}:${st.port}`).join(" or ")}`
                : `Running on port ${st.port}, but this computer has no local network address right now.`
              : st?.error
                ? `Not running: ${st.error}`
                : state && !state.has_password
                  ? "Set a password below to start it."
                  : "Starts once you save."}
          </p>
        )}
        <div className="rounded-md bg-raised p-3 text-xs text-muted">
          <p className="mb-1 font-medium text-fg">Connect qBitController</p>
          <ol className="list-decimal space-y-0.5 pl-4">
            <li>Turn this on, set a password and save.</li>
            <li>
              In qBitController, add a server with the address{" "}
              <span className="font-mono">
                {st?.running && st.addresses.length > 0 ? `${scheme}://${st.addresses[0]}:${st.port}` : `${scheme}://this-computer's-IP:${w.port}`}
              </span>
              , your username and password. The phone must be on the same network.
            </li>
            {w.https && <li>In the server's advanced settings, turn on "Trust self-signed certificates" (unless you use your own certificate).</li>}
          </ol>
        </div>
        <div className="grid grid-cols-[1fr_8rem] gap-3">
          <Field label="IP address" hint="* listens on every network adapter.">
            <input className="input font-mono text-xs" value={w.address} onChange={(e) => edit({ address: e.target.value.trim() || "*" })} />
          </Field>
          <Field label="Port">
            <input className="input" type="number" min={1024} max={65535} value={w.port} onChange={(e) => edit({ port: clampInt(e.target.value, 1024, 65535) })} />
          </Field>
        </div>
        <Toggle
          checked={w.allow_public}
          onChange={(v) => edit({ allow_public: v })}
          label="Accept connections from outside the local network"
          hint="Off: only devices on your home network can connect. Connections through the VPN tunnel are always refused."
        />
        <Toggle checked={w.upnp} onChange={(v) => edit({ upnp: v })} label="Use UPnP to forward the port from my router" />
        {(w.upnp || w.allow_public) && (
          <Warning>
            This makes remote control reachable from the internet. Use HTTPS, a strong password, and keep the ban limit on.
            {w.upnp && !w.allow_public && " Also turn on accepting connections from outside the local network, or forwarded connections are refused."}
          </Warning>
        )}
      </Group>

      <Group title="HTTPS">
        <Toggle
          checked={w.https}
          onChange={(v) => edit({ https: v })}
          label="Use HTTPS instead of HTTP"
          hint="Without your own certificate, Riptor makes a self-signed one. Apps that trust any self-signed certificate are protected from eavesdropping, not from someone intercepting the connection."
        />
        {w.https && (
          <>
            <div className="grid grid-cols-2 gap-3">
              <Field label="Certificate (PEM)">
                <FileInput value={w.cert_file} placeholder="Riptor's own" onChange={(v) => edit({ cert_file: v })} />
              </Field>
              <Field label="Key (PEM)">
                <FileInput value={w.key_file} placeholder="Riptor's own" onChange={(v) => edit({ key_file: v })} />
              </Field>
            </div>
            {st?.fingerprint && (
              <p className="text-xs text-muted">
                Certificate fingerprint (SHA-256): <span className="break-all font-mono text-fg">{st.fingerprint}</span>
              </p>
            )}
          </>
        )}
        {!w.https && w.enabled && <Warning>Without HTTPS, your password and everything you do travel unencrypted over the network.</Warning>}
      </Group>

      <Group title="Authentication">
        <div className="grid grid-cols-2 gap-3">
          <Field label="Username">
            <input className="input" value={w.username} onChange={(e) => edit({ username: e.target.value.trim() })} />
          </Field>
          <Field label="Password" hint={state?.has_password ? "A password is set." : "No password yet: remote control won't start."}>
            <div className="flex gap-1">
              <input
                className="input"
                type="password"
                autoComplete="new-password"
                placeholder={state?.has_password ? "Change current password" : "At least 8 characters"}
                value={password}
                onChange={(e) => setPassword(e.target.value)}
              />
              <button
                className="btn-ghost border border-border text-xs"
                disabled={password.length < 8}
                onClick={() => run(api.webuiSetPassword(password).then(() => setPassword("")), "Password saved")}
              >
                Set
              </button>
            </div>
          </Field>
        </div>
        <Field label="API key" hint="For apps that sign in with a key (sent as Authorization: Bearer). It's shown once; Riptor keeps only a hash.">
          <div className="flex items-center gap-1">
            <input
              className="input font-mono text-xs"
              readOnly
              value={newKey ?? (state?.has_api_key ? "•••••••• (set)" : "")}
              placeholder="Generate a key"
            />
            <button
              className="btn-ghost border border-border"
              aria-label="Copy the key"
              disabled={!newKey}
              onClick={() =>
                newKey &&
                navigator.clipboard.writeText(newKey).then(() => {
                  setCopied(true);
                  window.setTimeout(() => setCopied(false), 1500);
                })
              }
            >
              {copied ? <Check size={15} /> : <Copy size={15} />}
            </button>
            <button
              className="btn-ghost border border-border"
              aria-label="Generate a new key"
              onClick={() =>
                api.webuiNewApiKey().then(
                  (k) => {
                    setNewKey(k);
                    refresh();
                  },
                  (e) => toast("error", errorText(e)),
                )
              }
            >
              {state?.has_api_key ? <RefreshCw size={15} /> : <KeyRound size={15} />}
            </button>
            <button
              className="btn-ghost border border-border hover:text-danger"
              aria-label="Remove the key"
              disabled={!state?.has_api_key}
              onClick={() => run(api.webuiRemoveApiKey().then(() => setNewKey(null)), "API key removed")}
            >
              <Trash2 size={15} />
            </button>
          </div>
        </Field>
        <Toggle checked={w.bypass_localhost} onChange={(v) => edit({ bypass_localhost: v })} label="Bypass authentication for clients on localhost" />
        <Toggle checked={w.bypass_subnets} onChange={(v) => edit({ bypass_subnets: v })} label="Bypass authentication for clients in whitelisted IP subnets" />
        {w.bypass_subnets && (
          <Field label="IP subnet whitelist" hint="One per line, like 192.168.1.0/24.">
            <textarea
              className="input h-20 font-mono text-xs"
              defaultValue={w.subnet_whitelist.join("\n")}
              onBlur={(e) => edit({ subnet_whitelist: e.target.value.split(/\s+/).filter(Boolean) })}
            />
          </Field>
        )}
        {(w.bypass_localhost || w.bypass_subnets) && <Warning>Anyone who can reach Riptor from those addresses controls it without a password.</Warning>}
        <div className="grid grid-cols-3 gap-3">
          <Field label="Ban client after consecutive failures" hint="Each new ban on the same client lasts twice as long, up to a week.">
            <input className="input" type="number" min={1} max={20} value={w.max_failures} onChange={(e) => edit({ max_failures: clampInt(e.target.value, 1, 20) })} />
          </Field>
          <Field label="Ban for (minutes)">
            <input className="input" type="number" min={1} max={43200} value={minutes(w.ban_secs)} onChange={(e) => edit({ ban_secs: clampInt(e.target.value, 1, 43200) * 60 })} />
          </Field>
          <Field label="Session timeout (minutes)">
            <input
              className="input"
              type="number"
              min={1}
              max={43200}
              value={minutes(w.session_timeout_secs)}
              onChange={(e) => edit({ session_timeout_secs: clampInt(e.target.value, 1, 43200) * 60 })}
            />
          </Field>
        </div>
      </Group>

      <Group title="Security">
        <Toggle checked={w.clickjacking_protection} onChange={(v) => edit({ clickjacking_protection: v })} label="Enable clickjacking protection" />
        <Toggle checked={w.csrf_protection} onChange={(v) => edit({ csrf_protection: v })} label="Enable Cross-Site Request Forgery (CSRF) protection" />
        <Toggle checked={w.secure_cookie} onChange={(v) => edit({ secure_cookie: v })} label="Enable cookie Secure flag (requires HTTPS)" />
        <Toggle
          checked={w.host_validation}
          onChange={(v) => edit({ host_validation: v })}
          label="Enable Host header validation"
          hint="IP addresses and localhost are always accepted."
        />
        {w.host_validation && (
          <Field label="Server domains" hint="Separate with ;  * matches anything.">
            <input className="input font-mono text-xs" value={w.server_domains} onChange={(e) => edit({ server_domains: e.target.value })} />
          </Field>
        )}
      </Group>

      <Group title="Custom HTTP headers">
        <Toggle checked={w.custom_headers_enabled} onChange={(v) => edit({ custom_headers_enabled: v })} label="Add custom HTTP headers" />
        {w.custom_headers_enabled && (
          <textarea
            className="input h-20 font-mono text-xs"
            placeholder="Header: value pairs, one per line"
            defaultValue={w.custom_headers}
            onBlur={(e) => edit({ custom_headers: e.target.value })}
          />
        )}
      </Group>

      <Group title="Reverse proxy">
        <Toggle
          checked={w.reverse_proxy}
          onChange={(v) => edit({ reverse_proxy: v })}
          label="Enable reverse proxy support"
          hint="Believes X-Forwarded-For only from the proxies listed, so bans and whitelists see the real client."
        />
        {w.reverse_proxy && (
          <Field label="Trusted proxies" hint="IP addresses or ranges, separated with ;">
            <input className="input font-mono text-xs" value={w.trusted_proxies} onChange={(e) => edit({ trusted_proxies: e.target.value })} />
          </Field>
        )}
      </Group>

      <Group title="Dynamic DNS">
        <Toggle
          checked={w.ddns.enabled}
          onChange={(v) => edit({ ddns: { ...w.ddns, enabled: v } })}
          label="Update my dynamic domain name"
          hint="Keeps a domain name pointing at this computer, using the update protocol most dynamic DNS providers support. Checked every 10 minutes."
        />
        {w.ddns.enabled && (
          <>
            <Field label="Provider's update address" hint="From your provider's documentation, for example https://example.com/nic/update.">
              <input className="input font-mono text-xs" value={w.ddns.update_url} onChange={(e) => edit({ ddns: { ...w.ddns, update_url: e.target.value.trim() } })} />
            </Field>
            <div className="grid grid-cols-3 gap-3">
              <Field label="Domain name">
                <input className="input" value={w.ddns.domain} onChange={(e) => edit({ ddns: { ...w.ddns, domain: e.target.value.trim() } })} />
              </Field>
              <Field label="Username">
                <input className="input" value={w.ddns.username} onChange={(e) => edit({ ddns: { ...w.ddns, username: e.target.value.trim() } })} />
              </Field>
              <Field label="Password" hint={state?.has_ddns_password ? "Set." : undefined}>
                <div className="flex gap-1">
                  <input className="input" type="password" autoComplete="new-password" value={ddnsPassword} onChange={(e) => setDdnsPassword(e.target.value)} />
                  <button
                    className="btn-ghost border border-border text-xs"
                    disabled={!ddnsPassword}
                    onClick={() => run(api.webuiSetDdnsPassword(ddnsPassword).then(() => setDdnsPassword("")), "Password saved")}
                  >
                    Set
                  </button>
                </div>
              </Field>
            </div>
            {st?.ddns && <p className="text-xs text-muted">Last update: {st.ddns}</p>}
          </>
        )}
      </Group>
    </div>
  );
}
