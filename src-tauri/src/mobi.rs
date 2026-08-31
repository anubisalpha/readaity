//! MOBI / AZW / AZW3 cover extraction (DRM-free files only).
//!
//! MOBI is a PalmDB container. Record 0 holds the PalmDOC + MOBI headers and an
//! optional EXTH block; EXTH record 201 stores the cover's offset into the image
//! records, which begin at the MOBI header's "first image index". We locate that
//! record and hand its bytes (usually JPEG) to the shared thumbnailer.

use std::sync::OnceLock;

use base64::Engine as _;
use regex::Regex;

use crate::comic;

fn u16be(b: &[u8], o: usize) -> Option<u16> {
    b.get(o..o + 2).map(|s| u16::from_be_bytes([s[0], s[1]]))
}
fn u32be(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

/// Extract and thumbnail the cover, or None if there isn't one we can read.
pub fn cover(path: &str, max_dim: u32) -> Result<Option<(Vec<u8>, u32, u32)>, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let Some(nrec) = u16be(&data, 76) else {
        return Ok(None);
    };
    let nrec = nrec as usize;

    // PalmDB record offsets (8 bytes each from offset 78), plus EOF sentinel.
    let mut offs: Vec<usize> = Vec::with_capacity(nrec + 1);
    for i in 0..nrec {
        match u32be(&data, 78 + i * 8) {
            Some(o) => offs.push(o as usize),
            None => return Ok(None),
        }
    }
    offs.push(data.len());
    if offs.len() < 2 {
        return Ok(None);
    }

    let r0 = &data[offs[0].min(data.len())..offs[1].min(data.len())];
    if r0.len() < 132 || &r0[16..20] != b"MOBI" {
        return Ok(None);
    }
    let mobi_hdr_len = u32be(r0, 20).unwrap_or(0) as usize;
    let first_image = u32be(r0, 108).unwrap_or(0xFFFF_FFFF) as usize;
    let exth_flag = u32be(r0, 128).unwrap_or(0);
    if exth_flag & 0x40 == 0 || first_image == 0 || first_image == 0xFFFF_FFFF {
        return Ok(None);
    }

    // EXTH block follows the MOBI header inside record 0.
    let exth_start = 16 + mobi_hdr_len;
    if r0.get(exth_start..exth_start + 4) != Some(b"EXTH") {
        return Ok(None);
    }
    let cnt = u32be(r0, exth_start + 8).unwrap_or(0) as usize;
    let mut p = exth_start + 12;
    let mut cover_off: Option<usize> = None;
    let mut thumb_off: Option<usize> = None;
    for _ in 0..cnt {
        let Some(typ) = u32be(r0, p) else { break };
        let Some(len) = u32be(r0, p + 4) else { break };
        let len = len as usize;
        if len < 8 || p + len > r0.len() {
            break;
        }
        let val = if len >= 12 { u32be(r0, p + 8).map(|v| v as usize) } else { None };
        match typ {
            201 => cover_off = val,
            203 => thumb_off = val,
            _ => {}
        }
        p += len;
    }

    let off = match cover_off.or(thumb_off) {
        Some(o) => o,
        None => return Ok(None),
    };
    let idx = first_image + off;
    if idx + 1 >= offs.len() {
        return Ok(None);
    }
    let img = &data[offs[idx].min(data.len())..offs[idx + 1].min(data.len())];
    if img.len() < 4 {
        return Ok(None);
    }
    Ok(Some(comic::thumbnail_from_bytes(img, max_dim)?))
}

// ---------- Reading: decompress the book's HTML ----------

/// Bounds-safe PalmDOC (LZ77) decompression.
fn palmdoc(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 2);
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        i += 1;
        match b {
            0 => out.push(0),
            0x01..=0x08 => {
                let n = b as usize;
                let end = (i + n).min(data.len());
                out.extend_from_slice(&data[i..end]);
                i = end;
            }
            0x09..=0x7f => out.push(b),
            0x80..=0xbf => {
                if i >= data.len() {
                    break;
                }
                let b2 = data[i];
                i += 1;
                let v = (((b as usize) << 8) | b2 as usize) & 0x3fff;
                let dist = v >> 3;
                let len = (v & 7) + 3;
                if dist == 0 {
                    break;
                }
                for _ in 0..len {
                    if dist > out.len() {
                        break;
                    }
                    out.push(out[out.len() - dist]);
                }
            }
            0xc0..=0xff => {
                out.push(b' ');
                out.push(b ^ 0x80);
            }
        }
    }
    out
}

/// Size of the trailing data entry at the end of a text record (backward varint).
fn trailing_size(data: &[u8]) -> usize {
    let mut num = 0usize;
    let start = data.len().saturating_sub(4);
    for &v in &data[start..] {
        if v & 0x80 != 0 {
            num = 0;
        }
        num = (num << 7) | (v & 0x7f) as usize;
    }
    num
}

/// Strip per-record trailing entries and the multibyte-overlap bytes.
fn trim<'a>(mut rec: &'a [u8], trailers: u32, multibyte: bool) -> &'a [u8] {
    for _ in 0..trailers {
        let n = trailing_size(rec);
        if n == 0 || n > rec.len() {
            break;
        }
        rec = &rec[..rec.len() - n];
    }
    if multibyte && !rec.is_empty() {
        let n = ((rec[rec.len() - 1] & 3) as usize) + 1;
        if n <= rec.len() {
            rec = &rec[..rec.len() - n];
        }
    }
    rec
}

