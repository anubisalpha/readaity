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

/// Extract the book's HTML content (decompressed, images inlined as data URIs).
pub fn content(path: &str) -> Result<String, String> {
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
    // Encryption flag in the PalmDOC header (0 = none, 1/2 = DRM). We don't and
    // won't decrypt — surface a clear message instead.
    if u16be(r0, 12).unwrap_or(0) != 0 {
        return Err("This book is DRM-protected — Readaity can only open DRM-free files.".into());
    }
    let comp = u16be(r0, 0).unwrap_or(1);
    if comp == 17480 {
        return Err("This book uses HUFF/CDIC compression, which isn't supported yet.".into());
    }
    let text_recs = u16be(r0, 8).unwrap_or(0) as usize;
    let encoding = u32be(r0, 28).unwrap_or(1252);
    let mhl = u32be(r0, 20).unwrap_or(0) as usize;
    let first_image = u32be(r0, 108).unwrap_or(0) as usize;
    // extra_data_flags is at record-0 offset 242 (0xF2), present when hdr ≥ 0xE4.
    let edf = if mhl >= 0xE4 && r0.len() >= 244 {
        u16be(r0, 242).unwrap_or(0)
    } else {
        0
    };
    let trailers = (edf >> 1).count_ones();
    let multibyte = edf & 1 != 0;

    let last = text_recs.min(offs.len().saturating_sub(2));
    let mut body: Vec<u8> = Vec::new();
    for i in 1..=last {
        let rec = trim(&data[offs[i]..offs[i + 1]], trailers, multibyte);
        if comp == 2 {
            body.extend_from_slice(&palmdoc(rec));
        } else {
            body.extend_from_slice(rec);
        }
    }

    let html = if encoding == 65001 {
        String::from_utf8_lossy(&body).into_owned()
    } else {
        decode_cp1252(&body)
    };
    Ok(inline_images(&data, &offs, first_image, &html))
}
