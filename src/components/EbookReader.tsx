import type { BookRow } from "../types";
import { PdfReader } from "./PdfReader";
import { EpubReader } from "./EpubReader";
import { HtmlReader } from "./HtmlReader";
import { TxtReader } from "./TxtReader";
import { Reader } from "./Reader";
import { getMobiHtml, getRtfHtml } from "../lib/api";

interface Props {
  book: BookRow;
  initialPage: number;
  onBack: () => void;
  onPageChange: (page: number) => void;
}

/** Routes an ebook to the right renderer by format. */
export function EbookReader({ book, initialPage, onBack, onPageChange }: Props) {
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

  // Fixed-layout KF8 (comic / manga / picture book) → page-image pager.
  if (book.fixed_layout) {
    return (
      <Reader
        book={book}
        initialPage={initialPage}
        source="kf8"
        onBack={onBack}
        onPageChange={onPageChange}
      />
    );
  }

  // MOBI / PRC / AZW / AZW3 (DRM-free), reflowable.
  return (
    <HtmlReader
      book={book}
      initialPage={initialPage}
      load={getMobiHtml}
      onBack={onBack}
      onPageChange={onPageChange}
    />
  );
}
