//! SQLite-backed library database.
//!
//! Two-phase model:
//!   * Phase 1 (fast) inserts rows with `status = 'discovered'` — path/size/mtime
//!     only, no archive opened.
//!   * Phase 2 (sweep) validates each discovered/changed row, filling page_count,
//!     md5, and a cover thumbnail, then flips it to `status = 'ready'` (or
//!     `'invalid'` with a reason).
//!
//! Fast rescans compare (size, mtime): unchanged rows keep their cached cover and
//! metadata and are never reopened.

use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

/// Managed Tauri state: the one DB connection behind a mutex.
pub struct AppDb(pub Mutex<Connection>);

/// A library folder and how its contents are presented.
///
/// `mode`:
///   * "tree"    — keep the folder as one root with its subfolders navigable
///   * "flat"    — collapse all nested comics into one flat list
///   * "promote" — drop this wrapper; its subfolders become top-level roots
#[derive(Serialize, Clone)]
pub struct FolderRow {
    pub path: String,
    pub mode: String,
    pub library: String,
}

/// A book row as sent to the frontend (cover blob fetched separately).
#[derive(Serialize, Clone)]
pub struct BookRow {
    pub path: String,
    pub folder: String,
    pub format: String,
    pub title: String,
    pub size: i64,
    pub mtime: i64,
    pub page_count: i64,
    pub status: String,
    pub error: Option<String>,
    pub last_page: i64,
    pub has_cover: bool,
}

/// Open (or create) the DB and ensure the schema exists.
pub fn open(db_path: &std::path::Path) -> Result<Connection, String> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
    conn.pragma_update(None, "journal_mode", "WAL").ok();
    conn.pragma_update(None, "foreign_keys", "ON").ok();
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS folders (
            path      TEXT PRIMARY KEY,
            added_at  INTEGER NOT NULL,
            mode      TEXT NOT NULL DEFAULT 'tree',
            library   TEXT NOT NULL DEFAULT 'comics'
        );
        CREATE TABLE IF NOT EXISTS exclusions (
            path      TEXT PRIMARY KEY,
            added_at  INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS ignored_dupes (
            key       TEXT PRIMARY KEY,
            added_at  INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS settings (
            key       TEXT PRIMARY KEY,
            value     TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS share_audit (
            id        INTEGER PRIMARY KEY AUTOINCREMENT,
            ts        INTEGER NOT NULL,
            ip        TEXT NOT NULL,
            event     TEXT NOT NULL,
            detail    TEXT
        );
        CREATE TABLE IF NOT EXISTS books (
            path        TEXT PRIMARY KEY,
            folder      TEXT NOT NULL,
            format      TEXT NOT NULL,
            title       TEXT NOT NULL,
            size        INTEGER NOT NULL,
            mtime       INTEGER NOT NULL,
            md5         TEXT,
            page_count  INTEGER NOT NULL DEFAULT 0,
            cover       BLOB,
            cover_w     INTEGER,
            cover_h     INTEGER,
            status      TEXT NOT NULL DEFAULT 'discovered',
            error       TEXT,
            last_page   INTEGER NOT NULL DEFAULT 0,
            library     TEXT NOT NULL DEFAULT 'comics',
            updated_at  INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_books_folder ON books(folder);
        CREATE INDEX IF NOT EXISTS idx_books_status ON books(status);
        "#,
    )
    .map_err(|e| e.to_string())?;

    // Migration: add folders.mode to databases created before it existed.
    let has_mode: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('folders') WHERE name = 'mode'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .unwrap_or(false);
    if !has_mode {
        conn.execute(
            "ALTER TABLE folders ADD COLUMN mode TEXT NOT NULL DEFAULT 'tree'",
            [],
        )
        .ok();
    }

    // Migration: add the `library` column to folders and books (pre-ebooks DBs).
    for (table, _) in [("folders", ()), ("books", ())] {
        let has: bool = conn
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = 'library'"
                ),
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);
        if !has {
            conn.execute(
                &format!(
                    "ALTER TABLE {table} ADD COLUMN library TEXT NOT NULL DEFAULT 'comics'"
                ),
                [],
            )
            .ok();
        }
    }

    // Index on library must come after the column exists on migrated DBs.
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_books_library ON books(library)",
        [],
    )
    .ok();

    Ok(conn)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ---------- Folders ----------

