import { useCallback, useEffect, useState } from "react";
import type { Bookmark } from "../types";
import { addBookmark, listBookmarks, removeBookmark } from "../lib/api";

/** Load + mutate the bookmarks for one book. */
export function useBookmarks(path: string) {
  const [bookmarks, setBookmarks] = useState<Bookmark[]>([]);

  const reload = useCallback(() => {
    listBookmarks(path)
      .then(setBookmarks)
      .catch((e) => console.error("load bookmarks failed", e));
  }, [path]);

  useEffect(() => {
    reload();
  }, [reload]);

  const add = useCallback(
    async (position: number, label: string) => {
      try {
        const bm = await addBookmark(path, Math.round(position), label);
        setBookmarks((prev) =>
          [...prev, bm].sort((a, b) => a.position - b.position || a.id - b.id),
        );
      } catch (e) {
        console.error("add bookmark failed", e);
      }
    },
    [path],
  );

  const remove = useCallback(async (id: number) => {
    try {
      await removeBookmark(id);
      setBookmarks((prev) => prev.filter((b) => b.id !== id));
    } catch (e) {
      console.error("remove bookmark failed", e);
    }
  }, []);

  return { bookmarks, add, remove, reload };
}

interface PanelProps {
  bookmarks: Bookmark[];
  /** Human label for a stored position, e.g. "Page 12" or "34%". */
  describe: (position: number) => string;
  onAdd: () => void;
  onRemove: (id: number) => void;
  onJump: (position: number) => void;
  onClose: () => void;
}

/** Slide-in bookmarks list — mirrors the reader "Contents" panel. */
export function BookmarkPanel({
  bookmarks,
  describe,
  onAdd,
  onRemove,
  onJump,
  onClose,
}: PanelProps) {
  return (
    <nav className="toc-panel">
      <div className="toc-head">
        <span>Bookmarks</span>
        <button className="btn small ghost" onClick={onClose} aria-label="Close">
          ✕
        </button>
      </div>
      <button className="btn small bm-add" onClick={onAdd}>
        ＋ Bookmark this spot
      </button>
      {bookmarks.length === 0 ? (
        <p className="bm-empty">No bookmarks yet.</p>
      ) : (
        <ul>
          {bookmarks.map((b) => (
            <li key={b.id} className="bm-row">
              <button className="bm-jump" onClick={() => onJump(b.position)}>
                <span className="bm-where">{describe(b.position)}</span>
                {b.label && <span className="bm-label">{b.label}</span>}
              </button>
              <button
                className="btn small ghost bm-del"
                onClick={() => onRemove(b.id)}
                aria-label="Delete bookmark"
                title="Delete bookmark"
              >
                ✕
              </button>
            </li>
          ))}
        </ul>
      )}
    </nav>
  );
}
