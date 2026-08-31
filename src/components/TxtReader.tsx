import { useCallback, useEffect, useRef, useState } from "react";
import type { BookRow, ReaderPrefs } from "../types";
import { getTextContent } from "../lib/api";
import { READER_THEMES } from "../lib/readerTheme";
import { BookmarkPanel, useBookmarks } from "./Bookmarks";

interface Props {
  book: BookRow;
  initialPage: number;
  prefs: ReaderPrefs;
  onBack: () => void;
  onPageChange: (perMille: number) => void;
}

/** Plain-text reader: a clean, scrollable reading column. Progress = scroll %. */
export function TxtReader({
  book,
  initialPage,
  prefs,
  onBack,
  onPageChange,
}: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const posRef = useRef(Math.max(0, initialPage));
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pct, setPct] = useState(Math.round(Math.max(0, initialPage) / 10));
  const [bmOpen, setBmOpen] = useState(false);
  const { bookmarks, add, remove } = useBookmarks(book.path);
  const theme = READER_THEMES[prefs.theme];

  useEffect(() => {
    let cancelled = false;
    getTextContent(book.path)
      .then((t) => !cancelled && setText(t))
      .catch((e) => !cancelled && setError(String(e)));
    return () => {
      cancelled = true;
    };
  }, [book.path]);

  // Restore scroll position once the text is laid out.
  useEffect(() => {
    if (text == null) return;
    const el = ref.current;
    if (el && initialPage > 0) {
      el.scrollTop = (initialPage / 1000) * (el.scrollHeight - el.clientHeight);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [text]);

  const onScroll = () => {
    const el = ref.current;
    if (!el) return;
    const max = el.scrollHeight - el.clientHeight;
    const p = max > 0 ? el.scrollTop / max : 0;
    posRef.current = Math.round(p * 1000);
    setPct(Math.round(p * 100));
    onPageChange(posRef.current);
  };

  const jumpToPerMille = useCallback((perMille: number) => {
    const el = ref.current;
    if (!el) return;
    el.scrollTop = (perMille / 1000) * (el.scrollHeight - el.clientHeight);
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (bmOpen) setBmOpen(false);
        else onBack();
        return;
      }
      if (e.key === "b") {
        setBmOpen((o) => !o);
        return;
      }
      const el = ref.current;
      if (!el) return;
      const step = el.clientHeight * 0.9;
      if (["ArrowRight", "PageDown", " "].includes(e.key)) {
        e.preventDefault();
        el.scrollBy(0, step);
      } else if (["ArrowLeft", "PageUp"].includes(e.key)) {
        e.preventDefault();
        el.scrollBy(0, -step);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onBack, bmOpen]);

  return (
    <div className="reader">
      <div className="reader-bar">
        <button className="btn ghost" onClick={onBack}>
          ‹ Library
        </button>
        <span className="reader-title">{book.title}</span>
        <div className="reader-controls">
          <button
            className={`btn ghost${bmOpen ? " active" : ""}`}
            onClick={() => setBmOpen((o) => !o)}
            title="Bookmarks (B)"
          >
            🔖 {bookmarks.length || ""}
          </button>
          <span className="page-counter">{pct}%</span>
        </div>
      </div>
      <div className="reader-stage">
        {bmOpen && (
          <BookmarkPanel
            bookmarks={bookmarks}
            describe={(p) => `${Math.round(p / 10)}%`}
            onAdd={() => add(posRef.current, "")}
            onRemove={remove}
            onJump={(p) => {
              jumpToPerMille(p);
              setBmOpen(false);
            }}
            onClose={() => setBmOpen(false)}
          />
        )}
        {error ? (
          <div className="page-loading">Couldn’t open: {error}</div>
        ) : text == null ? (
          <div className="page-loading">Loading…</div>
        ) : (
          <div
            ref={ref}
            className="txt-scroll"
            onScroll={onScroll}
            style={{
              background: theme.bg,
              color: theme.fg,
              fontSize: `${(18 * prefs.fontScale).toFixed(1)}px`,
            }}
          >
            <div className="txt-content">{text}</div>
          </div>
        )}
      </div>
    </div>
  );
}
