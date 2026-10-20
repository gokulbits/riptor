import { formatRate } from "../../lib/format";
import { useApp } from "../../stores/app";

const W = 600;
const H = 140;

function path(values: number[], max: number): string {
  if (values.length < 2) return "";
  const step = W / (values.length - 1);
  return values.map((v, i) => `${i === 0 ? "M" : "L"}${(i * step).toFixed(1)},${(H - (v / max) * H).toFixed(1)}`).join(" ");
}

export function SpeedGraph() {
  const history = useApp((s) => s.history);
  const dl = history.map((h) => h.dl);
  const ul = history.map((h) => h.ul);
  const max = Math.max(1024, ...dl, ...ul) * 1.1;
  return (
    <div className="p-3">
      <div className="mb-2 flex gap-4 text-xs">
        <span className="flex items-center gap-1.5">
          <span className="h-0.5 w-3 rounded bg-accent" /> Download {formatRate(dl.at(-1) ?? 0)}
        </span>
        <span className="flex items-center gap-1.5">
          <span className="h-0.5 w-3 rounded bg-ok" /> Upload {formatRate(ul.at(-1) ?? 0)}
        </span>
        <span className="ml-auto text-muted">peak {formatRate(max / 1.1)}</span>
      </div>
      <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" className="h-36 w-full rounded-md border border-border bg-raised/40" role="img" aria-label="Speed over the last five minutes">
        {[0.25, 0.5, 0.75].map((f) => (
          <line key={f} x1={0} x2={W} y1={H * f} y2={H * f} className="stroke-border" strokeWidth={1} vectorEffect="non-scaling-stroke" />
        ))}
        <path d={path(dl, max)} fill="none" className="stroke-accent" strokeWidth={2} vectorEffect="non-scaling-stroke" />
        <path d={path(ul, max)} fill="none" className="stroke-ok" strokeWidth={2} vectorEffect="non-scaling-stroke" />
      </svg>
    </div>
  );
}
