//! Ebook metadata + cover extraction.
//!
//! * EPUB — a zip; we read the OPF manifest to find the cover image and count
//!   the spine (used as a rough "sections" figure). No new deps.
//! * PDF  — page count via `lopdf`. Its cover is rendered lazily in the
//!   frontend with pdf.js (no native renderer needed) and cached.
//! * MOBI/AZW3 — discovered and listed; richer metadata/reader comes later.

use std::io::Read;
use std::sync::OnceLock;

use regex::Regex;
use zip::ZipArchive;

use crate::comic;

/// Strip XML namespace prefixes from tag names (`<opf:item>` → `<item>`) so the
/// simple string parsing below works on namespaced OPFs (common in Simon &
/// Schuster / Adobe-produced EPUBs). Only touches tag names, not attributes.
fn strip_ns(opf: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(</?)[A-Za-z_][\w.\-]*:").unwrap());
    re.replace_all(opf, "$1").into_owned()
}

fn open_zip(path: &str) -> Result<ZipArchive<std::fs::File>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open {path}: {e}"))?;
    ZipArchive::new(file).map_err(|e| format!("read epub {path}: {e}"))
}

fn read_entry(zip: &mut ZipArchive<std::fs::File>, name: &str) -> Result<Vec<u8>, String> {
    let mut f = zip
        .by_name(name)
        .map_err(|e| format!("entry {name}: {e}"))?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

fn read_entry_str(zip: &mut ZipArchive<std::fs::File>, name: &str) -> Result<String, String> {
    Ok(String::from_utf8_lossy(&read_entry(zip, name)?).into_owned())
}

/// Value of an XML attribute like `attr="value"` (first occurrence).
fn find_attr(s: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=\"");
    let i = s.find(&needle)? + needle.len();
    let rest = &s[i..];
    let j = rest.find('"')?;
    Some(rest[..j].to_string())
}

fn opf_path(zip: &mut ZipArchive<std::fs::File>) -> Result<String, String> {
    let container = read_entry_str(zip, "META-INF/container.xml")?;
    find_attr(&container, "full-path").ok_or_else(|| "no rootfile in container.xml".into())
}

