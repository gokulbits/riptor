import clsx from "clsx";
import { X } from "lucide-react";
import { useEffect, useRef, type ReactNode } from "react";
import type { TorrentState } from "../lib/types";

export function Dialog({
  title,
  onClose,
  children,
  footer,
  width = "max-w-2xl",
}: {
  title: ReactNode;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  width?: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    ref.current?.querySelector<HTMLElement>("input,button,select,textarea")?.focus();
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <div className="fixed inset-0 z-40 flex items-center justify-center bg-black/50 p-4 backdrop-blur-[2px]" onMouseDown={onClose}>
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        className={clsx("card flex max-h-[90vh] w-full flex-col shadow-2xl", width)}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-border px-4 py-3">
          <h2 className="truncate text-base font-semibold">{title}</h2>
          <button className="btn-ghost -mr-1 p-1" onClick={onClose} aria-label="Close">
            <X size={16} />
          </button>
        </div>
        <div className="min-h-0 flex-1 overflow-auto p-4">{children}</div>
        {footer && <div className="flex justify-end gap-2 border-t border-border px-4 py-3">{footer}</div>}
      </div>
    </div>
  );
}

export function Field({ label, hint, children }: { label: string; hint?: ReactNode; children: ReactNode }) {
  // Not space-y-1: v4 puts that gap on the inline label span, where it does nothing.
  return (
    <label className="block [&>*+*]:mt-1">
      <span className="label">{label}</span>
      {children}
      {hint && <span className="block text-xs text-muted">{hint}</span>}
    </label>
  );
}

export function Toggle({
  checked,
  onChange,
  label,
  hint,
  disabled,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: ReactNode;
  hint?: ReactNode;
  disabled?: boolean;
}) {
  return (
    <label className={clsx("flex cursor-pointer items-start gap-3 py-1", disabled && "opacity-50")}>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        disabled={disabled}
        onClick={() => onChange(!checked)}
        className={clsx(
          "relative mt-0.5 block h-5 w-9 shrink-0 rounded-full p-0 transition-colors",
          checked ? "bg-accent" : "bg-border",
        )}
      >
        <span
          className={clsx(
            "absolute left-0 top-0.5 h-4 w-4 rounded-full bg-white shadow-sm transition-transform",
            checked ? "translate-x-[18px]" : "translate-x-0.5",
          )}
        />
      </button>
      <span>
        <span className="block text-sm">{label}</span>
        {hint && <span className="block text-xs text-muted">{hint}</span>}
      </span>
    </label>
  );
}

/** Empty means no limit (null). */
export function OptionalNumber({
  value,
  onChange,
  placeholder = "Unlimited",
  min = 0,
  step,
}: {
  value: number | null;
  onChange: (v: number | null) => void;
  placeholder?: string;
  min?: number;
  step?: number;
}) {
  return (
    <input
      className="input"
      type="number"
      min={min}
      step={step}
      placeholder={placeholder}
      value={value ?? ""}
      onChange={(e) => {
        const v = e.target.value;
        if (v === "") return onChange(null);
        const n = Number(v);
        if (Number.isFinite(n) && n >= min) onChange(n);
      }}
    />
  );
}

export function ProgressBar({ value, state }: { value: number; state?: TorrentState }) {
  const color =
    state === "error"
      ? "bg-danger"
      : state === "stopped" || state === "queued"
        ? "bg-muted"
        : value >= 1
          ? "bg-ok"
          : "bg-accent";
  return (
    <div className="h-1.5 w-full overflow-hidden rounded-full bg-raised">
      <div className={clsx("h-full rounded-full transition-[width] duration-500", color)} style={{ width: `${Math.min(100, value * 100)}%` }} />
    </div>
  );
}

const STATE_LABEL: Record<TorrentState, string> = {
  metadata: "Metadata",
  checking: "Checking",
  downloading: "Downloading",
  stalled_dl: "Stalled",
  seeding: "Seeding",
  stalled_up: "Seeding",
  queued: "Queued",
  stopped: "Stopped",
  error: "Error",
};

const STATE_COLOR: Record<TorrentState, string> = {
  metadata: "text-info",
  checking: "text-info",
  downloading: "text-accent",
  stalled_dl: "text-warn",
  seeding: "text-ok",
  stalled_up: "text-ok/70",
  queued: "text-muted",
  stopped: "text-muted",
  error: "text-danger",
};

export function StateLabel({ state, forced }: { state: TorrentState; forced?: boolean }) {
  return (
    <span className={clsx("font-medium", STATE_COLOR[state])}>
      {forced && state !== "stopped" ? "[F] " : ""}
      {STATE_LABEL[state]}
    </span>
  );
}

export function Tabs<T extends string>({
  tabs,
  value,
  onChange,
}: {
  tabs: { id: T; label: string }[];
  value: T;
  onChange: (t: T) => void;
}) {
  return (
    <div role="tablist" className="flex gap-1 border-b border-border px-2">
      {tabs.map((t) => (
        <button
          key={t.id}
          role="tab"
          aria-selected={value === t.id}
          onClick={() => onChange(t.id)}
          className={clsx(
            "-mb-px border-b-2 px-3 py-2 text-xs font-medium transition-colors",
            value === t.id ? "border-accent text-fg" : "border-transparent text-muted hover:text-fg",
          )}
        >
          {t.label}
        </button>
      ))}
    </div>
  );
}
