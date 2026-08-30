//! Reading comic archives — CBZ (zip-of-images) and CBR (rar-of-images).
//!
//! Both formats expose the identical `BookInfo` / page-fetch surface, so the
//! frontend never learns which archive type it's reading. New formats hang off
//! the same dispatch in `page_count` / `get_page` / `book_info`.

use std::cmp::Ordering;
use std::io::Read;
use std::path::Path;

use base64::Engine as _;
use serde::Serialize;
use zip::ZipArchive;

/// Image extensions we treat as comic pages, lower-cased.
const PAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "bmp"];

/// A single rendered page, ready to drop straight into an `<img src>`.
#[derive(Serialize, Clone)]
pub struct PageData {
    /// e.g. `image/jpeg`.
    pub mime: String,
    /// Base64 payload (no data-URL prefix; the frontend adds it).
    pub base64: String,
}

fn is_page_entry(name: &str) -> bool {
    // Skip directory entries and macOS resource-fork junk.
    if name.ends_with('/') || name.contains("__MACOSX") {
        return false;
    }
    match Path::new(name).extension().and_then(|e| e.to_str()) {
        Some(ext) => PAGE_EXTS.contains(&ext.to_ascii_lowercase().as_str()),
        None => false,
    }
}

fn mime_for(name: &str) -> &'static str {
    match Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        // jpg/jpeg and anything else we accepted defaults to jpeg.
        _ => "image/jpeg",
    }
}

/// Human/natural comparison so `page2.jpg` sorts before `page10.jpg`
/// even when the numbers aren't zero-padded.
fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) => {
                if ca.is_ascii_digit() && cb.is_ascii_digit() {
                    // Compare whole digit runs by numeric value.
                    let na: String = collect_digits(&mut ai);
                    let nb: String = collect_digits(&mut bi);
                    let va = na.trim_start_matches('0');
                    let vb = nb.trim_start_matches('0');
                    let ord = va.len().cmp(&vb.len()).then_with(|| va.cmp(vb));
                    if ord != Ordering::Equal {
                        return ord;
                    }
                    // Equal numeric value: shorter (fewer leading zeros) first.
                    let ord = na.len().cmp(&nb.len());
                    if ord != Ordering::Equal {
                        return ord;
                    }
                } else {
                    let ord = ca
                        .to_ascii_lowercase()
                        .cmp(&cb.to_ascii_lowercase());
                    ai.next();
                    bi.next();
                    if ord != Ordering::Equal {
                        return ord;
                    }
                }
            }
        }
    }
}

fn collect_digits(it: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut s = String::new();
    while let Some(c) = it.peek().copied() {
        if c.is_ascii_digit() {
            s.push(c);
            it.next();
        } else {
            break;
        }
    }
    s
}

fn encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn ext_of(path: &str) -> String {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
}

// ---------- CBZ (zip) ----------

fn cbz_open(path: &str) -> Result<ZipArchive<std::fs::File>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open {path}: {e}"))?;
    ZipArchive::new(file).map_err(|e| format!("read zip {path}: {e}"))
}

/// Natural-sorted list of page entry names inside a CBZ.
fn cbz_page_names(archive: &mut ZipArchive<std::fs::File>) -> Vec<String> {
    let mut names: Vec<String> = (0..archive.len())
        .filter_map(|i| archive.by_index(i).ok().map(|f| f.name().to_string()))
        .filter(|n| is_page_entry(n))
        .collect();
    names.sort_by(|a, b| natural_cmp(a, b));
    names
}

fn cbz_page_count(path: &str) -> Result<usize, String> {
    let mut archive = cbz_open(path)?;
    Ok(cbz_page_names(&mut archive).len())
}