/// Resolve a manifest href (relative to the OPF dir) to a zip entry path.
fn resolve(opf_path: &str, href: &str) -> String {
    let dir = opf_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let combined = if dir.is_empty() {
        href.to_string()
    } else {
        format!("{dir}/{href}")
    };
    // Normalise ./ and ../ and decode %20.
    let mut out: Vec<&str> = Vec::new();
    for seg in combined.split('/') {
        match seg {
            "." | "" => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    out.join("/").replace("%20", " ")
}

/// Find the cover image href in an OPF: `properties="cover-image"`, else a
/// `<meta name="cover" content="ID">` pointing at a manifest item id.
fn cover_href(opf: &str) -> Option<String> {
    // Manifest <item> tag bodies (skip <itemref> in the spine).
    let items: Vec<&str> = opf
        .split("<item")
        .skip(1)
        .filter(|frag| !frag.starts_with("ref"))
        .map(|frag| &frag[..frag.find('>').unwrap_or(frag.len())])
        .collect();

    // 1) EPUB3: item with properties="cover-image".
    for seg in &items {
        if seg.contains("cover-image") {
            if let Some(href) = find_attr(seg, "href") {
                return Some(href);
            }
        }
    }

    // 2) EPUB2: a <meta name="cover" content="ID"> (attributes in any order) →
    //    the manifest item whose id == ID.
    let cover_id = opf
        .split("<meta")
        .skip(1)
        .map(|frag| &frag[..frag.find('>').unwrap_or(frag.len())])
        .find(|seg| find_attr(seg, "name").as_deref() == Some("cover"))
        .and_then(|seg| find_attr(seg, "content"));
    if let Some(id) = cover_id {
        for seg in &items {
            if find_attr(seg, "id").as_deref() == Some(id.as_str()) {
                if let Some(href) = find_attr(seg, "href") {
                    return Some(href);
                }
            }
        }
    }

    // 3) Fallback: any image item whose id or href hints "cover" — but not the
    //    "buy other books" / thumbnail / logo images some publishers bundle.
    for seg in &items {
        let is_image = seg.contains("image/");
        let href = find_attr(seg, "href").unwrap_or_default().to_lowercase();
        let id = find_attr(seg, "id").unwrap_or_default().to_lowercase();
        let junk = ["buylink", "thumb", "logo", "brand", "ad_"]
            .iter()
            .any(|j| href.contains(j) || id.contains(j));
        let hints = id.contains("cover") || href.contains("cover");
        if is_image && hints && !junk {
            if let Some(href) = find_attr(seg, "href") {
                return Some(href);
            }
        }
    }
    None
}

/// `<reference type="cover" href="X">` from the OPF guide.
fn guide_cover_href(opf: &str) -> Option<String> {
    for f in opf.split("<reference").skip(1) {
        let seg = &f[..f.find('>').unwrap_or(f.len())];
        if find_attr(seg, "type").as_deref() == Some("cover") {
            if let Some(h) = find_attr(seg, "href") {
                return Some(h);
            }
        }
    }
    None
}

/// href of the first spine item (often the cover page).
fn first_spine_href(opf: &str) -> Option<String> {
    let idref = opf
        .split("<itemref")
        .nth(1)
        .and_then(|f| find_attr(&f[..f.find('>').unwrap_or(f.len())], "idref"))?;
    for f in opf.split("<item").skip(1) {
        if f.starts_with("ref") {
            continue;
        }
        let seg = &f[..f.find('>').unwrap_or(f.len())];
        if find_attr(seg, "id").as_deref() == Some(idref.as_str()) {
            return find_attr(seg, "href");
        }
    }
    None
}

/// First cover-ish image src in an xhtml document (prefers a doc-cover image).
fn image_src_in_html(html: &str) -> Option<String> {
    let mut first: Option<String> = None;
    for tag in ["<img", "<image"] {
        for f in html.split(tag).skip(1) {
            let seg = &f[..f.find('>').unwrap_or(f.len())];
            let src = find_attr(seg, "src")
                .or_else(|| find_attr(seg, "xlink:href"))
                .or_else(|| find_attr(seg, "href"));
            if let Some(s) = src {
                if seg.contains("doc-cover") || seg.contains("cover-image") {
                    return Some(s);
                }
                if first.is_none() {
                    first = Some(s);
                }
            }
        }
    }
    first
}

/// Read an xhtml entry and return the resolved zip path of its cover image.
fn image_in_html_entry(zip: &mut ZipArchive<std::fs::File>, entry: &str) -> Option<String> {
    let html = strip_ns(&read_entry_str(zip, entry).ok()?);
    let src = image_src_in_html(&html)?;
    Some(resolve(entry, &src))
}

/// Resolve the cover image's zip entry via layered strategies.
fn epub_cover_entry(
    zip: &mut ZipArchive<std::fs::File>,
    opf_p: &str,
    opf: &str,
) -> Option<String> {
    // 1) Declared in the OPF manifest/metadata.
    if let Some(href) = cover_href(opf) {
        let e = resolve(opf_p, &href);
        if e.to_lowercase().ends_with("html") {
            if let Some(img) = image_in_html_entry(zip, &e) {
                return Some(img);
            }
        } else {
            return Some(e);
        }
    }
    // 2) Guide reference → its cover page → the image inside.
    if let Some(href) = guide_cover_href(opf) {
        let e = resolve(opf_p, &href);
        if let Some(img) = image_in_html_entry(zip, &e) {
            return Some(img);
        }
        if !e.to_lowercase().ends_with("html") {
            return Some(e);
        }
    }
    // 3) First spine item is often the cover page.
    if let Some(href) = first_spine_href(opf) {
        let e = resolve(opf_p, &href);
        if let Some(img) = image_in_html_entry(zip, &e) {
            return Some(img);
        }
    }
    None
}

fn epub_spine_count(path: &str) -> Result<i64, String> {
    let mut zip = open_zip(path)?;
    let opf_p = opf_path(&mut zip)?;
    let opf = strip_ns(&read_entry_str(&mut zip, &opf_p)?);
    Ok(opf.matches("<itemref").count() as i64)
}

fn epub_cover(path: &str, max_dim: u32) -> Result<Option<(Vec<u8>, u32, u32)>, String> {
    let mut zip = open_zip(path)?;
    let opf_p = opf_path(&mut zip)?;
    let opf = strip_ns(&read_entry_str(&mut zip, &opf_p)?);
    let Some(entry) = epub_cover_entry(&mut zip, &opf_p, &opf) else {
        return Ok(None);
    };
    let bytes = match read_entry(&mut zip, &entry) {
        Ok(b) => b,
        Err(_) => return Ok(None),
    };
    Ok(Some(comic::thumbnail_from_bytes(&bytes, max_dim)?))
}

fn pdf_page_count(path: &str) -> Result<i64, String> {
    let doc = lopdf::Document::load(path).map_err(|e| format!("load pdf: {e}"))?;
    Ok(doc.get_pages().len() as i64)
}

// ---------- Public dispatch by format ----------

/// Page/section count for an ebook, best-effort (0 when unknown).
pub fn page_count(path: &str, format: &str) -> Result<i64, String> {
    match format {
        "epub" => epub_spine_count(path),
        "pdf" => pdf_page_count(path),
        _ => Ok(0), // mobi/azw3: unknown until the reader phase
    }
}

/// A cover thumbnail, or None if we can't extract one in the backend.
/// (PDF covers are produced in the frontend via pdf.js.)
pub fn cover(path: &str, format: &str, max_dim: u32) -> Result<Option<(Vec<u8>, u32, u32)>, String> {
    match format {
        "epub" => epub_cover(path, max_dim),
        "mobi" | "prc" | "azw" | "azw3" => crate::mobi::cover(path, max_dim),
        _ => Ok(None),
    }
}
