// Typed wrappers around the Rust commands. Keep every `invoke` behind one of
// these so the IPC surface stays in one place.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AuditRow,
  Bookmark,
  BookRow,
  DupGroup,
  FolderInfo,
  FolderMode,
  ImportPlan,
  LibraryKind,
  MoveOp,
  MovePlan,
  PageData,
  Peer,
  PeerBook,
  PeerCheck,
  ProbeResult,
  ScanStatus,
  ShareConfig,
  ShareStatus,
  VerifyReport,
} from "../types";

/** Open the native folder picker. Returns null if cancelled. */
export function pickFolder(): Promise<string | null> {
  return invoke<string | null>("pick_folder");
}

/** Native multi-file picker filtered to the library's formats. */
export function pickBookFiles(library: LibraryKind): Promise<string[]> {
  return invoke<string[]>("pick_book_files", { library });
}

/** Rank the library's folders as a destination for each picked file. */
export function suggestImport(
  paths: string[],
  library: LibraryKind,
): Promise<ImportPlan[]> {
  return invoke<ImportPlan[]>("suggest_import", { paths, library });
}

/** Copy each file into its chosen folder, then rescan. Returns the new list. */
export function importBooks(
  items: { path: string; dest: string }[],
  library: LibraryKind,
): Promise<BookRow[]> {
  return invoke<BookRow[]>("import_books", { items, library });
}

/** Fast pre-add probe (no archives opened) to decide the add mode. */
export function probeFolder(
  path: string,
  library: LibraryKind,
): Promise<ProbeResult> {
  return invoke<ProbeResult>("probe_folder", { path, library });
}

/** Phase-1 scan a new folder with a display mode; returns the library's books. */
export function addFolder(
  path: string,
  mode: FolderMode,
  library: LibraryKind,
): Promise<BookRow[]> {
  return invoke<BookRow[]>("add_folder", { path, mode, library });
}

/** Remove a whole library folder and its books (files kept on disk). */
export function removeFolder(
  path: string,
  library: LibraryKind,
): Promise<BookRow[]> {
  return invoke<BookRow[]>("remove_folder", { path, library });
}

/** Remove one book from the library (kept on disk, excluded from rescans). */
export function removeBook(
  path: string,
  library: LibraryKind,
): Promise<BookRow[]> {
  return invoke<BookRow[]>("remove_book", { path, library });
}

/** Remove a subfolder's books from the library (kept on disk, subtree excluded). */
export function removePath(
  path: string,
  library: LibraryKind,
): Promise<BookRow[]> {
  return invoke<BookRow[]>("remove_path", { path, library });
}

export function listFolders(library: LibraryKind): Promise<FolderInfo[]> {
  return invoke<FolderInfo[]>("list_folders", { library });
}

// ---- App settings (key/value preferences) ----

/** Read a persisted preference by key, or null if never set. */
export function getSetting(key: string): Promise<string | null> {
  return invoke<string | null>("get_setting", { key });
}

/** Persist a preference. */
export function setSetting(key: string, value: string): Promise<void> {
  return invoke("set_setting", { key, value });
}

// ---- Bookmarks ----

export function listBookmarks(path: string): Promise<Bookmark[]> {
  return invoke<Bookmark[]>("list_bookmarks", { path });
}

export function addBookmark(
  path: string,
  position: number,
  label: string,
): Promise<Bookmark> {
  return invoke<Bookmark>("add_bookmark", { path, position, label });
}

export function removeBookmark(id: number): Promise<void> {
  return invoke("remove_bookmark", { id });
}

// ---- Library integrity check ----

/** Start an on-demand re-hash of every indexed book. Returns how many will be
 *  checked (0 if a pass is already running / nothing to check). Progress via
 *  `onVerifyStatus`; the result via `onVerifyDone`. */
export function verifyLibrary(): Promise<number> {
  return invoke<number>("verify_library");
}

/** Re-queue specific books for validation (after verify finds changed files). */
export function recheckBooks(paths: string[]): Promise<void> {
  return invoke("recheck_books", { paths });
}

export function onVerifyStatus(
  cb: (s: { checked: number; total: number }) => void,
): Promise<UnlistenFn> {
  return listen<{ checked: number; total: number }>("verify-status", (e) =>
    cb(e.payload),
  );
}

export function onVerifyDone(
  cb: (r: VerifyReport) => void,
): Promise<UnlistenFn> {
  return listen<VerifyReport>("verify-done", (e) => cb(e.payload));
}

// ---- Network sharing (b4) ----

export function shareGetConfig(): Promise<ShareConfig> {
  return invoke<ShareConfig>("share_get_config");
}

