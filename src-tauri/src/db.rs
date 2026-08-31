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
    /// User-flagged favourite (per-library "Favourites" shelf).
    pub favorite: bool,
    /// Unix seconds this book was last opened, or `None` if never opened or
    /// dismissed from the per-library "Being Read" shelf.
    pub last_opened: Option<i64>,
    /// Fixed-layout KF8 (comic / manga / picture book): read as a page-image
    /// pager, not reflowable text.
    pub fixed_layout: bool,
}

/// Open (or create) the DB and ensure the schema exists.
///
/// A corrupt existing file is reported as `Err` starting with `CORRUPT:` so the
/// caller can decide whether to quarantine and rebuild — see [`open_resilient`].
pub fn open(db_path: &std::path::Path) -> Result<Connection, String> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let existed = db_path.exists();
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;

    // WAL + synchronous=FULL: FULL closes the small window where a hard kill
    // *during a checkpoint* (WAL flushed back into the main file) could tear a
    // page. The extra fsyncs are noise next to the sweep's archive I/O.
    conn.pragma_update(None, "journal_mode", "WAL").ok();
    conn.pragma_update(None, "synchronous", "FULL").ok();
    conn.pragma_update(None, "foreign_keys", "ON").ok();
    conn.busy_timeout(std::time::Duration::from_secs(5)).ok();

    // Fail fast on a pre-existing corrupt file rather than half-applying the
    // schema batch on top of it.
    if existed {
        let ok = conn
            .query_row("PRAGMA quick_check(1)", [], |r| r.get::<_, String>(0))
            .map(|s| s == "ok")
            .unwrap_or(false);
        if !ok {
            return Err(format!(
                "CORRUPT: {} failed its integrity check",
                db_path.display()
            ));
        }
    }

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
        CREATE TABLE IF NOT EXISTS bookmarks (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            book_path  TEXT NOT NULL,
            position   INTEGER NOT NULL,
            label      TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_bookmarks_book ON bookmarks(book_path);
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
            favorite    INTEGER NOT NULL DEFAULT 0,
            last_opened INTEGER,
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

    // Migration: add books.favorite / books.last_opened (pre-shelves DBs),
    // books.fixed_layout (comic/picture-book KF8 detection).
    for (col, decl) in [
        ("favorite", "INTEGER NOT NULL DEFAULT 0"),
        ("last_opened", "INTEGER"),
        ("fixed_layout", "INTEGER NOT NULL DEFAULT 0"),
        // User moved this book to a different library than its folder's.
        ("library_override", "TEXT"),
    ] {
        let has: bool = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM pragma_table_info('books') WHERE name = '{col}'"),
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);
        if !has {
            conn.execute(&format!("ALTER TABLE books ADD COLUMN {col} {decl}"), [])
                .ok();
        }
    }

    Ok(conn)
}

/// Open the DB, recovering from a corrupt file if needed.
///
/// If the existing file fails its integrity check, its folder list is salvaged
/// (best effort), the bad file is moved to `library.db.corrupt-<unix>`, a fresh
/// DB is created, and the folders are re-added. The book cache (covers, page
/// counts, hashes, reading positions) is lost but rebuilds on the next scan.
/// Returns `(conn, recovered)` — `recovered == true` means a rebuild happened.
pub fn open_resilient(db_path: &std::path::Path) -> Result<(Connection, bool), String> {
    match open(db_path) {
        Ok(conn) => Ok((conn, false)),
        Err(e) if e.starts_with("CORRUPT:") => {
            let folders = salvage_folders(db_path);
            quarantine(db_path)?;
            let conn = open(db_path)?;
            for f in &folders {
                let _ = add_folder(&conn, &f.path, &f.mode, &f.library);
            }
            Ok((conn, true))
        }
        Err(e) => Err(e),
    }
}

/// Read the `folders` table from a possibly-corrupt file, opened read-only and
/// `immutable` so a damaged `books` btree doesn't block it.
fn salvage_folders(db_path: &std::path::Path) -> Vec<FolderRow> {
    let uri = format!(
        "file:{}?mode=ro&immutable=1",
        db_path.to_string_lossy().replace('\\', "/")
    );
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI;
    let Ok(conn) = Connection::open_with_flags(uri, flags) else {
        return Vec::new();
    };
    let Ok(mut stmt) = conn.prepare("SELECT path, mode, library FROM folders") else {
        return Vec::new();
    };
    let rows = stmt.query_map([], |r| {
        Ok(FolderRow {
            path: r.get(0)?,
            mode: r.get(1)?,
            library: r.get(2)?,
        })
    });
    match rows {
        Ok(iter) => iter.filter_map(Result::ok).collect(),
        Err(_) => Vec::new(),
    }
}

