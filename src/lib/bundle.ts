// Bundle books that are the same title in different formats into one shelf tile.
//
// The library DB still holds one row per file — this is a pure display-layer
// grouping, and it is *library-wide*: the alternate formats of a title are
// bundled even when they live in different folders (a common setup — one folder
// of AZW3s, another of EPUBs). A bundle picks the format the app reads best as
// its "primary"; the tile shows every format as a badge, but opening / progress
// / favourite / move all act on the primary. The other files stay in the
// library (still counted, covered, shared) — they just don't get their own tile.

import type { BookRow } from "../types";

/**
 * Format preference, best-supported first. The primary of a bundle is the
 * lowest-ranked *ready* format present (falling back to lowest-ranked overall
 * while the sweep is still catching up). Rationale: reflowable EPUB gets
 * epub.js (locations %, themes, text-size); the Kindle formats share the
 * in-app KF8 / MOBI renderer; PDF is fixed-page via pdf.js; RTF/TXT are plain;
 * LRF cannot be opened at all, so it is never a primary when anything else is
 * present.
 */
const FORMAT_RANK: Record<string, number> = {
  epub: 0,
  azw3: 1,
  mobi: 2,
  prc: 3,
  azw: 4,
  cbz: 5,
  cbr: 6,
  pdf: 7,
  rtf: 8,
  txt: 9,
  lrf: 99,
};

function rank(format: string): number {
  return FORMAT_RANK[format] ?? 50;
}

/** A DRM-removal tool's trailing marker — the EPUB copy an Epubor/DeDRM export
 *  leaves gets "_nodrm", the AZW3 doesn't. Stripped for both matching and
 *  display so a pair lines up and the shelf never shows the wart. */
const NODRM_TAIL = /[\s_-]*no[\s_-]?drm\s*$/i;

/** Normalise a title for match purposes: lower-case, strip diacritics, drop the
 *  "_nodrm" marker, fold whitespace/underscores, trim edge punctuation. A
 *  trailing "(...)" qualifier is kept — it often distinguishes editions
 *  ("(2020 Edition)") or entries in different series ("(Exodus Trilogy Book 1)"
 *  vs "(Dead Planet Series Book 1)"). */
export function normalizeTitle(title: string): string {
  return title
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(NODRM_TAIL, "")
    .replace(/[\s_]+/g, " ")
    .replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, "")
    .trim();
}

/** The title as shown on a shelf tile — the real title minus the "_nodrm"
 *  marker and any underscore/space it leaves dangling. */
export function displayTitle(title: string): string {
  const t = title.replace(NODRM_TAIL, "").replace(/[\s_]+$/, "");
  return t || title;
}

/** A shelf tile: one primary book plus any same-title alternate-format files. */
export interface BookBundle {
  /** The file the tile represents — opened, tracked, favourited, moved. */
  primary: BookRow;
  /** Every file in the bundle, primary first, then by format rank. */
  members: BookRow[];
  /** Upper-case format labels, primary first (e.g. ["EPUB", "MOBI", "PDF"]). */
  formats: string[];
  /** All member paths — used when moving/removing the whole tile. */
  paths: string[];
  /** The primary is swept and openable (else the tile shows as pending). */
  ready: boolean;
}

function byRank(a: BookRow, b: BookRow): number {
  const r = rank(a.format) - rank(b.format);
  if (r !== 0) return r;
  if (a.has_cover !== b.has_cover) return a.has_cover ? -1 : 1;
  if ((a.last_page > 0) !== (b.last_page > 0)) return a.last_page > 0 ? -1 : 1;
  return b.size - a.size;
}

function makeBundle(members: BookRow[]): BookBundle {
  // Display order is always by format rank…
  const ordered = [...members].sort(byRank);
  // …but the primary (what opens) prefers a format that's actually ready now.
  const readyOrdered = ordered.filter((m) => m.status === "ready");
  const base = (readyOrdered[0] ?? ordered[0])!;
  const anyFav = members.some((m) => m.favorite);
  const lastOpened = members.reduce<number | null>(
    (acc, m) =>
      m.last_opened != null && (acc == null || m.last_opened > acc)
        ? m.last_opened
        : acc,
    null,
  );
  return {
    // Surface bundle-wide state on the primary so the tile reflects any member
    // being favourited / opened, and show a clean title.
    primary: {
      ...base,
      title: displayTitle(base.title),
      favorite: anyFav,
      last_opened: lastOpened,
    },
    members: ordered,
    formats: ordered.map((m) => m.format.toUpperCase()),
    paths: ordered.map((m) => m.path),
    ready: base.status === "ready",
  };
}

/**
 * Collapse a flat book list into bundles, grouped by normalised title across
 * the whole library. A group only merges into a single tile when every member
 * has a *distinct* format (so two EPUBs of the same title still show both —
 * nothing is ever hidden by accident). Everything else falls through as a
 * bundle of one.
 */
export function bundleBooks(books: BookRow[]): BookBundle[] {
  const groups = new Map<string, BookRow[]>();
  for (const b of books) {
    const key = normalizeTitle(b.title) || b.path;
    const g = groups.get(key);
    if (g) g.push(b);
    else groups.set(key, [b]);
  }

  const out: BookBundle[] = [];
  for (const g of groups.values()) {
    if (g.length === 1) {
      out.push(makeBundle(g));
      continue;
    }
    const distinct = new Set(g.map((b) => b.format));
    if (distinct.size === g.length) {
      out.push(makeBundle(g));
    } else {
      // Ambiguous (a repeated format) — don't merge, list each on its own.
      for (const b of g) out.push(makeBundle([b]));
    }
  }
  return out;
}

/** Keep only the bundles that have at least one file among `paths` (a Set of
 *  normalised-or-raw paths present in the current folder view). */
export function bundlesInView(
  bundles: BookBundle[],
  hasPath: (p: string) => boolean,
): BookBundle[] {
  return bundles.filter((bd) => bd.members.some((m) => hasPath(m.path)));
}
