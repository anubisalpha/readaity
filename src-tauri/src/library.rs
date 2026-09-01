//! The two-phase library pipeline.
//!
//! * [`quick_scan`] — Phase 1. Walks folders and upserts a row per comic file
//!   from directory metadata alone (no archive opened). Fast.
//! * [`validate_one`] — Phase 2, per book. Opens the archive, counts pages,
//!   builds a cover thumbnail, and hashes the file. Slow; run in the background.

use std::io::Read;
use std::path::Path;

use md5::{Digest, Md5};
use rusqlite::Connection;
use walkdir::WalkDir;

use crate::comic;
use crate::db;
use crate::ebook;
use crate::formats;
use crate::mobi;

/// Long edge (px) of the cached cover thumbnail.
const COVER_MAX_DIM: u32 = 360;

/// (size_bytes, mtime_secs) for a file, from its directory metadata.
pub fn file_meta(path: &Path) -> Result<(i64, i64), String> {
    let md = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let size = md.len() as i64;
    let mtime = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Ok((size, mtime))
}

/// Streaming MD5 of a file — never loads the whole file into memory at once.
pub fn md5_file(path: &str) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Md5::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Normalise a path for comparison: forward slashes, no trailing slash.
fn norm_path(p: &str) -> String {
    p.replace('\\', "/").trim_end_matches('/').to_string()
}

/// Is `pathN` (normalised) excluded by any exclusion (exact file or subtree)?
fn is_excluded(path_n: &str, exclusions: &[String]) -> bool {
    exclusions
        .iter()
        .any(|e| path_n == e.as_str() || path_n.starts_with(&format!("{e}/")))
}

/// Phase 1: walk every folder and upsert discovered comics, skipping anything
/// the user has removed from the library. Returns the number of rows that need
/// sweeping (new or changed).
pub fn quick_scan(conn: &Connection, library: &str, folders: &[String]) -> Result<usize, String> {
    let exclusions: Vec<String> = db::list_exclusions(conn)?
        .iter()
        .map(|e| norm_path(e))
        .collect();

    let mut needs_sweep = 0;
    for folder in folders {
        // One transaction per folder: thousands of individual auto-committed
        // INSERTs in WAL mode means thousands of fsyncs, which is what makes a
        // multi-thousand-file folder take minutes. Batched, it's seconds.
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        let mut seen: Vec<String> = Vec::new();
        for entry in WalkDir::new(folder)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            let Some(format) = formats::detect(path, library) else {
                continue;
            };
            let Some(path_str) = path.to_str() else { continue };
            if is_excluded(&norm_path(path_str), &exclusions) {
                continue; // removed from library — stay out of scope
            }
            let (size, mtime) = match file_meta(path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let title = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Untitled")
                .to_string();

            seen.push(path_str.to_string());
            let fresh = db::upsert_discovered(
                conn, path_str, folder, &format, &title, size, mtime, library,
            )?;
            if fresh {
                needs_sweep += 1;
                // Cheap record-0 read: flag fixed-layout KF8 now so the
                // "split libraries" prompt can fire before the full sweep.
                if matches!(format.as_str(), "mobi" | "prc" | "azw" | "azw3") {
                    let _ = db::set_fixed_layout(conn, path_str, mobi::meta(path_str).fixed_layout);
                }
            }
        }
        db::prune_missing(conn, folder, library, &seen)?;
        tx.commit().map_err(|e| e.to_string())?;
    }
    Ok(needs_sweep)
}

fn basename_of(p: &str) -> String {
    norm_path(p).rsplit('/').next().unwrap_or("").to_string()
}