export function shareSetConfig(
  port: number,
  name: string,
  allowlist: string,
  audit: boolean,
  maxConn: number,
  rateKbps: number,
): Promise<ShareConfig> {
  return invoke<ShareConfig>("share_set_config", {
    port,
    name,
    allowlist,
    audit,
    maxConn,
    rateKbps,
  });
}

/** Inline SVG QR code for a share URL. */
export function shareQr(url: string): Promise<string> {
  return invoke<string>("share_qr", { url });
}

export function shareSetPin(pin: string): Promise<void> {
  return invoke("share_set_pin", { pin });
}

export function shareGeneratePin(): Promise<string> {
  return invoke<string>("share_generate_pin");
}

export function shareStart(): Promise<ShareStatus> {
  return invoke<ShareStatus>("share_start");
}

export function shareStop(): Promise<void> {
  return invoke("share_stop");
}

export function shareStatus(): Promise<ShareStatus> {
  return invoke<ShareStatus>("share_status");
}

export function shareRegenerateCert(): Promise<string> {
  return invoke<string>("share_regenerate_cert");
}

export function shareAuditLog(limit: number): Promise<AuditRow[]> {
  return invoke<AuditRow[]>("share_audit_log", { limit });
}

export function shareClearAudit(): Promise<void> {
  return invoke("share_clear_audit");
}

export function listBooks(library: LibraryKind): Promise<BookRow[]> {
  return invoke<BookRow[]>("list_books", { library });
}

/** Ready-book counts per library (for the idle status bar). */
export function libraryCounts(): Promise<{ comics: number; ebooks: number }> {
  return invoke<{ comics: number; ebooks: number }>("library_counts");
}

/** Re-scan a library's folders for added/removed/changed files, then sweep. */
export function rescan(library: LibraryKind): Promise<BookRow[]> {
  return invoke<BookRow[]>("rescan", { library });
}

/** Re-index a library: reset its books and re-run the validation sweep. */
export function reindex(library: LibraryKind): Promise<BookRow[]> {
  return invoke<BookRow[]>("reindex", { library });
}

// ---- Duplicates / exclusions (global across libraries) ----

export function listExclusions(): Promise<string[]> {
  return invoke<string[]>("list_exclusions");
}

export function restoreExclusion(
  path: string,
  library: LibraryKind,
): Promise<BookRow[]> {
  return invoke<BookRow[]>("restore_exclusion", { path, library });
}

export function clearExclusions(library: LibraryKind): Promise<BookRow[]> {
  return invoke<BookRow[]>("clear_exclusions", { library });
}

export function listDuplicates(): Promise<DupGroup[]> {
  return invoke<DupGroup[]>("list_duplicates");
}

export function listNameDuplicates(): Promise<DupGroup[]> {
  return invoke<DupGroup[]>("list_name_duplicates");
}

export function ignoreDupe(key: string): Promise<void> {
  return invoke("ignore_dupe", { key });
}

export function unignoreDupe(key: string): Promise<void> {
  return invoke("unignore_dupe", { key });
}

export function listIgnoredDupes(): Promise<string[]> {
  return invoke<string[]>("list_ignored_dupes");
}

// ---- Covers / pages / files ----

/** Cached cover thumbnail as a data URL, or null if none. */
export async function getCoverUrl(path: string): Promise<string | null> {
  const b64 = await invoke<string | null>("get_cover", { path });
  return b64 ? `data:image/jpeg;base64,${b64}` : null;
}

/** Move a book to another library (comic-format azw3 → Comics). Returns the
 *  current library's refreshed list. */
export function setBookLibrary(
  path: string,
  to: string | null,
  library: string,
): Promise<BookRow[]> {
  return invoke<BookRow[]>("set_book_library", { path, to, library });
}

/** `[fixedLayoutCount, otherCount]` for a just-added folder. */
export function folderLayoutSplit(
  path: string,
  library: string,
): Promise<[number, number]> {
  return invoke<[number, number]>("folder_layout_split", { path, library });
}

/** Move every fixed-layout book under a folder into the Comics library. */
export function splitFolderLibraries(
  path: string,
  library: string,
): Promise<BookRow[]> {
  return invoke<BookRow[]>("split_folder_libraries", { path, library });
}

/** Store a frontend-generated cover (base64 JPEG) if the book has none. */
export function setCover(
  path: string,
  base64Jpeg: string,
  width: number,
  height: number,
): Promise<void> {
  return invoke("set_cover", { path, data: base64Jpeg, width, height });
}

/** Fetch one comic page (live from the archive) as a data URL. */
export async function getPageUrl(path: string, index: number): Promise<string> {
  const page = await invoke<PageData>("get_page", { path, index });
  return `data:${page.mime};base64,${page.base64}`;
}