pub fn add_folder(conn: &Connection, path: &str, mode: &str, library: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO folders(path, added_at, mode, library) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(path) DO UPDATE SET mode = excluded.mode, library = excluded.library",
        params![path, now(), mode, library],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn remove_folder(conn: &Connection, path: &str) -> Result<(), String> {
    // Remove the folder and every book discovered beneath it. The folder is no
    // longer scanned, so no exclusion is needed to keep it out of rescans.
    // Compare separator-insensitively: the UI may pass a '/'-normalised path
    // while the DB stores the OS-native (Windows '\') form.
    let norm = path.replace('\\', "/");
    conn.execute(
        "DELETE FROM books WHERE REPLACE(folder, '\\', '/') = ?1",
        params![norm],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM folders WHERE REPLACE(path, '\\', '/') = ?1",
        params![norm],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn list_folders(conn: &Connection, library: &str) -> Result<Vec<FolderRow>, String> {
    let mut stmt = conn
        .prepare("SELECT path, mode, library FROM folders WHERE library = ?1 ORDER BY added_at")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![library], |r| {
            Ok(FolderRow {
                path: r.get(0)?,
                mode: r.get(1)?,
                library: r.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// All folder paths across every library, with their library tag (for rescans).
pub fn all_folders(conn: &Connection) -> Result<Vec<FolderRow>, String> {
    let mut stmt = conn
        .prepare("SELECT path, mode, library FROM folders ORDER BY added_at")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(FolderRow {
                path: r.get(0)?,
                mode: r.get(1)?,
                library: r.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

// ---------- Settings (key/value app preferences) ----------

/// Read one preference, or `None` if it was never set.
pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// Insert or update one preference.
pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO settings(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------- Share audit log ----------

/// One recorded event from the network-sharing server.
#[derive(Serialize, Clone)]
pub struct AuditRow {
    pub ts: i64,
    pub ip: String,
    pub event: String,
    pub detail: Option<String>,
}

pub fn add_audit(conn: &Connection, ip: &str, event: &str, detail: Option<&str>) -> Result<(), String> {
    conn.execute(
        "INSERT INTO share_audit(ts, ip, event, detail) VALUES (?1, ?2, ?3, ?4)",
        params![now(), ip, event, detail],
    )
    .map_err(|e| e.to_string())?;
    // Keep the log bounded.
    conn.execute(
        "DELETE FROM share_audit WHERE id NOT IN
           (SELECT id FROM share_audit ORDER BY id DESC LIMIT 2000)",
        [],
    )
    .ok();
    Ok(())
}

pub fn list_audit(conn: &Connection, limit: i64) -> Result<Vec<AuditRow>, String> {
    let mut stmt = conn
        .prepare("SELECT ts, ip, event, detail FROM share_audit ORDER BY id DESC LIMIT ?1")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![limit], |r| {
            Ok(AuditRow {
                ts: r.get(0)?,
                ip: r.get(1)?,
                event: r.get(2)?,
                detail: r.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn clear_audit(conn: &Connection) -> Result<(), String> {
    conn.execute("DELETE FROM share_audit", [])
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------- Exclusions (removed-from-library, but kept on disk) ----------

/// Record a path (file or directory prefix) as excluded from the library, so a
/// later rescan won't re-add it. Files are left on disk untouched.
pub fn add_exclusion(conn: &Connection, path: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO exclusions(path, added_at) VALUES (?1, ?2)",
        params![path, now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn list_exclusions(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT path FROM exclusions")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// Drop one exclusion so the path can be re-discovered on the next scan.
pub fn remove_exclusion(conn: &Connection, path: &str) -> Result<(), String> {
    conn.execute("DELETE FROM exclusions WHERE path = ?1", params![path])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Drop every exclusion.
pub fn clear_exclusions(conn: &Connection) -> Result<(), String> {
    conn.execute("DELETE FROM exclusions", [])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Remove one book from the library and exclude its path from future rescans.
pub fn remove_book(conn: &Connection, path: &str) -> Result<(), String> {
    add_exclusion(conn, path)?;
    conn.execute("DELETE FROM books WHERE path = ?1", params![path])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Remove every book under a directory prefix and exclude that subtree.
pub fn remove_subtree(conn: &Connection, prefix: &str) -> Result<(), String> {
    add_exclusion(conn, prefix)?;
    // Compare separator-insensitively: normalise both the stored path and the
    // incoming prefix to '/'. Book paths are stored OS-native (Windows '\').
    let norm = prefix.replace('\\', "/");
    let like = format!("{}/%", norm);
    conn.execute(
        "DELETE FROM books WHERE REPLACE(path, '\\', '/') = ?1
            OR REPLACE(path, '\\', '/') LIKE ?2",
        params![norm, like],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------- Phase 1: discovery ----------

/// Insert a freshly discovered file, or reset an existing row to `discovered`
/// if its (size, mtime) changed. Unchanged rows are left untouched (returns
/// `false` — nothing to re-sweep). Returns `true` if the row needs sweeping.
#[allow(clippy::too_many_arguments)]
pub fn upsert_discovered(
    conn: &Connection,
    path: &str,
    folder: &str,
    format: &str,
    title: &str,
    size: i64,
    mtime: i64,
    library: &str,
) -> Result<bool, String> {
    let existing: Option<(i64, i64, String)> = conn
        .query_row(
            "SELECT size, mtime, status FROM books WHERE path = ?1",
            params![path],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;

    match existing {
        Some((s, m, status)) if s == size && m == mtime && status != "discovered" => {
            // Unchanged and already processed — reuse cache.
            Ok(false)
        }
        Some(_) => {
            // Changed (or still pending): reset to discovered, drop stale metadata
            // but keep the old cover so the shelf doesn't flicker until re-swept.
            conn.execute(
                "UPDATE books SET folder=?2, format=?3, title=?4, size=?5, mtime=?6,
                    md5=NULL, page_count=0, status='discovered', error=NULL, library=?8,
                    updated_at=?7
                 WHERE path=?1",
                params![path, folder, format, title, size, mtime, now(), library],
            )
            .map_err(|e| e.to_string())?;
            Ok(true)
        }
        None => {
            conn.execute(
                "INSERT INTO books(path, folder, format, title, size, mtime, status, library, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'discovered', ?8, ?7)",
                params![path, folder, format, title, size, mtime, now(), library],
            )
            .map_err(|e| e.to_string())?;
            Ok(true)
        }
    }
}

/// Delete rows under `folder` whose paths are not in `seen` (files removed on disk).
pub fn prune_missing(conn: &Connection, folder: &str, seen: &[String]) -> Result<usize, String> {
    let mut stmt = conn
        .prepare("SELECT path FROM books WHERE folder = ?1")
        .map_err(|e| e.to_string())?;
    let existing: Vec<String> = stmt
        .query_map(params![folder], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;

    let seen_set: std::collections::HashSet<&str> = seen.iter().map(|s| s.as_str()).collect();
    let mut removed = 0;
    for p in existing {
        if !seen_set.contains(p.as_str()) {
            conn.execute("DELETE FROM books WHERE path = ?1", params![p])
                .map_err(|e| e.to_string())?;
            removed += 1;
        }
    }
    Ok(removed)
}

// ---------- Phase 2: validation results ----------

/// Paths (and format) of books awaiting validation.
pub fn pending(conn: &Connection) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare("SELECT path, format FROM books WHERE status = 'discovered' ORDER BY title")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

#[allow(clippy::too_many_arguments)]
pub fn set_validated(
    conn: &Connection,
    path: &str,
    page_count: i64,
    md5: &str,
    cover: &[u8],
    cover_w: i64,
    cover_h: i64,
) -> Result<(), String> {
    // An empty cover (e.g. an ebook with no extractable image) is stored NULL.
    let cover_opt: Option<&[u8]> = if cover.is_empty() { None } else { Some(cover) };
    conn.execute(
        "UPDATE books SET page_count=?2, md5=?3, cover=?4, cover_w=?5, cover_h=?6,
            status='ready', error=NULL, updated_at=?7 WHERE path=?1",
        params![path, page_count, md5, cover_opt, cover_w, cover_h, now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_invalid(conn: &Connection, path: &str, error: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE books SET status='invalid', error=?2, updated_at=?3 WHERE path=?1",
        params![path, error, now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------- Queries for the UI ----------

fn map_book(r: &rusqlite::Row) -> rusqlite::Result<BookRow> {
    Ok(BookRow {
        path: r.get(0)?,
        folder: r.get(1)?,
        format: r.get(2)?,
        title: r.get(3)?,
        size: r.get(4)?,
        mtime: r.get(5)?,
        page_count: r.get(6)?,
        status: r.get(7)?,
        error: r.get(8)?,
        last_page: r.get(9)?,
        has_cover: r.get(10)?,
    })
}

const BOOK_COLS: &str = "path, folder, format, title, size, mtime, page_count, status, error, \
                         last_page, cover IS NOT NULL";

/// Books in one library (for the shelf).
pub fn list_books(conn: &Connection, library: &str) -> Result<Vec<BookRow>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {BOOK_COLS} FROM books WHERE library = ?1 ORDER BY title COLLATE NOCASE"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![library], map_book)
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// A ready book with its real path + hash, for the network-share server only.
#[derive(Clone)]
pub struct ShareBook {
    pub path: String,
    pub title: String,
    pub format: String,
    pub size: i64,
    pub page_count: i64,
    pub md5: Option<String>,
    pub has_cover: bool,
}

/// Ready books in one library (real paths — never sent to a client as-is).
pub fn share_list(conn: &Connection, library: &str) -> Result<Vec<ShareBook>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT path, title, format, size, page_count, md5, cover IS NOT NULL
             FROM books WHERE library = ?1 AND status = 'ready'
             ORDER BY title COLLATE NOCASE",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![library], |r| {
            Ok(ShareBook {
                path: r.get(0)?,
                title: r.get(1)?,
                format: r.get(2)?,
                size: r.get(3)?,
                page_count: r.get(4)?,
                md5: r.get(5)?,
                has_cover: r.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// Reset a library's books to `discovered` so the sweep re-extracts covers,
/// page counts and hashes with the current code. Reading progress is kept.
pub fn reindex(conn: &Connection, library: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE books SET status='discovered', cover=NULL, cover_w=NULL, cover_h=NULL,
            page_count=0, md5=NULL WHERE library=?1",
        params![library],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Count of `ready` books in one library.
pub fn ready_count(conn: &Connection, library: &str) -> Result<i64, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM books WHERE status = 'ready' AND library = ?1",
        params![library],
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}

/// Every book across all libraries (for duplicate detection).
pub fn list_all_books(conn: &Connection) -> Result<Vec<BookRow>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {BOOK_COLS} FROM books ORDER BY title COLLATE NOCASE"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], map_book)
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// A set of books that are byte-identical (same MD5).
#[derive(Serialize, Clone)]
pub struct DupGroup {
    pub key: String,
    pub books: Vec<BookRow>,
}

/// Groups of byte-identical books (same MD5, more than one copy).
pub fn list_duplicates(conn: &Connection) -> Result<Vec<DupGroup>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT path, folder, format, title, size, mtime, page_count, status, error,
                    last_page, cover IS NOT NULL, md5
             FROM books
             WHERE md5 IS NOT NULL
               AND md5 IN (SELECT md5 FROM books WHERE md5 IS NOT NULL
                           GROUP BY md5 HAVING COUNT(*) > 1)
             ORDER BY md5, title COLLATE NOCASE",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |r| {
            let book = BookRow {
                path: r.get(0)?,
                folder: r.get(1)?,
                format: r.get(2)?,
                title: r.get(3)?,
                size: r.get(4)?,
                mtime: r.get(5)?,
                page_count: r.get(6)?,
                status: r.get(7)?,
                error: r.get(8)?,
                last_page: r.get(9)?,
                has_cover: r.get(10)?,
            };
            let md5: String = r.get(11)?;
            Ok((md5, book))
        })
        .map_err(|e| e.to_string())?;

    let mut groups: Vec<DupGroup> = Vec::new();
    for row in rows {
        let (md5, book) = row.map_err(|e| e.to_string())?;
        match groups.last_mut() {
            Some(g) if g.key == md5 => g.books.push(book),
            _ => groups.push(DupGroup {
                key: md5,
                books: vec![book],
            }),
        }
    }
    Ok(groups)
}

/// Immediate parent folder name of a stored path (for series context).
fn parent_name(path: &str) -> String {
    let n = path.replace('\\', "/");
    match n.rfind('/') {
        Some(i) => n[..i].rsplit('/').next().unwrap_or("").to_string(),
        None => String::new(),
    }
}

pub fn ignore_dupe(conn: &Connection, key: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO ignored_dupes(key, added_at) VALUES (?1, ?2)",
        params![key, now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn unignore_dupe(conn: &Connection, key: &str) -> Result<(), String> {
    conn.execute("DELETE FROM ignored_dupes WHERE key = ?1", params![key])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn list_ignored_dupes(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT key FROM ignored_dupes ORDER BY key")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// Groups of books that *look* like the same issue by filename (fuzzy), sorted
/// largest-file-first within each group so the frontend can suggest keeping it.
/// The parent folder name is factored into the series; ignored keys are omitted.
pub fn list_name_duplicates(conn: &Connection) -> Result<Vec<DupGroup>, String> {
    let ignored: std::collections::HashSet<String> =
        list_ignored_dupes(conn)?.into_iter().collect();

    let books = list_all_books(conn)?;
    let mut map: std::collections::HashMap<String, Vec<BookRow>> = std::collections::HashMap::new();
    for b in books.into_iter().filter(|b| b.status != "invalid") {
        let folder = parent_name(&b.path);
        if let Some(key) = crate::library::name_key_ctx(&b.title, &folder) {
            map.entry(key).or_default().push(b);
        }
    }
    let mut groups: Vec<DupGroup> = map
        .into_iter()
        .filter(|(k, v)| v.len() > 1 && !ignored.contains(k))
        .map(|(k, mut v)| {
            v.sort_by(|a, b| b.size.cmp(&a.size)); // largest first = suggested keep
            DupGroup { key: k, books: v }
        })
        .collect();
    groups.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(groups)
}

/// Store a cover thumbnail generated elsewhere (e.g. a PDF page rendered by
/// pdf.js in the frontend). Only sets it if the book currently has none.
pub fn set_cover(
    conn: &Connection,
    path: &str,
    cover: &[u8],
    cover_w: i64,
    cover_h: i64,
) -> Result<(), String> {
    conn.execute(
        "UPDATE books SET cover=?2, cover_w=?3, cover_h=?4
         WHERE path=?1 AND cover IS NULL",
        params![path, cover, cover_w, cover_h],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_cover(conn: &Connection, path: &str) -> Result<Option<Vec<u8>>, String> {
    conn.query_row(
        "SELECT cover FROM books WHERE path = ?1",
        params![path],
        |r| r.get::<_, Option<Vec<u8>>>(0),
    )
    .optional()
    .map(|opt| opt.flatten())
    .map_err(|e| e.to_string())
}

/// Rewrite book paths after a filesystem move from `src_norm` to `target_norm`
/// (both '/'-normalised). Handles a single file or a whole subtree, and updates
/// each moved book's library `folder` to the root that now contains it.
pub fn relocate(conn: &Connection, src_norm: &str, target_norm: &str) -> Result<(), String> {
    // Longest library root (normalised) that contains the target.
    let root: Option<String> = {
        let mut stmt = conn
            .prepare("SELECT path FROM folders")
            .map_err(|e| e.to_string())?;
        let paths: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        paths
            .into_iter()
            .filter(|p| {
                let pn = p.replace('\\', "/");
                target_norm == pn || target_norm.starts_with(&format!("{pn}/"))
            })
            .max_by_key(|p| p.replace('\\', "/").len())
    };

    // Books at the source path or nested beneath it.
    let like = format!("{src_norm}/%");
    let olds: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT path FROM books
                 WHERE REPLACE(path, '\\', '/') = ?1 OR REPLACE(path, '\\', '/') LIKE ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows: Vec<String> = stmt
            .query_map(params![src_norm, like], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        rows
    };

    for old in olds {
        let old_norm = old.replace('\\', "/");
        let suffix = &old_norm[src_norm.len()..]; // "" for the file itself, "/…" nested
        let new_path = format!("{target_norm}{suffix}");
        match &root {
            Some(r) => conn.execute(
                "UPDATE books SET path = ?1, folder = ?2 WHERE path = ?3",
                params![new_path, r, old],
            ),
            None => conn.execute(
                "UPDATE books SET path = ?1 WHERE path = ?2",
                params![new_path, old],
            ),
        }
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn set_progress(conn: &Connection, path: &str, page: i64) -> Result<(), String> {
    conn.execute(
        "UPDATE books SET last_page = ?2 WHERE path = ?1",
        params![path, page],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
