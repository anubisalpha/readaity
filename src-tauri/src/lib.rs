mod comic;
mod db;
mod ebook;
mod formats;
mod library;
mod mobi;
mod rtf;
mod share;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use base64::Engine as _;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::DialogExt;

use comic::PageData;
use db::{AppDb, BookRow, FolderRow};

/// Guards against two background sweeps running at once.
struct Sweeping(AtomicBool);

/// When true, the background sweep stops after the current book (resumable).
struct Paused(AtomicBool);

/// Global work status pushed to the UI's progress bar.
#[derive(Clone, serde::Serialize)]
struct ScanStatus {
    /// "idle" | "scanning" (Phase 1) | "indexing" (Phase 2)
    phase: String,
    current: i64,
    total: i64,
}

fn emit_status(app: &AppHandle, phase: &str, current: i64, total: i64) {
    let _ = app.emit(
        "scan-status",
        ScanStatus {
            phase: phase.to_string(),
            current,
            total,
        },
    );
}

fn db_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join("library.db"))
}

/// Open a native folder picker. Returns the chosen path, or `None` if cancelled.
#[tauri::command]
async fn pick_folder(app: AppHandle) -> Option<String> {
    app.dialog()
        .file()
        .blocking_pick_folder()
        .and_then(|p| p.into_path().ok())
        .and_then(|p| p.to_str().map(|s| s.to_string()))
}

/// Phase 1 for one new folder, then kick off the background validity sweep.
/// Returns the current book list immediately (discovered rows show as placeholders).
#[tauri::command]
fn add_folder(
    app: AppHandle,
    path: String,
    mode: String,
    library: String,
) -> Result<Vec<BookRow>, String> {
    emit_status(&app, "scanning", 0, 0);
    {
        let db = app.state::<AppDb>();
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        db::add_folder(&conn, &path, &mode, &library)?;
        library::quick_scan(&conn, &library, &[path])?;
    }
    app.state::<Paused>().0.store(false, Ordering::SeqCst); // adding = index now
    start_sweep(app.clone());
    list_books(app, library)
}

#[tauri::command]
fn remove_folder(app: AppHandle, path: String, library: String) -> Result<Vec<BookRow>, String> {
    {
        let db = app.state::<AppDb>();
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        db::remove_folder(&conn, &path)?;
    }
    list_books(app, library)
}

/// Remove one book from the library (kept on disk, excluded from rescans).
#[tauri::command]
fn remove_book(app: AppHandle, path: String, library: String) -> Result<Vec<BookRow>, String> {
    {
        let db = app.state::<AppDb>();
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        db::remove_book(&conn, &path)?;
    }
    list_books(app, library)
}

/// Remove a subfolder's books from the library (kept on disk, subtree excluded).
#[tauri::command]
fn remove_path(app: AppHandle, path: String, library: String) -> Result<Vec<BookRow>, String> {
    {
        let db = app.state::<AppDb>();
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        db::remove_subtree(&conn, &path)?;
    }
    list_books(app, library)
}

/// Read a persisted app preference by key (`None` if never set).
#[tauri::command]
fn get_setting(app: AppHandle, key: String) -> Result<Option<String>, String> {
    let db = app.state::<AppDb>();
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    db::get_setting(&conn, &key)
}

/// Persist an app preference.
#[tauri::command]
fn set_setting(app: AppHandle, key: String, value: String) -> Result<(), String> {
    let db = app.state::<AppDb>();
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    db::set_setting(&conn, &key, &value)
}

// ---------- Network sharing (b4) ----------

#[tauri::command]
fn share_get_config(app: AppHandle) -> share::ShareConfig {
    share::load_config(&app)
}

#[tauri::command]
fn share_set_config(
    app: AppHandle,
    port: u16,
    name: String,
    allowlist: String,
    audit: bool,
) -> Result<share::ShareConfig, String> {
    share::save_config(&app, port, &name, &allowlist, audit)?;
    Ok(share::load_config(&app))
}