/// Move a corrupt DB (and its sidecars) aside so a fresh one can be created.
fn quarantine(db_path: &std::path::Path) -> Result<(), String> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let dead = db_path.with_file_name(format!(
        "{}.corrupt-{ts}",
        db_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "library.db".into())
    ));
    std::fs::rename(db_path, &dead).map_err(|e| format!("could not quarantine corrupt DB: {e}"))?;
    for ext in ["-wal", "-shm"] {
        let side = db_path.with_file_name(format!(
            "{}{ext}",
            db_path.file_name().unwrap_or_default().to_string_lossy()
        ));
        let _ = std::fs::remove_file(side);
    }
    Ok(())
}

/// Flush the WAL back into the main file and truncate it. Call on a clean exit
/// so the next launch opens a single consistent file with no journal to replay.
pub fn checkpoint(conn: &Connection) {
    let _ = conn.pragma_update(None, "wal_checkpoint", "TRUNCATE");
    let _ = conn.execute_batch("PRAGMA optimize");
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
    // Folders registered for this library, plus any folder that has books in
    // this library via a per-book move (`library_override`) — so a comic-format
    // azw3 moved out of its Ebooks folder still shows under a Comics folder node.
    let mut stmt = conn
        .prepare(
            "SELECT path, mode, library FROM folders WHERE library = ?1
             UNION
             SELECT f.path, f.mode, ?1 FROM folders f
               WHERE f.library <> ?1
                 AND EXISTS (SELECT 1 FROM books b
                             WHERE b.folder = f.path AND b.library = ?1)
             ORDER BY 1",
        )
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

/// Delete one preference.
pub fn del_setting(conn: &Connection, key: &str) -> Result<(), String> {
    conn.execute("DELETE FROM settings WHERE key = ?1", params![key])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// The set of MD5 hashes of `ready` books in one library — for import dedupe.
pub fn hashes(conn: &Connection, library: &str) -> Result<std::collections::HashSet<String>, String> {
    let mut stmt = conn
        .prepare("SELECT md5 FROM books WHERE library = ?1 AND md5 IS NOT NULL")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![library], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

// ---------- Bookmarks ----------

/// One saved place in a book. `position` is in the same unit the reader stores
/// as `last_page`: a page index for comic/PDF, per-mille (0–1000) for reflowable.
#[derive(Serialize, Clone)]
pub struct Bookmark {
    pub id: i64,
    pub position: i64,
    pub label: String,
    pub created_at: i64,
}

pub fn add_bookmark(
    conn: &Connection,
    book_path: &str,
    position: i64,
    label: &str,
) -> Result<Bookmark, String> {
    let ts = now();
    conn.execute(
        "INSERT INTO bookmarks(book_path, position, label, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![book_path, position, label, ts],
    )
    .map_err(|e| e.to_string())?;
    Ok(Bookmark {
        id: conn.last_insert_rowid(),
        position,
        label: label.to_string(),
        created_at: ts,
    })
}

pub fn list_bookmarks(conn: &Connection, book_path: &str) -> Result<Vec<Bookmark>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, position, label, created_at FROM bookmarks
             WHERE book_path = ?1 ORDER BY position, id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![book_path], |r| {
            Ok(Bookmark {
                id: r.get(0)?,
                position: r.get(1)?,
                label: r.get(2)?,
                created_at: r.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn remove_bookmark(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM bookmarks WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Drop bookmarks whose book is no longer in the library (called after any bulk
/// book deletion — folder removal, subtree removal, prune).
pub fn prune_orphan_bookmarks(conn: &Connection) {
    let _ = conn.execute(
        "DELETE FROM bookmarks WHERE book_path NOT IN (SELECT path FROM books)",
        [],
    );
}

// ---------- Library integrity check ----------

/// `(path, title, md5, library)` for every ready book that has a stored hash.
pub fn all_hashed(conn: &Connection) -> Result<Vec<(String, String, String, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT path, title, md5, library FROM books
             WHERE status = 'ready' AND md5 IS NOT NULL ORDER BY title COLLATE NOCASE",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// Reset specific books to `discovered` so the next sweep re-validates them.
pub fn recheck(conn: &Connection, paths: &[String]) -> Result<(), String> {
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    for p in paths {
        tx.execute(
            "UPDATE books SET status = 'discovered', updated_at = ?2 WHERE path = ?1",
            params![p, now()],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
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
    conn.execute("DELETE FROM bookmarks WHERE book_path = ?1", params![path])
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
    prune_orphan_bookmarks(conn);
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
            // A user's library move (library_override) wins over the folder's.
            conn.execute(
                "UPDATE books SET folder=?2, format=?3, title=?4, size=?5, mtime=?6,
                    md5=NULL, page_count=0, status='discovered', error=NULL,
                    library=COALESCE(library_override, ?8), updated_at=?7
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
pub fn prune_missing(
    conn: &Connection,
    folder: &str,
    library: &str,
    seen: &[String],
) -> Result<usize, String> {
    // Only this library's books under the folder — a Comics rescan of a shared
    // folder must not delete the Ebooks-library rows (and vice versa).
    let mut stmt = conn
        .prepare("SELECT path FROM books WHERE folder = ?1 AND library = ?2")
        .map_err(|e| e.to_string())?;
    let existing: Vec<String> = stmt
        .query_map(params![folder, library], |r| r.get::<_, String>(0))
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
    if removed > 0 {
        prune_orphan_bookmarks(conn);
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
    fixed_layout: bool,
) -> Result<(), String> {
    // An empty cover (e.g. an ebook with no extractable image) is stored NULL.
    let cover_opt: Option<&[u8]> = if cover.is_empty() { None } else { Some(cover) };
    conn.execute(
        "UPDATE books SET page_count=?2, md5=?3, cover=?4, cover_w=?5, cover_h=?6,
            fixed_layout=?8, status='ready', error=NULL, updated_at=?7 WHERE path=?1",
        params![path, page_count, md5, cover_opt, cover_w, cover_h, now(), fixed_layout as i64],
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
        favorite: r.get::<_, i64>(11)? != 0,
        last_opened: r.get(12)?,
        fixed_layout: r.get::<_, i64>(13)? != 0,
    })
}

const BOOK_COLS: &str = "path, folder, format, title, size, mtime, page_count, status, error, \
                         last_page, cover IS NOT NULL, favorite, last_opened, fixed_layout";

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

/// `(title, format)` for the books in one folder of one library — for the
/// smart-import destination scorer.
pub fn books_in_folder(
    conn: &Connection,
    folder: &str,
    library: &str,
) -> Result<Vec<(String, String)>, String> {
    let norm = folder.replace('\\', "/");
    let mut stmt = conn
        .prepare(
            "SELECT title, format FROM books
             WHERE REPLACE(folder, '\\', '/') = ?1 AND library = ?2",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![norm, library], |r| Ok((r.get(0)?, r.get(1)?)))
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
        .prepare(&format!(
            "SELECT {BOOK_COLS}, md5
             FROM books
             WHERE md5 IS NOT NULL
               AND md5 IN (SELECT md5 FROM books WHERE md5 IS NOT NULL
                           GROUP BY md5 HAVING COUNT(*) > 1)
             ORDER BY md5, title COLLATE NOCASE"
        ))
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |r| {
            let book = map_book(r)?;
            let md5: String = r.get(13)?;
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

/// Record whether a book is a fixed-layout KF8 (set cheaply in Phase 1 so the
/// "split libraries" prompt can fire before the full sweep).
pub fn set_fixed_layout(conn: &Connection, path: &str, fixed: bool) -> Result<(), String> {
    conn.execute(
        "UPDATE books SET fixed_layout = ?2 WHERE path = ?1",
        params![path, fixed as i64],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// `(fixed_layout_count, other_count)` among a folder's books in a library —
/// for the folder-add "these look like comics" prompt.
pub fn layout_split(conn: &Connection, folder: &str, library: &str) -> Result<(i64, i64), String> {
    conn.query_row(
        "SELECT
           COALESCE(SUM(fixed_layout), 0),
           COALESCE(SUM(CASE WHEN fixed_layout = 0 THEN 1 ELSE 0 END), 0)
         FROM books WHERE folder = ?1 AND library = ?2",
        params![folder, library],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .map_err(|e| e.to_string())
}

/// Move every fixed-layout book under `folder` (in `from`) to library `to`.
pub fn move_folder_fixed_layout(
    conn: &Connection,
    folder: &str,
    from: &str,
    to: &str,
) -> Result<usize, String> {
    conn.execute(
        "UPDATE books SET library = ?3, library_override = ?3, updated_at = ?4
         WHERE folder = ?1 AND library = ?2 AND fixed_layout = 1",
        params![folder, from, to, now()],
    )
    .map_err(|e| e.to_string())
}

/// Move a book to a different library than its folder's (or `None` to clear the
/// override and follow the folder again). Recorded so rescans keep the choice.
pub fn set_book_library(
    conn: &Connection,
    path: &str,
    library: Option<&str>,
) -> Result<(), String> {
    match library {
        Some(lib) => conn.execute(
            "UPDATE books SET library=?2, library_override=?2, updated_at=?3 WHERE path=?1",
            params![path, lib, now()],
        ),
        None => conn.execute(
            "UPDATE books SET library=(SELECT library FROM folders WHERE ?1 LIKE folder || '%'),
                library_override=NULL, updated_at=?2 WHERE path=?1",
            params![path, now()],
        ),
    }
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------- Favourites / Being Read shelves ----------

/// Flag or unflag a book as a favourite.
pub fn set_favorite(conn: &Connection, path: &str, favorite: bool) -> Result<(), String> {
    conn.execute(
        "UPDATE books SET favorite = ?2 WHERE path = ?1",
        params![path, favorite as i64],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Stamp a book as opened now — puts it at the top of its library's Being Read
/// shelf. Called whenever a reader is opened for the book.
pub fn mark_opened(conn: &Connection, path: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE books SET last_opened = ?2 WHERE path = ?1",
        params![path, now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Remove a book from the Being Read shelf (does not touch reading progress).
pub fn clear_opened(conn: &Connection, path: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE books SET last_opened = NULL WHERE path = ?1",
        params![path],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod recovery_tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "readaity-dbtest-{}-{}.db",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn fresh_open_creates_schema() {
        let p = tmp("fresh");
        let conn = open(&p).unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('folders','books','settings','share_audit')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 4);
        drop(conn);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn corrupt_file_is_rejected() {
        let p = tmp("corrupt");
        std::fs::write(&p, vec![0u8; 8192]).unwrap();
        let err = open(&p).unwrap_err();
        assert!(err.starts_with("CORRUPT:"), "{err}");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn open_resilient_rebuilds_and_quarantines_corrupt_db() {
        let p = tmp("resilient");
        std::fs::write(&p, b"this is not a sqlite database at all").unwrap();

        let (conn, recovered) = open_resilient(&p).unwrap();
        assert!(recovered);
        // fresh DB is usable
        add_folder(&conn, "X:/Books", "tree", "ebooks").unwrap();
        assert_eq!(list_folders(&conn, "ebooks").unwrap().len(), 1);
        drop(conn);

        // the bad file was moved aside, not deleted
        let quarantined = std::fs::read_dir(p.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .any(|e| {
                e.file_name()
                    .to_string_lossy()
                    .contains(&format!("{}.corrupt-", p.file_name().unwrap().to_string_lossy()))
            });
        assert!(quarantined, "corrupt file should be quarantined");

        // cleanup
        let _ = std::fs::remove_file(&p);
        for e in std::fs::read_dir(p.parent().unwrap()).unwrap().flatten() {
            let n = e.file_name();
            if n.to_string_lossy()
                .starts_with(&p.file_name().unwrap().to_string_lossy().to_string())
            {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }

    #[test]
    fn salvage_folders_reads_a_healthy_db() {
        let p = tmp("salvage");
        {
            let conn = open(&p).unwrap();
            add_folder(&conn, "K:/Comics/Farscape", "tree", "comics").unwrap();
            add_folder(&conn, "K:/eBooks/Novels", "flat", "ebooks").unwrap();
        }
        let got = salvage_folders(&p);
        assert_eq!(got.len(), 2);
        assert!(got.iter().any(|f| f.library == "comics" && f.mode == "tree"));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn checkpoint_is_harmless() {
        let p = tmp("ckpt");
        let conn = open(&p).unwrap();
        add_folder(&conn, "A:/x", "tree", "comics").unwrap();
        checkpoint(&conn); // must not panic
        assert_eq!(list_folders(&conn, "comics").unwrap().len(), 1);
        drop(conn);
        let _ = std::fs::remove_file(&p);
    }
}

#[cfg(test)]
mod shelf_tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("readaity-shelftest-{}-{}.db", std::process::id(), name));
        let _ = std::fs::remove_file(&p);
        p
    }

    fn seed_book(conn: &Connection, path: &str, library: &str) {
        let now = now();
        conn.execute(
            "INSERT INTO books(path,folder,format,title,size,mtime,page_count,status,last_page,library,updated_at)
             VALUES(?1,'f','txt',?1,1,?2,0,'ready',0,?3,?2)",
            params![path, now, library],
        )
        .unwrap();
    }

    #[test]
    fn favorite_toggle_and_open_stamp_round_trip() {
        let p = tmp("fav");
        let conn = open(&p).unwrap();
        seed_book(&conn, "a.txt", "ebooks");
        seed_book(&conn, "b.txt", "ebooks");
        seed_book(&conn, "c.cbz", "comics");

        set_favorite(&conn, "a.txt", true).unwrap();
        mark_opened(&conn, "b.txt").unwrap();

        let ebooks = list_books(&conn, "ebooks").unwrap();
        let a = ebooks.iter().find(|x| x.path == "a.txt").unwrap();
        let b = ebooks.iter().find(|x| x.path == "b.txt").unwrap();
        assert!(a.favorite && a.last_opened.is_none());
        assert!(!b.favorite && b.last_opened.is_some());

        // comics library is unaffected by the ebook shelf actions
        let comics = list_books(&conn, "comics").unwrap();
        assert!(!comics[0].favorite && comics[0].last_opened.is_none());

        // un-favourite and clear being-read
        set_favorite(&conn, "a.txt", false).unwrap();
        clear_opened(&conn, "b.txt").unwrap();
        let ebooks = list_books(&conn, "ebooks").unwrap();
        assert!(ebooks.iter().all(|x| !x.favorite && x.last_opened.is_none()));

        drop(conn);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn bookmarks_round_trip_and_orphan_prune() {
        let p = tmp("bm");
        let conn = open(&p).unwrap();
        seed_book(&conn, "a.txt", "ebooks");

        let b1 = add_bookmark(&conn, "a.txt", 120, "Chapter 3").unwrap();
        add_bookmark(&conn, "a.txt", 40, "start").unwrap();
        let list = list_bookmarks(&conn, "a.txt").unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].position, 40, "sorted by position");
        assert_eq!(list[1].label, "Chapter 3");

        remove_bookmark(&conn, b1.id).unwrap();
        assert_eq!(list_bookmarks(&conn, "a.txt").unwrap().len(), 1);

        // Orphan prune drops bookmarks whose book no longer exists.
        conn.execute("DELETE FROM books WHERE path = 'a.txt'", []).unwrap();
        prune_orphan_bookmarks(&conn);
        assert_eq!(list_bookmarks(&conn, "a.txt").unwrap().len(), 0);

        drop(conn);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn all_hashed_and_recheck() {
        let p = tmp("verify");
        let conn = open(&p).unwrap();
        seed_book(&conn, "a.txt", "ebooks");
        seed_book(&conn, "b.cbz", "comics");
        conn.execute("UPDATE books SET md5 = 'deadbeef' WHERE path = 'a.txt'", [])
            .unwrap();

        // Only the hashed, ready book is returned.
        let hashed = all_hashed(&conn).unwrap();
        assert_eq!(hashed.len(), 1);
        assert_eq!(hashed[0].0, "a.txt");
        assert_eq!(hashed[0].2, "deadbeef");
        assert_eq!(hashed[0].3, "ebooks");

        recheck(&conn, &["a.txt".to_string()]).unwrap();
        let pending = pending(&conn).unwrap();
        assert!(pending.iter().any(|(path, _)| path == "a.txt"));

        drop(conn);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn migration_adds_shelf_columns_to_old_books_table() {
        let p = tmp("migrate");
        {
            let conn = Connection::open(&p).unwrap();
            // The pre-shelves books schema (b4 and earlier): has cover/md5 but
            // no favorite/last_opened.
            conn.execute_batch(
                "CREATE TABLE books(
                    path TEXT PRIMARY KEY, folder TEXT NOT NULL, format TEXT NOT NULL,
                    title TEXT NOT NULL, size INTEGER NOT NULL, mtime INTEGER NOT NULL,
                    md5 TEXT, page_count INTEGER NOT NULL DEFAULT 0,
                    cover BLOB, cover_w INTEGER, cover_h INTEGER,
                    status TEXT NOT NULL DEFAULT 'discovered',
                    error TEXT, last_page INTEGER NOT NULL DEFAULT 0,
                    library TEXT NOT NULL DEFAULT 'comics', updated_at INTEGER NOT NULL);
                 CREATE TABLE folders(path TEXT PRIMARY KEY, added_at INTEGER NOT NULL,
                    mode TEXT NOT NULL DEFAULT 'tree', library TEXT NOT NULL DEFAULT 'comics');",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO books(path,folder,format,title,size,mtime,updated_at)
                 VALUES('old.txt','f','txt','Old',1,1,1)",
                [],
            )
            .unwrap();
        }
        // open() must migrate without dropping the row
        let conn = open(&p).unwrap();
        let books = list_books(&conn, "comics").unwrap();
        assert_eq!(books.len(), 1);
        assert!(!books[0].favorite && books[0].last_opened.is_none());
        drop(conn);
        let _ = std::fs::remove_file(&p);
    }
}
