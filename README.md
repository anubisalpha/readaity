# Readaity

A local, DRM-free **ebook & comic** reader for Windows. Built with **Tauri 2 + React**.

Everything stays on your machine — you point it at folders of books and it builds
a library. No cloud, no accounts, no vendor lock-in.

> **Scope note:** Readaity opens **DRM-free** files you own. It does not, and will
> not, strip or circumvent DRM from protected/purchased ebooks — that's legally
> fraught (DMCA §1201 / EU Copyright Directive) and out of scope by design.
> Encrypted files are detected and shown with a clean "protected — can't open"
> message rather than failing silently.

## Status

Two libraries — **Comics** and **Ebooks** — each with its own folders, shelves and
reader, switched from a header toggle. Duplicate detection and the removed-items
list are shared across both.

**Comics** — `CBZ`, `CBR`

**Ebooks** — `EPUB`, `PDF`, `MOBI` / `PRC` / `AZW` / `AZW3` (unencrypted),
`TXT`, `RTF`. `LRF` is catalogued but not read natively — convert to EPUB
(e.g. with Calibre) to open it fully. Kindle-format support and its known gaps
(KF8-only `.azw3`, HUFF/CDIC, `.kfx`) are written up in
[`docs/KINDLE_READING.md`](docs/KINDLE_READING.md).

- [x] Add library folders (native picker), recursively scanned **in place** —
      your existing folder structure is never touched or moved
- [x] **Two-phase scan** (see below): instant shelf, background validation
- [x] SQLite library DB with cached cover thumbnails and MD5 integrity hashes
- [x] Cover grid with format badges; covers pop in live as they validate
- [x] Per-format readers with place-remembering:
  - Comics — paged image view, keyboard + click-zone nav, single/dual spread,
    fit-width / fit-height
  - EPUB — reflowable via `epub.js`, locations-based % progress
  - PDF — `pdf.js`, with a generated shelf cover
  - MOBI / AZW / RTF — extracted to HTML, rendered in an isolated frame
  - TXT — scroll reader with BOM-aware UTF-8 / UTF-16 decoding
- [x] Reading position remembered per book (page index for comics/PDF,
      per-mille for reflowable formats) and resumed on reopen
- [x] Drag books/folders between folders (with collision handling)
- [x] **Duplicate detection** — byte-identical (content hash) and fuzzy
      same-issue-by-filename, with "keep the largest" bulk removal and an
      ignore list
- [x] **Removed-from-library** list — remove a book or subtree from the library
      without deleting it from disk; restore individually or all at once
- [x] Re-index / re-scan on demand (picks up new files and newly-supported formats)
- [x] Invalid/corrupt files collected under a note instead of failing silently
- [x] Settings → **Show first** — pick whether Comics or Ebooks is the library
      Readaity opens on and the one that leads the sidebar switcher (persisted
      in the DB `settings` table)
- [x] **Network sharing** (Settings → Network sharing) — serve your libraries to
      other devices on the LAN over **HTTPS (TLS 1.3 only)**, behind a PIN. Any
      browser can browse and download; a self-signed certificate with a
      "Trust this device" flow. Read-only, private-range-only, per-IP lockout.
      Design + build notes: [`docs/NETWORK_SHARING.md`](docs/NETWORK_SHARING.md)

### Roadmap

- [ ] **Calibre bridge** — native `LRF` support and universal convert-to-EPUB
- [ ] **Managed library area** — an optional Readaity-owned root you can migrate/
      import books into, with "preserve source structure" or "flatten" on import
- [ ] **Smart single-book import** — suggest a destination folder from the
      existing structure / the book's metadata, with "drop in Unsorted" fallback
- [ ] Bookmarks, full-text search, reading themes; on-demand "verify library" (re-hash)
- [ ] **Network sharing b5** — discover other Readaity instances on the LAN
      (mDNS) and import books from them. Design: [`docs/NETWORK_SHARING.md`](docs/NETWORK_SHARING.md)
- [ ] Favourites and a "Being Read" shelf; audiobooks as a third library