#[tauri::command]
fn share_set_pin(app: AppHandle, pin: String) -> Result<(), String> {
    share::set_pin(&app, &pin)
}

#[tauri::command]
fn share_generate_pin(app: AppHandle) -> Result<String, String> {
    share::generate_pin(&app)
}

#[tauri::command]
fn share_start(app: AppHandle) -> Result<share::ShareStatus, String> {
    share::start(&app)
}

#[tauri::command]
fn share_stop(app: AppHandle) -> Result<(), String> {
    share::stop(&app)
}

#[tauri::command]
fn share_status(app: AppHandle) -> share::ShareStatus {
    share::status(&app)
}

#[tauri::command]
fn share_regenerate_cert(app: AppHandle) -> Result<String, String> {
    let was_running = share::status(&app).running;
    if was_running {
        share::stop(&app)?;
    }
    let fp = share::regenerate_cert(&app)?;
    if was_running {
        share::start(&app)?;
    }
    Ok(fp)
}

#[tauri::command]
fn share_audit_log(app: AppHandle, limit: i64) -> Result<Vec<db::AuditRow>, String> {
    let db = app.state::<AppDb>();
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    db::list_audit(&conn, limit.clamp(1, 2000))
}

#[tauri::command]
fn share_clear_audit(app: AppHandle) -> Result<(), String> {
    let db = app.state::<AppDb>();
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    db::clear_audit(&conn)
}

#[tauri::command]
fn list_folders(app: AppHandle, library: String) -> Result<Vec<FolderRow>, String> {
    let db = app.state::<AppDb>();
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    db::list_folders(&conn, &library)
}

/// The paths currently excluded from the library (removed but kept on disk).
#[tauri::command]
fn list_exclusions(app: AppHandle) -> Result<Vec<String>, String> {
    let db = app.state::<AppDb>();
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    db::list_exclusions(&conn)
}

/// Rescan every folder (each with its own library) — used after un-excluding.
fn rescan_all(conn: &rusqlite::Connection) -> Result<(), String> {
    for f in db::all_folders(conn)? {
        library::quick_scan(conn, &f.library, &[f.path])?;
    }
    Ok(())
}

/// Restore one excluded path: drop the exclusion, rescan, and re-index it.
#[tauri::command]
fn restore_exclusion(
    app: AppHandle,
    path: String,
    library: String,
) -> Result<Vec<BookRow>, String> {
    {
        let db = app.state::<AppDb>();
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        db::remove_exclusion(&conn, &path)?;
        rescan_all(&conn)?;
    }
    start_sweep(app.clone());
    list_books(app, library)
}

/// Restore everything: clear all exclusions and rescan.
#[tauri::command]
fn clear_exclusions(app: AppHandle, library: String) -> Result<Vec<BookRow>, String> {
    {
        let db = app.state::<AppDb>();
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        db::clear_exclusions(&conn)?;
        rescan_all(&conn)?;
    }
    start_sweep(app.clone());
    list_books(app, library)
}

/// One planned move: whether it collides with an existing item, or is invalid.
#[derive(serde::Serialize)]
struct MovePlan {
    src: String,
    name: String,
    collides: bool,
    error: Option<String>,
}

/// Plan a drag-move without touching disk (collision + validity check).
#[tauri::command]
fn plan_move(sources: Vec<String>, dest_dir: String) -> Vec<MovePlan> {
    library::plan_moves(&sources, &dest_dir)
        .into_iter()
        .map(|(src, name, collides, error)| MovePlan {
            src,
            name,
            collides,
            error,
        })
        .collect()
}

#[derive(serde::Deserialize)]
struct MoveOp {
    src: String,
    /// "move" | "skip" | "rename" | "replace"
    action: String,
}

