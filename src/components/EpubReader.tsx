import { useCallback, useEffect, useRef, useState } from "react";
import ePub from "epubjs";
import type { BookRow, ReaderPrefs } from "../types";
import { readBookBytes } from "../lib/api";
import { READER_THEMES } from "../lib/readerTheme";
import { BookmarkPanel, useBookmarks } from "./Bookmarks";

interface Props {
  book: BookRow;
  /** Stored progress in per-mille (0–1000) of the whole book by content length. */
  initialPage: number;
  prefs: ReaderPrefs;
  onBack: () => void;
  onPageChange: (perMille: number) => void;
}

function b64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function applyTheme(rendition: any, prefs: ReaderPrefs) {
  const t = READER_THEMES[prefs.theme];
  rendition.themes.register(prefs.theme, {
    body: {
      background: `${t.bg} !important`,
      color: `${t.fg} !important`,
      "font-family": "Georgia, serif",
      "line-height": "1.5",
    },
    a: { color: `${t.link} !important` },
  });
  rendition.themes.select(prefs.theme);
  rendition.themes.fontSize(`${Math.round(prefs.fontScale * 100)}%`);
}

/**
 * EPUB reader (epub.js). Progress is a true percentage of the whole book by
 * content length (via generated "locations"), stored as per-mille in last_page.
 */
export function EpubReader({
  book,
  initialPage,
  prefs,
  onBack,
  onPageChange,
}: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const renditionRef = useRef<any>(null);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const bookRef = useRef<any>(null);
  const locReady = useRef(false);
  const posRef = useRef(Math.max(0, initialPage));
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [pct, setPct] = useState(Math.round(Math.max(0, initialPage) / 10));
  const [bmOpen, setBmOpen] = useState(false);
  const { bookmarks, add, remove } = useBookmarks(book.path);

  useEffect(() => {
    let cancelled = false;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    let book_: any = null;

    (async () => {
      try {
        const bytes = b64ToBytes(await readBookBytes(book.path));
        if (cancelled || !hostRef.current) return;
        book_ = ePub(bytes.buffer);
        bookRef.current = book_;
        const rendition = book_.renderTo(hostRef.current, {
          width: "100%",
          height: "100%",
          flow: "paginated",
          spread: "none",
        });
        renditionRef.current = rendition;

        applyTheme(rendition, prefs);

        // Force aspect-correct cover/image rendering inside each section. SVG
        // covers (common in EPUBs) can't be fixed via CSS — preserveAspectRatio
        // is a presentation attribute, so set it on the DOM directly.
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        rendition.hooks.content.register((contents: any) => {
          const d: Document = contents.document;
          d.querySelectorAll("svg").forEach((s) => {
            s.setAttribute("preserveAspectRatio", "xMidYMid meet");
            (s as SVGElement).style.maxWidth = "100%";
            (s as SVGElement).style.maxHeight = "100vh";
          });
          d.querySelectorAll("image").forEach((im) =>
            im.setAttribute("preserveAspectRatio", "xMidYMid meet"),
          );
          d.querySelectorAll("img").forEach((im) => {
            const s = (im as HTMLElement).style;
            s.maxWidth = "100%";
            s.maxHeight = "100vh";
            s.width = "auto";
            s.height = "auto";
            s.objectFit = "contain";
          });
        });

        await book_.ready;
        await rendition.display();
        if (cancelled) return;
        setLoading(false);

        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        rendition.on("relocated", (loc: any) => {
          if (!locReady.current || !book_.locations.length()) return;
          const p = book_.locations.percentageFromCfi(loc?.start?.cfi);
          if (typeof p === "number") {
            posRef.current = Math.round(p * 1000);
            setPct(Math.round(p * 100));
            onPageChange(posRef.current); // per-mille
          }
        });

        // Build even-sized locations, then jump to the saved position.
        book_.locations
          .generate(1200)
          .then(() => {
            if (cancelled) return;
            locReady.current = true;
            if (initialPage > 0) {
              const cfi = book_.locations.cfiFromPercentage(initialPage / 1000);
              if (cfi) rendition.display(cfi);
            }
          })
          .catch(() => {});
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    })();

    return () => {
      cancelled = true;
      try {
        book_?.destroy?.();
      } catch {
        /* ignore */
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [book.path]);

  // Live theme / font-size change.
  useEffect(() => {
    if (renditionRef.current) applyTheme(renditionRef.current, prefs);
  }, [prefs]);

  const jumpToPerMille = useCallback((perMille: number) => {
    const b = bookRef.current;
    const r = renditionRef.current;
    if (!b || !r || !locReady.current) return;
    const cfi = b.locations.cfiFromPercentage(perMille / 1000);
    if (cfi) r.display(cfi);
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const r = renditionRef.current;
      if (e.key === "b") {
        setBmOpen((o) => !o);
        return;
      }
      if (!r) return;
      if (["ArrowRight", "PageDown", " "].includes(e.key)) {
        e.preventDefault();
        r.next();
      } else if (["ArrowLeft", "PageUp"].includes(e.key)) {
        e.preventDefault();
        r.prev();
      } else if (e.key === "Escape") {
        if (bmOpen) setBmOpen(false);
        else onBack();
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
        <button
          className="nav-arrow left"
          onClick={() => renditionRef.current?.prev()}
          aria-label="Previous"
        >
          <span className="nav-chevron">‹</span>
        </button>
        <div className="epub-host-wrap">
          {error && <div className="page-loading">Couldn’t open EPUB: {error}</div>}
          {loading && !error && <div className="page-loading">Loading…</div>}
          <div ref={hostRef} className="epub-host" />
        </div>
        <button
          className="nav-arrow right"
          onClick={() => renditionRef.current?.next()}
          aria-label="Next"
        >
          <span className="nav-chevron">›</span>
        </button>
      </div>
    </div>
  );
}