fn sniff_mime(img: &[u8]) -> &'static str {
    if img.starts_with(&[0xFF, 0xD8]) {
        "image/jpeg"
    } else if img.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if img.starts_with(b"GIF") {
        "image/gif"
    } else {
        "image/jpeg"
    }
}

/// Replace MOBI `recindex="N"` image references with inline data URIs.
fn inline_images(data: &[u8], offs: &[usize], first_image: usize, html: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r#"recindex=["']?0*([0-9]+)["']?"#).unwrap());
    re.replace_all(html, |c: &regex::Captures| {
        let n: usize = c[1].parse().unwrap_or(0);
        if n == 0 || first_image == 0 {
            return String::new();
        }
        let rec = first_image + n - 1;
        if rec + 1 < offs.len() {
            let img = &data[offs[rec].min(data.len())..offs[rec + 1].min(data.len())];
            format!(
                r#"src="data:{};base64,{}""#,
                sniff_mime(img),
                base64::engine::general_purpose::STANDARD.encode(img)
            )
        } else {
            String::new()
        }
    })
    .into_owned()
}

/// Decode Windows-1252 bytes to a String (used for non-UTF-8 MOBIs).
fn decode_cp1252(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| match b {
            0x80 => '€', 0x82 => '‚', 0x83 => 'ƒ', 0x84 => '„', 0x85 => '…',
            0x86 => '†', 0x87 => '‡', 0x88 => 'ˆ', 0x89 => '‰', 0x8A => 'Š',
            0x8B => '‹', 0x8C => 'Œ', 0x8E => 'Ž', 0x91 => '\u{2018}',
            0x92 => '\u{2019}', 0x93 => '\u{201C}', 0x94 => '\u{201D}',
            0x95 => '•', 0x96 => '–', 0x97 => '—', 0x98 => '˜', 0x99 => '™',
            0x9A => 'š', 0x9B => '›', 0x9C => 'œ', 0x9E => 'ž', 0x9F => 'Ÿ',
            other => other as char,
        })
        .collect()
}

/// A MOBI file with its text records decompressed into one `text` blob.
struct Loaded {
    data: Vec<u8>,
    offs: Vec<usize>,
    text: Vec<u8>,
    first_image: usize,
    is_kf8: bool,
    encoding: u32,
}

/// Parse the PalmDB container and decompress records `1..text_recs` (PalmDOC,
/// HUFF/CDIC, or stored). Surfaces DRM / unsupported-compression as user errors.
fn load(path: &str) -> Result<Loaded, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let nrec = u16be(&data, 76).ok_or("bad MOBI")? as usize;
    let mut offs: Vec<usize> = Vec::with_capacity(nrec + 1);
    for i in 0..nrec {
        offs.push(u32be(&data, 78 + i * 8).ok_or("bad MOBI")? as usize);
    }
    offs.push(data.len());
    if offs.len() < 2 {
        return Err("empty MOBI".into());
    }
    let r0 = &data[offs[0].min(data.len())..offs[1].min(data.len())];
    if r0.len() < 132 || &r0[16..20] != b"MOBI" {
        return Err("not a MOBI file".into());
    }
    if u16be(r0, 12).unwrap_or(0) != 0 {
        return Err("This book is DRM-protected — Readaity can only open DRM-free files.".into());
    }
    let comp = u16be(r0, 0).unwrap_or(1);
    let is_kf8 = u32be(r0, 36).unwrap_or(6) >= 8;
    let text_recs = u16be(r0, 8).unwrap_or(0) as usize;
    let encoding = u32be(r0, 28).unwrap_or(1252);
    let mhl = u32be(r0, 20).unwrap_or(0) as usize;
    let first_image = u32be(r0, 108).unwrap_or(0) as usize;
    let edf = if mhl >= 0xE4 && r0.len() >= 244 {
        u16be(r0, 242).unwrap_or(0)
    } else {
        0
    };
    let trailers = (edf >> 1).count_ones();
    let multibyte = edf & 1 != 0;

    let mut huff = if comp == 17480 {
        let ho = u32be(r0, 112).unwrap_or(0) as usize;
        let hc = u32be(r0, 116).unwrap_or(0) as usize;
        if ho == 0 || hc == 0 || ho + hc >= offs.len() {
            return Err("This book uses HUFF/CDIC compression but its tables are missing.".into());
        }
        let huff_rec = &data[offs[ho].min(data.len())..offs[ho + 1].min(data.len())];
        let cdics: Vec<&[u8]> = (1..hc)
            .map(|k| &data[offs[ho + k].min(data.len())..offs[ho + k + 1].min(data.len())])
            .collect();
        Some(HuffCdic::new(huff_rec, &cdics)?)
    } else {
        None
    };

    let last = text_recs.min(offs.len().saturating_sub(2));
    let mut text: Vec<u8> = Vec::new();
    for i in 1..=last {
        let rec = trim(&data[offs[i]..offs[i + 1]], trailers, multibyte);
        match comp {
            2 => text.extend_from_slice(&palmdoc(rec)),
            17480 => text.extend_from_slice(&huff.as_mut().unwrap().unpack(rec)),
            _ => text.extend_from_slice(rec),
        }
    }
    Ok(Loaded { data, offs, text, first_image, is_kf8, encoding })
}

