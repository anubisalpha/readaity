# Kindle-format reading (DRM-free only)

Investigation notes, 2026-08-30. Scope: opening **DRM-free** Kindle-format
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
  5. concat every section's `<body>` inner as `<div class="kf8-section">`, emit
     the CSS flows once as `<style>`, inline `kindle:embed:NNNN` images as data
     URIs, neutralise leftover `kindle:pos:` links.

  Tests (both `#[ignore]`, env-var gated):
  - `kf8_real` — asserts on one file: `READAITY_KF8_FILE=<path> cargo test kf8_real -- --ignored`
  - `kf8_dir` — batch health survey over a folder, writes rebuilt HTML to
    `<dir>/_out/`: `READAITY_KF8_DIR=<dir> cargo test kf8_dir -- --ignored --nocapture`

  Verified across **27 real commercial azw3** (prose, illustrated classics,
  image comics, huge multi-book collections): all parse with **zero UTF-8
  replacement chars and zero un-inlined images**; prose spot-checked in a
  browser renders clean. Small books with no FDST / no skel+frag tables fall
  back to rendering flow 0 directly. No TOC/chapter nav yet (one scroll blob,
  same as MOBI6). A couple of files reassemble into oddly few sections
  (`The Magic of Oz` → 2) but with all text present — worth a look later.

### 1b. Fixed-layout KF8 (comics / manga / picture books) — **done 2026-08-31**

EXTH **122** `fixed-layout` == `"true"` (also EXTH 123 `book-type` = comic/children)
marks a page-per-section image book. `mobi::meta()` reads it in a cheap record-0
pass during the sweep and stores `books.fixed_layout`. Such books route to the
**page-image pager** (`Reader` with `source="kf8"`), not the reflowable
`HtmlReader`.

Page images: `mobi::page_images()` reassembles the KF8 once, then per section
resolves the page image — an element `id` in the body whose CSS rule (scoped to
that section's linked `kindle:flow`s) has `background-image: url(kindle:embed:N)`,
else the first inline `kindle:embed`. Command `get_kf8_pages` returns them all;
the pager caches the lot on open. Page count is 0 until the pager fills it in
(a "deeper scan" could compute it earlier, but it's not worth the scan cost).

Verified on 11 real fixed-layout azw3 (Marvel Infinite comics, Star Wars gift
books, Usborne/《Coronavirus》picture books, Minecraft activity book, a Defiance
guide): all detected, page images extract (Minecrafters drops 1 SVG-art page;
Defiance repeats shared spread images across its 163 text-overlay pages). In-app:
Dinosaurs Vs Aliens (PalmDOC) and Coronavirus (HUFF/CDIC) page correctly.

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

MOBI6 chapter breaks live in INDX records; KF8 has a real NCX. Currently the whole
book is one HTML blob with no chapter list; progress is scroll-only. Fine for MVP,
noted for later.

## Suggested priority order

1. ~~**Detect KF8-only azw3/azw**~~ — superseded.
2. ~~**Full KF8 reader**~~ — **done 2026-08-31** (`mobi.rs` module `kf8`). Needs
   testing against more real files; watch for the edges noted above.
   Verify against real files with `C:\Python314\python.exe` per the project's
   parsing-gotchas rule.
3. **HUFF/CDIC** — nice-to-have, do when convenient.
4. **KFX** — defer to the Calibre bridge; don't build native.