/// Physically move items into `dest_dir` and rewrite their DB paths.
#[tauri::command]
fn move_items(
    app: AppHandle,
    dest_dir: String,
    ops: Vec<MoveOp>,
    library: String,
) -> Result<Vec<BookRow>, String> {
    // Do filesystem moves outside the DB lock (a cross-volume copy can be slow).
    let mut relocations: Vec<(String, String)> = Vec::new();
    for op in &ops {
        if let Some(pair) = library::perform_move(&op.src, &dest_dir, &op.action)? {
            relocations.push(pair);
        }
    }
    {
        let db = app.state::<AppDb>();
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        for (src_norm, target_norm) in &relocations {
            db::relocate(&conn, src_norm, target_norm)?;
        }
    }
    list_books(app, library)
}

/// Fast pre-add probe: how many comics, how many nested, how many subfolders.
#[derive(serde::Serialize)]
struct ProbeResult {
    total: i64,
    nested: i64,
    subfolders: i64,
}

#[tauri::command]
async fn probe_folder(path: String, library: String) -> ProbeResult {
    let (total, nested, subfolders) =
        tauri::async_runtime::spawn_blocking(move || library::probe(&path, &library))
            .await
            .unwrap_or((0, 0, 0));
    ProbeResult {
        total,
        nested,
        subfolders,
    }
}

#[tauri::command]
fn list_books(app: AppHandle, library: String) -> Result<Vec<BookRow>, String> {
    let db = app.state::<AppDb>();
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    db::list_books(&conn, &library)
}

/// Ready-book counts per library, for the idle status bar.
#[derive(serde::Serialize)]
struct Counts {
    comics: i64,
    ebooks: i64,
}

#[tauri::command]
fn library_counts(app: AppHandle) -> Result<Counts, String> {
    let db = app.state::<AppDb>();
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    Ok(Counts {
        comics: db::ready_count(&conn, "comics")?,
        ebooks: db::ready_count(&conn, "ebooks")?,
    })
}

/// Re-run Phase 1 across all folders (picks up added/removed/changed files),
/// then sweep. Returns the refreshed list.
#[tauri::command]
fn rescan(app: AppHandle, library: String) -> Result<Vec<BookRow>, String> {
    emit_status(&app, "scanning", 0, 0);
    {
        let db = app.state::<AppDb>();
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let paths: Vec<String> = db::list_folders(&conn, &library)?
            .into_iter()
            .map(|f| f.path)
            .collect();
        library::quick_scan(&conn, &library, &paths)?;
    }
    app.state::<Paused>().0.store(false, Ordering::SeqCst);
    start_sweep(app.clone());
    list_books(app, library)
}

/// Re-index a library: reset its books and re-run the validation sweep
/// (re-extracts covers, page counts and hashes with the current code).
#[tauri::command]
fn reindex(app: AppHandle, library: String) -> Result<Vec<BookRow>, String> {
    {
        let db = app.state::<AppDb>();
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        // Re-walk folders first, so newly-supported formats get discovered…
        let paths: Vec<String> = db::list_folders(&conn, &library)?
            .into_iter()
            .map(|f| f.path)
            .collect();
        library::quick_scan(&conn, &library, &paths)?;
        // …then reset everything so covers/pages/hashes are rebuilt.
        db::reindex(&conn, &library)?;
    }
    app.state::<Paused>().0.store(false, Ordering::SeqCst);
    start_sweep(app.clone());
    list_books(app, library)
}

/// Groups of byte-identical books (same content hash) for the duplicates view.
#[tauri::command]
fn list_duplicates(app: AppHandle) -> Result<Vec<db::DupGroup>, String> {
    let db = app.state::<AppDb>();
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    db::list_duplicates(&conn)
}

/// Groups of books that look like the same issue by filename (fuzzy).
#[tauri::command]
fn list_name_duplicates(app: AppHandle) -> Result<Vec<db::DupGroup>, String> {
    let db = app.state::<AppDb>();
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    db::list_name_duplicates(&conn)
}

/// Hide a possible-duplicate group so it stops appearing.
#[tauri::command]
fn ignore_dupe(app: AppHandle, key: String) -> Result<(), String> {
    let db = app.state::<AppDb>();
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    db::ignore_dupe(&conn, &key)
}

