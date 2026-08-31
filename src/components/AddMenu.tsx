import { useEffect, useRef, useState } from "react";

interface Props {
  onAddFolder: () => void;
  onAddBooks: () => void;
}

/** The "＋ Add" button: a small menu — a whole folder, or individual files. */
export function AddMenu({ onAddFolder, onAddBooks }: Props) {
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
        className="btn primary"
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="menu"
        aria-expanded={open}
      >
        ＋ Add ▾
      </button>
      {open && (
        <div className="refresh-menu-pop" role="menu">
          <button
            className="refresh-menu-item"
            role="menuitem"
            onClick={() => pick(onAddFolder)}
          >
            <span className="refresh-menu-title">Add a folder…</span>
            <span className="refresh-menu-sub">
              Scan a whole folder of books in place
            </span>
          </button>
          <button
            className="refresh-menu-item"
            role="menuitem"
            onClick={() => pick(onAddBooks)}
          >
            <span className="refresh-menu-title">Add books…</span>
            <span className="refresh-menu-sub">
              Pick files — Readaity copies them into the right folder
            </span>
          </button>
        </div>
      )}
    </div>
  );
}
