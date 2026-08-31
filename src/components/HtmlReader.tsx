import { useCallback, useEffect, useRef, useState } from "react";
import type { BookRow } from "../types";

interface Props {
  book: BookRow;
  /** Stored progress in per-mille (0–1000) of scroll position. */
  initialPage: number;
  /** Loads the book's HTML (e.g. MOBI decompression or RTF conversion). */
  load: (path: string) => Promise<string>;
  onBack: () => void;
  onPageChange: (perMille: number) => void;
}

interface TocEntry {
  title: string;
  id: string;
}

const READER_CSS = `
  html, body { margin: 0; background: #14161a; color: #d8dade; }
  body { font-family: Georgia, serif; line-height: 1.6; font-size: 18px; }
  .rdr { max-width: 42rem; margin: 0 auto; padding: 28px 28px 96px; }
  img, image, svg { max-width: 100% !important; height: auto !important; object-fit: contain; }
  a { color: #6ea8fe; }
  h1, h2, h3 { line-height: 1.25; }
  p { margin: 0.6em 0; }
  #kf8-toc { display: none; }
`;

/**
 * Renders an HTML book (from a loader) in a sandboxed iframe as a clean
 * scrolling column. Progress is the scroll fraction, stored as per-mille.
 * If the HTML carries a `#kf8-toc` nav, a chapter list is offered.
 */
export function HtmlReader({ book, initialPage, load, onBack, onPageChange }: Props) {
  const iframeRef = useRef<HTMLIFrameElement>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [pct, setPct] = useState(Math.round(Math.max(0, initialPage) / 10));
  const [toc, setToc] = useState<TocEntry[]>([]);
  const [tocOpen, setTocOpen] = useState(false);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const raw = await load(book.path);
        if (cancelled) return;
        const m = raw.match(/<body[^>]*>([\s\S]*?)<\/body>/i);
        const inner = m ? m[1] : raw;
        const doc = `<!doctype html><html><head><meta charset="utf-8"><style>${READER_CSS}</style></head><body><div class="rdr">${inner}</div></body></html>`;
        const iframe = iframeRef.current;
        if (!iframe) return;
        iframe.onload = () => {
          setLoading(false);
          const idoc = iframe.contentDocument;
          const scroller = idoc?.scrollingElement as HTMLElement | null;
          const win = iframe.contentWindow;
          if (!idoc || !scroller || !win) return;

          const nav = idoc.getElementById("kf8-toc");
          if (nav) {
            setToc(
              [...nav.querySelectorAll("a")].map((a) => ({
                title: a.textContent?.trim() || "—",
                id: (a.getAttribute("href") || "").replace(/^#/, ""),
              })),
            );
          }

          if (initialPage > 0) {
            scroller.scrollTop =
              (initialPage / 1000) * (scroller.scrollHeight - scroller.clientHeight);
          }
          win.addEventListener(
            "scroll",
            () => {
              const max = scroller.scrollHeight - scroller.clientHeight;
              const p = max > 0 ? scroller.scrollTop / max : 0;
              setPct(Math.round(p * 100));
              onPageChange(Math.round(p * 1000));
            },
            { passive: true },
          );
        };
        iframe.srcdoc = doc;
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [book.path]);

  const jumpTo = useCallback((id: string) => {
    const target = iframeRef.current?.contentDocument?.getElementById(id);
    target?.scrollIntoView({ block: "start" });
    setTocOpen(false);
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (tocOpen) setTocOpen(false);
        else onBack();
        return;
      }
      if (e.key === "t" && toc.length) {
        setTocOpen((o) => !o);
        return;
      }
      const iframe = iframeRef.current;
      const scroller = iframe?.contentDocument?.scrollingElement as HTMLElement | null;
      const win = iframe?.contentWindow;
      if (!scroller || !win) return;
      const step = scroller.clientHeight * 0.9;
      if (["ArrowRight", "PageDown", " "].includes(e.key)) {
        e.preventDefault();
        win.scrollBy(0, step);
      } else if (["ArrowLeft", "PageUp"].includes(e.key)) {
        e.preventDefault();
        win.scrollBy(0, -step);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onBack, tocOpen, toc.length]);

  return (
    <div className="reader">
      <div className="reader-bar">
        <button className="btn ghost" onClick={onBack}>
          ‹ Library
        </button>
        <span className="reader-title">{book.title}</span>
        <div className="reader-controls">
          {toc.length > 0 && (
            <button
              className="btn ghost"
              onClick={() => setTocOpen((o) => !o)}
              title="Contents (T)"
            >
              ☰ Contents
            </button>
          )}
          <span className="page-counter">{pct}%</span>
        </div>
      </div>
      <div className="reader-stage">
        {error ? (
          <div className="page-loading">Couldn’t open: {error}</div>
        ) : (
          <>
            {loading && <div className="page-loading">Loading…</div>}
            {tocOpen && (
              <nav className="toc-panel">
                <div className="toc-head">Contents</div>
                <ul>
                  {toc.map((e, i) => (
                    <li key={i}>
                      <button onClick={() => jumpTo(e.id)}>{e.title}</button>
                    </li>
                  ))}
                </ul>
              </nav>
            )}
            <iframe
              ref={iframeRef}
              className="mobi-frame"
              title={book.title}
              sandbox="allow-same-origin"
            />
          </>
        )}
      </div>
    </div>
  );
}