/// Un-hide a previously ignored possible-duplicate group.
#[tauri::command]
fn unignore_dupe(app: AppHandle, key: String) -> Result<(), String> {
    let db = app.state::<AppDb>();
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    db::unignore_dupe(&conn, &key)
}

/// Keys currently hidden from the possible-duplicates view.
#[tauri::command]
fn list_ignored_dupes(app: AppHandle) -> Result<Vec<String>, String> {
    let db = app.state::<AppDb>();
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    db::list_ignored_dupes(&conn)
}

/// The cached cover thumbnail as base64 JPEG, or `None` if not swept yet.
#[tauri::command]
fn get_cover(app: AppHandle, path: String) -> Result<Option<String>, String> {
    let db = app.state::<AppDb>();
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    Ok(db::get_cover(&conn, &path)?
        .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// Store a cover thumbnail generated in the frontend (e.g. a PDF page via
/// pdf.js). `data` is base64 JPEG. Only applied if the book has no cover yet.
#[tauri::command]
fn set_cover(
    app: AppHandle,
    path: String,
    data: String,
    width: i64,
    height: i64,
) -> Result<(), String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|e| e.to_string())?;
    let db = app.state::<AppDb>();
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    db::set_cover(&conn, &path, &bytes, width, height)
}

/// Decode a text file, honouring a UTF-8 / UTF-16 BOM, else UTF-8 lossy.
fn decode_text(b: &[u8]) -> String {
    if b.starts_with(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(&b[3..]).into_owned()
    } else if b.starts_with(&[0xFF, 0xFE]) {
        let u: Vec<u16> = b[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&u)
    } else if b.starts_with(&[0xFE, 0xFF]) {
        let u: Vec<u16> = b[2..]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&u)
    } else {
        String::from_utf8_lossy(b).into_owned()
    }
}

/// Convert an RTF book to HTML for the reader.
#[tauri::command]
async fn get_rtf_html(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || rtf::to_html(&path))
        .await
        .map_err(|e| e.to_string())?
}

/// Read a plain-text book's content as a decoded string.
#[tauri::command]
async fn get_text_content(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::read(&path)
            .map(|b| decode_text(&b))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Extract a MOBI/AZW book's HTML content (decompressed, images inlined).
#[tauri::command]
async fn get_mobi_html(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || mobi::content(&path))
        .await
        .map_err(|e| e.to_string())?
}

