import { useEffect, useMemo, useState } from "react";
import type { ImportPlan, LibraryKind } from "../types";

function baseName(p: string): string {
  return p.split(/[\\/]/).filter(Boolean).pop() ?? p;
}

interface Props {
  library: LibraryKind;
  plans: ImportPlan[];
  busy: boolean;
  onImport: (items: { path: string; dest: string }[]) => void;
  onAddFolderFirst: () => void;
  onCancel: () => void;
}

/**
 * Confirm where each picked book file should be copied. Readaity pre-selects
 * the best-matching existing folder; the user can override per file.
 */
export function AddBooksDialog({
  library,
  plans,
  busy,
  onImport,
  onAddFolderFirst,
  onCancel,
}: Props) {
  // path -> chosen destination folder
  const [dest, setDest] = useState<Record<string, string>>({});

  const hasFolders = plans.some((p) => p.suggestions.length > 0);

  useEffect(() => {
    const init: Record<string, string> = {};
    for (const p of plans) {
      if (p.suggestions[0]) init[p.path] = p.suggestions[0].folder;
    }
    setDest(init);
  }, [plans]);

  const items = useMemo(
    () =>
      plans
        .filter((p) => dest[p.path])
        .map((p) => ({ path: p.path, dest: dest[p.path] })),
    [plans, dest],
  );

  return (
    <div className="modal-overlay" onClick={onCancel}>
      <div
        className="modal add-books"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="modal-title">
          Add {plans.length} book{plans.length === 1 ? "" : "s"} to{" "}
          {library === "comics" ? "Comics" : "Ebooks"}
        </h2>

        {!hasFolders ? (
          <>
            <p className="modal-sub">
              Books are copied into one of your library folders — you don’t have
              any {library === "comics" ? "Comics" : "Ebooks"} folders yet.
            </p>
            <div className="modal-options">
              <button className="modal-option" onClick={onAddFolderFirst}>
                <span className="opt-title">Add a folder first</span>
                <span className="opt-desc">Then come back and add these books.</span>
              </button>
              <button className="modal-option" onClick={onCancel}>
                <span className="opt-title">Cancel</span>
              </button>
            </div>
          </>
        ) : (
          <>
            <p className="modal-sub">
              Each file is copied into the folder you choose. The originals stay
              where they are.
            </p>
            <ul className="import-list">
              {plans.map((p) => {
                const top = p.suggestions[0];
                return (
                  <li key={p.path} className="import-row">
                    <div className="import-file">
                      <span className="import-name">{baseName(p.path)}</span>
                      {top && dest[p.path] === top.folder && (
                        <span className="import-reason">{top.reason}</span>
                      )}
                    </div>
                    <select
                      className="import-dest"
                      value={dest[p.path] ?? ""}
                      onChange={(e) =>
                        setDest((d) => ({ ...d, [p.path]: e.target.value }))
                      }
                    >
                      {p.suggestions.map((s, i) => (
                        <option key={s.folder} value={s.folder}>
                          {baseName(s.folder)}
                          {i === 0 ? "  — suggested" : ""}
                        </option>
                      ))}
                    </select>
                  </li>
                );
              })}
            </ul>
            <div className="modal-actions">
              <button className="btn ghost" onClick={onCancel} disabled={busy}>
                Cancel
              </button>
              <button
                className="btn primary"
                onClick={() => onImport(items)}
                disabled={busy || items.length === 0}
              >
                {busy
                  ? "Importing…"
                  : `Import ${items.length} book${items.length === 1 ? "" : "s"}`}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