/// Drop everything inside (...) and [...] groups (years, scan-group tags, etc).
fn strip_brackets(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

fn is_volume(t: &str) -> bool {
    (t.len() >= 2 && t.starts_with('v') && t[1..].chars().all(|c| c.is_ascii_digit()))
        || t == "vol"
        || (t.len() > 3 && t.starts_with("vol") && t[3..].chars().all(|c| c.is_ascii_digit()))
}

/// Heuristic "series#issue" key from a comic filename stem (+ its folder as a
/// series fallback), for grouping likely duplicates that differ only by
/// scan/edition. Returns None if no issue number can be found. Deliberately
/// fuzzy — a "possible" match, not an exact one.
pub fn name_key_ctx(title: &str, folder: &str) -> Option<String> {
    let cleaned = strip_brackets(&title.to_lowercase());
    let norm: String = cleaned
        .chars()
        .map(|c| match c {
            '-' | '_' | '#' | '.' => ' ',
            other => other,
        })
        .collect();
    let tokens: Vec<String> = norm.split_whitespace().map(|s| s.to_string()).collect();
    if tokens.is_empty() {
        return None;
    }

    let mut issue: Option<u32> = None;
    let mut pos: Option<usize> = None;

    // "N of M" → N is the issue.
    for i in 1..tokens.len() {
        if tokens[i] == "of" {
            if let Ok(n) = tokens[i - 1].parse::<u32>() {
                issue = Some(n);
                pos = Some(i - 1);
                break;
            }
        }
    }
    // "v<k> N" → N is the issue.
    if issue.is_none() {
        for i in 0..tokens.len() {
            if is_volume(&tokens[i]) && i + 1 < tokens.len() {
                if let Ok(n) = tokens[i + 1].parse::<u32>() {
                    issue = Some(n);
                    pos = Some(i + 1);
                    break;
                }
            }
        }
    }
    // Fallback: the last standalone number.
    if issue.is_none() {
        for i in (0..tokens.len()).rev() {
            if let Ok(n) = tokens[i].parse::<u32>() {
                issue = Some(n);
                pos = Some(i);
                break;
            }
        }
    }

    let issue = issue?;
    let p = pos.unwrap_or(tokens.len());
    let series: Vec<&str> = tokens[..p]
        .iter()
        .filter(|t| !is_volume(t) && t.as_str() != "of" && t.parse::<u32>().is_err())
        .map(|s| s.as_str())
        .collect();
    let mut series = series.join(" ").trim().to_string();
    if series.is_empty() {
        // Factor in the folder: use its cleaned name as the series.
        series = strip_brackets(&folder.to_lowercase())
            .chars()
            .map(|c| match c {
                '-' | '_' | '#' | '.' => ' ',
                o => o,
            })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
    }
    if series.is_empty() {
        return None;
    }
    Some(format!("{series}#{issue:04}"))
}

// ---------- smart single-book import ----------

/// Lower-cased word tokens of a title: brackets stripped, punctuation to spaces,
/// very short and common filler words dropped.
fn title_tokens(s: &str) -> std::collections::HashSet<String> {
    const STOP: &[&str] = &[
        "the", "a", "an", "of", "and", "or", "to", "in", "for", "vol", "volume",
        "book", "issue", "part", "illustrated", "edition", "no", "vs",
    ];
    strip_brackets(&s.to_lowercase())
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .filter(|t| t.len() > 2 && !STOP.contains(t) && t.parse::<u64>().is_err())
        .map(|t| t.to_string())
        .collect()
}

/// A candidate destination folder for an imported book, with why it was ranked.
pub struct FolderSuggestion {
    pub folder: String,
    pub score: i32,
    pub reason: String,
}

/// Rank every folder in `library` as a destination for a book titled `title`
/// (format `fmt`), best first. Looks at the folder name and the books already
/// in it (series match, title-word overlap, same format).
pub fn suggest_folders(
    conn: &Connection,
    library: &str,
    title: &str,
    fmt: &str,
) -> Vec<FolderSuggestion> {
    let want = title_tokens(title);
    let want_series = name_key_ctx(title, "").map(|k| k.split('#').next().unwrap_or("").to_string());

    let folders = db::list_folders(conn, library).unwrap_or_default();
    let single = folders.len() == 1;
    let mut out: Vec<FolderSuggestion> = folders
        .into_iter()
        .map(|f| {
            let folder = f.path;
            let mut score = 0i32;
            let mut reason = String::new();

            let fname = basename_of(&folder);
            let fname_tokens = title_tokens(&fname);
            let fn_overlap = want.intersection(&fname_tokens).count();
            if fn_overlap > 0 {
                score += fn_overlap as i32 * 8;
                reason = format!("folder name matches “{fname}”");
            }

            let books = db::books_in_folder(conn, &folder, library).unwrap_or_default();
            let mut best_ov = 0usize;
            let mut best_title = String::new();
            let mut series_hit = false;
            let mut format_hit = false;
            for (bt, bfmt) in &books {
                if bfmt == fmt {
                    format_hit = true;
                }
                if let (Some(a), Some(b)) = (&want_series, name_key_ctx(bt, "")) {
                    if !a.is_empty() && b.starts_with(&format!("{a}#")) {
                        series_hit = true;
                    }
                }
                let ov = want.intersection(&title_tokens(bt)).count();
                if ov > best_ov {
                    best_ov = ov;
                    best_title = bt.clone();
                }
            }
            if series_hit {
                score += 60;
                reason = "same series as books already here".into();
            } else if best_ov >= 2 {
                score += best_ov as i32 * 10;
                if reason.is_empty() {
                    reason = format!("similar to “{best_title}”");
                }
            }
            if format_hit {
                score += 3;
            }
            if single {
                score += 1;
            }
            if reason.is_empty() {
                reason = if books.is_empty() {
                    "empty folder".into()
                } else {
                    format!("{} book{} here", books.len(), if books.len() == 1 { "" } else { "s" })
                };
            }
            FolderSuggestion { folder, score, reason }
        })
        .collect();
    out.sort_by(|a, b| b.score.cmp(&a.score).then(a.folder.cmp(&b.folder)));
    out
}

/// A filesystem-safe `<title>.<ext>` name for an imported book.
pub fn safe_book_name(title: &str, fmt: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c })
        .collect();
    let stem = cleaned.trim().trim_matches('.').trim();
    let stem = if stem.is_empty() { "book" } else { stem };
    let stem: String = stem.chars().take(120).collect();
    format!("{stem}.{fmt}")
}