/// Extract the book's HTML content (decompressed, images inlined as data URIs).
pub fn content(path: &str) -> Result<String, String> {
    let Loaded { data, offs, text: body, first_image, is_kf8, encoding } = load(path)?;
    let r0 = &data[offs[0].min(data.len())..offs[1].min(data.len())];

    if is_kf8 {
        return kf8::assemble(&body, &data, &offs, r0, first_image);
    }

    let html = if encoding == 65001 {
        String::from_utf8_lossy(&body).into_owned()
    } else {
        decode_cp1252(&body)
    };
    Ok(inline_images(&data, &offs, first_image, &html))
}

/// Layout hints from a MOBI/KF8's EXTH metadata.
#[derive(Default, Debug, Clone)]
pub struct Meta {
    /// EXTH 122 `fixed-layout` == "true" — a page-per-section image book
    /// (comic, manga, picture book) rather than reflowable text.
    pub fixed_layout: bool,
    /// EXTH 123 `book-type` — e.g. "comic", "children".
    pub book_type: Option<String>,
    /// EXTH 126 `original-resolution` — `(width, height)` in px, if present.
    pub original_resolution: Option<(u32, u32)>,
}

/// Read the EXTH layout hints from record 0. Never errors — absent = default.
pub fn meta(path: &str) -> Meta {
    let mut m = Meta::default();
    let Ok(data) = std::fs::read(path) else { return m };
    let Some(nrec) = u16be(&data, 76) else { return m };
    let Some(o0) = u32be(&data, 78).map(|v| v as usize) else { return m };
    let o1 = u32be(&data, 86).map(|v| v as usize).unwrap_or(data.len());
    let r0 = &data[o0.min(data.len())..o1.min(data.len())];
    if r0.len() < 132 || &r0[16..20] != b"MOBI" || nrec == 0 {
        return m;
    }
    let mhl = u32be(r0, 20).unwrap_or(0) as usize;
    let exth = 16 + mhl;
    if r0.get(exth..exth + 4) != Some(b"EXTH") {
        return m;
    }
    let cnt = u32be(r0, exth + 8).unwrap_or(0) as usize;
    let mut p = exth + 12;
    for _ in 0..cnt {
        let Some(typ) = u32be(r0, p) else { break };
        let Some(len) = u32be(r0, p + 4).map(|v| v as usize) else { break };
        if len < 8 || p + len > r0.len() {
            break;
        }
        let val = &r0[p + 8..p + len];
        match typ {
            122 => m.fixed_layout = val.iter().all(|&b| b != 0) && val == b"true",
            123 => {
                m.book_type =
                    Some(String::from_utf8_lossy(val).trim_matches('\0').to_string())
            }
            126 => {
                let s = String::from_utf8_lossy(val);
                if let Some((w, h)) = s.trim().split_once('x') {
                    if let (Ok(w), Ok(h)) = (w.trim().parse(), h.trim().parse()) {
                        m.original_resolution = Some((w, h));
                    }
                }
            }
            _ => {}
        }
        p += len;
    }
    m
}

/// One page of a fixed-layout KF8 book: a self-contained HTML document sized to
/// `w`×`h` CSS px (images inlined, `kindle:` refs resolved). The frontend renders
/// it in a scaled iframe.
#[derive(serde::Serialize, Clone)]
pub struct Kf8Page {
    pub html: String,
    pub w: u32,
    pub h: u32,
}

/// Drop CSS rules whose `#id` / `.class` selector doesn't appear in `body`.
/// Rules with no id/class selector (globals, `@font-face`, `@page`, …) are kept.
/// Cheap and lenient — the point is to stop a book-wide stylesheet dragging
/// every image into every page.
fn prune_css(css: &str, body: &str) -> String {
    static TOK: OnceLock<Regex> = OnceLock::new();
    let tok = TOK.get_or_init(|| Regex::new(r"[#.][A-Za-z_][\w-]*").unwrap());

    let mut out = String::with_capacity(css.len() / 4);
    let mut rest = css;
    while let Some(open) = rest.find('{') {
        // A selector list ends at the previous `}` or start; the block at `}`.
        let sel = rest[..open].trim();
        let Some(close) = rest[open..].find('}') else {
            out.push_str(rest);
            break;
        };
        let block = &rest[open..=open + close];
        // A descendant selector applies only if EVERY `#id` it names is on the
        // page; classes we treat leniently (any match).
        let id_present = |name: &str| {
            body.contains(&format!("id=\"{name}\"")) || body.contains(&format!("id='{name}'"))
        };
        let class_present = |name: &str| {
            body.contains(&format!("\"{name}\""))
                || body.contains(&format!(" {name} "))
                || body.contains(&format!("\"{name} "))
                || body.contains(&format!(" {name}\""))
        };
        let ids: Vec<&str> = tok
            .find_iter(sel)
            .filter(|m| m.as_str().starts_with('#'))
            .map(|m| &m.as_str()[1..])
            .collect();
        let classes: Vec<&str> = tok
            .find_iter(sel)
            .filter(|m| m.as_str().starts_with('.'))
            .map(|m| &m.as_str()[1..])
            .collect();
        let keep = sel.starts_with('@')
            || (ids.is_empty() && classes.is_empty())
            || (!ids.is_empty() && ids.iter().all(|n| id_present(n)))
            || (ids.is_empty() && classes.iter().any(|n| class_present(n)));
        if keep {
            out.push_str(sel);
            out.push_str(block);
            out.push('\n');
        }
        rest = &rest[open + close + 1..];
    }
    out
}

