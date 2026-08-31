# Kindle-format reading (DRM-free only)

Investigation notes, 2026-08-30; implementation landed in v0.6.0, 2026-08-31.
Scope: opening **DRM-free** Kindle-format
ebooks in Readaity. Nothing here involves DRM circumvention — that boundary
is firm (see project memory / `feedback-drm-free-reader-boundary`). Protected
files stay detected-and-refused with a clean message.

Relevant code:
- `src-tauri/src/mobi.rs` — MOBI/AZW cover + content engine
- `src-tauri/src/formats.rs` — extension → library/format mapping
- `src-tauri/src/ebook.rs:276` — cover dispatch
- `src-tauri/src/lib.rs:502` — `get_mobi_html` command

## What works today

| Area | Status |
|---|---|
| Extensions → MOBI engine | `.mobi .prc .azw .azw3` (all routed to `mobi.rs`) |
| Container parsing | PalmDB record table; record 0 = PalmDOC + MOBI header + EXTH |
| Text decompression | uncompressed (`comp==1`), PalmDOC/LZ77 (`comp==2`), **HUFF/CDIC (`comp==17480`)** — `HuffCdic` in `mobi.rs`, tables from HUFF record + CDIC dict records (r0 offsets 112/116). Most commercial kindlegen output is HUFF/CDIC. |
| Trailing-byte handling | `extra_data_flags` at record-0 offset 242; multibyte overlap trim (the `?`-every-4KB bug is fixed) |
| KF8 (MOBI-8) | full reassembly — see gap 1 below |
| Images | MOBI-6 `recindex="N"` and KF8 `kindle:embed:NNNN` refs → inline base64 data URIs; cover via EXTH 201/203 |
| Encoding | UTF-8 (65001) and CP1252 |
| DRM | PalmDOC encryption flag (offset 12) != 0 -> clean "DRM-protected" message |

Reader UI: whole book is decompressed to one HTML blob, fed to `HtmlReader`
(iframe srcdoc). Progress tracked per-mille by scroll.

## Gaps

### 1. KF8 / MOBI-8 — the big one

Modern `.azw3` and many newer `.azw` files are KF8. KF8 is an EPUB-like set of
XHTML "skeleton" + "fragment" records living **after a boundary record** in the
same PalmDB container (boundary pointed at by EXTH record 121, or a `BOUNDARY`
marker record).

`content()` currently reads text records `1..text_recs` from the **start** of
the container:

- **Hybrid files** (MOBI6 + KF8 both present — older Kindlegen output): reads the
  MOBI6 copy, renders fine. This is why azw3 appeared to work in b2 testing.
- **Hybrid files** stay on the MOBI6 path (record-0 file version 6).
- **KF8-only files** (record-0 file version 8) are now **fully rendered**
  (2026-08-31), via `mobi.rs` module `kf8`:
  1. decompress records `1..text_recs` exactly as MOBI6 (PalmDOC + trailing-byte
     trim — same `trim()`), giving the raw KF8 text;
  2. **FDST** record (header ptr `0xC0`) splits it into flows: flow 0 = the
     skeleton+fragment stream, flows 1.. = CSS/SVG;
  3. **skeleton** INDX (ptr `0xFC`) + **fragment/"chunk"** INDX (ptr `0xF8`):
     generic TAGX/IDXT parser (`parse_indx`) with the control-byte value-count
     decode. Skeleton tag 1 = chunk count, tag 6 = [start,len]; chunk entry
     *name* = absolute insert offset, tag 6 = [_, len];
  4. for each skeleton, splice its chunks in by `insert - skel_start`;
  5. concat every section's `<body>` inner as `<div class="kf8-section" id="kf8-sN">`,
     emit the CSS flows once as `<style>`, inline `kindle:embed:NNNN` images as
     data URIs, neutralise leftover `kindle:pos:` links.
  6. **Chapter TOC** (2026-08-31): `assemble()` also emits a hidden
     `<nav id="kf8-toc">` of `<a href="#kf8-sN">` from each section's `<title>`
     — a JUNK denylist drops boilerplate ("Book Title", "Cover", …) and
     consecutive duplicate titles are collapsed. There is **no** frequency
     filter (a multi-chapter novel legitimately titles every section with the
     book name). `HtmlReader` lifts the nav into a "☰ Contents" side panel and
     `scrollIntoView`s the anchor. MOBI-6 NCX TOC still not done.

  Tests (`#[ignore]`, env-var gated): `kf8_real` (one file), `kf8_dir` (batch
  survey → `<dir>/_out/`), `kf8_pages_dir` (fixed-layout page survey).

  Verified across **27 real commercial azw3**: all parse with **zero UTF-8
  replacement chars and zero un-inlined images**; prose + chapter nav
  spot-checked in the app. Small books with no FDST / no skel+frag tables fall
  back to flow 0 directly. A couple of reflowable files reassemble into oddly
  few `<div>` sections (`The Magic of Oz` → 2) but with all text present.