/// Copy one file into `dest_dir`, giving it a clean `<title>.<ext>` name and
/// renaming on collision. Returns the final path.
pub fn import_one(src: &str, dest_dir: &str) -> Result<String, String> {
    let src_p = std::path::Path::new(src);
    let ext = src_p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let stem = src_p
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("book");
    let name = if ext.is_empty() {
        stem.to_string()
    } else {
        format!("{stem}.{ext}")
    };
    let dest = norm_path(dest_dir);
    std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    let mut target = format!("{dest}/{name}");
    if std::path::Path::new(&target).exists() {
        target = unique_target(&dest, &name);
    }
    std::fs::copy(src, &target).map_err(|e| format!("copy {src}: {e}"))?;
    Ok(target)
}

/// Plan a move of `sources` into `dest_dir`, without touching disk.
/// Returns (src, display-name, collides, error) per source.
pub fn plan_moves(
    sources: &[String],
    dest_dir: &str,
) -> Vec<(String, String, bool, Option<String>)> {
    let dest = norm_path(dest_dir);
    sources
        .iter()
        .map(|s| {
            let sn = norm_path(s);
            let name = basename_of(s);
            let target = format!("{dest}/{name}");
            let error = if sn == target {
                Some("already in this folder".to_string())
            } else if dest == sn || dest.starts_with(&format!("{sn}/")) {
                Some("can't move a folder into itself".to_string())
            } else {
                None
            };
            let collides = error.is_none() && std::path::Path::new(&target).exists();
            (s.clone(), name, collides, error)
        })
        .collect()
}

/// A unique target under `dest` for `name`, inserting " (2)", " (3)"… on clash.
fn unique_target(dest: &str, name: &str) -> String {
    let base = std::path::Path::new(name);
    let stem = base
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name)
        .to_string();
    let ext = base
        .extension()
        .and_then(|s| s.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    let mut i = 2;
    loop {
        let candidate = format!("{dest}/{stem} ({i}){ext}");
        if !std::path::Path::new(&candidate).exists() {
            return candidate;
        }
        i += 1;
    }
}

fn remove_fs(path: &str) -> Result<(), String> {
    let p = std::path::Path::new(path);
    let r = if p.is_dir() {
        std::fs::remove_dir_all(p)
    } else {
        std::fs::remove_file(p)
    };
    r.map_err(|e| format!("remove {path}: {e}"))
}