/// `(width, height)` for a fixed-layout section — from its `<meta viewport>` or
/// `<body style="width:…px">`.
fn section_dims(sec: &str) -> Option<(u32, u32)> {
    static VP: OnceLock<Regex> = OnceLock::new();
    static BODY: OnceLock<Regex> = OnceLock::new();
    let vp = VP.get_or_init(|| {
        Regex::new(r#"viewport[^>]*content="[^"]*?width\s*=\s*(\d+)[^"]*?height\s*=\s*(\d+)"#).unwrap()
    });
    let body = BODY.get_or_init(|| {
        Regex::new(r#"<body[^>]*style="[^"]*?width\s*:\s*(\d+)px[^"]*?height\s*:\s*(\d+)px"#).unwrap()
    });
    vp.captures(sec)
        .or_else(|| body.captures(sec))
        .and_then(|c| Some((c[1].parse().ok()?, c[2].parse().ok()?)))
}

/// Every page of a fixed-layout KF8 book, in reading order, each as a sized HTML
/// document. Empty when the book isn't fixed-layout KF8 or can't be reassembled.
pub fn kf8_pages(path: &str) -> Result<Vec<Kf8Page>, String> {
    let Loaded { data, offs, text, first_image, is_kf8, .. } = load(path)?;
    if !is_kf8 {
        return Ok(Vec::new());
    }
    let r0 = &data[offs[0].min(data.len())..offs[1].min(data.len())];
    let Some(book) = kf8::reassemble(&text, &data, &offs, r0) else {
        return Ok(Vec::new());
    };
    let (dw, dh) = meta(path).original_resolution.unwrap_or((1200, 1600));

    static FLOWLINK: OnceLock<Regex> = OnceLock::new();
    let flow_re = FLOWLINK.get_or_init(|| Regex::new(r"kindle:flow:0*([0-9]+)").unwrap());

    let pages = book
        .sections
        .iter()
        .map(|sec| {
            let (w, h) = section_dims(sec).unwrap_or((dw, dh));
            let head = &sec[..sec.find("</head>").unwrap_or(sec.len())];

            // Only the CSS flows this section actually links — inlining every
            // flow into every page blows up on books with per-page stylesheets.
            let linked: Vec<usize> = flow_re
                .captures_iter(head)
                .filter_map(|c| c[1].parse::<usize>().ok())
                .collect();
            // `kindle:flow:0001` is the first non-text flow, i.e. book.flows[0].
            let raw_css: String = if linked.is_empty() {
                book.flows.join("\n")
            } else {
                linked
                    .iter()
                    .filter_map(|&n| book.flows.get(n.checked_sub(1)?))
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n")
            };

            let body = kf8::body_inner(sec);
            // Fixed-layout books share one big stylesheet whose per-page
            // `#fsN-img { background-image: … }` rules each pull in an image.
            // Keep only the rules whose id/class is on this page, so we inline
            // one image per page instead of the whole book on every page.
            let css = prune_css(&raw_css, body);
            let mut html = format!(
                "<!doctype html><html><head><meta charset=\"utf-8\"><style>\n\
                 html,body{{margin:0;padding:0;width:{w}px;height:{h}px;overflow:hidden}}\n\
                 {css}\n</style></head><body>{body}</body></html>"
            );
            html = kf8::inline_kf8_images(&data, &offs, first_image, &html);
            html = kf8::strip_kindle_refs(&html);
            Kf8Page { html, w, h }
        })
        .collect();
    Ok(pages)
}

#[cfg(test)]
mod tests {
    /// Reassemble a real KF8-only file when READAITY_KF8_FILE points at one.
    /// Ignored by default (no fixture is checked in). Run with, e.g.:
    ///   READAITY_KF8_FILE=/path/book.azw3 cargo test kf8_real -- --ignored --nocapture
    #[test]
    #[ignore]
    fn kf8_real() {
        let path = std::env::var("READAITY_KF8_FILE").expect("set READAITY_KF8_FILE");
        let html = super::content(&path).expect("KF8 content");
        assert!(html.contains("<body"), "has a body");
        assert!(
            !html.contains('\u{FFFD}'),
            "no UTF-8 replacement chars in output"
        );
        assert!(
            !html.contains("kindle:embed"),
            "all embedded images inlined"
        );
        assert!(
            html.contains("kf8-section"),
            "sections were assembled"
        );
        eprintln!("KF8 output: {} bytes, {} sections", html.len(), html.matches("kf8-section").count());
    }

    /// Batch-run `content()` over every .azw3/.mobi in READAITY_KF8_DIR and print
    /// a one-line health report per file. Writes each rebuilt HTML next to a
    /// `_out/` dir for eyeballing. Never asserts — it's a survey.
    #[test]
    #[ignore]
    fn kf8_dir() {
        let dir = std::env::var("READAITY_KF8_DIR").expect("set READAITY_KF8_DIR");
        let out = std::path::Path::new(&dir).join("_out");
        let _ = std::fs::create_dir_all(&out);
        let mut entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                matches!(
                    p.extension().and_then(|x| x.to_str()).map(|s| s.to_lowercase()).as_deref(),
                    Some("azw3") | Some("azw") | Some("mobi") | Some("prc")
                )
            })
            .collect();
        entries.sort();
        for p in entries {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            let bytes = std::fs::read(&p).unwrap();
            let ver = super::u32be(
                &bytes[super::u32be(&bytes, 78).unwrap_or(0) as usize + 16..],
                20,
            );
            match super::content(&p.to_string_lossy()) {
                Ok(html) => {
                    let repl = html.matches('\u{FFFD}').count();
                    let secs = html.matches("kf8-section").count();
                    let leftover = html.matches("kindle:embed").count();
                    let imgs = html.matches("data:image").count();
                    eprintln!(
                        "OK   v{:?} {:>8}B sec={:<3} img={:<3} repl={} embedleft={}  {}",
                        ver, html.len(), secs, imgs, repl, leftover, name
                    );
                    let stem = p.file_stem().unwrap().to_string_lossy();
                    let _ = std::fs::write(out.join(format!("{stem}.html")), &html);
                }
                Err(e) => eprintln!("ERR  v{ver:?}  {name}  -> {e}"),
            }
        }
    }

    /// Report fixed-layout detection + page-image extraction over READAITY_KF8_DIR.
    #[test]
    #[ignore]
    fn kf8_pages_dir() {
        let dir = std::env::var("READAITY_KF8_DIR").expect("set READAITY_KF8_DIR");
        let mut entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension().and_then(|x| x.to_str()) == Some("azw3")
            })
            .collect();
        entries.sort();
        for p in entries {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            let m = super::meta(&p.to_string_lossy());
            if !m.fixed_layout {
                continue;
            }
            match super::kf8_pages(&p.to_string_lossy()) {
                Ok(pages) => {
                    let imgs: usize = pages.iter().filter(|p| p.html.contains("data:image")).count();
                    let bytes: usize = pages.iter().map(|p| p.html.len()).sum();
                    let dims: std::collections::BTreeSet<_> =
                        pages.iter().map(|p| (p.w, p.h)).collect();
                    eprintln!(
                        "FL  type={:<9} pages={:<3} with_img={:<3} tot={:>4}KB dims={:?}  {}",
                        m.book_type.as_deref().unwrap_or("-"),
                        pages.len(),
                        imgs,
                        bytes / 1024,
                        dims,
                        name
                    )
                }
                Err(e) => eprintln!("FL  ERR {e}  {name}"),
            }
        }
    }
}