### 1c. Comic-azw3 → Comics library — **done 2026-08-31 (Part D)**

A fixed-layout azw3 catalogued in Ebooks can be moved to Comics:
- `books.library_override` column — the move survives rescans
  (`upsert_discovered` COALESCEs it over the folder library;
  `prune_missing` is library-scoped so the other library's scan won't drop it);
- commands `set_book_library`, `folder_layout_split`, `split_folder_libraries`;
- `quick_scan` runs a cheap record-0 `mobi::meta` in Phase 1 to flag
  `fixed_layout` early, so the badge/prompt appear before the sweep;
- UI: "▣ comic" tile badge, "→ Comics" hover button, a nudge banner in
  `Kf8Reader`, and — on adding a folder that holds both — a "split libraries"
  dialog ("Move N to Comics");
- `db::list_folders` unions in any folder that has books in the library via
  override, so a moved book appears under a Comics folder node, not just the
  flat shelves.
Verified end-to-end in-app.

### 1b. Fixed-layout KF8 (comics / manga / picture books) — **done 2026-08-31**

EXTH **122** `fixed-layout` == `"true"` (also EXTH 123 `book-type` = comic/children)
marks a page-per-section book. `mobi::meta()` reads it in a cheap record-0 pass
during the sweep → `books.fixed_layout`. Such books open in **`Kf8Reader`**
(a page pager), not the reflowable `HtmlReader`.

**Rendering (revised 2026-08-31):** each page is the section's *actual* XHTML,
not an extracted image — so positioned-text pages (a picture-book ToC) and
image + text-overlay pages both render. `mobi::kf8_pages()` per section:
- dims from `<meta viewport>` / `<body style="width:…px">`, else EXTH 126
  `original-resolution`;
- `prune_css()` keeps only the CSS rules whose `#id` / `.class` is on this page
  (fixed-layout books share one book-wide stylesheet whose `#fsN-img {
  background-image }` rules would otherwise pull *every* image into *every* page
  — Dinosaurs went 499 MB → 17 MB, Coronavirus 464 → 34);
- inline that page's `kindle:embed` images, neutralise `kindle:` refs.
`Kf8Reader` renders the page in a `sandbox=""` iframe scaled (`transform:
scale`) to fit. **Lazy:** `get_kf8_page_dims` triggers + caches the reassembly
(`Kf8Cache` app-state, one book), `get_kf8_page(i)` fetches one page; the reader
holds current ±1. Page count is 0 in the DB until the reader fills it in.

Verified in-app on 11 real fixed-layout azw3 (Marvel/Silver Surfer comics, Star
Wars gift books, Usborne/Coronavirus picture books, Minecraft activity book,
Defiance guide): all pages render. Defiance (163 text-overlay pages sharing ~53
spreads) still ~290 MB total in the cache — big but lazy-loaded so it's usable;
a shared-image URL scheme would fix it properly.

### 2. HUFF/CDIC decompression — **done 2026-08-31** (`HuffCdic` in `mobi.rs`)

A *compression* scheme, not DRM. Turned out **not** rare: ~24 of 27 real
commercial azw3 test files use it (kindlegen applies it by default; Standard
Ebooks / hand-rolled output uses plain PalmDOC). Clean-room implementation from
the format spec, prototyped in Python (`scratchpad/huffprobe.py`) first: parse
HUFF dispatch tables (cache + base), CDIC phrase dictionary, bit-unpack with
recursive phrase expansion. Verified: output length matches FDST total, clean
UTF-8.

### 3. `.kfx` — unsupported, and hard

Current Kindle app / newer devices produce KFX (KDF container: an SQLite DB of
Amazon "ion" symbol data). Even DRM-free KFX needs substantial container + ion
parsing. Belongs behind the planned **Phase 3 Calibre bridge** (convert -> EPUB),
not a native parser. `.kfx` is deliberately not in `EBOOK_EXTS`.

### 4. TOC / chapter navigation

- **KF8** — done (gap 1, step 6): section-`<title>` list → "☰ Contents" panel.
- **MOBI-6** — still nothing. Chapter breaks live in INDX / a real NCX; would
  need INDX parsing + filepos→anchor mapping. Not started.

## Still open

- **MOBI-6 NCX TOC** (above).
- **Fixed-layout image sharing** — Defiance-class books duplicate spread images
  across pages; serve images by a per-book URL instead of inlining
  (Defiance ~290 MB in the lazy cache today).
- **KFX** — defer to the Calibre bridge; don't build native.
- **Done:** KF8 reassembly, HUFF/CDIC, fixed-layout page reader, KF8 chapter
  TOC, comic-azw3 → Comics (Part D). Shipped in **v0.6.0** (2026-08-31).
