//! Which file extensions belong to which library, and format classification.

use std::path::Path;

/// Comic archive formats (the "comics" library).
pub const COMIC_EXTS: &[&str] = &["cbz", "cbr"];
/// Ebook formats (the "ebooks" library). DRM-free only — DRM'd Kindle files
/// (encrypted AZW/KFX) are not, and cannot be, supported.
///   * mobi/prc/azw/azw3 → the MOBI engine
///   * epub, pdf         → their own engines
///   * txt               → plain text
pub const EBOOK_EXTS: &[&str] =
    &["epub", "pdf", "mobi", "prc", "azw", "azw3", "txt", "rtf", "lrf"];

/// If `path` belongs to `library`, return its lower-cased format tag.
pub fn detect(path: &Path, library: &str) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let allowed: &[&str] = if library == "ebooks" {
        EBOOK_EXTS
    } else {
        COMIC_EXTS
    };
    allowed.contains(&ext.as_str()).then_some(ext)
}

pub fn is_comic(format: &str) -> bool {
    COMIC_EXTS.contains(&format)
}
