import { useEffect, useRef, useState, type ReactNode } from "react";

export interface MenuItem {
  label: string;
  icon?: ReactNode;
  onClick?: () => void;
  danger?: boolean;
  disabled?: boolean;
  /** For toggles, so several can be flipped in one go. */
  keepOpen?: boolean;
  children?: MenuItem[];
}

export function ContextMenu({ x, y, items, onClose }: { x: number; y: number; items: (MenuItem | "sep")[]; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x, y });
  const [open, setOpen] = useState<number | null>(null);

  useEffect(() => {
    const el = ref.current;
    if (el) {
      const r = el.getBoundingClientRect();
      setPos({ x: Math.min(x, window.innerWidth - r.width - 8), y: Math.min(y, window.innerHeight - r.height - 8) });
    }
    const close = (e: Event) => {
      if (e instanceof KeyboardEvent && e.key !== "Escape") return;
      if (e instanceof MouseEvent && ref.current?.contains(e.target as Node)) return;
      onClose();
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", close);
    window.addEventListener("blur", onClose);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", close);
      window.removeEventListener("blur", onClose);
    };
  }, [x, y, onClose]);

  const render = (list: (MenuItem | "sep")[]) =>
    list.map((it, i) =>
      it === "sep" ? (
        <div key={i} className="my-1 h-px bg-border" />
      ) : (
        <div key={i} className="relative" onMouseEnter={() => setOpen(it.children ? i : null)}>
          <button
            disabled={it.disabled}
            onClick={() => {
              if (it.children) return;
              it.onClick?.();
              if (!it.keepOpen) onClose();
            }}
            className={`flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-[13px] hover:bg-raised disabled:opacity-40 ${it.danger ? "text-danger" : ""}`}
          >
            <span className="w-4 text-muted">{it.icon}</span>
            <span className="flex-1">{it.label}</span>
            {it.children && <span className="text-muted">›</span>}
          </button>
          {it.children && open === i && (
            <div className="card absolute left-full top-0 ml-1 min-w-44 p-1 shadow-xl">{render(it.children)}</div>
          )}
        </div>
      ),
    );

  return (
    <div ref={ref} role="menu" className="card fixed z-50 min-w-52 p-1 shadow-xl" style={{ left: pos.x, top: pos.y }}>
      {render(items)}
    </div>
  );
}
