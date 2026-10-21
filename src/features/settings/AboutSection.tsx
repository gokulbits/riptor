import clsx from "clsx";
import { Bug, Check, CodeXml, Copy, FileText, GitFork, HeartHandshake, Puzzle, ShieldCheck, UserRound, Users } from "lucide-react";
import { useEffect, useState, version as reactVersion, type ReactNode } from "react";
import { createPortal } from "react-dom";
import licenseText from "../../../LICENSE?raw";
import noticesText from "../../../THIRD-PARTY-NOTICES.md?raw";
import logo from "../../../app-icon.svg";
import { api, errorText } from "../../lib/ipc";
import type { AboutInfo } from "../../lib/types";
import { useApp } from "../../stores/app";
import { LogsDialog } from "../about/LogsDialog";
import { Updates } from "../about/Updates";

const REPO = "https://github.com/gokulbits/riptor";

type Tab = "about" | "authors" | "thanks" | "license" | "software";

const TABS: { id: Tab; label: string }[] = [
  { id: "about", label: "About" },
  { id: "authors", label: "Authors" },
  { id: "thanks", label: "Thanks" },
  { id: "license", label: "License" },
  { id: "software", label: "Software Used" },
];

const THANKS: { icon: ReactNode; who: string; why: string }[] = [
  {
    icon: <CodeXml size={16} />,
    who: "Open-source developers",
    why: "The people who build librqbit, Tauri, Rust, React and everything else Riptor stands on.",
  },
  { icon: <Bug size={16} />, who: "Bug reporters and testers", why: "Everyone who reports problems, tries new versions and tells us what's confusing." },
  { icon: <ShieldCheck size={16} />, who: "Security researchers", why: "Those who report vulnerabilities privately, so they're fixed before they're public." },
  { icon: <Puzzle size={16} />, who: "Search plugin authors", why: "Community members who write and maintain the nova3 plugins that search runs." },
  { icon: <HeartHandshake size={16} />, who: "Seeders", why: "Everyone who keeps sharing after the download is done, so others can get it too." },
];

const open = (url: string) => api.openWebUrl(url).catch(() => {});

function LinkButton({ icon, label, url }: { icon: ReactNode; label: string; url: string }) {
  return (
    <button className="btn-ghost border border-border bg-panel text-xs" onClick={() => open(url)}>
      {icon} {label}
    </button>
  );
}

