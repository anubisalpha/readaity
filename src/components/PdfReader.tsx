import { useCallback, useEffect, useRef, useState } from "react";
import * as pdfjsLib from "pdfjs-dist";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";
import type { BookRow } from "../types";
import { readBookBytes, setCover } from "../lib/api";

pdfjsLib.GlobalWorkerOptions.workerSrc = workerUrl;

interface Props {
  book: BookRow;
  initialPage: number;
  onBack: () => void;
  onPageChange: (page: number) => void;
}

function b64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

/** PDF reader backed by pdf.js. Renders a page to a canvas; caches a cover. */
export function PdfReader({ book, initialPage, onBack, onPageChange }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const docRef = useRef<any>(null);
  const [page, setPage] = useState(Math.max(0, initialPage));
  const [numPages, setNumPages] = useState(book.page_count || 0);
  const [fitWidth, setFitWidth] = useState(true);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Load the document once.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const bytes = b64ToBytes(await readBookBytes(book.path));
        const doc = await pdfjsLib.getDocument({ data: bytes }).promise;
        if (cancelled) return;
        docRef.current = doc;
        setNumPages(doc.numPages);
        setLoading(false);
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    })();
    return () => {
      cancelled = true;
      docRef.current?.destroy?.();
    };
  }, [book.path]);

  const render = useCallback(
    async (p: number) => {
      const doc = docRef.current;
      const canvas = canvasRef.current;
      if (!doc || !canvas) return;
      const pg = await doc.getPage(p + 1); // pdf.js is 1-based
      const container = canvas.parentElement as HTMLElement;
      const base = pg.getViewport({ scale: 1 });
      const scale = fitWidth
        ? container.clientWidth / base.width
        : (container.clientHeight - 4) / base.height;
      const viewport = pg.getViewport({ scale: Math.max(scale, 0.1) });
      canvas.width = viewport.width;
      canvas.height = viewport.height;
      const ctx = canvas.getContext("2d")!;
      await pg.render({ canvasContext: ctx, viewport }).promise;

      // Generate + cache a cover from page 1 the first time.
      if (p === 0 && !book.has_cover) cacheCover(pg);
    },
    [fitWidth, book.has_cover],
  );

  // Render on page / fit change.
  useEffect(() => {
    if (loading) return;
    render(page);
    onPageChange(page);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [page, loading, fitWidth]);

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const cacheCover = async (pg: any) => {
    try {
      const base = pg.getViewport({ scale: 1 });
      const scale = Math.min(360 / base.width, 540 / base.height);
      const vp = pg.getViewport({ scale });
      // Viewport dims are fractional; round before they reach the DB / Tauri.
      const w = Math.round(vp.width);
      const h = Math.round(vp.height);
      const c = document.createElement("canvas");
      c.width = w;
      c.height = h;
      await pg.render({ canvasContext: c.getContext("2d")!, viewport: vp }).promise;
      const data = c.toDataURL("image/jpeg", 0.8).split(",")[1];
      await setCover(book.path, data, w, h);
    } catch {
      /* cover is best-effort */
    }
  };

  const go = useCallback(
    (d: number) => setPage((p) => Math.min(Math.max(p + d, 0), numPages - 1)),
    [numPages],
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
      else if (e.key === "End") setPage(numPages - 1);
      else if (e.key === "Escape") onBack();
      else if (e.key === "f") setFitWidth((f) => !f);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [go, numPages, onBack]);

  return (
    <div className="reader">
      <div className="reader-bar">
        <button className="btn ghost" onClick={onBack}>
          ‹ Library
        </button>
        <span className="reader-title">{book.title}</span>
        <div className="reader-controls">
          <button
            className="btn ghost"
            onClick={() => setFitWidth((f) => !f)}
            title="Toggle fit (F)"
          >
            {fitWidth ? "Fit height" : "Fit width"}
          </button>
          <span className="page-counter">
            {numPages ? `${page + 1} / ${numPages}` : "…"}
          </span>
        </div>
      </div>

      <div className="reader-stage">
        <button
          className="nav-arrow left"
          onClick={() => go(-1)}
          disabled={page === 0}
          aria-label="Previous page"
        >
          <span className="nav-chevron">‹</span>
        </button>
        <div className="pdf-scroll">
          {error ? (
            <div className="page-loading">Couldn’t open PDF: {error}</div>
          ) : loading ? (
            <div className="page-loading">Loading…</div>
          ) : (
            <canvas ref={canvasRef} className="pdf-canvas" />
          )}
        </div>
        <button
          className="nav-arrow right"
          onClick={() => go(1)}
          disabled={numPages > 0 && page >= numPages - 1}
          aria-label="Next page"
        >
          <span className="nav-chevron">›</span>
        </button>
      </div>
    </div>
  );
}