fn cbz_page_bytes(path: &str, index: usize) -> Result<(String, Vec<u8>), String> {
    let mut archive = cbz_open(path)?;
    let names = cbz_page_names(&mut archive);
    let name = names
        .get(index)
        .ok_or_else(|| format!("page {index} out of range (have {})", names.len()))?
        .clone();

    let mut buf = Vec::new();
    archive
        .by_name(&name)
        .map_err(|e| format!("entry {name}: {e}"))?
        .read_to_end(&mut buf)
        .map_err(|e| format!("read {name}: {e}"))?;

    Ok((mime_for(&name).to_string(), buf))
}

// ---------- CBR (rar) ----------

/// Normalise a RAR entry path to forward slashes for consistent sorting/matching.
fn norm(name: &std::path::Path) -> String {
    name.to_string_lossy().replace('\\', "/")
}

/// Natural-sorted list of page entry names inside a CBR.
fn cbr_page_names(path: &str) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    let archive = unrar::Archive::new(path)
        .open_for_listing()
        .map_err(|e| format!("open rar {path}: {e}"))?;
    for entry in archive {
        let entry = entry.map_err(|e| format!("read rar entry: {e}"))?;
        if entry.is_file() {
            let name = norm(&entry.filename);
            if is_page_entry(&name) {
                names.push(name);
            }
        }
    }
    names.sort_by(|a, b| natural_cmp(a, b));
    Ok(names)
}

fn cbr_page_count(path: &str) -> Result<usize, String> {
    Ok(cbr_page_names(path)?.len())
}

fn cbr_page_bytes(path: &str, index: usize) -> Result<(String, Vec<u8>), String> {
    let names = cbr_page_names(path)?;
    let target = names
        .get(index)
        .ok_or_else(|| format!("page {index} out of range (have {})", names.len()))?
        .clone();

    // RAR extraction is sequential: walk headers, read the one we want, skip the rest.
    let mut archive = unrar::Archive::new(path)
        .open_for_processing()
        .map_err(|e| format!("open rar {path}: {e}"))?;
    while let Some(header) = archive.read_header().map_err(|e| e.to_string())? {
        let name = norm(header.entry().filename.as_path());
        if header.entry().is_file() && name == target {
            let (buf, _rest) = header.read().map_err(|e| format!("read {name}: {e}"))?;
            return Ok((mime_for(&name).to_string(), buf));
        }
        archive = header.skip().map_err(|e| e.to_string())?;
    }
    Err(format!("page {target} not found in archive"))
}

// ---------- Format dispatch (public API) ----------

/// Count the image pages in a comic without decoding them.
pub fn page_count(path: &str) -> Result<usize, String> {
    match ext_of(path).as_str() {
        "cbz" => cbz_page_count(path),
        "cbr" => cbr_page_count(path),
        other => Err(format!("unsupported format: {other}")),
    }
}

/// Raw bytes of one page (mime, bytes). `index` is 0-based into natural order.
pub fn page_bytes(path: &str, index: usize) -> Result<(String, Vec<u8>), String> {
    match ext_of(path).as_str() {
        "cbz" => cbz_page_bytes(path, index),
        "cbr" => cbr_page_bytes(path, index),
        other => Err(format!("unsupported format: {other}")),
    }
}

/// Fetch one page as base64. `index` is 0-based into the natural-sorted pages.
pub fn get_page(path: &str, index: usize) -> Result<PageData, String> {
    let (mime, buf) = page_bytes(path, index)?;
    Ok(PageData {
        mime,
        base64: encode(&buf),
    })
}

/// Downscale raw image bytes to fit `max_dim` on the long edge, re-encoded as
/// JPEG. Returns (jpeg, w, h). Shared by comic and ebook cover generation.
pub fn thumbnail_from_bytes(buf: &[u8], max_dim: u32) -> Result<(Vec<u8>, u32, u32), String> {
    let img = image::load_from_memory(buf).map_err(|e| format!("decode cover: {e}"))?;
    let thumb = img.thumbnail(max_dim, max_dim); // preserves aspect ratio
    let (w, h) = (thumb.width(), thumb.height());

    let mut out = std::io::Cursor::new(Vec::new());
    thumb
        .to_rgb8()
        .write_to(&mut out, image::ImageFormat::Jpeg)
        .map_err(|e| format!("encode cover: {e}"))?;
    Ok((out.into_inner(), w, h))
}

