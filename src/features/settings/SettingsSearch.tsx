import clsx from "clsx";
import { type SettingEntry, sectionName } from "./searchIndex";

export function SearchResults({
  query,
  results,
  active,
  onPick,
  onHover,
}: {
  query: string;
  results: SettingEntry[];
  active: number;
  onPick: (e: SettingEntry) => void;
  onHover: (i: number) => void;
}) {
  if (results.length === 0) return <p className="text-sm text-muted">No settings match “{query.trim()}”.</p>;
  return (
    <ul id="settings-results" role="listbox" aria-label="Matching settings" className="space-y-0.5">
      {results.map((r, i) => (
        <li
          key={`${r.section}|${r.group}|${r.label}`}
          id={`settings-result-${i}`}
          role="option"
          aria-selected={i === active}
          onMouseEnter={() => onHover(i)}
          onClick={() => onPick(r)}
          className={clsx("cursor-pointer rounded-md px-3 py-2", i === active ? "bg-accent/15" : "hover:bg-raised")}
        >
          <div className="text-sm">{r.label}</div>
          <div className="text-xs text-muted">
            {sectionName(r.section)}
            {r.group !== r.label && r.group !== sectionName(r.section) && ` › ${r.group}`}
          </div>
        </li>
      ))}
    </ul>
  );
}

const FLASH = ["bg-accent/15", "rounded-md"];

/** Scrolls to the setting (or its group), flashes it and puts focus on its control. Returns a cleanup. */
export function reveal(root: HTMLElement, entry: SettingEntry): () => void {
  const group = [...root.querySelectorAll("fieldset")].find(
    (f) => f.querySelector(":scope > legend")?.textContent?.trim().toLowerCase() === entry.group.toLowerCase(),
  );
  const scope: ParentNode = group ?? root;
  const labelled = [...scope.querySelectorAll<HTMLElement>("[data-label]")].find((el) => el.dataset.label === entry.label);
  const target = labelled ?? group;
  if (!target) return () => {};
  target.scrollIntoView?.({ block: "center", behavior: "smooth" });
  target.classList.add(...FLASH);
  labelled?.querySelector<HTMLElement>("input,select,textarea,button")?.focus({ preventScroll: true });
  const t = window.setTimeout(() => target.classList.remove(...FLASH), 1600);
  return () => {
    window.clearTimeout(t);
    target.classList.remove(...FLASH);
  };
}
