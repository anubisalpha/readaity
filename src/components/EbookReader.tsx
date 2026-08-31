import type { BookRow, ReaderPrefs } from "../types";
import { PdfReader } from "./PdfReader";
import { EpubReader } from "./EpubReader";
import { HtmlReader } from "./HtmlReader";
import { TxtReader } from "./TxtReader";
import { Kf8Reader } from "./Kf8Reader";
import { getMobiHtml, getRtfHtml } from "../lib/api";

interface Props {
  book: BookRow;
  initialPage: number;
  prefs: ReaderPrefs;
  onBack: () => void;
  onPageChange: (page: number) => void;
  /** This is a fixed-layout book open in the Ebooks library — nudge to Comics. */
  suggestComics?: boolean;
  onMoveToComics?: () => void;
}

/** Routes an ebook to the right renderer by format. */
export function EbookReader({
  book,
  initialPage,
  prefs,
  onBack,
  onPageChange,
  suggestComics,
  onMoveToComics,
}: Props) {
  if (book.format === "pdf") {
    return (
      <PdfReader
        book={book}
        initialPage={initialPage}
        onBack={onBack}
        onPageChange={onPageChange}
      />
    );
  }
  if (book.format === "epub") {
    return (
      <EpubReader
        book={book}
        initialPage={initialPage}
        prefs={prefs}
        onBack={onBack}
        onPageChange={onPageChange}
      />
    );
  }
  if (book.format === "txt") {
    return (
      <TxtReader
        book={book}
        initialPage={initialPage}
        prefs={prefs}
        onBack={onBack}
        onPageChange={onPageChange}
      />
    );
  }
  if (book.format === "rtf") {
    return (
      <HtmlReader
        book={book}
        initialPage={initialPage}
        load={getRtfHtml}
        prefs={prefs}
        onBack={onBack}
        onPageChange={onPageChange}
      />
    );
  }
  if (book.format === "lrf") {
    // Covers/reading for LRF come via the Calibre bridge (Phase 3).
    return (
      <div className="reader">
        <div className="reader-bar">
          <button className="btn ghost" onClick={onBack}>
            ‹ Library
          </button>
          <span className="reader-title">{book.title}</span>
          <div className="reader-controls">
            <span className="format-tag">lrf</span>
          </div>
        </div>
        <div className="reader-stage">
          <div className="ebook-placeholder">
            <p className="empty-title">{book.title}</p>
            <p className="empty-sub">
              LRF (Sony Reader) isn’t readable natively yet. Convert it to EPUB
              (e.g. with Calibre) and it’ll open fully.
            </p>
          </div>
        </div>
      </div>
    );
  }

  // Fixed-layout KF8 (comic / manga / picture book) → page pager.
  if (book.fixed_layout) {
    return (
      <Kf8Reader
        book={book}
        initialPage={initialPage}
        onBack={onBack}
        onPageChange={onPageChange}
        suggestComics={suggestComics}
        onMoveToComics={onMoveToComics}
      />
    );
  }

  // MOBI / PRC / AZW / AZW3 (DRM-free), reflowable.
  return (
    <HtmlReader
      book={book}
      initialPage={initialPage}
      load={getMobiHtml}
      prefs={prefs}
      onBack={onBack}
      onPageChange={onPageChange}
    />
  );
}
