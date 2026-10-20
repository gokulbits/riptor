import clsx from "clsx";
import { Eye, EyeOff, Loader2, RefreshCw } from "lucide-react";
import { useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { useApp } from "../stores/app";

const mask = (ip: string) => ip.replace(/[0-9a-f]/gi, "•");

/** Masked until revealed; never remembered, so a new session starts hidden. */
export function PublicIpView({ compact }: { compact?: boolean }) {
  const { value, error, loading, refresh } = useApp(
    useShallow((s) => ({ ...s.publicIp, refresh: s.refreshPublicIp })),
  );
  const [shown, setShown] = useState(false);
  const via = value?.via_vpn ? "through the VPN" : "on your normal connection";
  const title = error
    ? `Couldn't check the public IP: ${error}`
    : value
      ? `Your public IP ${via}, as peers and trackers see it. Click to check again.`
      : "Check the public IP";

  return (
    <span className={clsx("flex items-center gap-1", compact ? "text-xs" : "text-sm")}>
      <button
        className="flex items-center gap-1 rounded-sm px-1 py-0.5 tabular-nums hover:bg-raised"
        onClick={refresh}
        title={title}
        aria-label="Public IP address"
      >
        {loading ? (
          <Loader2 size={12} className="animate-spin" />
        ) : !value ? (
          <RefreshCw size={12} />
        ) : null}
        <span className={clsx("font-mono", error && "text-muted")}>
          {value ? (shown ? value.ip : mask(value.ip)) : error ? "IP unavailable" : "IP"}
        </span>
      </button>
      {value && (
        <button
          className="rounded-sm p-0.5 text-muted hover:bg-raised hover:text-fg"
          onClick={() => setShown(!shown)}
          title={shown ? "Hide IP address" : "Show IP address"}
          aria-label={shown ? "Hide IP address" : "Show IP address"}
          aria-pressed={shown}
        >
          {shown ? <EyeOff size={12} /> : <Eye size={12} />}
        </button>
      )}
    </span>
  );
}