// ---------- HUFF/CDIC decompression ----------
//
// A compression scheme Amazon's kindlegen applies to most commercial MOBI/KF8
// books (nothing to do with DRM). One HUFF record holds the Huffman dispatch
// tables; the CDIC records hold a phrase dictionary. Symbols decode either to a
// literal byte run or to a dictionary phrase that is itself HUFF/CDIC-coded.

struct HuffCdic {
    dict1: Vec<u32>,
    mincode: [u64; 33],
    maxcode: [u64; 33],
    /// (bytes, already-expanded?) — non-terminal phrases are expanded on first use.
    dictionary: Vec<(Vec<u8>, bool)>,
}

impl HuffCdic {
    fn new(huff: &[u8], cdics: &[&[u8]]) -> Result<Self, String> {
        if huff.get(0..4) != Some(b"HUFF") {
            return Err("HUFF/CDIC: bad HUFF record".into());
        }
        let off1 = u32be(huff, 8).ok_or("HUFF/CDIC: short HUFF")? as usize;
        let off2 = u32be(huff, 12).ok_or("HUFF/CDIC: short HUFF")? as usize;
        let dict1: Vec<u32> = (0..256)
            .map(|i| u32be(huff, off1 + i * 4).unwrap_or(0))
            .collect();
        let mut mincode = [0u64; 33];
        let mut maxcode = [0u64; 33];
        for codelen in 1..=32usize {
            let mn = u32be(huff, off2 + (codelen - 1) * 8).unwrap_or(0) as u64;
            let mx = u32be(huff, off2 + (codelen - 1) * 8 + 4).unwrap_or(0) as u64;
            mincode[codelen] = mn << (32 - codelen);
            maxcode[codelen] = ((mx + 1) << (32 - codelen)).wrapping_sub(1);
        }

        let mut dictionary: Vec<(Vec<u8>, bool)> = Vec::new();
        for cdic in cdics {
            if cdic.get(0..4) != Some(b"CDIC") {
                return Err("HUFF/CDIC: bad CDIC record".into());
            }
            let phrases = u32be(cdic, 8).unwrap_or(0) as usize;
            let bits = u32be(cdic, 12).unwrap_or(0) as usize;
            let n = (1usize << bits).min(phrases.saturating_sub(dictionary.len()));
            for j in 0..n {
                let off = match u16be(cdic, 16 + j * 2) {
                    Some(o) => o as usize,
                    None => break,
                };
                let blen = u16be(cdic, 16 + off).unwrap_or(0) as usize;
                let slen = blen & 0x7fff;
                let term = blen & 0x8000 != 0;
                let s = cdic
                    .get(16 + off + 2..16 + off + 2 + slen)
                    .unwrap_or_default()
                    .to_vec();
                dictionary.push((s, term));
            }
        }
        Ok(Self { dict1, mincode, maxcode, dictionary })
    }

    fn phrase(&mut self, r: usize) -> Vec<u8> {
        let (bytes, expanded) = &self.dictionary[r];
        if *expanded {
            return bytes.clone();
        }
        let coded = bytes.clone();
        let out = self.unpack(&coded);
        self.dictionary[r] = (out.clone(), true);
        out
    }