/** One fixed-layout KF8 page: a self-contained HTML doc sized to w×h CSS px. */
export interface Kf8Page {
  html: string;
  w: number;
  h: number;
}

/** Page dimensions for a fixed-layout KF8 book — `[w, h]` per page. Triggers
 *  (and caches) the reassembly; pages themselves load lazily. */
export function getKf8PageDims(path: string): Promise<[number, number][]> {
  return invoke<[number, number][]>("get_kf8_page_dims", { path });
}

/** One page of a fixed-layout KF8 book. */
export function getKf8Page(path: string, index: number): Promise<Kf8Page> {
  return invoke<Kf8Page>("get_kf8_page", { path, index });
}

/** Read an entire book file as base64 (for epub.js / pdf.js). */
export function readBookBytes(path: string): Promise<string> {
  return invoke<string>("read_book_bytes", { path });
}

/** Extract a MOBI/AZW book's HTML content (decompressed, images inlined). */
export function getMobiHtml(path: string): Promise<string> {
  return invoke<string>("get_mobi_html", { path });
}

/** Convert an RTF book to HTML for the reader. */
export function getRtfHtml(path: string): Promise<string> {
  return invoke<string>("get_rtf_html", { path });
}

/** Read a plain-text book's decoded content. */
export function getTextContent(path: string): Promise<string> {
  return invoke<string>("get_text_content", { path });
}

// ---- Progress / indexing ----

export function setProgress(path: string, page: number): Promise<void> {
  return invoke("set_progress", { path, page });
}

// ---- Favourites / Being Read ----

/** Toggle a book's favourite flag; returns the library's refreshed list. */
export function setFavorite(
  path: string,
  favorite: boolean,
  library: LibraryKind,
): Promise<BookRow[]> {
  return invoke<BookRow[]>("set_favorite", { path, favorite, library });
}

/** Mark a book as opened now (adds it to Being Read). Fire-and-forget. */
export function markOpened(path: string): Promise<void> {
  return invoke("mark_opened", { path });
}

/** Remove a book from the Being Read shelf; returns the refreshed list. */
export function clearBeingRead(
  path: string,
  library: LibraryKind,
): Promise<BookRow[]> {
  return invoke<BookRow[]>("clear_being_read", { path, library });
}

export function pauseIndexing(): Promise<void> {
  return invoke("pause_indexing");
}

export function resumeIndexing(): Promise<void> {
  return invoke("resume_indexing");
}

/** Plan a drag-move (collision + validity check, no disk changes). */
export function planMove(sources: string[], destDir: string): Promise<MovePlan[]> {
  return invoke<MovePlan[]>("plan_move", { sources, dest_dir: destDir });
}

/** Physically move items into a folder and rewrite their DB paths. */
export function moveItems(
  destDir: string,
  ops: MoveOp[],
  library: LibraryKind,
): Promise<BookRow[]> {
  return invoke<BookRow[]>("move_items", { dest_dir: destDir, ops, library });
}

// ---- LAN discovery + peer import ----

export function peerBrowse(): Promise<Peer[]> {
  return invoke<Peer[]>("peer_browse");
}

export function peerCheck(host: string, port: number): Promise<PeerCheck> {
  return invoke<PeerCheck>("peer_check", { host, port });
}

export function peerTrust(host: string, fingerprint: string): Promise<void> {
  return invoke("peer_trust", { host, fingerprint });
}

export function peerForget(host: string): Promise<void> {
  return invoke("peer_forget", { host });
}

export function peerBooks(
  host: string,
  port: number,
  pin: string,
  library: LibraryKind,
): Promise<PeerBook[]> {
  return invoke<PeerBook[]>("peer_books", { host, port, pin, library });
}

export function peerImport(
  host: string,
  port: number,
  pin: string,
  library: LibraryKind,
  ids: string[],
  dest: string,
): Promise<BookRow[]> {
  return invoke<BookRow[]>("peer_import", { host, port, pin, library, ids, dest });
}

export function onPeerImportStatus(
  cb: (s: { done: number; total: number }) => void,
): Promise<UnlistenFn> {
  return listen<{ done: number; total: number }>("peer-import-status", (e) =>
    cb(e.payload),
  );
}

// ---- Events ----

export function onBookUpdated(cb: (book: BookRow) => void): Promise<UnlistenFn> {
  return listen<BookRow>("book-updated", (e) => cb(e.payload));
}

export function onScanStatus(cb: (s: ScanStatus) => void): Promise<UnlistenFn> {
  return listen<ScanStatus>("scan-status", (e) => cb(e.payload));
}

/** Fired once at startup if a corrupt DB was quarantined and rebuilt. */
export function onLibraryRecovered(cb: () => void): Promise<UnlistenFn> {
  return listen("library-recovered", () => cb());
}
