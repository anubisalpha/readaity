import { useCallback, useEffect, useRef, useState } from "react";
import type { BookRow, ReaderPrefs } from "../types";
import { readerThemeCss } from "../lib/readerTheme";
import { BookmarkPanel, useBookmarks } from "./Bookmarks";

interface Props {
  book: BookRow;
  /** Stored progress in per-mille (0–1000) of scroll position. */
  initialPage: number;
  /** Loads the book's HTML (e.g. MOBI decompression or RTF conversion). */
  load: (path: string) => Promise<string>;
  prefs: ReaderPrefs;
  onBack: () => void;
  onPageChange: (perMille: number) => void;
}

interface TocEntry {
  title: string;
  id: string;
  depth: number;
}

/** Short text near the top of the reading viewport, for a bookmark label. */
function snippetAtTop(idoc: Document): string {
  const x = (idoc.defaultView?.innerWidth ?? 400) / 2;
  try {
    // Walk down from the element under the point to its first sizable text-
    // bearing descendant, so the label is a sentence, not a whole chapter.
    let el = idoc.elementFromPoint(x, 16) as Element | null;
    for (let i = 0; i < 4 && el; i++) {
      const child = [...el.children].find((c) => {
        const r = c.getBoundingClientRect();
        return r.top >= -4 && r.height > 0 && (c.textContent || "").trim().length > 8;
      });
      if (!child) break;
      el = child;
    }
    const text = (el?.textContent || "").replace(/\s+/g, " ").trim();
    return text.slice(0, 70);
  } catch {
    return "";
  }
}

/**
 * Renders an HTML book (from a loader) in a sandboxed iframe as a clean
 * scrolling column. Progress is the scroll fraction, stored as per-mille.
 * If the HTML carries a `#kf8-toc` nav, a chapter list is offered.
 */
export function HtmlReader({
  book,
  initialPage,
  load,
  prefs,
  onBack,
  onPageChange,
}: Props) {
  const iframeRef = useRef<HTMLIFrameElement>(null);
  const posRef = useRef(Math.max(0, initialPage)); // live per-mille position
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [pct, setPct] = useState(Math.round(Math.max(0, initialPage) / 10));
  const [toc, setToc] = useState<TocEntry[]>([]);
  const [tocOpen, setTocOpen] = useState(false);
  const [bmOpen, setBmOpen] = useState(false);
  const { bookmarks, add, remove } = useBookmarks(book.path);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const raw = await load(book.path);
        if (cancelled) return;
        const m = raw.match(/<body[^>]*>([\s\S]*?)<\/body>/i);
        const inner = m ? m[1] : raw;
        const doc = `<!doctype html><html><head><meta charset="utf-8"><style id="reader-theme">${readerThemeCss(
          prefs,
        )}</style></head><body><div class="rdr">${inner}</div></body></html>`;
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
                depth: Math.max(0, Math.min(4, Number(a.getAttribute("data-depth")) || 0)),
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
              posRef.current = Math.round(p * 1000);
              setPct(Math.round(p * 100));
              onPageChange(posRef.current);
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

  // Live theme / font-size change — restyle in place without reloading.
  useEffect(() => {
    const style = iframeRef.current?.contentDocument?.getElementById("reader-theme");
    if (style) style.textContent = readerThemeCss(prefs);
  }, [prefs]);

  const scrollToPerMille = useCallback((perMille: number) => {
    const scroller = iframeRef.current?.contentDocument
      ?.scrollingElement as HTMLElement | null;
    if (!scroller) return;
    const max = scroller.scrollHeight - scroller.clientHeight;
    scroller.scrollTop = (perMille / 1000) * max;
  }, []);

  const jumpTo = useCallback((id: string) => {
    const target = iframeRef.current?.contentDocument?.getElementById(id);
    target?.scrollIntoView({ block: "start" });
    setTocOpen(false);
  }, []);

  const addHere = useCallback(() => {
    const idoc = iframeRef.current?.contentDocument;
    add(posRef.current, idoc ? snippetAtTop(idoc) : "");
  }, [add]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (tocOpen) setTocOpen(false);
        else if (bmOpen) setBmOpen(false);
        else onBack();
        return;
      }
      if (e.key === "t" && toc.length) {
        setTocOpen((o) => !o);
        return;
      }
      if (e.key === "b") {
        setBmOpen((o) => !o);
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
  }, [onBack, tocOpen, bmOpen, toc.length]);

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
              className={`btn ghost${tocOpen ? " active" : ""}`}
              onClick={() => setTocOpen((o) => !o)}
              title="Contents (T)"
            >
              ☰ Contents
            </button>
          )}
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
                    <li key={i} style={{ paddingLeft: `${e.depth * 14}px` }}>
                      <button onClick={() => jumpTo(e.id)}>{e.title}</button>
                    </li>
                  ))}
                </ul>
              </nav>
            )}
            {bmOpen && (
              <BookmarkPanel
                bookmarks={bookmarks}
                describe={(p) => `${Math.round(p / 10)}%`}
                onAdd={addHere}
                onRemove={remove}
                onJump={(p) => {
                  scrollToPerMille(p);
                  setBmOpen(false);
                }}
                onClose={() => setBmOpen(false)}
              />
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