    fn unpack(&mut self, data: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(data.len() + 8);
        buf.extend_from_slice(data);
        buf.extend_from_slice(&[0u8; 8]);
        let total_bits = data.len() * 8;
        let mut out = Vec::new();
        let mut pos = 0usize;
        while pos < total_bits {
            let bo = pos >> 3;
            let mut x = u64::from_be_bytes(buf[bo..bo + 8].try_into().unwrap());
            x <<= (pos & 7) as u32;
            let code = x >> 32; // top 32 bits, left-aligned
            let v = self.dict1[(code >> 24) as usize];
            let mut codelen = (v & 0x1f) as usize;
            let term = v & 0x80 != 0;
            let mut mc = if codelen != 0 {
                (((v >> 8) as u64 + 1) << (32 - codelen)).wrapping_sub(1)
            } else {
                0
            };
            if !term {
                if codelen == 0 {
                    codelen = 1;
                }
                while codelen < 33 && code < self.mincode[codelen] {
                    codelen += 1;
                }
                if codelen >= 33 {
                    break;
                }
                mc = self.maxcode[codelen];
            }
            pos += codelen;
            if pos > total_bits {
                break;
            }
            let r = ((mc.wrapping_sub(code)) >> (32 - codelen)) as usize;
            if r >= self.dictionary.len() {
                break;
            }
            out.extend_from_slice(&self.phrase(r));
        }
        out
    }
}

// ---------- KF8 (MOBI-8) reassembly ----------

mod kf8 {
    use super::{sniff_mime, u32be};
    use base64::Engine as _;
    use std::collections::BTreeMap;
    use std::sync::OnceLock;

    use regex::Regex;

    /// One INDX entry: its name plus the decoded values for each TAGX tag.
    type Entry = (String, BTreeMap<u8, Vec<u32>>);

    /// Forward MOBI varint: bytes are big-endian 7-bit groups, the byte with the
    /// high bit set terminates. Returns (value, bytes_consumed).
    fn varint(b: &[u8], mut p: usize) -> (u32, usize) {
        let mut v: u32 = 0;
        loop {
            let Some(&byte) = b.get(p) else { return (v, p) };
            p += 1;
            v = (v << 7) | (byte & 0x7f) as u32;
            if byte & 0x80 != 0 {
                return (v, p);
            }
        }
    }

    /// Parse an INDX (index) block group starting at record `idx`: a header
    /// record, `count` data records, then CNCX records we don't need here.
    fn parse_indx(data: &[u8], offs: &[usize], idx: usize) -> Result<Vec<Entry>, String> {
        let rec = |i: usize| -> &[u8] {
            match (offs.get(i), offs.get(i + 1)) {
                (Some(&a), Some(&b)) => &data[a.min(data.len())..b.min(data.len())],
                _ => &[],
            }
        };
        let hdr = rec(idx);
        if hdr.get(0..4) != Some(b"INDX") {
            return Err("KF8: bad INDX header".into());
        }
        let nblocks = u32be(hdr, 0x18).unwrap_or(0) as usize;

        // TAGX table: tag, values-per-entry, bitmask, end-flag (4 bytes each).
        let tagx_off = hdr
            .windows(4)
            .position(|w| w == b"TAGX")
            .ok_or("KF8: no TAGX")?;
        let tagx = &hdr[tagx_off..];
        let tagx_len = u32be(tagx, 4).unwrap_or(0) as usize;
        let ncontrol = u32be(tagx, 8).unwrap_or(1).max(1) as usize;
        let mut tags: Vec<(u8, u8, u8, u8)> = Vec::new();
        let mut i = 12;
        while i + 4 <= tagx_len.min(tagx.len()) {
            tags.push((tagx[i], tagx[i + 1], tagx[i + 2], tagx[i + 3]));
            i += 4;
        }

        let mut out: Vec<Entry> = Vec::new();
        for blk in 0..nblocks {
            let db = rec(idx + 1 + blk);
            if db.get(0..4) != Some(b"INDX") {
                return Err("KF8: bad INDX data block".into());
            }
            let idxt_pos = u32be(db, 0x14).unwrap_or(0) as usize;
            let nentries = u32be(db, 0x18).unwrap_or(0) as usize;

            // IDXT: "IDXT" marker then one u16 offset per entry.
            let mut ends: Vec<usize> = Vec::with_capacity(nentries + 1);
            for e in 0..nentries {
                let o = idxt_pos + 4 + e * 2;
                let Some(s) = db.get(o..o + 2) else { break };
                ends.push(u16::from_be_bytes([s[0], s[1]]) as usize);
            }
            ends.push(idxt_pos);

            for w in ends.windows(2) {
                let (start, end) = (w[0], w[1].min(db.len()));
                if start >= end {
                    continue;
                }
                let entry = &db[start..end];
                let nlen = entry[0] as usize;
                if 1 + nlen > entry.len() {
                    continue;
                }
                let name = String::from_utf8_lossy(&entry[1..1 + nlen]).into_owned();
                let mut p = 1 + nlen;
                let control = &entry[p..(p + ncontrol).min(entry.len())];
                p += ncontrol;
                let cbyte = control.first().copied().unwrap_or(0);

                // How many values each present tag carries.
                let mut plan: Vec<(u8, usize)> = Vec::new();
                let mut cb_idx = 0usize;
                for &(tag, nvals, mask, endflag) in &tags {
                    if endflag & 1 != 0 {
                        cb_idx += 1;
                        continue;
                    }
                    let byte = control.get(cb_idx).copied().unwrap_or(cbyte);
                    let mut value = (byte & mask) as u32;
                    if value == 0 {
                        continue;
                    }
                    let count = if value == mask as u32 {
                        if (mask as u32).count_ones() > 1 {
                            let (c, np) = varint(entry, p);
                            p = np;
                            c as usize
                        } else {
                            1
                        }
                    } else {
                        let mut m = mask;
                        while m & 1 == 0 {
                            m >>= 1;
                            value >>= 1;
                        }
                        value as usize
                    };
                    plan.push((tag, count * nvals as usize));
                }

                let mut vals: BTreeMap<u8, Vec<u32>> = BTreeMap::new();
                for (tag, total) in plan {
                    let mut got = Vec::with_capacity(total);
                    for _ in 0..total {
                        let (v, np) = varint(entry, p);
                        p = np;
                        got.push(v);
                    }
                    vals.entry(tag).or_default().extend(got);
                }
                out.push((name, vals));
            }
        }
        Ok(out)
    }

