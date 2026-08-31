import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { BookRow } from "../types";
import { getKf8PageDims, getKf8Page, type Kf8Page } from "../lib/api";

interface Props {
  book: BookRow;
  initialPage: number;
  onBack: () => void;
  onPageChange: (page: number) => void;
  suggestComics?: boolean;
  onMoveToComics?: () => void;
}

type FitMode = "width" | "height" | "page";

/**
 * Fixed-layout KF8 reader (comics / manga / picture books). The container is
 * reassembled once (cached backend-side); each page is a self-contained HTML
 * document sized in CSS px, fetched lazily and rendered in a sandboxed iframe
 * scaled to fit the stage.
 */
export function Kf8Reader({
  book,
  initialPage,
  onBack,
  onPageChange,
  suggestComics,
  onMoveToComics,
}: Props) {
  const [dims, setDims] = useState<[number, number][] | null>(null);
  const [nudgeDismissed, setNudgeDismissed] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [page, setPage] = useState(Math.max(0, initialPage));
  const [fit, setFit] = useState<FitMode>("page");
  const [chromeVisible, setChromeVisible] = useState(true);
  const [stage, setStage] = useState({ w: 0, h: 0 });
  const stageRef = useRef<HTMLDivElement>(null);

  // page index → html; a tick forces re-render when one lands.
  const htmlCache = useRef<Map<number, string>>(new Map());
  const [, setTick] = useState(0);

  // Reassemble + get page dimensions once.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const d = await getKf8PageDims(book.path);
        if (cancelled) return;
        if (d.length === 0) setError("This book has no readable pages.");
        else setDims(d);
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [book.path]);

  const count = dims?.length ?? 0;
  const clamped = Math.min(page, Math.max(0, count - 1));
  const dim = dims?.[clamped];

  // Fetch the current page (and neighbours) lazily.
  useEffect(() => {
    if (!dims) return;
    const want = [clamped, clamped + 1, clamped - 1].filter(
      (i) => i >= 0 && i < count && !htmlCache.current.has(i),
    );
    let cancelled = false;
    want.forEach(async (i) => {
      try {
        const p: Kf8Page = await getKf8Page(book.path, i);
        if (cancelled) return;
        htmlCache.current.set(i, p.html);
        setTick((t) => t + 1);
      } catch (e) {
        if (i === clamped && !cancelled) setError(String(e));
      }
    });
    return () => {
      cancelled = true;
    };
  }, [dims, clamped, count, book.path]);

  // Track stage size for scaling.
  useEffect(() => {
    const el = stageRef.current;
    if (!el) return;
    const update = () => setStage({ w: el.clientWidth, h: el.clientHeight });
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  }, [dims]);

  useEffect(() => {
    if (dims) onPageChange(clamped);
  }, [clamped, dims, onPageChange]);

  const go = useCallback(
    (d: number) =>
      setPage((p) => Math.min(Math.max(p + d, 0), Math.max(0, count - 1))),
    [count],
  );

  const cycleFit = useCallback(
    () =>
      setFit((m) => (m === "page" ? "width" : m === "width" ? "height" : "page")),
    [],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (["ArrowRight", "PageDown", " "].includes(e.key)) {
        e.preventDefault();
        go(1);
      } else if (["ArrowLeft", "PageUp"].includes(e.key)) {
        e.preventDefault();
        go(-1);
      } else if (e.key === "Home") setPage(0);
      else if (e.key === "End") setPage(count - 1);
      else if (e.key === "Escape") onBack();
      else if (e.key === "f") cycleFit();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [go, count, onBack, cycleFit]);

  const scale = useMemo(() => {
    if (!dim || !stage.w || !stage.h) return 1;
    const sw = stage.w / dim[0];
    const sh = stage.h / dim[1];
    return fit === "width" ? sw : fit === "height" ? sh : Math.min(sw, sh);
  }, [dim, stage, fit]);

  const html = htmlCache.current.get(clamped);

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
            onClick={cycleFit}
            title="Cycle fit: whole page / width / height (F)"
          >
            {fit === "page" ? "Fit page" : fit === "width" ? "Fit width" : "Fit height"}
          </button>
          <span className="page-counter">
            {count ? `${clamped + 1} / ${count}` : "…"}
          </span>
        </div>
      </div>

      {suggestComics && onMoveToComics && !nudgeDismissed && (
        <div className="kf8-nudge">
          <span>
            This is a fixed-layout book — it belongs in your Comics library for
            proper page-turning.
          </span>
          <button className="btn" onClick={onMoveToComics}>
            Move to Comics
          </button>
          <button
            className="btn ghost"
            onClick={() => setNudgeDismissed(true)}
            aria-label="Dismiss"
          >
            ✕
          </button>
        </div>
      )}

      <div className="reader-stage">
        <button
          className="nav-arrow left"
          onClick={() => go(-1)}
          disabled={clamped === 0}
          aria-label="Previous page"
        >
          <span className="nav-chevron">‹</span>
        </button>

        <div className="pages kf8-stage" ref={stageRef}>
          {error ? (
            <div className="page-loading">Couldn’t open: {error}</div>
          ) : !dim ? (
            <div className="page-loading">Rebuilding pages…</div>
          ) : !html ? (
            <div className="page-loading">Loading page…</div>
          ) : (
            <div
              className="kf8-page-frame"
              style={{ width: dim[0] * scale, height: dim[1] * scale }}
            >
              <iframe
                key={clamped}
                title={`Page ${clamped + 1}`}
                sandbox=""
                srcDoc={html}
                style={{
                  width: dim[0],
                  height: dim[1],
                  border: 0,
                  transform: `scale(${scale})`,
                  transformOrigin: "top left",
                }}
              />
            </div>
          )}
        </div>

        <button
          className="nav-arrow right"
          onClick={() => go(1)}
          disabled={count > 0 && clamped >= count - 1}
          aria-label="Next page"
        >
          <span className="nav-chevron">›</span>
        </button>
      </div>
    </div>
  );
}
