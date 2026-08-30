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
}

/** One rendered page (from `get_page`). */
export interface PageData {
  mime: string;
  base64: string;
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