## The two-phase scan

Designed so a large library feels instant and files are only opened when needed:

1. **Phase 1 — fast discovery** (`library::quick_scan`). Walks folders and upserts
   one DB row per book from directory metadata only (path, size, mtime, format).
   No file is opened. The shelf renders immediately from these rows.
2. **Phase 2 — background sweep** (`library::validate_one`, driven by `start_sweep`).
   Per book: open the file, count pages, build + cache a cover thumbnail, and
   compute an MD5. Success → `status = 'ready'`; failure → `'invalid'` + reason.
   Each result emits a `book-updated` event so the shelf fills in progressively.
   The sweep is pausable and resumes automatically on next launch.

**Fast rescans:** later scans compare `size`+`mtime`. Unchanged → cache hit, the
file is never reopened. Changed → re-queued for the sweep. MD5 is the strong
integrity/dedup signal computed during the sweep, *not* on every scan.

**Durability.** The DB runs WAL with `synchronous=FULL`, and a clean exit flushes
the WAL back into the main file (`db::checkpoint` on `RunEvent::Exit`, after the
sweep is paused on `ExitRequested`). If a launch still finds the file corrupt, it
salvages the folder list, moves the bad file to `library.db.corrupt-<unix>`,
builds a fresh DB and re-scans — the book cache (covers, hashes, reading
positions) is lost but rebuilds; folders survive. A one-time banner tells you.

## Architecture

```
src-tauri/src/
  formats.rs   Extension → library + format-tag classification
  comic.rs     CBZ (zip) + CBR (rar) page-fetch surface; natural sort; cover
  ebook.rs     EPUB / PDF metadata + cover extraction for the sweep
  mobi.rs      PalmDOC/MOBI decode → HTML (DRM flag detection, EXTH cover)
  rtf.rs       RTF → HTML
  db.rs        SQLite: folders + books, two-phase status lifecycle, cover
               BLOBs, reading progress, exclusions, duplicate groups,
               key/value settings, share audit log
  library.rs   quick_scan (phase 1) + validate_one (phase 2) + move planning
  lib.rs       Tauri commands + background sweep emitting book-updated events
  share/       LAN share server — mod (lifecycle), cert (rcgen self-signed),
               tls (rustls TLS 1.3), guard (IP gating + lockout), auth
               (Argon2 PIN + signed cookies), ids (opaque book tokens),
               routes (axum), assets/ (embedded browse UI). See docs/.

src/
  types.ts             Shared types mirroring the Rust surface
  lib/api.ts           Typed invoke() wrappers + event listeners
  lib/formats.ts       Format lists (mirrors formats.rs)
  components/
    Library.tsx        Shelf grid, folder chips, library toggle, scan status
    Cover.tsx          Lazy cover load from the DB thumbnail cache
    Reader.tsx         Comic paged image view
    EbookReader.tsx    Routes an ebook to the right renderer by format
    EpubReader / PdfReader / HtmlReader / TxtReader
    Settings.tsx       General prefs + removed-items list + duplicate detection
    MoveDialog.tsx     Drag-to-move collision handling
  App.tsx              Folder management, live sweep merge, view routing
```

## Develop

```bash
npm install
npm run tauri dev
```

Sample books for testing live in `testdata/` (generated locally, git-ignored).

## Build

```bash
npm run tauri build -- --bundles nsis
```

The installer is small (~4 MB — WebView2 is fetched on demand) and unsigned, so
SmartScreen will warn on first run.

> On Windows with real-time AV, a release build can hit spurious `LNK1104` /
> "not a writable directory" errors as newly-linked artifacts are scanned.
> Re-run the build (it links further each time); if `num-traits` fails, delete
> `src-tauri/target/release/build` first.

## Test

```bash
cd src-tauri
cargo test
```

## License

All rights reserved — see [LICENSE](LICENSE). This repository is public so the
code and its history are visible, but it is not open-source: no licence is
granted to use, copy, modify, or redistribute it. Running an official released
build for personal use is fine.
