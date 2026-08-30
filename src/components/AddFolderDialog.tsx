import type { FolderMode, ProbeResult } from "../types";

interface Props {
  path: string;
  probe: ProbeResult;
  onChoose: (mode: FolderMode) => void;
  onCancel: () => void;
}

function folderName(p: string): string {
  const parts = p.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? p;
}

/**
 * Shown when a picked folder has comics nested in subfolders, so the user can
 * decide how it should be organised in the library.
 */
export function AddFolderDialog({ path, probe, onChoose, onCancel }: Props) {
  return (
    <div className="modal-overlay" onClick={onCancel}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2 className="modal-title">Add “{folderName(path)}”</h2>
        <p className="modal-sub">
          Found <strong>{probe.total}</strong> comics — <strong>{probe.nested}</strong>{" "}
          inside <strong>{probe.subfolders}</strong> subfolder
          {probe.subfolders === 1 ? "" : "s"}. How should they be organised?
        </p>

        <div className="modal-options">
          <button className="modal-option" onClick={() => onChoose("tree")}>
            <span className="opt-title">Keep folder structure</span>
            <span className="opt-desc">
              Add “{folderName(path)}” as one library folder; browse its subfolders.
            </span>
          </button>

          <button className="modal-option" onClick={() => onChoose("promote")}>
            <span className="opt-title">Add subfolders as top-level</span>
            <span className="opt-desc">
              Drop this wrapper — each subfolder becomes its own library entry,
              keeping its own structure.
            </span>
          </button>

          <button className="modal-option" onClick={() => onChoose("flat")}>
            <span className="opt-title">Flatten into one list</span>
            <span className="opt-desc">
              Ignore subfolders — show all {probe.total} comics in a single flat
              list.
            </span>
          </button>
        </div>

        <div className="modal-actions">
          <button className="btn ghost" onClick={onCancel}>
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}