fn copy_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<(), String> {
    if src.is_dir() {
        std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
        for entry in std::fs::read_dir(src).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            copy_recursive(&entry.path(), &dst.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::copy(src, dst).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn move_fs(src: &str, target: &str) -> Result<(), String> {
    match std::fs::rename(src, target) {
        Ok(_) => Ok(()),
        // Cross-volume rename fails; fall back to copy-then-delete.
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
            copy_recursive(std::path::Path::new(src), std::path::Path::new(target))?;
            remove_fs(src)
        }
        Err(e) => Err(format!("move {src}: {e}")),
    }
}

/// Perform one move on disk. Returns Some((src_norm, target_norm)) when moved,
/// or None when skipped. `action`: "move" | "skip" | "rename" | "replace".
pub fn perform_move(
    src: &str,
    dest_dir: &str,
    action: &str,
) -> Result<Option<(String, String)>, String> {
    if action == "skip" {
        return Ok(None);
    }
    let dest = norm_path(dest_dir);
    let sn = norm_path(src);
    let name = basename_of(src);
    let mut target = format!("{dest}/{name}");

    if std::path::Path::new(&target).exists() {
        match action {
            "replace" => remove_fs(&target)?,
            "rename" => target = unique_target(&dest, &name),
            _ => return Ok(None), // unexpected collision on a plain "move"
        }
    }

    move_fs(&sn, &target)?;
    Ok(Some((sn, target)))
}

/// Fast pre-add probe of a picked folder (no archives opened). Returns
/// (total comics, comics nested in subfolders, immediate subfolders w/ comics).
pub fn probe(path: &str, library: &str) -> (i64, i64, i64) {
    let base = norm_path(path);
    let mut total = 0i64;
    let mut nested = 0i64;
    let mut immediate: std::collections::HashSet<String> = std::collections::HashSet::new();

    for entry in WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        if formats::detect(entry.path(), library).is_none() {
            continue;
        }
        total += 1;
        let dir = entry
            .path()
            .parent()
            .map(|p| norm_path(&p.to_string_lossy()))
            .unwrap_or_default();
        if dir != base {
            nested += 1;
            if let Some(rest) = dir.strip_prefix(&format!("{base}/")) {
                if let Some(seg) = rest.split('/').next() {
                    immediate.insert(seg.to_string());
                }
            }
        }
    }
    (total, nested, immediate.len() as i64)
}

/// The product of validating one book.
pub struct Validated {
    pub page_count: i64,
    pub md5: String,
    pub cover: Vec<u8>,
    pub cover_w: i64,
    pub cover_h: i64,
    /// Fixed-layout KF8 (comic / picture book) — read via the page-image pager.
    pub fixed_layout: bool,
}

/// Phase 2 for a single book: hash it, count pages, build a cover if we can.
/// Branches by format (comic archive vs ebook). Any hard failure → invalid.
pub fn validate_one(path: &str, format: &str) -> Result<Validated, String> {
    let md5 = md5_file(path)?;

    if formats::is_comic(format) {
        let page_count = comic::page_count(path)? as i64;
        if page_count == 0 {
            return Err("archive contains no image pages".into());
        }
        let (cover, cover_w, cover_h) = comic::cover_thumbnail(path, COVER_MAX_DIM)?;
        return Ok(Validated {
            page_count,
            md5,
            cover,
            cover_w: cover_w as i64,
            cover_h: cover_h as i64,
            fixed_layout: false,
        });
    }

    // Ebook: page count is best-effort; cover may be absent (PDF covers are
    // produced later in the frontend). An empty cover is stored as NULL.
    let page_count = ebook::page_count(path, format)?;
    let (cover, cover_w, cover_h) = match ebook::cover(path, format, COVER_MAX_DIM)? {
        Some((c, w, h)) => (c, w as i64, h as i64),
        None => (Vec::new(), 0, 0),
    };

    // Fixed-layout KF8 (comic / manga / picture book): EXTH 122 says so. This is
    // a cheap record-0 read — the actual page images are only extracted later,
    // once, when the pager opens the book. Page count stays 0 until then.
    let fixed_layout = matches!(format, "mobi" | "prc" | "azw" | "azw3")
        && mobi::meta(path).fixed_layout;

    Ok(Validated {
        page_count,
        md5,
        cover,
        cover_w,
        cover_h,
        fixed_layout,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    /// A small but genuinely-decodable PNG, so `cover_thumbnail` can decode it.
    fn png_bytes() -> Vec<u8> {
        let mut img = image::RgbImage::new(40, 60);
        for p in img.pixels_mut() {
            *p = image::Rgb([90, 120, 160]);
        }
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    /// Full pipeline against a real generated CBZ: quick_scan → validate → DB.
    #[test]
    fn two_phase_pipeline_end_to_end() {
        use std::io::Write;
        use zip::write::SimpleFileOptions;

        // Isolated temp workspace.
        let base = std::env::temp_dir().join(format!("readaity_lib_test_{}", std::process::id()));
        let lib_dir = base.join("comics");
        std::fs::create_dir_all(&lib_dir).unwrap();

        // Write a valid 2-page CBZ into the library folder.
        let cbz = lib_dir.join("Test Comic.cbz");
        {
            let file = std::fs::File::create(&cbz).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = SimpleFileOptions::default();
            for name in ["001.png", "002.png"] {
                zip.start_file(name, opts).unwrap();
                zip.write_all(&png_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }

        let conn = db::open(&base.join("library.db")).unwrap();
        let folder = lib_dir.to_str().unwrap().to_string();
        db::add_folder(&conn, &folder, "tree", "comics").unwrap();

        // Phase 1: one new book needs sweeping.
        let needs = quick_scan(&conn, "comics", &[folder.clone()]).unwrap();
        assert_eq!(needs, 1);
        let pending = db::pending(&conn).unwrap();
        assert_eq!(pending.len(), 1);
        let (path, format) = &pending[0];
        assert_eq!(format, "cbz");

        // A book still awaiting validation is correctly re-queued on rescan.
        assert_eq!(quick_scan(&conn, "comics", &[folder.clone()]).unwrap(), 1);

        // Phase 2: validate and store.
        let v = validate_one(path, "cbz").unwrap();
        assert_eq!(v.page_count, 2);
        assert!(!v.cover.is_empty(), "cover thumbnail should be produced");
        assert_eq!(v.md5.len(), 32, "md5 hex should be 32 chars");
        db::set_validated(
            &conn, path, v.page_count, &v.md5, &v.cover, v.cover_w, v.cover_h, v.fixed_layout,
        )
        .unwrap();

        // Now that it's ready and unchanged, a rescan is a cache hit (0 to sweep).
        assert_eq!(quick_scan(&conn, "comics", &[folder.clone()]).unwrap(), 0);

        // The book is now ready, with a cover cached in the DB.
        let books = db::list_books(&conn, "comics").unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].status, "ready");
        assert_eq!(books[0].page_count, 2);
        assert!(books[0].has_cover);
        assert!(db::get_cover(&conn, path).unwrap().is_some());

        // Progress round-trips.
        db::set_progress(&conn, path, 1).unwrap();
        assert_eq!(db::list_books(&conn, "comics").unwrap()[0].last_page, 1);

        // Removing the file then rescanning prunes the row.
        std::fs::remove_file(&cbz).unwrap();
        quick_scan(&conn, "comics", &[folder.clone()]).unwrap();
        assert_eq!(db::list_books(&conn, "comics").unwrap().len(), 0);

        std::fs::remove_dir_all(&base).ok();
    }

    /// probe() counts nested comics, and removed items stay out of rescans.
    #[test]
    fn probe_and_exclusions() {
        use std::io::Write;
        use zip::write::SimpleFileOptions;

        let base = std::env::temp_dir().join(format!("readaity_excl_test_{}", std::process::id()));
        let root = base.join("lib");
        let sub = root.join("Series A");
        std::fs::create_dir_all(&sub).unwrap();

        // A fake .cbz (discovery only checks extension, never opens it).
        let write_cbz = |p: &std::path::Path| {
            let f = std::fs::File::create(p).unwrap();
            let mut zip = zip::ZipWriter::new(f);
            zip.start_file("001.jpg", SimpleFileOptions::default()).unwrap();
            zip.write_all(b"x").unwrap();
            zip.finish().unwrap();
        };
        let top = root.join("Top.cbz");
        let nested = sub.join("Nested.cbz");
        write_cbz(&top);
        write_cbz(&nested);

        // probe sees 2 comics, 1 nested, 1 subfolder.
        let (total, nested_count, subs) = probe(root.to_str().unwrap(), "comics");
        assert_eq!((total, nested_count, subs), (2, 1, 1));

        let conn = db::open(&base.join("library.db")).unwrap();
        let root_s = root.to_str().unwrap().to_string();
        db::add_folder(&conn, &root_s, "tree", "comics").unwrap();
        quick_scan(&conn, "comics", &[root_s.clone()]).unwrap();
        assert_eq!(db::list_books(&conn, "comics").unwrap().len(), 2);

        // Remove the top book → excluded, not re-added on rescan.
        db::remove_book(&conn, top.to_str().unwrap()).unwrap();
        assert_eq!(db::list_books(&conn, "comics").unwrap().len(), 1);
        quick_scan(&conn, "comics", &[root_s.clone()]).unwrap();
        assert_eq!(db::list_books(&conn, "comics").unwrap().len(), 1);

        // Remove the subfolder subtree → excluded, not re-added.
        db::remove_subtree(&conn, sub.to_str().unwrap()).unwrap();
        assert_eq!(db::list_books(&conn, "comics").unwrap().len(), 0);
        quick_scan(&conn, "comics", &[root_s]).unwrap();
        assert_eq!(db::list_books(&conn, "comics").unwrap().len(), 0);

        // Files still exist on disk.
        assert!(top.exists() && nested.exists());

        std::fs::remove_dir_all(&base).ok();
    }

    /// Moving a book into a subfolder relocates it on disk and in the DB,
    /// and a name collision resolves with rename.
    #[test]
    fn move_and_relocate() {
        use std::io::Write;
        use zip::write::SimpleFileOptions;

        let base = std::env::temp_dir().join(format!("readaity_move_test_{}", std::process::id()));
        let lib = base.join("lib");
        let sub = lib.join("Sub");
        std::fs::create_dir_all(&sub).unwrap();

        let write_cbz = |p: &std::path::Path| {
            let f = std::fs::File::create(p).unwrap();
            let mut zip = zip::ZipWriter::new(f);
            zip.start_file("001.jpg", SimpleFileOptions::default()).unwrap();
            zip.write_all(b"x").unwrap();
            zip.finish().unwrap();
        };
        let a = lib.join("A.cbz");
        write_cbz(&a);

        let conn = db::open(&base.join("library.db")).unwrap();
        let lib_s = lib.to_str().unwrap().to_string();
        db::add_folder(&conn, &lib_s, "tree", "comics").unwrap();
        quick_scan(&conn, "comics", &[lib_s]).unwrap();
        assert_eq!(db::list_books(&conn, "comics").unwrap().len(), 1);

        // Plan: no collision moving A.cbz into Sub.
        let plans = plan_moves(&[a.to_str().unwrap().to_string()], sub.to_str().unwrap());
        assert!(!plans[0].2 && plans[0].3.is_none());

        // Perform + relocate.
        let (sn, target) = perform_move(a.to_str().unwrap(), sub.to_str().unwrap(), "move")
            .unwrap()
            .unwrap();
        db::relocate(&conn, &sn, &target).unwrap();
        assert!(!a.exists());
        assert!(sub.join("A.cbz").exists());
        let books = db::list_books(&conn, "comics").unwrap();
        assert_eq!(books[0].path.replace('\\', "/"), target);

        // Collision: a fresh A.cbz moved into Sub (which now has one) → rename.
        write_cbz(&a);
        let plans2 = plan_moves(&[a.to_str().unwrap().to_string()], sub.to_str().unwrap());
        assert!(plans2[0].2, "should collide");
        let (_s2, target2) =
            perform_move(a.to_str().unwrap(), sub.to_str().unwrap(), "rename")
                .unwrap()
                .unwrap();
        assert!(target2.ends_with("A (2).cbz"));
        assert!(sub.join("A (2).cbz").exists());

        // Moving a folder into itself is rejected at plan time.
        let self_plan = plan_moves(&[sub.to_str().unwrap().to_string()], sub.to_str().unwrap());
        assert!(self_plan[0].3.is_some());

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn suggest_folders_ranks_by_series_and_name() {
        let base = std::env::temp_dir().join(format!("readaity_sugg_{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let conn = db::open(&base.join("l.db")).unwrap();

        let dune = base.join("Dune").to_string_lossy().into_owned();
        let random = base.join("Random").to_string_lossy().into_owned();
        db::add_folder(&conn, &dune, "tree", "ebooks").unwrap();
        db::add_folder(&conn, &random, "tree", "ebooks").unwrap();
        // A Dune book already lives in the Dune folder.
        conn.execute(
            "INSERT INTO books(path,folder,format,title,size,mtime,page_count,status,last_page,library,updated_at)
             VALUES('a','{d}','epub','Dune Messiah',1,0,0,'ready',0,'ebooks',0)"
                .replace("{d}", &dune)
                .as_str(),
            [],
        )
        .unwrap();

        let s = suggest_folders(&conn, "ebooks", "Dune - Children of Dune", "epub");
        assert_eq!(basename_of(&s[0].folder), "Dune", "series match wins");
        assert!(s[0].score > s[1].score);

        // A title matching neither falls back to a neutral, still-listed folder.
        let s2 = suggest_folders(&conn, "ebooks", "The Hobbit", "epub");
        assert_eq!(s2.len(), 2, "every folder is offered");

        std::fs::remove_dir_all(&base).ok();
    }

    /// Timing check: a folder with several thousand files must scan quickly
    /// (batched into one transaction). Run with `cargo test large_folder -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn large_folder_quick_scan_is_fast() {
        let base = std::env::temp_dir().join(format!("readaity_big_{}", std::process::id()));
        let lib = base.join("comics");
        std::fs::create_dir_all(&lib).unwrap();

        let n = 4000;
        for i in 0..n {
            std::fs::write(lib.join(format!("Book {i:05}.cbz")), b"not a real zip").unwrap();
        }

        let conn = db::open(&base.join("library.db")).unwrap();
        let folder = lib.to_str().unwrap().to_string();
        db::add_folder(&conn, &folder, "tree", "comics").unwrap();

        let t = std::time::Instant::now();
        let needs = quick_scan(&conn, "comics", &[folder.clone()]).unwrap();
        let elapsed = t.elapsed();
        eprintln!("quick_scan of {n} files: {elapsed:?} ({needs} to sweep)");

        assert_eq!(needs, n);
        assert_eq!(db::list_books(&conn, "comics").unwrap().len(), n);
        assert!(elapsed.as_secs() < 10, "scan took too long: {elapsed:?}");

        // Re-scan cost (these fake files never validate, so they re-queue —
        // we're only timing the walk + upsert path here).
        let t2 = std::time::Instant::now();
        quick_scan(&conn, "comics", &[folder]).unwrap();
        eprintln!("rescan: {:?}", t2.elapsed());

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn name_key_groups_different_scans() {
        // Two different scans of Farscape #1 collapse to the same key.
        assert_eq!(
            name_key_ctx("(01)Farscape - 001 of 004 (2008)", "").as_deref(),
            Some("farscape#0001"),
        );
        assert_eq!(
            name_key_ctx("Farscape v1 001 (2008) (Digital-SD) (Kileko-Empire)", "").as_deref(),
            Some("farscape#0001"),
        );
        // A different sub-series keeps a distinct key.
        assert_eq!(
            name_key_ctx("Farscape - Strange Detractors 01 (2009)", "").as_deref(),
            Some("farscape strange detractors#0001"),
        );
        // Bare filename with no series → folder name is used as the series.
        assert_eq!(
            name_key_ctx("01", "Batman (2016)").as_deref(),
            Some("batman#0001"),
        );
        // No parseable issue number and no folder → no key.
        assert!(name_key_ctx("Some Cover Special", "").is_none());
    }
}
