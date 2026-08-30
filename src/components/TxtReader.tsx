import { useEffect, useRef, useState } from "react";
import type { BookRow } from "../types";
import { getTextContent } from "../lib/api";

interface Props {
  book: BookRow;
  initialPage: number;
  onBack: () => void;
  onPageChange: (perMille: number) => void;
}

/** Plain-text reader: a clean, scrollable reading column. Progress = scroll %. */
export function TxtReader({ book, initialPage, onBack, onPageChange }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pct, setPct] = useState(Math.round(Math.max(0, initialPage) / 10));

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
    setPct(Math.round(p * 100));
    onPageChange(Math.round(p * 1000));
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onBack();
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
  }, [onBack]);

  return (
    <div className="reader">
      <div className="reader-bar">
        <button className="btn ghost" onClick={onBack}>
          ‹ Library
        </button>
        <span className="reader-title">{book.title}</span>
        <div className="reader-controls">
          <span className="page-counter">{pct}%</span>
        </div>
      </div>
      <div className="reader-stage">
        {error ? (
          <div className="page-loading">Couldn’t open: {error}</div>
        ) : text == null ? (
          <div className="page-loading">Loading…</div>
        ) : (
          <div ref={ref} className="txt-scroll" onScroll={onScroll}>
            <div className="txt-content">{text}</div>
          </div>
        )}
      </div>
    </div>
  );
}
