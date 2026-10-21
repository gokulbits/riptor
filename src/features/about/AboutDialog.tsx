import { ExternalLink } from "lucide-react";
import { useEffect, useState } from "react";
import { Dialog } from "../../components/ui";
import { api } from "../../lib/ipc";
import logo from "../../../app-icon.svg";
import { Updates } from "./Updates";

const REPO = "https://github.com/gokulbits/riptor";

export function AboutDialog({ onClose, onLogs }: { onClose: () => void; onLogs: () => void }) {
  const [version, setVersion] = useState("");
  useEffect(() => {
    api.version().then(setVersion).catch(() => {});
  }, []);

  const link = (label: string, url: string) => (
    <button className="btn-ghost border border-border" onClick={() => api.openWebUrl(url).catch(() => {})}>
      <ExternalLink size={14} /> {label}
    </button>
  );

  return (
    <Dialog
      title="About Riptor"
      onClose={onClose}
      width="max-w-md"
      footer={
        <>
          <button className="btn-ghost" onClick={onLogs}>
            View logs
          </button>
          <button className="btn-primary" onClick={onClose}>
            Close
          </button>
        </>
      }
    >
      <div className="flex flex-col items-center gap-3 text-center">
        <img src={logo} alt="" className="h-16 w-16 rounded-xl" />
        <div>
          <h3 className="text-lg font-semibold">Riptor</h3>
          <p className="text-sm text-muted">Version {version || "…"}</p>
        </div>
        <p className="text-[13px] text-muted">
          A BitTorrent client for Windows and Linux with built-in search, an app-only VPN and a kill switch.
        </p>
        <div className="flex flex-wrap justify-center gap-2">
          {link("Source code", REPO)}
          {link("Report a problem", `${REPO}/issues`)}
          {link("Licenses", `${REPO}/blob/main/THIRD-PARTY-NOTICES.md`)}
        </div>
        <Updates />
        <p className="text-xs text-muted">MIT License. Built on librqbit, Tauri and React.</p>
      </div>
    </Dialog>
  );
}
