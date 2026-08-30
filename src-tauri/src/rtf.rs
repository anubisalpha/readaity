//! Pragmatic RTF → HTML conversion for the reader.
//!
//! Not a full RTF implementation — it extracts readable text with paragraph and
//! line structure, decodes `\'hh` (Windows-1252) and `\uN` unicode escapes, and
//! skips control/destination groups (font tables, colour tables, stylesheets,
//! pictures, etc). Fine styling is intentionally dropped for reliability.

fn cp1252(v: u8) -> char {
    match v {
        0x80 => '€', 0x82 => '‚', 0x83 => 'ƒ', 0x84 => '„', 0x85 => '…',
        0x86 => '†', 0x87 => '‡', 0x88 => 'ˆ', 0x89 => '‰', 0x8A => 'Š',
        0x8B => '‹', 0x8C => 'Œ', 0x8E => 'Ž', 0x91 => '\u{2018}',
        0x92 => '\u{2019}', 0x93 => '\u{201C}', 0x94 => '\u{201D}',
        0x95 => '•', 0x96 => '–', 0x97 => '—', 0x99 => '™', 0x9C => 'œ',
        other => other as char,
    }
}

fn push_escaped(out: &mut String, c: char) {
    match c {
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '&' => out.push_str("&amp;"),
        _ => out.push(c),
    }
}

/// Destination groups whose contents should not be rendered.
fn is_skip_dest(word: &str) -> bool {
    matches!(
        word,
        "fonttbl" | "colortbl" | "stylesheet" | "info" | "pict" | "header"
            | "footer" | "footnote" | "annotation" | "generator" | "themedata"
            | "colorschememapping" | "latentstyles" | "datastore" | "listtable"
            | "listoverridetable" | "revtbl" | "xmlnstbl" | "rsidtbl"
    )
}

pub fn to_html(path: &str) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    // RTF is 7-bit ASCII with \'hh escapes for high bytes.
    let s: Vec<u8> = bytes;
    let n = s.len();
    let mut i = 0;
    let mut out = String::from("<div><p>");

    // Per-group "skip this destination" flags.
    let mut skip_stack: Vec<bool> = Vec::new();
    let mut skipping = false;

    while i < n {
        match s[i] {
            b'{' => {
                skip_stack.push(skipping);
                i += 1;
            }
            b'}' => {
                skipping = skip_stack.pop().unwrap_or(false);
                i += 1;
            }
            b'\\' => {
                i += 1;
                if i >= n {
                    break;
                }
                let c = s[i];
                if c == b'\'' {
                    if i + 2 < n {
                        let hex = std::str::from_utf8(&s[i + 1..i + 3]).unwrap_or("");
                        if let Ok(v) = u8::from_str_radix(hex, 16) {
                            if !skipping {
                                push_escaped(&mut out, cp1252(v));
                            }
                        }
                        i += 3;
                    } else {
                        i += 1;
                    }
                } else if c.is_ascii_alphabetic() {
                    let start = i;
                    while i < n && s[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    let word = std::str::from_utf8(&s[start..i]).unwrap_or("");
                    let pstart = i;
                    if i < n && (s[i] == b'-' || s[i].is_ascii_digit()) {
                        if s[i] == b'-' {
                            i += 1;
                        }
                        while i < n && s[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                    let param: Option<i32> = if i > pstart {
                        std::str::from_utf8(&s[pstart..i]).ok().and_then(|x| x.parse().ok())
                    } else {
                        None
                    };
                    if i < n && s[i] == b' ' {
                        i += 1; // control-word delimiter space
                    }

                    if is_skip_dest(word) {
                        skipping = true;
                    } else if !skipping {
                        match word {
                            "par" | "pard" => out.push_str("</p><p>"),
                            "line" => out.push_str("<br>"),
                            "tab" => out.push_str("&emsp;"),
                            "emdash" => out.push('—'),
                            "endash" => out.push('–'),
                            "lquote" => out.push('\u{2018}'),
                            "rquote" => out.push('\u{2019}'),
                            "ldblquote" => out.push('\u{201C}'),
                            "rdblquote" => out.push('\u{201D}'),
                            "bullet" => out.push('•'),
                            "u" => {
                                if let Some(cp) = param {
                                    if let Some(ch) = char::from_u32(cp.rem_euclid(0x10000) as u32) {
                                        push_escaped(&mut out, ch);
                                    }
                                    // Skip the following fallback char (uc1 default).
                                    if i < n && s[i] != b'\\' && s[i] != b'{' && s[i] != b'}' {
                                        i += 1;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                } else {
                    // Control symbol.
                    match c {
                        b'\\' | b'{' | b'}' => {
                            if !skipping {
                                push_escaped(&mut out, c as char);
                            }
                        }
                        b'~' => {
                            if !skipping {
                                out.push('\u{00A0}');
                            }
                        }
                        b'*' => skipping = true, // \* → ignorable destination
                        _ => {}
                    }
                    i += 1;
                }
            }
            b'\r' | b'\n' => i += 1, // raw newlines are not content in RTF
            other => {
                if !skipping {
                    push_escaped(&mut out, other as char);
                }
                i += 1;
            }
        }
    }

    out.push_str("</p></div>");
    Ok(out)
}