/// A downscaled cover thumbnail for a comic. Decodes page 0.
pub fn cover_thumbnail(path: &str, max_dim: u32) -> Result<(Vec<u8>, u32, u32), String> {
    let (_mime, buf) = page_bytes(path, 0)?;
    thumbnail_from_bytes(&buf, max_dim)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_sort_orders_pages_numerically() {
        let mut v = vec![
            "page10.jpg".to_string(),
            "page2.jpg".to_string(),
            "page1.jpg".to_string(),
        ];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["page1.jpg", "page2.jpg", "page10.jpg"]);
    }

    #[test]
    fn natural_sort_handles_zero_padding() {
        let mut v = vec!["003.png".to_string(), "10.png".to_string(), "2.png".to_string()];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["2.png", "003.png", "10.png"]);
    }

    #[test]
    fn rejects_non_images_and_macos_junk() {
        assert!(is_page_entry("001.jpg"));
        assert!(!is_page_entry("__MACOSX/._001.jpg"));
        assert!(!is_page_entry("folder/"));
        assert!(!is_page_entry("readme.txt"));
    }

    #[test]
    fn mime_matches_extension() {
        assert_eq!(mime_for("a.png"), "image/png");
        assert_eq!(mime_for("a.JPG"), "image/jpeg");
        assert_eq!(mime_for("a.webp"), "image/webp");
    }

    /// End-to-end: build a real CBZ on disk, then read it back through the same
    /// code paths the app uses. Exercises zip parsing, page filtering, natural
    /// ordering, and byte extraction together.
    #[test]
    fn reads_a_real_cbz_end_to_end() {
        use std::io::Write;
        use zip::write::SimpleFileOptions;

        let dir = std::env::temp_dir().join(format!("readaity_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cbz = dir.join("My Comic.cbz");

        {
            let file = std::fs::File::create(&cbz).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = SimpleFileOptions::default();
            // Deliberately out of order + junk entries.
            for (name, body) in [
                ("010.jpg", b"TEN".as_slice()),
                ("002.png", b"TWO".as_slice()),
                ("001.jpg", b"ONE".as_slice()),
                ("notes.txt", b"ignore me".as_slice()),
            ] {
                zip.start_file(name, opts).unwrap();
                zip.write_all(body).unwrap();
            }
            zip.start_file("__MACOSX/._001.jpg", opts).unwrap();
            zip.write_all(b"junk").unwrap();
            zip.finish().unwrap();
        }

        let path = cbz.to_str().unwrap();

        // Only the 3 real images count; txt and macOS junk are excluded.
        assert_eq!(page_count(path).unwrap(), 3);

        // Pages come back in natural order: 001, 002, 010.
        let p0 = get_page(path, 0).unwrap();
        assert_eq!(p0.mime, "image/jpeg");
        assert_eq!(
            String::from_utf8(base64::engine::general_purpose::STANDARD.decode(&p0.base64).unwrap())
                .unwrap(),
            "ONE"
        );

        let p1 = get_page(path, 1).unwrap();
        assert_eq!(p1.mime, "image/png");

        let p2 = get_page(path, 2).unwrap();
        assert_eq!(
            String::from_utf8(base64::engine::general_purpose::STANDARD.decode(&p2.base64).unwrap())
                .unwrap(),
            "TEN"
        );

        // Out-of-range is an error, not a panic.
        assert!(get_page(path, 3).is_err());

        // format detection recognises the extension.
        assert_eq!(
            crate::formats::detect(&cbz, "comics"),
            Some("cbz".to_string())
        );

        std::fs::remove_file(&cbz).ok();
    }
}
