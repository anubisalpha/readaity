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
| Text decompression | PalmDOC/LZ77 (`comp==2`) and uncompressed (`comp==1`) |
| Trailing-byte handling | `extra_data_flags` at record-0 offset 242; multibyte overlap trim (the `?`-every-4KB bug is fixed) |
| Images | `recindex="N"` refs rewritten to inline base64 data URIs; cover via EXTH 201/203 |
| Encoding | UTF-8 (65001) and CP1252 |
| DRM | PalmDOC encryption flag (offset 12) != 0 -> clean "DRM-protected" message |
| HUFF/CDIC | Detected (`comp==17480`) -> clean "not supported yet" message |

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
- **KF8-only files** (newer, no MOBI6 compat stream): those early records are raw
  KF8 markup with `aid=` / fragment placeholders and no skeleton reassembly ->
  renders **garbled or partial**. No version check (MOBI version is at record-0
  offset 36), no "this is KF8-only" detection — user just sees a mangled book
  with no explanation.

Proper fix = a KF8 reader: follow the KF8 boundary, parse FDST, read skeleton +
division-fragment records, splice fragments into skeletons by offset, then inline
images/CSS. Non-trivial (~a few hundred lines) but well-documented and entirely
legitimate. Minimum viable improvement: detect KF8-only and show a clean message
(like HUFF/CDIC) instead of garbling.

### 2. HUFF/CDIC decompression

A *compression* scheme, not DRM — implementing it is fine. Affects a minority of
older MOBIs. ~150 lines (parse HUFF record + CDIC dictionary records, bit-unpack).
Medium effort, low urgency.

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

1. **Detect KF8-only azw3/azw and show a clean message** — small, stops silent
   garbling. Do first.
2. **Full KF8 reader** — the real win; makes modern `.azw3` genuinely supported.
   Verify against real files with `C:\Python314\python.exe` per the project's
   parsing-gotchas rule.
3. **HUFF/CDIC** — nice-to-have, do when convenient.
4. **KFX** — defer to the Calibre bridge; don't build native.