    /// A reassembled KF8 book: each section's full XHTML, plus the non-text flows
    /// (CSS / SVG) as UTF-8-lossy strings.
    pub struct Book {
        pub sections: Vec<String>,
        pub flows: Vec<String>,
    }

    /// Splice text fragments into their XHTML skeletons. `None` when the
    /// skeleton/fragment tables are missing or unusable.
    pub fn reassemble(raw: &[u8], data: &[u8], offs: &[usize], r0: &[u8]) -> Option<Book> {
        let nrec = offs.len().saturating_sub(1);
        let rec = |i: usize| -> &[u8] {
            if i >= nrec {
                return &[];
            }
            &data[offs[i].min(data.len())..offs[i + 1].min(data.len())]
        };
        let opt_rec = |v: Option<u32>| -> usize {
            match v {
                Some(n) if (n as usize) < nrec => n as usize,
                _ => usize::MAX,
            }
        };

        // FDST splits `raw` into flows: flow 0 = skeleton+fragment text, the rest
        // are CSS / SVG referenced as `kindle:flow:NNNN`.
        let fdst = rec(opt_rec(u32be(r0, 0xC0)));
        let mut flow_spans: Vec<(usize, usize)> = Vec::new();
        if fdst.get(0..4) == Some(b"FDST") {
            let n_flows = u32be(fdst, 8).unwrap_or(0) as usize;
            for k in 0..n_flows {
                let a = u32be(fdst, 12 + k * 8).unwrap_or(0) as usize;
                let b = u32be(fdst, 16 + k * 8).unwrap_or(0) as usize;
                flow_spans.push((a.min(raw.len()), b.min(raw.len())));
            }
        }
        if flow_spans.is_empty() {
            flow_spans.push((0, raw.len()));
        }
        let text0 = &raw[flow_spans[0].0..flow_spans[0].1];
        let flows: Vec<String> = flow_spans
            .iter()
            .skip(1)
            .map(|&(a, b)| String::from_utf8_lossy(&raw[a..b]).into_owned())
            .collect();

        let skel_idx = opt_rec(u32be(r0, 0xFC));
        let frag_idx = opt_rec(u32be(r0, 0xF8));
        if skel_idx == usize::MAX || frag_idx == usize::MAX {
            return None;
        }
        let (skels, frags) = match (
            parse_indx(data, offs, skel_idx),
            parse_indx(data, offs, frag_idx),
        ) {
            (Ok(s), Ok(f)) if !s.is_empty() => (s, f),
            _ => return None,
        };

        let mut sections = Vec::with_capacity(skels.len());
        let mut fp = 0usize;
        for (_, sv) in &skels {
            let nchunks = sv.get(&1).and_then(|v| v.first()).copied().unwrap_or(0) as usize;
            let geom = sv.get(&6).cloned().unwrap_or_default();
            let (sstart, slen) = (
                *geom.first().unwrap_or(&0) as usize,
                *geom.get(1).unwrap_or(&0) as usize,
            );
            let mut file: Vec<u8> = text0
                .get(sstart..(sstart + slen).min(text0.len()))
                .unwrap_or_default()
                .to_vec();
            let mut base = sstart + slen;
            for _ in 0..nchunks {
                let Some((cname, cv)) = frags.get(fp) else { break };
                fp += 1;
                let insert = cname.parse::<usize>().unwrap_or(base);
                let clen = cv.get(&6).and_then(|v| v.get(1)).copied().unwrap_or(0) as usize;
                let chunk = text0.get(base..(base + clen).min(text0.len())).unwrap_or_default();
                base += clen;
                let at = insert.saturating_sub(sstart).min(file.len());
                file.splice(at..at, chunk.iter().copied());
            }
            sections.push(String::from_utf8_lossy(&file).into_owned());
        }
        Some(Book { sections, flows })
    }

