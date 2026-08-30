import { useCallback, useEffect, useRef, useState } from "react";
import type { BookRow } from "../types";
import { getPageUrl } from "../lib/api";

interface Props {
  book: BookRow;
  initialPage: number;
  onBack: () => void;
  onPageChange: (page: number) => void;
}

type FitMode = "width" | "height";

/** Which page indices are visible for a given primary (left) page. */
function spreadPages(page: number, spread: boolean, count: number): number[] {
  // Cover (page 0) is always shown alone; then pages pair up.
  if (!spread || page === 0) return [page];
  return page + 1 <= count - 1 ? [page, page + 1] : [page];
}

function nextIndex(p: number, spread: boolean, count: number): number {
  if (!spread) return Math.min(p + 1, count - 1);
  if (p === 0) return count > 1 ? 1 : 0;
  const cand = p + 2;
  return cand <= count - 1 ? cand : p; // stay if no further spread
}

function prevIndex(p: number, spread: boolean): number {
  if (!spread) return Math.max(p - 1, 0);
  if (p <= 1) return 0;
  return p - 2;
}

/** Left page index of the last spread/page. */
function endIndex(count: number, spread: boolean): number {
  const last = count - 1;
  if (!spread || last <= 0) return Math.max(last, 0);
  return last % 2 === 1 ? last : last - 1;
}

export function Reader({ book, initialPage, onBack, onPageChange }: Props) {
  const count = book.page_count;
  const [page, setPage] = useState(clamp(initialPage, count));
  const [spread, setSpread] = useState(false);
  const [fit, setFit] = useState<FitMode>("height");
  const [chromeVisible, setChromeVisible] = useState(true);

  // Cache of page index → data URL; `tick` forces a re-render when one lands.
  const cache = useRef<Map<number, string>>(new Map());
  const [, setTick] = useState(0);

  const ensure = useCallback(
    async (indices: number[]) => {
      await Promise.all(
        indices.map(async (i) => {
          if (i < 0 || i >= count || cache.current.has(i)) return;
          try {
            cache.current.set(i, await getPageUrl(book.path, i));
            setTick((t) => t + 1);
          } catch (e) {
            console.error("page load failed", e);
          }
        }),
      );
    },
    [book.path, count],
  );

  const pages = spreadPages(page, spread, count);

  // Load visible pages, report progress, and preload the next spread.
  useEffect(() => {
    ensure(pages);
    onPageChange(page);
    const np = nextIndex(page, spread, count);
    if (np !== page) ensure(spreadPages(np, spread, count));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [page, spread, book.path, count]);

  const go = useCallback(
    (dir: number) => {
      setPage((p) =>
        dir > 0 ? nextIndex(p, spread, count) : prevIndex(p, spread),
      );
    },
    [spread, count],
  );

  const toggleSpread = useCallback(() => {
    setSpread((s) => {
      const ns = !s;
      // Align the primary page to an odd (left) index when enabling spreads.
      if (ns && page > 0 && page % 2 === 0) setPage(page - 1);
      return ns;
    });
  }, [page]);

  // Keyboard navigation.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      switch (e.key) {
        case "ArrowRight":
        case "PageDown":
        case " ":
          e.preventDefault();
          go(1);
          break;
        case "ArrowLeft":
        case "PageUp":
          e.preventDefault();
          go(-1);
          break;
        case "Home":
          setPage(0);
          break;
        case "End":
          setPage(endIndex(count, spread));
          break;
        case "Escape":
          onBack();
          break;
        case "f":
          setFit((m) => (m === "width" ? "height" : "width"));
          break;
        case "d":
          toggleSpread();
          break;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [go, onBack, count, spread, toggleSpread]);

  const atStart = page === 0;
  const atEnd = pages[pages.length - 1] >= count - 1;

  const counter =
    pages.length === 1
      ? `${pages[0] + 1} / ${count}`
      : `${pages[0] + 1}–${pages[1] + 1} / ${count}`;

  return (
    <div className="reader" onMouseMove={() => setChromeVisible(true)}>
      <div className={`reader-bar${chromeVisible ? "" : " hidden"}`}>
        <button className="btn ghost" onClick={onBack}>
          ‹ Library
        </button>
        <span className="reader-title">{book.title}</span>
        <div className="reader-controls">
          <button
            className="btn ghost"
            onClick={toggleSpread}
            title="Toggle single / two-page view (D)"
          >
            {spread ? "▐▐ Two pages" : "▐ Single page"}
          </button>
          <button
            className="btn ghost"
            onClick={() => setFit((m) => (m === "width" ? "height" : "width"))}
            title="Toggle fit (F)"
          >
            {fit === "width" ? "Fit height" : "Fit width"}
          </button>
          <span className="page-counter">{counter}</span>
        </div>
      </div>

      <div className="reader-stage">
        <button
          className="nav-arrow left"
          onClick={() => go(-1)}
          disabled={atStart}
          aria-label="Previous page"
        >
          <span className="nav-chevron">‹</span>
        </button>

        <div className={`pages${pages.length > 1 ? " dual" : ""}`}>
          {pages.map((i) => {
            const u = cache.current.get(i);
            return u ? (
              <img
                key={i}
                className={`page-image fit-${fit}`}
                src={u}
                alt={`Page ${i + 1}`}
                draggable={false}
              />
            ) : (
              <div key={i} className="page-loading">
                Loading…
              </div>
            );
          })}
        </div>

        <button
          className="nav-arrow right"
          onClick={() => go(1)}
          disabled={atEnd}
          aria-label="Next page"
        >
          <span className="nav-chevron">›</span>
        </button>
      </div>
    </div>
  );
}

function clamp(n: number, count: number): number {
  if (Number.isNaN(n) || n < 0) return 0;
  return Math.min(n, Math.max(0, count - 1));
}
