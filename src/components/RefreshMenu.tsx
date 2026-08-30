import { useEffect, useRef, useState } from "react";

interface Props {
  onRescan: () => void;
  onReindex: () => void;
}

/** The ↻ button: a small menu offering the two kinds of refresh. */
export function RefreshMenu({ onRescan, onReindex }: Props) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const pick = (fn: () => void) => {
    setOpen(false);
    fn();
  };

  return (
    <div className="refresh-menu" ref={ref}>
      <button
        className="btn ghost icon"
        onClick={() => setOpen((v) => !v)}
        title="Refresh library"
        aria-label="Refresh library"
        aria-haspopup="menu"
        aria-expanded={open}
      >
        ↻
      </button>
      {open && (
        <div className="refresh-menu-pop" role="menu">
          <button
            className="refresh-menu-item"
            role="menuitem"
            onClick={() => pick(onRescan)}
          >
            <span className="refresh-menu-title">Scan for new books</span>
            <span className="refresh-menu-sub">
              Add files that appeared, drop ones that are gone
            </span>
          </button>
          <button
            className="refresh-menu-item"
            role="menuitem"
            onClick={() => pick(onReindex)}
          >
            <span className="refresh-menu-title">Rebuild covers &amp; metadata</span>
            <span className="refresh-menu-sub">
              Re-read every book — covers, page counts, hashes
            </span>
          </button>
        </div>
      )}
    </div>
  );
}