    /// Rebuild a KF8 book into one HTML document: concatenate every section's
    /// body, inline the CSS flows and the embedded images.
    pub fn assemble(
        raw: &[u8],
        data: &[u8],
        offs: &[usize],
        r0: &[u8],
        first_image: usize,
    ) -> Result<String, String> {
        let Some(book) = reassemble(raw, data, offs, r0) else {
            // No usable tables — flow 0 of a single-file KF8 book is already XHTML.
            let text0 = flow0(raw, data, offs, r0);
            let html = String::from_utf8_lossy(&text0);
            let mut doc = format!(
                "<!doctype html><html><head><meta charset=\"utf-8\"></head><body>\n<div class=\"kf8-section\">{}</div>\n</body></html>",
                body_inner(&html)
            );
            doc = inline_kf8_images(data, offs, first_image, &doc);
            return Ok(strip_kindle_refs(&doc));
        };

        let mut sections = String::new();
        let mut titles: Vec<Option<String>> = Vec::with_capacity(book.sections.len());
        for (i, s) in book.sections.iter().enumerate() {
            titles.push(section_title(s));
            sections.push_str(&format!("<div class=\"kf8-section\" id=\"kf8-s{i}\">"));
            sections.push_str(body_inner(s));
            sections.push_str("</div>\n");
        }

        // A flat chapter list from the sections' <title>s. Drop generic
        // boilerplate ("Book Title", "Cover", …) then collapse consecutive runs
        // of the same title (a multi-chapter novel titled with the book name).
        const JUNK: &[&str] = &[
            "book title", "cover", "title", "[title]", "titlepage", "title page",
            "copyright", "contents", "table of contents", "toc", "untitled",
        ];
        let mut toc: Vec<(usize, &str)> = Vec::new();
        for (i, t) in titles.iter().enumerate() {
            let Some(t) = t else { continue };
            if JUNK.contains(&t.to_lowercase().trim()) {
                continue;
            }
            if toc.last().map(|(_, p)| *p != t.as_str()).unwrap_or(true) {
                toc.push((i, t.as_str()));
            }
        }
        let nav = if toc.len() > 1 {
            let items: String = toc
                .iter()
                .map(|(i, t)| format!("<a href=\"#kf8-s{i}\">{}</a>", esc(t)))
                .collect();
            format!("<nav id=\"kf8-toc\" hidden>{items}</nav>\n")
        } else {
            String::new()
        };

        let css = book.flows.join("\n");
        let mut doc = format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><style>\n{css}\n</style></head><body>\n{nav}{sections}</body></html>"
        );
        doc = inline_kf8_images(data, offs, first_image, &doc);
        doc = strip_kindle_refs(&doc);
        Ok(doc)
    }

    /// Trimmed text of a section's `<title>`, if it has meaningful content.
    fn section_title(sec: &str) -> Option<String> {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| Regex::new(r"(?is)<title[^>]*>(.*?)</title>").unwrap());
        let raw = re.captures(sec)?.get(1)?.as_str();
        let t = raw
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&#160;", " ")
            .replace('\u{00a0}', " ");
        let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
        (!t.is_empty() && t.len() < 200).then_some(t)
    }

    fn esc(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    /// Just flow 0 (the skeleton+fragment text stream).
    fn flow0(raw: &[u8], data: &[u8], offs: &[usize], r0: &[u8]) -> Vec<u8> {
        let nrec = offs.len().saturating_sub(1);
        let fdst_no = u32be(r0, 0xC0).map(|n| n as usize).unwrap_or(usize::MAX);
        if fdst_no < nrec {
            let fdst = &data[offs[fdst_no].min(data.len())..offs[fdst_no + 1].min(data.len())];
            if fdst.get(0..4) == Some(b"FDST") {
                let a = u32be(fdst, 12).unwrap_or(0) as usize;
                let b = u32be(fdst, 16).unwrap_or(raw.len() as u32) as usize;
                return raw[a.min(raw.len())..b.min(raw.len())].to_vec();
            }
        }
        raw.to_vec()
    }

    /// Inner HTML of a `<body>…</body>`, or the whole string if there's no body.
    pub fn body_inner(html: &str) -> &str {
        let Some(open) = html.find("<body") else { return html };
        let Some(gt) = html[open..].find('>') else { return html };
        let start = open + gt + 1;
        match html[start..].rfind("</body>") {
            Some(end) => &html[start..start + end],
            None => &html[start..],
        }
    }

    /// `kindle:embed:NNNN?mime=…` → data URI from the image record.
    pub fn inline_kf8_images(data: &[u8], offs: &[usize], first_image: usize, html: &str) -> String {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| {
            Regex::new(r#"kindle:embed:0*([0-9A-Za-z]+)(\?mime=[^"'\s)]*)?"#).unwrap()
        });
        re.replace_all(html, |c: &regex::Captures| {
            // The id is base-32 in some files but plain decimal in practice here.
            let n = u32::from_str_radix(&c[1], 32).or_else(|_| c[1].parse::<u32>()).unwrap_or(0) as usize;
            if n == 0 || first_image == 0 {
                return String::new();
            }
            let r = first_image + n - 1;
            if r + 1 >= offs.len() {
                return String::new();
            }
            let img = &data[offs[r].min(data.len())..offs[r + 1].min(data.len())];
            format!(
                "data:{};base64,{}",
                sniff_mime(img),
                base64::engine::general_purpose::STANDARD.encode(img)
            )
        })
        .into_owned()
    }

    /// Neutralise leftover `kindle:` URIs (internal position links, flow links)
    /// so they don't render as broken links in the single-document reader.
    pub fn strip_kindle_refs(html: &str) -> String {
        static LINK: OnceLock<Regex> = OnceLock::new();
        static HREF: OnceLock<Regex> = OnceLock::new();
        let link = LINK.get_or_init(|| Regex::new(r#"<link[^>]*kindle:[^>]*>"#).unwrap());
        let href = HREF.get_or_init(|| Regex::new(r#"(href|src)="kindle:[^"]*""#).unwrap());
        let s = link.replace_all(html, "");
        href.replace_all(&s, r##"$1="#""##).into_owned()
    }
}
