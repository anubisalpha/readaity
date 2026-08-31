// Shared types mirroring the Rust command surface.

/** Top-level library the user is browsing. */
export type LibraryKind = "comics" | "ebooks";

/**
 * How a library folder's contents are presented:
 *   tree    — keep the folder as one root, subfolders navigable
 *   flat    — collapse all nested comics into one flat list
 *   promote — drop this wrapper; its subfolders become top-level roots
 */
export type FolderMode = "tree" | "flat" | "promote";

export interface FolderInfo {
  path: string;
  mode: FolderMode;
}

/** Result of probing a picked folder before adding it. */
export interface ProbeResult {
  total: number;
  nested: number;
  subfolders: number;
}

/** A set of byte-identical books (same content hash). */
export interface DupGroup {
  key: string;
  books: BookRow[];
}

export type MoveAction = "move" | "skip" | "rename" | "replace";

/** A planned move for one source item. */
export interface MovePlan {
  src: string;
  name: string;
  collides: boolean;
  error: string | null;
}

export interface MoveOp {
  src: string;
  action: MoveAction;
}

/** A book row from the library DB. */
export interface BookRow {
  path: string;
  folder: string;
  format: string;
  title: string;
  size: number;
  mtime: number;
  page_count: number;
  /** 'discovered' | 'ready' | 'invalid' */
  status: string;
  error: string | null;
  last_page: number;
  has_cover: boolean;
  favorite: boolean;
  /** Unix seconds last opened, or null if never / removed from Being Read. */
  last_opened: number | null;
  /** Fixed-layout KF8 (comic / picture book): read via the page-image pager. */
  fixed_layout: boolean;
}

/** One rendered page (from `get_page`). */
export interface PageData {
  mime: string;
  base64: string;
}

/** A ranked destination folder for a book being imported. */
export interface ImportSuggestion {
  folder: string;
  score: number;
  reason: string;
}

/** A picked file plus where Readaity thinks it should go. */
export interface ImportPlan {
  path: string;
  title: string;
  format: string;
  /** Every library folder, best destination first. */
  suggestions: ImportSuggestion[];
}

/** A saved place in a book. `position` matches the reader's `last_page` unit:
 *  a page index for comic/PDF, per-mille (0–1000) for reflowable formats. */
export interface Bookmark {
  id: number;
  position: number;
  label: string;
  created_at: number;
}

export type ReaderThemeId = "dark" | "light" | "sepia";

/** Reader appearance preferences (persisted in the settings k/v table). */
export interface ReaderPrefs {
  theme: ReaderThemeId;
  /** Font scale multiplier, 0.8–1.6 (1 = default). */
  fontScale: number;
}

/** One book in a verify report. */
export interface VerifyItem {
  path: string;
  title: string;
  library: string;
}

/** Result of an on-demand library integrity check (`verify-done` event). */
export interface VerifyReport {
  checked: number;
  ok: number;
  changed: VerifyItem[];
  missing: VerifyItem[];
}

/** Global work status for the top progress bar. */
export interface ScanStatus {
  phase: "idle" | "scanning" | "indexing" | "paused";
  current: number;
  total: number;
}

/** Network-sharing configuration (mirrors `share::ShareConfig`). */
export interface ShareConfig {
  enabled: boolean;
  port: number;
  name: string;
  pin_set: boolean;
  allowlist: string;
  audit: boolean;
  /** Max simultaneous downloads (0 = unlimited). */
  max_conn: number;
  /** Per-download bandwidth ceiling in KB/s (0 = unlimited). */
  rate_kbps: number;
}

/** Live state of the share server (mirrors `share::ShareStatus`). */
export interface ShareStatus {
  running: boolean;
  port: number;
  urls: string[];
  fingerprint: string;
  pin_set: boolean;
}

/** One recorded share-server event. */
export interface AuditRow {
  ts: number;
  ip: string;
  event: string;
  detail: string | null;
}