/// Read an entire book file as base64 (for the ebook readers: epub.js / pdf.js).
#[tauri::command]
async fn read_book_bytes(path: String) -> Result<String, String> {
    let bytes = tauri::async_runtime::spawn_blocking(move || std::fs::read(&path))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// Fetch a single page image (base64) straight from the archive (for the reader).
#[tauri::command]
async fn get_page(path: String, index: usize) -> Result<PageData, String> {
    tauri::async_runtime::spawn_blocking(move || comic::get_page(&path, index))
        .await
        .map_err(|e| e.to_string())?
}

/// Pause the background indexing sweep (resumable; nothing is lost).
/// Emits the paused status immediately so the UI reacts even if a large book
/// is still being hashed when the button is clicked.
#[tauri::command]
fn pause_indexing(app: AppHandle) -> Result<(), String> {
    app.state::<Paused>().0.store(true, Ordering::SeqCst);
    let (done, total) = {
        let db = app.state::<AppDb>();
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let books = db::list_all_books(&conn)?;
        let discovered = books.iter().filter(|b| b.status == "discovered").count() as i64;
        (books.len() as i64 - discovered, books.len() as i64)
    };
    emit_status(&app, "paused", done, total);
    Ok(())
}

/// Resume indexing from where it left off.
#[tauri::command]
fn resume_indexing(app: AppHandle) {
    app.state::<Paused>().0.store(false, Ordering::SeqCst);
    start_sweep(app);
}

#[tauri::command]
fn set_progress(app: AppHandle, path: String, page: i64) -> Result<(), String> {
    let db = app.state::<AppDb>();
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    db::set_progress(&conn, &path, page)
}

/// Start the background Phase-2 sweep if one isn't already running.
/// Each validated book emits a `book-updated` event so the shelf fills in live.
fn start_sweep(app: AppHandle) {
    let already = app.state::<Sweeping>().0.swap(true, Ordering::SeqCst);
    if already {
        return; // a sweep is already draining the queue
    }

    tauri::async_runtime::spawn(async move {
        let mut done: i64 = 0;
        loop {
            // Pull the next pending book + how many remain (short lock).
            let (next, remaining) = {
                let db = app.state::<AppDb>();
                let conn = db.0.lock().unwrap();
                let pending = db::pending(&conn).unwrap_or_default();
                let remaining = pending.len() as i64;
                (pending.into_iter().next(), remaining)
            };

            // Honour a pause request: stop here, leaving the rest 'discovered'.
            if app.state::<Paused>().0.load(Ordering::SeqCst) {
                app.state::<Sweeping>().0.store(false, Ordering::SeqCst);
                emit_status(&app, "paused", done, done + remaining);
                return;
            }

            let Some((path, fmt)) = next else { break };

            // total adapts if books are added mid-sweep: completed + still pending.
            emit_status(&app, "indexing", done, done + remaining);

            // Heavy work (open archive, thumbnail, hash) off the async thread.
            let result = tauri::async_runtime::spawn_blocking({
                let p = path.clone();
                let f = fmt.clone();
                move || library::validate_one(&p, &f)
            })
            .await;

            // Write the outcome and emit the updated row (short lock).
            let row = {
                let db = app.state::<AppDb>();
                let conn = db.0.lock().unwrap();
                match result {
                    Ok(Ok(v)) => {
                        let _ = db::set_validated(
                            &conn, &path, v.page_count, &v.md5, &v.cover, v.cover_w, v.cover_h,
                        );
                    }
                    Ok(Err(e)) => {
                        let _ = db::set_invalid(&conn, &path, &e);
                    }
                    Err(join_err) => {
                        let _ = db::set_invalid(&conn, &path, &join_err.to_string());
                    }
                }
                db::list_all_books(&conn)
                    .ok()
                    .and_then(|books| books.into_iter().find(|b| b.path == path))
            };
            done += 1;
            if let Some(row) = row {
                let _ = app.emit("book-updated", row);
            }
        }

        app.state::<Sweeping>().0.store(false, Ordering::SeqCst);
        emit_status(&app, "idle", done, done);
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let path = db_path(&app.handle()).map_err(|e| e.to_string())?;
            let conn = db::open(&path).map_err(|e| e.to_string())?;
            app.manage(AppDb(Mutex::new(conn)));
            app.manage(Sweeping(AtomicBool::new(false)));
            app.manage(Paused(AtomicBool::new(false)));
            app.manage(share::ShareState::default());
            // Catch up on any books left pending from a previous session.
            start_sweep(app.handle().clone());
            // Re-arm the LAN share server if the user had it on.
            share::autostart(&app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            pick_folder,
            add_folder,
            remove_folder,
            remove_book,
            remove_path,
            list_folders,
            get_setting,
            set_setting,
            share_get_config,
            share_set_config,
            share_set_pin,
            share_generate_pin,
            share_start,
            share_stop,
            share_status,
            share_regenerate_cert,
            share_audit_log,
            share_clear_audit,
            library_counts,
            probe_folder,
            plan_move,
            move_items,
            list_books,
            list_exclusions,
            restore_exclusion,
            clear_exclusions,
            list_duplicates,
            list_name_duplicates,
            ignore_dupe,
            unignore_dupe,
            list_ignored_dupes,
            rescan,
            reindex,
            get_cover,
            set_cover,
            read_book_bytes,
            get_mobi_html,
            get_rtf_html,
            get_text_content,
            get_page,
            set_progress,
            pause_indexing,
            resume_indexing
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
