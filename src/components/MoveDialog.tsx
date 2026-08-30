import type { MoveAction } from "../types";

interface Props {
  destName: string;
  collisions: { src: string; name: string }[];
  onResolve: (action: MoveAction) => void;
  onCancel: () => void;
}

/**
 * Shown when a drag-move would overwrite existing items. The chosen action
 * applies to all conflicting items (non-conflicting ones just move).
 */
export function MoveDialog({ destName, collisions, onResolve, onCancel }: Props) {
  return (
    <div className="modal-overlay" onClick={onCancel}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2 className="modal-title">Name conflict</h2>
        <p className="modal-sub">
          <strong>{collisions.length}</strong> item
          {collisions.length === 1 ? "" : "s"} already exist in “{destName}”. What
          should happen to {collisions.length === 1 ? "it" : "them"}?
        </p>

        <ul className="conflict-list">
          {collisions.slice(0, 8).map((c) => (
            <li key={c.src}>{c.name}</li>
          ))}
          {collisions.length > 8 && <li>…and {collisions.length - 8} more</li>}
        </ul>

        <div className="modal-options">
          <button className="modal-option" onClick={() => onResolve("rename")}>
            <span className="opt-title">Keep both (rename)</span>
            <span className="opt-desc">
              The moved item gets “ (2)” appended so nothing is overwritten.
            </span>
          </button>
          <button className="modal-option" onClick={() => onResolve("replace")}>
            <span className="opt-title">Replace</span>
            <span className="opt-desc">
              Overwrite the existing item at the destination.
            </span>
          </button>
          <button className="modal-option" onClick={() => onResolve("skip")}>
            <span className="opt-title">Skip</span>
            <span className="opt-desc">
              Leave the conflicting item where it is; move the rest.
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
