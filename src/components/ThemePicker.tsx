import clsx from "clsx";
import { Check, Monitor, Palette } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { ACCENTS, THEMES, type ThemeDef } from "../lib/themes";
import { useApp } from "../stores/app";

function Swatch({ t }: { t: ThemeDef }) {
  const c = (k: keyof ThemeDef["tokens"]) => `rgb(${t.tokens[k]})`;
  return (
    <span className="flex h-6 w-10 shrink-0 overflow-hidden rounded-sm border" style={{ borderColor: c("border"), background: c("bg") }}>
      <span className="w-3" style={{ background: c("panel") }} />
      <span className="flex flex-1 flex-col justify-center gap-0.5 px-1">
        <span className="h-1 rounded-full" style={{ background: c("accent") }} />
        <span className="h-1 w-3/4 rounded-full" style={{ background: c("fg"), opacity: 0.6 }} />
      </span>
    </span>
  );
}

export function ThemePicker() {
  const { theme, accent, setTheme, setAccent } = useApp(
    useShallow((s) => ({ theme: s.theme, accent: s.accent, setTheme: s.setTheme, setAccent: s.setAccent })),
  );
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent | KeyboardEvent) => {
      if (e instanceof KeyboardEvent ? e.key === "Escape" : !ref.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", close);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", close);
    };
  }, [open]);

  const row = (id: string, label: string, preview: React.ReactNode) => (
    <button
      key={id}
      role="menuitemradio"
      aria-checked={theme === id}
      onClick={() => setTheme(id)}
      className="flex w-full items-center gap-2.5 rounded-sm px-2 py-1.5 text-left text-[13px] hover:bg-raised"
    >
      {preview}
      <span className="flex-1">{label}</span>
      {theme === id && <Check size={14} className="text-accent" />}
    </button>
  );

  return (
    <div ref={ref} className="relative">
      <button className="btn-ghost" onClick={() => setOpen((o) => !o)} title="Theme" aria-haspopup="menu" aria-expanded={open}>
        <Palette size={16} />
      </button>
      {open && (
        <div role="menu" className="card absolute right-0 top-full z-50 mt-1 w-64 p-1.5 shadow-xl">
          <p className="px-2 pb-1 pt-0.5 text-xxs font-semibold uppercase tracking-wider text-muted">Theme</p>
          {row(
            "system",
            "Follow system",
            <span className="flex h-6 w-10 items-center justify-center rounded-sm border border-border">
              <Monitor size={13} />
            </span>,
          )}
          {THEMES.map((t) => row(t.id, t.label, <Swatch t={t} />))}
          <p className="px-2 pb-1 pt-2 text-xxs font-semibold uppercase tracking-wider text-muted">Accent</p>
          <div className="flex flex-wrap gap-1.5 px-2 pb-1">
            {ACCENTS.map((a) => (
              <button
                key={a.id}
                title={a.label}
                aria-label={`Accent: ${a.label}`}
                aria-pressed={accent === a.id}
                onClick={() => setAccent(a.id)}
                className={clsx(
                  "h-6 w-6 rounded-full border-2 transition-transform hover:scale-110",
                  accent === a.id ? "border-fg" : "border-transparent",
                  a.id === "theme" && "bg-linear-to-br/srgb from-accent to-raised",
                )}
                style={a.rgb ? { background: `rgb(${a.rgb})` } : undefined}
              />
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
