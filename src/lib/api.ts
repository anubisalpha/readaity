// Typed wrappers around the Rust commands. Keep every `invoke` behind one of
// these so the IPC surface stays in one place.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  BookRow,
  DupGroup,
  FolderInfo,
  FolderMode,
  LibraryKind,
  MoveOp,
  MovePlan,
  PageData,
  ProbeResult,
  ScanStatus,
} from "../types";

/** Open the native folder picker. Returns null if cancelled. */
export function pickFolder(): Promise<string | null> {
  return invoke<string | null>("pick_folder");
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

// ---- Events ----

export function onBookUpdated(cb: (book: BookRow) => void): Promise<UnlistenFn> {
  return listen<BookRow>("book-updated", (e) => cb(e.payload));
}

export function onScanStatus(cb: (s: ScanStatus) => void): Promise<UnlistenFn> {
  return listen<ScanStatus>("scan-status", (e) => cb(e.payload));
}