function Card({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={clsx("rounded-lg border border-border bg-panel p-4", className)}>{children}</div>;
}

const osName = (os: string) => ({ windows: "Windows", linux: "Linux", macos: "macOS" })[os] ?? os;

function softwareRows(info: AboutInfo | null): [string, string][] {
  if (!info) return [];
  const windows = info.os === "windows";
  return [
    ["Riptor", info.version],
    ["Operating system", `${osName(info.os)} (${info.arch})`],
    ["librqbit", info.librqbit],
    ["Tauri", info.tauri],
    [windows ? "WebView2" : "WebKitGTK", info.webview ?? "Unknown"],
    ["React", reactVersion],
    ["Python (search plugins)", windows ? `${info.python} (bundled)` : "The system's python3"],
    ["OpenVPN", info.openvpn_installed ? "Installed" : "Not found"],
  ];
}

function Notice({ title, text }: { title: string; text: string }) {
  return (
    <section className="space-y-2">
      <h4 className="text-sm font-medium">{title}</h4>
      <pre className="max-h-56 overflow-auto rounded-lg border border-border bg-raised p-3 font-mono text-xs leading-relaxed whitespace-pre-wrap text-muted">
        {text.trim()}
      </pre>
    </section>
  );
}

export function AboutSection({ checkUpdates, onCheckUpdates }: { checkUpdates: boolean; onCheckUpdates: (v: boolean) => void }) {
  const toast = useApp((s) => s.toast);
  const [tab, setTab] = useState<Tab>("about");
  const [info, setInfo] = useState<AboutInfo | null>(null);
  const [logs, setLogs] = useState(false);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    api.aboutInfo().then(setInfo, (e) => toast("error", errorText(e)));
  }, [toast]);

  const rows = softwareRows(info);
  const copy = () =>
    navigator.clipboard.writeText(rows.map(([k, v]) => `${k}: ${v}`).join("\n")).then(
      () => {
        setCopied(true);
        window.setTimeout(() => setCopied(false), 1500);
      },
      (e) => toast("error", errorText(e)),
    );

  return (
    <div className="space-y-5">
      <div className="relative overflow-hidden rounded-xl border border-border bg-gradient-to-br from-accent/15 via-panel to-panel p-5">
        <div className="flex items-center gap-4">
          <img src={logo} alt="" className="h-16 w-16 shrink-0 rounded-2xl shadow-lg shadow-accent/20" />
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <h3 className="text-xl font-semibold tracking-tight">Riptor</h3>
              <span className="rounded-full bg-accent/15 px-2 py-0.5 text-xs font-medium text-accent">
                {info ? `v${info.version}` : "…"}
              </span>
            </div>
            <p className="mt-1 text-[13px] text-muted">
              A BitTorrent client for Windows and Linux with built-in search, an app-only VPN and a kill switch.
            </p>
          </div>
        </div>
      </div>

      <div role="tablist" aria-label="About Riptor" className="inline-flex flex-wrap gap-1 rounded-lg bg-raised p-1">
        {TABS.map((t) => (
          <button
            key={t.id}
            role="tab"
            aria-selected={tab === t.id}
            onClick={() => setTab(t.id)}
            className={clsx(
              "rounded-md px-3 py-1 text-xs font-medium transition-colors",
              tab === t.id ? "bg-panel text-fg shadow-sm" : "text-muted hover:text-fg",
            )}
          >
            {t.label}
          </button>
        ))}
      </div>

      <div role="tabpanel" className="space-y-4">
        {tab === "about" && (
          <>
            <div className="flex flex-wrap gap-2">
              <LinkButton icon={<CodeXml size={14} />} label="Source code" url={REPO} />
              <LinkButton icon={<Bug size={14} />} label="Report a problem" url={`${REPO}/issues`} />
              <LinkButton icon={<ShieldCheck size={14} />} label="Security policy" url={`${REPO}/blob/main/SECURITY.md`} />
            </div>
            <Updates auto={checkUpdates} onAuto={onCheckUpdates} />
            <Card className="flex items-center gap-3">
              <FileText size={18} className="shrink-0 text-muted" />
              <div className="min-w-0 flex-1">
                <p className="text-sm font-medium">Logs</p>
                <p className="text-xs text-muted">What Riptor did recently. Useful when reporting a problem.</p>
              </div>
              <button className="btn-ghost border border-border text-xs" onClick={() => setLogs(true)}>
                View logs
              </button>
            </Card>
            <p className="text-xs text-muted">Copyright © 2026 gokulbits. Released under the MIT License.</p>
          </>
        )}

        {tab === "authors" && (
          <>
            <Card className="flex items-center gap-4">
              <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-full bg-accent/15 text-base font-semibold text-accent">
                G
              </div>
              <div className="min-w-0 flex-1">
                <p className="text-sm font-medium">gokulbits</p>
                <p className="text-xs text-muted">Creator and maintainer</p>
              </div>
              <LinkButton icon={<UserRound size={14} />} label="Profile" url="https://github.com/gokulbits" />
            </Card>
            <Card className="flex items-center gap-3">
              <Users size={18} className="shrink-0 text-muted" />
              <div className="min-w-0 flex-1">
                <p className="text-sm font-medium">Everyone who contributes</p>
                <p className="text-xs text-muted">Code, bug reports and ideas all count. Riptor is open source and welcomes help.</p>
              </div>
              <LinkButton icon={<GitFork size={14} />} label="Contributors" url={`${REPO}/graphs/contributors`} />
            </Card>
          </>
        )}

        {tab === "thanks" && (
          <>
            <ul className="grid grid-cols-2 gap-2">
              {THANKS.map((t) => (
                <li key={t.who} className="flex items-start gap-3 rounded-lg border border-border bg-panel p-3">
                  <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-accent/15 text-accent">{t.icon}</span>
                  <div className="min-w-0">
                    <p className="text-sm font-medium">{t.who}</p>
                    <p className="text-xs text-muted">{t.why}</p>
                  </div>
                </li>
              ))}
            </ul>
            <p className="text-xs text-muted">
              Want to help?{" "}
              <button className="text-accent hover:underline" onClick={() => open(`${REPO}/issues`)}>
                Open an issue
              </button>{" "}
              with a bug or an idea.
            </p>
          </>
        )}

        {tab === "license" && (
          <>
            <Notice title="Riptor" text={licenseText} />
            <Notice title="Software shipped with Riptor" text={noticesText} />
          </>
        )}

        {tab === "software" && (
          <>
            <div className="overflow-hidden rounded-lg border border-border">
              <table className="w-full text-[13px]">
                <tbody>
                  {rows.map(([k, v]) => (
                    <tr key={k} className="border-b border-border last:border-0 odd:bg-raised/50">
                      <td className="w-1/2 px-3 py-1.5 text-muted">{k}</td>
                      <td className="px-3 py-1.5 font-mono text-xs">{v}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <div className="flex items-center gap-3">
              <button className="btn-ghost border border-border text-xs" disabled={!info} onClick={copy}>
                {copied ? <Check size={13} className="text-ok" /> : <Copy size={13} />} {copied ? "Copied" : "Copy to clipboard"}
              </button>
            </div>
            <p className="text-xs text-muted">
              Peer countries use{" "}
              <button className="text-accent hover:underline" onClick={() => open("https://db-ip.com")}>
                IP geolocation by DB-IP
              </button>
              , licensed under{" "}
              <button className="text-accent hover:underline" onClick={() => open("https://creativecommons.org/licenses/by/4.0/")}>
                CC BY 4.0
              </button>
              .
            </p>
          </>
        )}
      </div>

      {logs && createPortal(<LogsDialog onClose={() => setLogs(false)} />, document.body)}
    </div>
  );
}
